mod cursor;
mod scan;
mod table;
mod types;

use uuid::Uuid;

use crate::module::schema::core::domain::{
    diagnostic_code as code, Attribute, Cardinality, Diagnostic, Entity, ForeignKeyRef,
    Relationship, Report, SchemaDraft,
};
use crate::package::errdef::Error;

use self::cursor::Cursor;
use self::scan::{scan, LayoutHint, Statement};
use self::table::{ParsedForeignKey, ParsedTable};

use super::grid_slot;

/// How much of a statement goes into the warning. Enough to recognise it,
/// short enough that a wall of them still reads.
const SOURCE_LIMIT: usize = 120;

pub(super) fn read(content: &str, include_positions: bool) -> Result<(SchemaDraft, Report), Error> {
    let (statements, hints) = scan(content);
    let mut report = Report::default();
    let mut tables = Vec::new();
    let mut table_at = Vec::new();
    let mut comments = Vec::new();
    let mut late_keys = Vec::new();

    for (at, statement) in statements.iter().enumerate() {
        match classify(statement) {
            Read::Table(parsed) => {
                table_at.push(at);
                tables.push(parsed);
            }
            Read::ForeignKey(parsed) => late_keys.push(parsed),
            Read::Comment(comment) => comments.push(comment),
            Read::Skipped => report.push(skipped(&statement.source)),
        }
    }

    if tables.is_empty() {
        return Err(
            Error::validation("the file holds no table the canvas could draw")
                .add_violation("content", "expected at least one CREATE TABLE statement"),
        );
    }

    let mut draft = SchemaDraft {
        name: String::new(),
        description: String::new(),
        entities: Vec::new(),
        relationships: Vec::new(),
    };

    for parsed in &tables {
        for note in &parsed.skipped {
            report.push(Diagnostic::warning(
                code::IMPORT_SKIPPED,
                format!("`{}` kept, without {}", parsed.name, note),
            ));
        }

        draft.entities.push(entity_from(parsed));
    }

    for parsed in tables.iter().flat_map(|parsed| {
        parsed
            .foreign_keys
            .iter()
            .map(move |key| (parsed.name.as_str(), key))
    }) {
        link(&mut draft, parsed.0, parsed.1, &mut report);
    }

    for boxed in late_keys {
        let (holder, key) = *boxed;

        link(&mut draft, &holder, &key, &mut report);
    }

    for comment in comments {
        describe(&mut draft, comment, &mut report);
    }

    place(&mut draft, &hints, &table_at, include_positions);

    Ok((draft, report))
}

enum Read {
    Table(Box<ParsedTable>),
    ForeignKey(Box<(String, ParsedForeignKey)>),
    Comment(Comment),
    Skipped,
}

struct Comment {
    table: String,
    column: Option<String>,
    text: String,
}

fn classify(statement: &Statement) -> Read {
    let first = statement.tokens.first();

    if first.is_some_and(|token| token.is_word("create")) {
        return match table::parse(&statement.tokens) {
            Some(parsed) => Read::Table(Box::new(parsed)),
            None => Read::Skipped,
        };
    }

    if first.is_some_and(|token| token.is_word("alter")) {
        return match alter(&statement.tokens) {
            Some(parsed) => Read::ForeignKey(Box::new(parsed)),
            None => Read::Skipped,
        };
    }

    if first.is_some_and(|token| token.is_word("comment")) {
        return match comment(&statement.tokens) {
            Some(parsed) => Read::Comment(parsed),
            None => Read::Skipped,
        };
    }

    Read::Skipped
}

/// `ALTER TABLE x ADD CONSTRAINT c FOREIGN KEY (a) REFERENCES y (b)`, which is
/// how the generator writes every reference and how most dumps do.
fn alter(tokens: &[scan::Token]) -> Option<(String, ParsedForeignKey)> {
    let mut cursor = Cursor::new(tokens);

    cursor.eat_word("alter");

    if !cursor.eat_word("table") {
        return None;
    }

    cursor.eat_words(&["if", "exists"]);
    cursor.eat_word("only");

    let holder = cursor.qualified_name()?;

    if !cursor.eat_word("add") {
        return None;
    }

    if cursor.eat_word("constraint") {
        cursor.next();
    }

    if !cursor.eat_words(&["foreign", "key"]) {
        return None;
    }

    let columns = cursor.name_list()?;

    if !cursor.eat_word("references") {
        return None;
    }

    let table = cursor.qualified_name()?;
    let targets = cursor.name_list();

    if columns.len() != 1 {
        return None;
    }

    Some((
        holder,
        ParsedForeignKey {
            column: columns[0].clone(),
            table,
            target: targets.and_then(|names| names.first().cloned()),
        },
    ))
}

fn comment(tokens: &[scan::Token]) -> Option<Comment> {
    let mut cursor = Cursor::new(tokens);

    cursor.eat_word("comment");

    if !cursor.eat_word("on") {
        return None;
    }

    let on_column = if cursor.eat_word("column") {
        true
    } else if cursor.eat_word("table") {
        false
    } else {
        return None;
    };

    let name = cursor.qualified_name()?;

    if !cursor.eat_word("is") {
        return None;
    }

    let text = match cursor.next()? {
        scan::Token::Text(text) => text.clone(),
        // `COMMENT ON TABLE x IS NULL` clears a comment, which here means
        // there is nothing to carry onto the canvas.
        _ => return None,
    };

    let (table, column) = match on_column {
        true => {
            let (table, column) = name.rsplit_once('.')?;
            (table.to_owned(), Some(column.to_owned()))
        }
        false => (name, None),
    };

    Some(Comment {
        table,
        column,
        text,
    })
}

