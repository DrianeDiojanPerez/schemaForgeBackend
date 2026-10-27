use crate::module::schema::core::domain::{
    Attribute, Category, Comparison, Difference, Entity, Relationship, SchemaDraft,
};

/// How much of `expected` came back in `actual`. Names are what the two
/// schemas have in common, because a round trip through SQL mints fresh ids,
/// so matching is by name throughout, case-insensitively as the verifier
/// treats them.
pub fn compare(
    expected: &SchemaDraft,
    actual: &SchemaDraft,
    include_positions: bool,
) -> Comparison {
    let mut comparison = Comparison::default();

    for (left, right) in entities(expected, actual, &mut comparison) {
        entity(left, right, include_positions, &mut comparison);
    }

    foreign_keys(expected, actual, &mut comparison);
    relationships(expected, actual, &mut comparison);

    comparison
}

fn entities<'a>(
    expected: &'a SchemaDraft,
    actual: &'a SchemaDraft,
    comparison: &mut Comparison,
) -> Vec<(&'a Entity, &'a Entity)> {
    let mut taken = vec![false; actual.entities.len()];
    let mut pairs = Vec::new();

    for left in &expected.entities {
        let found = actual
            .entities
            .iter()
            .enumerate()
            .find(|(at, right)| !taken[*at] && same(&right.name, &left.name));

        match found {
            Some((at, right)) => {
                taken[at] = true;
                comparison.counts_mut(Category::Tables).matched += 1;
                pairs.push((left, right));
            }
            None => comparison.record(
                Category::Tables,
                Difference::missing(table_path(&left.name), &left.name),
            ),
        }
    }

    for (_, right) in actual
        .entities
        .iter()
        .enumerate()
        .filter(|(at, _)| !taken[*at])
    {
        comparison.record(
            Category::Tables,
            Difference::extra(table_path(&right.name), &right.name),
        );
    }

    pairs
}

fn entity(
    expected: &Entity,
    actual: &Entity,
    include_positions: bool,
    comparison: &mut Comparison,
) {
    let path = table_path(&expected.name);

    description(
        &path,
        &expected.description,
        &actual.description,
        comparison,
    );

    if include_positions {
        position(&path, expected, actual, comparison);
    }

    let mut taken = vec![false; actual.attributes.len()];

    for left in &expected.attributes {
        let found = actual
            .attributes
            .iter()
            .enumerate()
            .find(|(at, right)| !taken[*at] && same(&right.name, &left.name));

        match found {
            Some((at, right)) => {
                taken[at] = true;
                attribute(&path, left, right, comparison);
            }
            None => comparison.record(
                Category::Columns,
                Difference::missing(column_path(&path, &left.name), &left.name),
            ),
        }
    }

    for (_, right) in actual
        .attributes
        .iter()
        .enumerate()
        .filter(|(at, _)| !taken[*at])
    {
        comparison.record(
            Category::Columns,
            Difference::extra(column_path(&path, &right.name), &right.name),
        );
    }
}

fn attribute(table: &str, expected: &Attribute, actual: &Attribute, comparison: &mut Comparison) {
    let path = column_path(table, &expected.name);

    settle(
        comparison,
        Category::Columns,
        [
            field(&path, "dataType", &expected.data_type, &actual.data_type),
            field(&path, "nullable", &expected.nullable, &actual.nullable),
            field(
                &path,
                "default",
                &optional(&expected.default_value),
                &optional(&actual.default_value),
            ),
        ],
    );

    keys(&path, expected, actual, comparison);
    description(
        &path,
        &expected.description,
        &actual.description,
        comparison,
    );
}

