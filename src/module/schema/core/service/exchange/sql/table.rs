use crate::module::schema::core::domain::{DataType, DataTypeKind};

use super::cursor::{render, Cursor};
use super::scan::Token;
use super::types::{is_serial, kind_of};

#[derive(Debug, Clone)]
pub(super) struct ParsedTable {
    pub(super) name: String,
    pub(super) columns: Vec<ParsedColumn>,
    pub(super) foreign_keys: Vec<ParsedForeignKey>,
    pub(super) skipped: Vec<String>,
}

#[derive(Debug, Clone)]
pub(super) struct ParsedColumn {
    pub(super) name: String,
    pub(super) data_type: DataType,
    pub(super) nullable: bool,
    pub(super) primary_key: bool,
    pub(super) unique: bool,
    pub(super) default_value: Option<String>,
}

#[derive(Debug, Clone)]
pub(super) struct ParsedForeignKey {
    pub(super) column: String,
    pub(super) table: String,
    /// Absent when the script wrote `REFERENCES other` with no column, which
    /// PostgreSQL reads as the other table's primary key.
    pub(super) target: Option<String>,
}

/// The words that can only be a constraint, so collecting a type name knows
/// where to stop.
const AFTER_TYPE: [&str; 10] = [
    "not",
    "null",
    "primary",
    "unique",
    "default",
    "references",
    "check",
    "constraint",
    "collate",
    "generated",
];

pub(super) fn parse(tokens: &[Token]) -> Option<ParsedTable> {
    let mut cursor = Cursor::new(tokens);

    if !cursor.eat_word("create") {
        return None;
    }

    // `UNLOGGED`, `TEMP` and friends change nothing about the shape.
    while !cursor.at_word("table") && !cursor.done() {
        cursor.next();
    }

    if !cursor.eat_word("table") {
        return None;
    }

    cursor.eat_words(&["if", "not", "exists"]);

    let name = cursor.qualified_name()?;
    let mut table = ParsedTable {
        name,
        columns: Vec::new(),
        foreign_keys: Vec::new(),
        skipped: Vec::new(),
    };

    if !cursor.eat_punct('(') {
        // `CREATE TABLE x ()` and `CREATE TABLE x AS ...` both land here.
        return Some(table);
    }

    while !cursor.done() && !cursor.at_punct(')') {
        item(&mut cursor, &mut table);

        if !cursor.eat_punct(',') {
            break;
        }
    }

    Some(table)
}

fn item(cursor: &mut Cursor<'_>, table: &mut ParsedTable) {
    let named_constraint = cursor.eat_word("constraint");

    if named_constraint {
        cursor.next();
    }

    if cursor.eat_words(&["primary", "key"]) {
        match cursor.name_list() {
            Some(names) => mark_primary(table, &names),
            None => table
                .skipped
                .push("a primary key it could not read".to_owned()),
        }

        return;
    }

    if cursor.eat_words(&["foreign", "key"]) {
        read_foreign_key(cursor, table);
        return;
    }

    if cursor.eat_word("unique") {
        match cursor.name_list().as_deref() {
            Some([name]) => mark_unique(table, name),
            // One column at a time is all the model can say, so a key across
            // several of them is dropped rather than half applied.
            _ => table
                .skipped
                .push("a unique constraint across several columns".to_owned()),
        }

        return;
    }

    if cursor.at_word("check") || cursor.at_word("exclude") {
        cursor.next();
        cursor.skip_balanced();
        table.skipped.push("a check constraint".to_owned());
        return;
    }

    if named_constraint {
        let rest = cursor.take_until_separator(&[]);
        table
            .skipped
            .push(format!("the constraint `{}`", render(&rest)));
        return;
    }

    match column(cursor) {
        Ok(mut parsed) => {
            if inline_reference(cursor, &parsed.name, table) {
                constraints(cursor, &mut parsed);
            }

            table.columns.push(parsed);
        }
        Err(name) => {
            let rest = cursor.take_until_separator(&[]);
            let written = render(&rest);

            table.skipped.push(match written.is_empty() {
                true => format!("the column `{name}`"),
                false => format!("the column `{name} {written}`"),
            });
        }
    }
}

fn read_foreign_key(cursor: &mut Cursor<'_>, table: &mut ParsedTable) {
    let Some(columns) = cursor.name_list() else {
        table
            .skipped
            .push("a foreign key it could not read".to_owned());
        return;
    };

    if !cursor.eat_word("references") {
        table
            .skipped
            .push("a foreign key with no target".to_owned());
        return;
    }

    let Some(target_table) = cursor.qualified_name() else {
        table
            .skipped
            .push("a foreign key with no target".to_owned());
        return;
    };

    let targets = cursor.name_list();

    // A composite foreign key is several columns pointing together, and the
    // model holds one column per reference, so it is dropped whole.
    if columns.len() != 1 {
        table
            .skipped
            .push("a foreign key across several columns".to_owned());
        return;
    }

    table.foreign_keys.push(ParsedForeignKey {
        column: columns[0].clone(),
        table: target_table,
        target: targets.and_then(|names| names.first().cloned()),
    });

    cursor.take_until_separator(&[]);
}

