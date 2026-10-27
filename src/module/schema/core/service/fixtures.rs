//! The schemas the tests agree on. Generation, export and import all have to
//! describe the same diagram, so they are measured against one set of
//! schemas rather than against whatever each test happened to build.
//!
//! Ids follow the names (`e.students`, `a.id`) so a fixture can be read
//! without holding a table of identifiers in your head. Attribute ids repeat
//! across tables, which is harmless because an attribute is only ever looked
//! up inside the entity that holds it.

use chrono::DateTime;

use crate::module::schema::core::domain::{
    Attribute, Cardinality, DataType, DataTypeKind, Entity, ForeignKeyRef, Position, Relationship,
    Schema, SchemaDraft,
};

pub(crate) fn key(name: &str) -> Attribute {
    typed_key(name, DataType::simple(DataTypeKind::Uuid))
}

/// A key column is unique, and the generator leaves the word off because the
/// key already says so. The model says it out loud.
pub(crate) fn typed_key(name: &str, data_type: DataType) -> Attribute {
    column(name, data_type).as_primary_key().unique()
}

pub(crate) fn column(name: &str, data_type: DataType) -> Attribute {
    Attribute::new(format!("a.{name}"), name, data_type)
}

pub(crate) fn text(name: &str) -> Attribute {
    column(name, DataType::simple(DataTypeKind::Text))
}

pub(crate) fn reference(name: &str, entity: &str, attribute: &str) -> Attribute {
    column(name, DataType::simple(DataTypeKind::Uuid)).referencing(ForeignKeyRef::new(
        format!("e.{entity}"),
        format!("a.{attribute}"),
    ))
}

pub(crate) fn table(name: &str, attributes: Vec<Attribute>) -> Entity {
    Entity::new(format!("e.{name}"), name).with_attributes(attributes)
}

pub(crate) fn placed(name: &str, at: (f64, f64), attributes: Vec<Attribute>) -> Entity {
    table(name, attributes).with_position(Position::new(at.0, at.1))
}

/// The canvas draws a relationship from the key it points at to the column
/// that holds it, so that is the direction every fixture is written in.
pub(crate) fn link(id: &str, from: (&str, &str), to: (&str, &str)) -> Relationship {
    let (from_entity, from_attribute) = (format!("e.{}", from.0), format!("a.{}", from.1));
    let (to_entity, to_attribute) = (format!("e.{}", to.0), format!("a.{}", to.1));

    Relationship::new(
        id,
        (&from_entity, &from_attribute),
        (&to_entity, &to_attribute),
        Cardinality::OneToMany,
    )
}

pub(crate) fn schema(entities: Vec<Entity>) -> Schema {
    case_study("case study", entities, Vec::new())
}

pub(crate) fn case_study(
    name: &str,
    entities: Vec<Entity>,
    relationships: Vec<Relationship>,
) -> Schema {
    Schema::new(
        "s1",
        SchemaDraft {
            name: name.to_owned(),
            description: String::new(),
            entities,
            relationships,
        },
        DateTime::from_timestamp(1_700_000_000, 0).expect("a fixed instant"),
    )
}

pub(crate) fn draft_of(schema: &Schema) -> SchemaDraft {
    SchemaDraft {
        name: schema.name.clone(),
        description: schema.description.clone(),
        entities: schema.entities.clone(),
        relationships: schema.relationships.clone(),
    }
}

/// The academic case study from the proposal: one table owning a key, one
/// holding a reference to it.
pub(crate) fn course_registration() -> Schema {
    let students = placed(
        "students",
        (40.0, 80.0),
        vec![key("id").with_description("Surrogate key")],
    )
    .with_description("Everyone enrolled");

    let enrollments = placed(
        "enrollments",
        (400.0, 80.0),
        vec![
            key("id"),
            reference("student_id", "students", "id").required(),
            column("grade", DataType::numeric(4, 2)).with_default("0.00"),
            column("note", DataType::varchar(255)),
        ],
    );

    let mut schema = case_study(
        "Course Registration",
        vec![students, enrollments],
        vec![
            link("r1", ("students", "id"), ("enrollments", "student_id"))
                .with_name("enrolls")
                .with_description("A student may enrol many times"),
        ],
    );

    schema.description = "The academic case study".to_owned();
    schema
}