/// A key column is counted only when one of the two schemas says it is one,
/// so a table of plain columns does not score a key it never had.
fn keys(path: &str, expected: &Attribute, actual: &Attribute, comparison: &mut Comparison) {
    let flagged = |attribute: &Attribute| attribute.primary_key || attribute.unique;
    let spell = |attribute: &Attribute| match (attribute.primary_key, attribute.unique) {
        (true, true) => "primary key, unique".to_owned(),
        (true, false) => "primary key".to_owned(),
        (false, true) => "unique".to_owned(),
        (false, false) => String::new(),
    };

    match (flagged(expected), flagged(actual)) {
        (false, false) => {}
        (true, false) => comparison.record(
            Category::Keys,
            Difference::missing(path.to_owned(), spell(expected)),
        ),
        (false, true) => comparison.record(
            Category::Keys,
            Difference::extra(path.to_owned(), spell(actual)),
        ),
        (true, true) => settle(
            comparison,
            Category::Keys,
            [
                field(
                    path,
                    "primaryKey",
                    &expected.primary_key,
                    &actual.primary_key,
                ),
                field(path, "unique", &expected.unique, &actual.unique),
            ],
        ),
    }
}

/// Only descriptions somebody wrote are counted. Two blank ones agreeing is
/// not documentation that survived.
fn description(path: &str, expected: &str, actual: &str, comparison: &mut Comparison) {
    let path = format!("{path}.description");
    let (expected, actual) = (expected.trim(), actual.trim());

    match (expected.is_empty(), actual.is_empty()) {
        (true, true) => {}
        (false, true) => {
            comparison.record(Category::Descriptions, Difference::missing(path, expected))
        }
        (true, false) => comparison.record(Category::Descriptions, Difference::extra(path, actual)),
        (false, false) if expected == actual => {
            comparison.counts_mut(Category::Descriptions).matched += 1
        }
        (false, false) => comparison.record(
            Category::Descriptions,
            Difference::changed(path, expected, actual),
        ),
    }
}

fn position(path: &str, expected: &Entity, actual: &Entity, comparison: &mut Comparison) {
    let spell = |entity: &Entity| format!("{}, {}", entity.position.x, entity.position.y);

    match expected.position == actual.position {
        true => comparison.counts_mut(Category::Positions).matched += 1,
        false => comparison.record(
            Category::Positions,
            Difference::changed(format!("{path}.position"), spell(expected), spell(actual)),
        ),
    }
}

fn foreign_keys(expected: &SchemaDraft, actual: &SchemaDraft, comparison: &mut Comparison) {
    let left = references(expected);
    let right = references(actual);

    for (holder, target) in &left {
        let path = format!("{holder}.foreignKey");

        match right.iter().find(|(name, _)| same(name, holder)) {
            None => comparison.record(Category::ForeignKeys, Difference::missing(path, target)),
            Some((_, found)) if same(found, target) => {
                comparison.counts_mut(Category::ForeignKeys).matched += 1
            }
            Some((_, found)) => comparison.record(
                Category::ForeignKeys,
                Difference::changed(path, target, found),
            ),
        }
    }

    for (holder, target) in &right {
        if !left.iter().any(|(name, _)| same(name, holder)) {
            comparison.record(
                Category::ForeignKeys,
                Difference::extra(format!("{holder}.foreignKey"), target),
            );
        }
    }
}

fn relationships(expected: &SchemaDraft, actual: &SchemaDraft, comparison: &mut Comparison) {
    let right = links(actual);
    let mut taken = vec![false; right.len()];

    for (link, ends) in expected.relationships.iter().zip(links(expected)) {
        let path = format!("relationships[{ends}]");
        let found = right
            .iter()
            .enumerate()
            .find(|(at, other)| !taken[*at] && same(other, &ends));

        let Some((at, _)) = found else {
            comparison.record(
                Category::Relationships,
                Difference::missing(path, link.cardinality.as_str()),
            );
            continue;
        };

        taken[at] = true;

        let other = &actual.relationships[at];

        match other.cardinality == link.cardinality {
            true => comparison.counts_mut(Category::Relationships).matched += 1,
            false => comparison.record(
                Category::Relationships,
                Difference::changed(
                    path.as_str(),
                    link.cardinality.as_str(),
                    other.cardinality.as_str(),
                ),
            ),
        }

        description(&path, &link.description, &other.description, comparison);
    }

    for (at, ends) in right.iter().enumerate().filter(|(at, _)| !taken[*at]) {
        comparison.record(
            Category::Relationships,
            Difference::extra(
                format!("relationships[{ends}]"),
                actual.relationships[at].cardinality.as_str(),
            ),
        );
    }
}