fn entity_from(parsed: &ParsedTable) -> Entity {
    Entity::new(fresh("e"), &parsed.name).with_attributes(
        parsed
            .columns
            .iter()
            .map(|column| {
                let mut attribute = Attribute::new(fresh("a"), &column.name, column.data_type);

                attribute.nullable = column.nullable;
                attribute.primary_key = column.primary_key;
                // The generator leaves UNIQUE off a key column because the
                // key already says it, so the flag is put back rather than
                // read off a script that had no reason to carry it.
                attribute.unique = column.unique || column.primary_key;
                attribute.default_value = column.default_value.clone();

                attribute
            })
            .collect(),
    )
}

fn link(draft: &mut SchemaDraft, holder: &str, key: &ParsedForeignKey, report: &mut Report) {
    let note = format!(
        "`{}.{}` is not a foreign key, because it points at `{}`",
        holder, key.column, key.table
    );

    let (Some(holder_index), Some(target_index)) = (find(draft, holder), find(draft, &key.table))
    else {
        report.push(Diagnostic::warning(code::IMPORT_SKIPPED, note));
        return;
    };

    let Some(holder_attribute) = attribute_id(&draft.entities[holder_index], &key.column) else {
        report.push(Diagnostic::warning(code::IMPORT_SKIPPED, note));
        return;
    };

    let target_attribute = match &key.target {
        Some(name) => attribute_id(&draft.entities[target_index], name),
        // `REFERENCES other` with no column means the other table's key.
        None => draft.entities[target_index]
            .primary_key()
            .first()
            .map(|attribute| attribute.id.clone()),
    };

    let Some(target_attribute) = target_attribute else {
        report.push(Diagnostic::warning(code::IMPORT_SKIPPED, note));
        return;
    };

    let (holder_entity, target_entity) = (
        draft.entities[holder_index].id.clone(),
        draft.entities[target_index].id.clone(),
    );

    if let Some(attribute) = draft.entities[holder_index]
        .attributes
        .iter_mut()
        .find(|attribute| attribute.id == holder_attribute)
    {
        attribute.foreign_key = Some(ForeignKeyRef::new(&target_entity, &target_attribute));
    }

    // The canvas draws the line from the key it points at to the column that
    // holds it, and the generator never reads the direction back, so the one
    // side leads here too.
    draft.relationships.push(Relationship::new(
        fresh("r"),
        (&target_entity, &target_attribute),
        (&holder_entity, &holder_attribute),
        Cardinality::OneToMany,
    ));
}

fn describe(draft: &mut SchemaDraft, comment: Comment, report: &mut Report) {
    let Some(index) = find(draft, &comment.table) else {
        report.push(Diagnostic::warning(
            code::IMPORT_SKIPPED,
            format!("a comment on `{}`, which is not in the file", comment.table),
        ));
        return;
    };

    let entity = &mut draft.entities[index];

    let Some(column) = comment.column else {
        entity.description = comment.text;
        return;
    };

    match entity
        .attributes
        .iter_mut()
        .find(|attribute| attribute.name.eq_ignore_ascii_case(&column))
    {
        Some(attribute) => attribute.description = comment.text,
        None => report.push(Diagnostic::warning(
            code::IMPORT_SKIPPED,
            format!(
                "a comment on `{}.{column}`, which is not in the file",
                comment.table
            ),
        )),
    }
}

/// A line written directly above the table beats one from the block older
/// files wrote at the foot of the script, so a file carrying both opens the
/// way its author last arranged it.
fn place(
    draft: &mut SchemaDraft,
    hints: &[LayoutHint],
    table_at: &[usize],
    include_positions: bool,
) {
    for (index, entity) in draft.entities.iter_mut().enumerate() {
        let hint = include_positions
            .then(|| {
                let named = |hint: &&LayoutHint| same_table(&hint.table, &entity.name);

                hints
                    .iter()
                    .find(|hint| hint.above == table_at.get(index).copied() && named(hint))
                    .or_else(|| hints.iter().find(named))
            })
            .flatten();

        entity.position = match hint {
            Some(hint) => crate::module::schema::core::domain::Position::new(hint.x, hint.y),
            None => grid_slot(index),
        };
    }
}

/// A script is free to write `users`, `"users"` or `public.users` for the
/// same table, so the schema is only compared when both sides carry one.
fn same_table(left: &str, right: &str) -> bool {
    if left.eq_ignore_ascii_case(right) {
        return true;
    }

    let tail = |name: &str| name.rsplit('.').next().unwrap_or(name).to_owned();

    tail(left).eq_ignore_ascii_case(&tail(right))
}

fn find(draft: &SchemaDraft, name: &str) -> Option<usize> {
    draft
        .entities
        .iter()
        .position(|entity| entity.name.eq_ignore_ascii_case(name))
        .or_else(|| {
            draft
                .entities
                .iter()
                .position(|entity| same_table(&entity.name, name))
        })
}

fn attribute_id(entity: &Entity, name: &str) -> Option<String> {
    entity
        .attribute_by_name(name)
        .map(|attribute| attribute.id.clone())
}

fn fresh(prefix: &str) -> String {
    format!("{prefix}{}", Uuid::new_v4().simple())
}

fn skipped(source: &str) -> Diagnostic {
    let single_line = source.split_whitespace().collect::<Vec<_>>().join(" ");

    let shortened = match single_line.chars().count() > SOURCE_LIMIT {
        true => format!(
            "{}...",
            single_line.chars().take(SOURCE_LIMIT).collect::<String>()
        ),
        false => single_line,
    };

    Diagnostic::warning(
        code::IMPORT_SKIPPED,
        format!("skipped a statement the reader does not handle: `{shortened}`"),
    )
}