/// The error is the column's own name, so a type nobody knows can be
/// reported against the column that has it rather than the whole table.
fn column(cursor: &mut Cursor<'_>) -> Result<ParsedColumn, String> {
    let name = cursor
        .peek()
        .and_then(Token::name)
        .ok_or_else(String::new)?
        .to_owned();

    cursor.next();

    let type_name = type_name(cursor);
    let kind = kind_of(&type_name).ok_or_else(|| match type_name.is_empty() {
        true => name.clone(),
        false => format!("{name} {type_name}"),
    })?;
    let (length, precision, scale) = parameters(cursor, kind);

    let mut parsed = ParsedColumn {
        name,
        data_type: DataType {
            kind,
            length,
            precision,
            scale,
        },
        nullable: !is_serial(&type_name),
        primary_key: false,
        unique: false,
        default_value: None,
    };

    constraints(cursor, &mut parsed);

    Ok(parsed)
}

fn type_name(cursor: &mut Cursor<'_>) -> String {
    let mut words = Vec::new();

    while let Some(Token::Word(word)) = cursor.peek() {
        if AFTER_TYPE.contains(&word.as_str()) {
            break;
        }

        words.push(word.clone());
        cursor.next();
    }

    words.join(" ")
}

fn parameters(
    cursor: &mut Cursor<'_>,
    kind: DataTypeKind,
) -> (Option<u32>, Option<u32>, Option<u32>) {
    let mut numbers: Vec<u32> = Vec::new();

    if cursor.eat_punct('(') {
        while let Some(Token::Number(number)) = cursor.peek() {
            if let Ok(parsed) = number.parse() {
                numbers.push(parsed);
            }

            cursor.next();

            if !cursor.eat_punct(',') {
                break;
            }
        }

        cursor.eat_punct(')');
    }

    // Trailing words such as `with time zone` belong to the type, and the
    // name was already settled before the parameters, so they are dropped.
    while let Some(Token::Word(word)) = cursor.peek() {
        if AFTER_TYPE.contains(&word.as_str()) {
            break;
        }

        cursor.next();
    }

    if kind.takes_length() {
        return (numbers.first().copied(), None, None);
    }

    if kind.takes_precision() {
        return (None, numbers.first().copied(), numbers.get(1).copied());
    }

    (None, None, None)
}

fn constraints(cursor: &mut Cursor<'_>, column: &mut ParsedColumn) {
    while !cursor.done() && !cursor.at_punct(',') && !cursor.at_punct(')') {
        if cursor.eat_words(&["not", "null"]) {
            column.nullable = false;
            continue;
        }

        if cursor.eat_words(&["primary", "key"]) {
            column.primary_key = true;
            column.nullable = false;
            continue;
        }

        if cursor.eat_word("unique") {
            column.unique = true;
            continue;
        }

        if cursor.eat_word("null") {
            column.nullable = true;
            continue;
        }

        if cursor.eat_word("default") {
            let expression = cursor.take_until_separator(&AFTER_TYPE);

            if !expression.is_empty() {
                column.default_value = Some(render(&expression));
            }

            continue;
        }

        if cursor.eat_word("constraint") {
            cursor.next();
            continue;
        }

        if cursor.eat_word("check") {
            cursor.skip_balanced();
            continue;
        }

        // `REFERENCES` is read by the caller, which can see the other
        // tables, so it is left in place here.
        if cursor.at_word("references") {
            return;
        }

        cursor.next();
    }
}

/// The inline `REFERENCES` on a column, read once the column itself is known.
pub(super) fn inline_reference(
    cursor: &mut Cursor<'_>,
    column: &str,
    table: &mut ParsedTable,
) -> bool {
    if !cursor.eat_word("references") {
        return false;
    }

    let Some(target_table) = cursor.qualified_name() else {
        table.skipped.push("a reference with no target".to_owned());
        return true;
    };

    let targets = cursor.name_list();

    table.foreign_keys.push(ParsedForeignKey {
        column: column.to_owned(),
        table: target_table,
        target: targets.and_then(|names| names.first().cloned()),
    });

    true
}

fn mark_primary(table: &mut ParsedTable, names: &[String]) {
    for name in names {
        if let Some(column) = find(table, name) {
            column.primary_key = true;
            column.nullable = false;
        }
    }
}

fn mark_unique(table: &mut ParsedTable, name: &str) {
    if let Some(column) = find(table, name) {
        column.unique = true;
    }
}

fn find<'a>(table: &'a mut ParsedTable, name: &str) -> Option<&'a mut ParsedColumn> {
    table
        .columns
        .iter_mut()
        .find(|column| column.name.eq_ignore_ascii_case(name))
}