/// Every foreign key as `table.column` pointing at `table.column`, which is
/// what the two schemas can be compared on once the ids have been reminted.
fn references(draft: &SchemaDraft) -> Vec<(String, String)> {
    let mut found = Vec::new();

    for entity in &draft.entities {
        for attribute in &entity.attributes {
            let Some(reference) = &attribute.foreign_key else {
                continue;
            };

            found.push((
                format!("{}.{}", entity.name, attribute.name),
                point_at(draft, &reference.entity_id, &reference.attribute_id),
            ));
        }
    }

    found
}

/// Both ends of every relationship as `table.column -> table.column`, in
/// the order the schema holds them, so a link can be found again by the
/// columns it joins.
fn links(draft: &SchemaDraft) -> Vec<String> {
    draft
        .relationships
        .iter()
        .map(|link: &Relationship| {
            format!(
                "{} -> {}",
                point_at(draft, &link.from_entity_id, &link.from_attribute_id),
                point_at(draft, &link.to_entity_id, &link.to_attribute_id)
            )
        })
        .collect()
}

/// An end written as `table.column`. An end nothing resolves is a broken
/// schema rather than a comparison failure, so the ids are written through
/// and compared as they stand.
fn point_at(draft: &SchemaDraft, entity_id: &str, attribute_id: &str) -> String {
    let Some(entity) = draft.entities.iter().find(|entity| entity.id == entity_id) else {
        return format!("{entity_id}.{attribute_id}");
    };

    match entity.attribute(attribute_id) {
        Some(attribute) => format!("{}.{}", entity.name, attribute.name),
        None => format!("{}.{attribute_id}", entity.name),
    }
}

/// One count for the thing, however many of its fields moved.
fn settle<const N: usize>(
    comparison: &mut Comparison,
    category: Category,
    fields: [Option<Difference>; N],
) {
    let found: Vec<Difference> = fields.into_iter().flatten().collect();

    match found.is_empty() {
        true => comparison.counts_mut(category).matched += 1,
        false => {
            comparison.counts_mut(category).changed += 1;
            comparison.differences.extend(found);
        }
    }
}

fn field<T: ToString + PartialEq>(
    path: &str,
    name: &str,
    expected: &T,
    actual: &T,
) -> Option<Difference> {
    if expected == actual {
        return None;
    }

    Some(Difference::changed(
        format!("{path}.{name}"),
        expected.to_string(),
        actual.to_string(),
    ))
}

fn optional(value: &Option<String>) -> String {
    value.clone().unwrap_or_default()
}

fn same(left: &str, right: &str) -> bool {
    left.eq_ignore_ascii_case(right)
}

fn table_path(name: &str) -> String {
    format!("entities[{name}]")
}

fn column_path(table: &str, name: &str) -> String {
    format!("{table}.attributes[{name}]")
}

#[cfg(test)]
mod tests {
    use crate::module::schema::core::domain::{
        Cardinality, Counts, DataType, DataTypeKind, DifferenceKind, ForeignKeyRef, Position,
    };

    use super::*;

    fn key(id: &str, name: &str) -> Attribute {
        Attribute::new(id, name, DataType::simple(DataTypeKind::Uuid))
            .as_primary_key()
            .unique()
    }

    fn column(id: &str, name: &str) -> Attribute {
        Attribute::new(id, name, DataType::varchar(255))
    }