/// Every name here is a word PostgreSQL would otherwise read as syntax, so
/// the script only parses if both ends quote what they write.
pub(crate) fn reserved_words() -> Schema {
    let user = placed("user", (40.0, 80.0), vec![key("id"), text("select")]);

    let order = placed(
        "order",
        (400.0, 80.0),
        vec![
            key("id"),
            reference("user", "user", "id").required(),
            text("from"),
            column("default", DataType::simple(DataTypeKind::Boolean)).with_default("false"),
        ],
    );

    case_study(
        "Reserved Words",
        vec![user, order],
        vec![link("r1", ("user", "id"), ("order", "user"))],
    )
}

/// A key across two columns, which SQL can only write at the foot of the
/// table rather than against either column.
pub(crate) fn composite_key() -> Schema {
    let students = placed("students", (40.0, 80.0), vec![key("id")]);
    let courses = placed("courses", (40.0, 340.0), vec![key("id")]);

    let enrollments = placed(
        "enrollments",
        (400.0, 200.0),
        vec![
            typed_key("student_id", DataType::simple(DataTypeKind::Uuid))
                .referencing(ForeignKeyRef::new("e.students", "a.id")),
            typed_key("course_id", DataType::simple(DataTypeKind::Uuid))
                .referencing(ForeignKeyRef::new("e.courses", "a.id")),
            column("grade", DataType::numeric(4, 2)),
        ],
    );

    case_study(
        "Composite Key",
        vec![students, courses, enrollments],
        vec![
            link("r1", ("students", "id"), ("enrollments", "student_id")),
            link("r2", ("courses", "id"), ("enrollments", "course_id")),
        ],
    )
}

/// A table pointing at itself, which is legal and has to stay legal: the
/// reference is nullable, so the first row has somewhere to start.
pub(crate) fn self_reference() -> Schema {
    let categories = placed(
        "categories",
        (40.0, 80.0),
        vec![
            key("id"),
            column("name", DataType::varchar(120)).required(),
            reference("parent_id", "categories", "id"),
        ],
    );

    case_study(
        "Self Reference",
        vec![categories],
        vec![link(
            "r1",
            ("categories", "id"),
            ("categories", "parent_id"),
        )],
    )
}

/// Two tables pointing at each other. Nothing can be inserted into either
/// without the other unless both references are nullable, which they are.
pub(crate) fn nullable_cycle() -> Schema {
    let employees = placed(
        "employees",
        (40.0, 80.0),
        vec![key("id"), reference("department_id", "departments", "id")],
    );

    let departments = placed(
        "departments",
        (400.0, 80.0),
        vec![key("id"), reference("head_id", "employees", "id")],
    );

    case_study(
        "Nullable Cycle",
        vec![employees, departments],
        vec![
            link("r1", ("departments", "id"), ("employees", "department_id")),
            link("r2", ("employees", "id"), ("departments", "head_id")),
        ],
    )
}

/// Names long enough that the constraint built from them runs past the 63
/// bytes PostgreSQL keeps, so the generator has to cut it and the reader has
/// to not care that it was cut.
pub(crate) fn long_names() -> Schema {
    let departments = placed("departments", (40.0, 80.0), vec![key("id")]);

    let records = placed(
        "student_enrollment_registration_records",
        (400.0, 80.0),
        vec![
            key("id"),
            reference("originating_department_identifier", "departments", "id").required(),
        ],
    );

    case_study(
        "Long Names",
        vec![departments, records],
        vec![link(
            "r1",
            ("departments", "id"),
            (
                "student_enrollment_registration_records",
                "originating_department_identifier",
            ),
        )],
    )
}

/// Prose with the two characters that end a quoted run in SQL. An apostrophe
/// closes a string and a double quote closes an identifier, so a description
/// carrying both is the one that breaks a naive writer.
pub(crate) fn quoted_prose() -> Schema {
    let notes = placed(
        "notes",
        (40.0, 80.0),
        vec![
            key("id").with_description("The note's own id"),
            text("body")
                .with_description("Holds the \"body\" of the note, apostrophes ' and all")
                .required(),
        ],
    )
    .with_description("A student's notes, the \"free text\" kind");

    case_study("Quoted Prose", vec![notes], Vec::new())
}