    /// Two tables, one reference, one relationship: enough shape for every
    /// category to have something in it.
    fn drawn() -> SchemaDraft {
        let users = Entity::new("e1", "users")
            .with_description("Everyone signed up")
            .with_position(Position::new(40.0, 80.0))
            .with_attributes(vec![key("a1", "id"), column("a2", "email")]);

        let posts = Entity::new("e2", "posts")
            .with_position(Position::new(400.0, 80.0))
            .with_attributes(vec![
                key("a3", "id"),
                Attribute::new("a4", "author_id", DataType::simple(DataTypeKind::Uuid))
                    .required()
                    .referencing(ForeignKeyRef::new("e1", "a1")),
            ]);

        SchemaDraft {
            name: "blog".to_owned(),
            description: String::new(),
            entities: vec![users, posts],
            relationships: vec![Relationship::new(
                "r1",
                ("e1", "a1"),
                ("e2", "a4"),
                Cardinality::OneToMany,
            )],
        }
    }

    fn paths(comparison: &Comparison, kind: DifferenceKind) -> Vec<&str> {
        comparison
            .differences
            .iter()
            .filter(|difference| difference.kind == kind)
            .map(|difference| difference.path.as_str())
            .collect()
    }

    #[test]
    fn a_schema_compared_with_itself_is_equal() {
        let comparison = compare(&drawn(), &drawn(), true);

        assert!(comparison.is_equal(), "{:?}", comparison.differences);
        assert_eq!(comparison.tables.matched, 2);
        assert_eq!(comparison.columns.matched, 4);
        assert_eq!(comparison.keys.matched, 2);
        assert_eq!(comparison.foreign_keys.matched, 1);
        assert_eq!(comparison.relationships.matched, 1);
        assert_eq!(comparison.descriptions.matched, 1);
        assert_eq!(comparison.positions.matched, 2);
    }

    #[test]
    fn the_same_schema_under_a_different_case_still_matches() {
        let mut shouted = drawn();
        shouted.entities[0].name = "USERS".to_owned();
        shouted.entities[0].attributes[1].name = "EMAIL".to_owned();

        assert!(compare(&drawn(), &shouted, true).is_equal());
    }

    #[test]
    fn a_table_that_did_not_come_back_is_missing_and_named() {
        let mut half = drawn();
        half.entities.remove(1);
        half.relationships.clear();

        let comparison = compare(&drawn(), &half, true);

        assert!(!comparison.is_equal());
        assert_eq!(comparison.tables.missing, 1);
        assert_eq!(comparison.tables.matched, 1);
        assert_eq!(
            paths(&comparison, DifferenceKind::Missing)[0],
            "entities[posts]"
        );
        assert_eq!(comparison.tables.recall(), 0.5);
        assert_eq!(comparison.tables.precision(), 1.0);
    }

    #[test]
    fn a_table_nobody_asked_for_is_extra() {
        let mut grown = drawn();
        grown
            .entities
            .push(Entity::new("e3", "audit").with_attributes(vec![key("a5", "id")]));

        let comparison = compare(&drawn(), &grown, true);

        assert_eq!(comparison.tables.extra, 1);
        assert_eq!(comparison.tables.recall(), 1.0);
        assert!(comparison.tables.precision() < 1.0);
        assert_eq!(
            paths(&comparison, DifferenceKind::Extra),
            vec!["entities[audit]"]
        );
    }

    #[test]
    fn a_column_counts_once_however_many_of_its_fields_moved() {
        let mut changed = drawn();
        changed.entities[0].attributes[1].data_type = DataType::simple(DataTypeKind::Text);
        changed.entities[0].attributes[1].nullable = false;

        let comparison = compare(&drawn(), &changed, true);

        assert_eq!(comparison.columns.changed, 1);
        assert_eq!(comparison.columns.matched, 3);
        assert_eq!(
            paths(&comparison, DifferenceKind::Changed),
            vec![
                "entities[users].attributes[email].dataType",
                "entities[users].attributes[email].nullable"
            ]
        );
    }

    #[test]
    fn a_type_that_kept_its_kind_but_lost_its_size_is_a_change() {
        let mut changed = drawn();
        changed.entities[0].attributes[1].data_type = DataType::varchar(40);

        let comparison = compare(&drawn(), &changed, true);
        let difference = &comparison.differences[0];

        assert_eq!(difference.expected, "varchar(255)");
        assert_eq!(difference.actual, "varchar(40)");
    }

    #[test]
    fn a_key_that_came_back_as_a_plain_column_is_missing() {
        let mut flattened = drawn();
        flattened.entities[0].attributes[0].primary_key = false;
        flattened.entities[0].attributes[0].unique = false;

        let comparison = compare(&drawn(), &flattened, true);

        assert_eq!(comparison.keys.missing, 1);
        assert_eq!(comparison.keys.matched, 1);
        assert_eq!(comparison.differences[0].expected, "primary key, unique");
    }

    #[test]
    fn a_column_that_is_a_key_on_neither_side_is_not_counted_as_one() {
        let plain = SchemaDraft {
            name: "logs".to_owned(),
            description: String::new(),
            entities: vec![Entity::new("e1", "logs").with_attributes(vec![column("a1", "line")])],
            relationships: Vec::new(),
        };

        let comparison = compare(&plain, &plain.clone(), true);

        assert_eq!(comparison.keys, Counts::default());
        assert_eq!(comparison.keys.recall(), 1.0);
    }

    #[test]
    fn a_reference_that_points_somewhere_else_is_a_change() {
        let mut moved = drawn();
        moved.entities[1].attributes[1].foreign_key = Some(ForeignKeyRef::new("e1", "a2"));

        let comparison = compare(&drawn(), &moved, true);

        assert_eq!(comparison.foreign_keys.changed, 1);

        let difference = &comparison.differences[0];

        assert_eq!(difference.path, "posts.author_id.foreignKey");
        assert_eq!(difference.expected, "users.id");
        assert_eq!(difference.actual, "users.email");
    }

    #[test]
    fn a_relationship_is_matched_by_the_columns_it_joins() {
        let mut reminted = drawn();

        for entity in &mut reminted.entities {
            entity.id = format!("fresh-{}", entity.id);
        }

        reminted.relationships[0].from_entity_id = "fresh-e1".to_owned();
        reminted.relationships[0].to_entity_id = "fresh-e2".to_owned();
        reminted.entities[1].attributes[1].foreign_key = Some(ForeignKeyRef::new("fresh-e1", "a1"));

        assert!(
            compare(&drawn(), &reminted, true).is_equal(),
            "fresh ids are what an import always hands back"
        );
    }

    #[test]
    fn a_relationship_that_turned_into_another_kind_is_a_change() {
        let mut turned = drawn();
        turned.relationships[0].cardinality = Cardinality::OneToOne;

        let comparison = compare(&drawn(), &turned, true);

        assert_eq!(comparison.relationships.changed, 1);
        assert_eq!(comparison.differences[0].expected, "1:N");
        assert_eq!(comparison.differences[0].actual, "1:1");
    }

    #[test]
    fn a_description_nobody_wrote_on_either_side_is_not_a_score() {
        let mut undocumented = drawn();
        undocumented.entities[0].description = String::new();

        let bare = {
            let mut draft = drawn();
            draft.entities[0].description = String::new();
            draft
        };

        let comparison = compare(&bare, &undocumented, true);

        assert_eq!(comparison.descriptions, Counts::default());
    }

    #[test]
    fn a_description_that_did_not_survive_is_missing() {
        let mut stripped = drawn();
        stripped.entities[0].description = String::new();

        let comparison = compare(&drawn(), &stripped, true);

        assert_eq!(comparison.descriptions.missing, 1);
        assert_eq!(
            paths(&comparison, DifferenceKind::Missing),
            vec!["entities[users].description"]
        );
    }

    #[test]
    fn the_layout_is_only_compared_when_it_is_asked_for() {
        let mut moved = drawn();
        moved.entities[0].position = Position::new(999.0, 999.0);

        assert_eq!(compare(&drawn(), &moved, true).positions.changed, 1);

        let ignored = compare(&drawn(), &moved, false);

        assert_eq!(ignored.positions, Counts::default());
        assert!(ignored.is_equal());
    }
}
