use chrono::DateTime;

use crate::module::schema::core::domain::{
    diagnostic_code as code, Attribute, Cardinality, DataType, DataTypeKind, Entity, ForeignKeyRef,
    Relationship, Report, Schema, SchemaDraft, Severity,
};
use crate::module::schema::core::ports::Verifier;

use super::SchemaVerifier;

fn key(id: &str, name: &str) -> Attribute {
    Attribute::new(id, name, DataType::simple(DataTypeKind::Uuid))
        .as_primary_key()
        .with_description(format!("The {name} key"))
}

fn column(id: &str, name: &str, data_type: DataType) -> Attribute {
    Attribute::new(id, name, data_type)
        .required()
        .with_description(format!("The {name} column"))
}

fn reference(id: &str, name: &str, entity_id: &str, attribute_id: &str) -> Attribute {
    column(id, name, DataType::simple(DataTypeKind::Uuid))
        .referencing(ForeignKeyRef::new(entity_id, attribute_id))
}

fn table(id: &str, name: &str, attributes: Vec<Attribute>) -> Entity {
    Entity::new(id, name)
        .with_description(format!("The {name} table"))
        .with_attributes(attributes)
}

fn link(id: &str, from: (&str, &str), to: (&str, &str), cardinality: Cardinality) -> Relationship {
    Relationship::new(id, from, to, cardinality)
        .with_name(id)
        .with_description(format!("The {id} relationship"))
}

fn schema(entities: Vec<Entity>, relationships: Vec<Relationship>) -> Schema {
    Schema::new(
        "s1",
        SchemaDraft {
            name: "case study".to_owned(),
            description: String::new(),
            entities,
            relationships,
        },
        DateTime::from_timestamp(0, 0).expect("a fixed instant"),
    )
}

/// The academic case study from the proposal, fully documented.
fn course_registration() -> Schema {
    schema(
        vec![
            table(
                "students",
                "students",
                vec![
                    key("students.id", "id"),
                    column(
                        "students.name",
                        "full_name",
                        DataType::simple(DataTypeKind::Text),
                    ),
                    column("students.email", "email", DataType::varchar(255)).unique(),
                ],
            ),
            table(
                "courses",
                "courses",
                vec![
                    key("courses.id", "id"),
                    column("courses.code", "code", DataType::varchar(12)).unique(),
                    column("courses.credits", "credits", DataType::numeric(3, 1)),
                ],
            ),
            table(
                "enrollments",
                "enrollments",
                vec![
                    key("enrollments.id", "id"),
                    reference(
                        "enrollments.student",
                        "student_id",
                        "students",
                        "students.id",
                    ),
                    reference("enrollments.course", "course_id", "courses", "courses.id"),
                    column(
                        "enrollments.at",
                        "enrolled_at",
                        DataType::simple(DataTypeKind::TimestampTz),
                    ),
                ],
            ),
        ],
        vec![
            link(
                "enrolls",
                ("enrollments", "enrollments.student"),
                ("students", "students.id"),
                Cardinality::OneToMany,
            ),
            link(
                "fills",
                ("enrollments", "enrollments.course"),
                ("courses", "courses.id"),
                Cardinality::OneToMany,
            ),
        ],
    )
}

/// The e-commerce case study, which has the recursive category the proposal
/// calls out and a relationship drawn the way the canvas draws it, from the
/// referenced key to the foreign key column.
fn ecommerce() -> Schema {
    schema(
        vec![
            table(
                "categories",
                "categories",
                vec![
                    key("categories.id", "id"),
                    Attribute::new(
                        "categories.parent",
                        "parent_id",
                        DataType::simple(DataTypeKind::Uuid),
                    )
                    .referencing(ForeignKeyRef::new("categories", "categories.id"))
                    .with_description("The parent category, empty at the top"),
                ],
            ),
            table(
                "products",
                "products",
                vec![
                    key("products.id", "id"),
                    reference(
                        "products.category",
                        "category_id",
                        "categories",
                        "categories.id",
                    ),
                    column("products.price", "price", DataType::numeric(10, 2)),
                ],
            ),
        ],
        vec![
            link(
                "parent",
                ("categories", "categories.parent"),
                ("categories", "categories.id"),
                Cardinality::OneToMany,
            ),
            link(
                "holds",
                ("categories", "categories.id"),
                ("products", "products.category"),
                Cardinality::OneToMany,
            ),
        ],
    )
}

fn verify(schema: &Schema) -> Report {
    SchemaVerifier.verify(schema)
}

fn codes(report: &Report) -> Vec<&'static str> {
    report.diagnostics.iter().map(|d| d.code).collect()
}

fn entity<'a>(schema: &'a mut Schema, id: &str) -> &'a mut Entity {
    schema
        .entities
        .iter_mut()
        .find(|entity| entity.id == id)
        .expect("the fixture should hold the entity")
}

fn attribute<'a>(schema: &'a mut Schema, entity_id: &str, id: &str) -> &'a mut Attribute {
    entity(schema, entity_id)
        .attributes
        .iter_mut()
        .find(|attribute| attribute.id == id)
        .expect("the fixture should hold the attribute")
}

#[test]
fn both_case_studies_are_clean() {
    for case in [course_registration(), ecommerce()] {
        let report = verify(&case);

        assert!(report.diagnostics.is_empty(), "{:#?}", report.diagnostics);
    }
}

#[test]
fn an_empty_schema_warns_but_stays_valid() {
    let report = verify(&schema(Vec::new(), Vec::new()));

    assert_eq!(codes(&report), vec![code::EMPTY_SCHEMA]);
    assert!(report.is_valid());
}

#[test]
fn a_blank_table_or_column_name_is_an_error() {
    let mut case = course_registration();
    entity(&mut case, "courses").name = "  ".to_owned();
    attribute(&mut case, "students", "students.email").name = String::new();

    let report = verify(&case);
    let blank: Vec<_> = report
        .diagnostics
        .iter()
        .filter(|d| d.code == code::EMPTY_NAME)
        .collect();

    assert_eq!(blank.len(), 2);
    assert_eq!(blank[0].element_ids, vec!["students", "students.email"]);
    assert_eq!(blank[1].element_ids, vec!["courses"]);
    assert!(!report.is_valid());
}

#[test]
fn a_type_parameter_postgres_would_refuse_is_an_error() {
    let mut case = course_registration();
    attribute(&mut case, "students", "students.email").data_type = DataType::varchar(0);
    attribute(&mut case, "courses", "courses.credits").data_type = DataType {
        kind: DataTypeKind::Numeric,
        length: None,
        precision: None,
        scale: Some(2),
    };

    let report = verify(&case);

    assert_eq!(
        codes(&report),
        vec![code::INVALID_TYPE_PARAMETER, code::INVALID_TYPE_PARAMETER]
    );
}

#[test]
fn a_scale_larger_than_the_precision_is_allowed() {
    let mut case = course_registration();
    attribute(&mut case, "courses", "courses.credits").data_type = DataType::numeric(2, 5);

    assert!(verify(&case).diagnostics.is_empty());
}

#[test]
fn a_relationship_pointing_at_a_missing_column_dangles() {
    let mut case = course_registration();
    case.relationships[0].to_attribute_id = "students.gone".to_owned();

    let report = verify(&case);

    assert_eq!(codes(&report), vec![code::DANGLING_RELATIONSHIP]);
    assert_eq!(report.diagnostics[0].element_ids, vec!["enrolls"]);
}

#[test]
fn table_names_that_only_differ_in_case_are_duplicates() {
    let mut case = course_registration();
    entity(&mut case, "courses").name = "Students".to_owned();

    let report = verify(&case);

    assert_eq!(codes(&report), vec![code::DUPLICATE_ENTITY_NAME]);
    assert_eq!(
        report.diagnostics[0].element_ids,
        vec!["students", "courses"]
    );
}

#[test]
fn two_columns_with_one_name_are_duplicates() {
    let mut case = course_registration();
    attribute(&mut case, "students", "students.email").name = "full_name".to_owned();

    let report = verify(&case);

    assert_eq!(codes(&report), vec![code::DUPLICATE_ATTRIBUTE_NAME]);
    assert_eq!(
        report.diagnostics[0].element_ids,
        vec!["students", "students.name", "students.email"]
    );
}

#[test]
fn a_removed_primary_key_is_reported_along_with_the_keys_that_needed_it() {
    let mut case = course_registration();
    attribute(&mut case, "students", "students.id").primary_key = false;

    let report = verify(&case);

    assert_eq!(
        codes(&report),
        vec![code::MISSING_PRIMARY_KEY, code::FOREIGN_KEY_NOT_A_KEY]
    );
    assert_eq!(report.diagnostics[0].element_ids, vec!["students"]);
}

#[test]
fn a_broken_foreign_key_is_an_error() {
    let mut case = course_registration();
    attribute(&mut case, "enrollments", "enrollments.course").foreign_key =
        Some(ForeignKeyRef::new("courses", "courses.gone"));

    let report = verify(&case);

    assert!(report.has(code::INVALID_FOREIGN_KEY));
    assert!(
        report.has(code::RELATIONSHIP_WITHOUT_FOREIGN_KEY),
        "the line on the canvas no longer has a constraint behind it"
    );
    assert!(!report.is_valid());
}

#[test]
fn a_foreign_key_to_a_column_that_is_not_unique_is_an_error() {
    let mut case = course_registration();
    attribute(&mut case, "enrollments", "enrollments.student").foreign_key =
        Some(ForeignKeyRef::new("students", "students.name"));
    attribute(&mut case, "enrollments", "enrollments.student").data_type =
        DataType::simple(DataTypeKind::Text);

    let report = verify(&case);

    assert!(report.has(code::FOREIGN_KEY_NOT_A_KEY));
}

#[test]
fn one_column_of_a_composite_key_is_not_enough_to_reference() {
    let mut case = course_registration();
    attribute(&mut case, "students", "students.email").primary_key = true;

    let report = verify(&case);

    assert_eq!(codes(&report), vec![code::FOREIGN_KEY_NOT_A_KEY]);
}

#[test]
fn a_foreign_key_across_type_families_is_an_error() {
    let mut case = course_registration();
    attribute(&mut case, "enrollments", "enrollments.student").data_type =
        DataType::simple(DataTypeKind::Integer);

    let report = verify(&case);

    assert_eq!(codes(&report), vec![code::TYPE_MISMATCH]);
    assert_eq!(report.diagnostics[0].severity, Severity::Error);
}

#[test]
fn a_narrower_foreign_key_warns_and_a_wider_one_is_fine() {
    let mut narrow = course_registration();
    attribute(&mut narrow, "students", "students.id").data_type =
        DataType::simple(DataTypeKind::BigInt);
    attribute(&mut narrow, "enrollments", "enrollments.student").data_type =
        DataType::simple(DataTypeKind::Integer);

    let report = verify(&narrow);

    assert_eq!(codes(&report), vec![code::TYPE_MISMATCH]);
    assert_eq!(report.diagnostics[0].severity, Severity::Warning);
    assert!(report.is_valid());

    let mut wide = course_registration();
    attribute(&mut wide, "students", "students.id").data_type =
        DataType::simple(DataTypeKind::Integer);
    attribute(&mut wide, "enrollments", "enrollments.student").data_type =
        DataType::simple(DataTypeKind::BigInt);

    assert!(verify(&wide).diagnostics.is_empty());
}

#[test]
fn a_bounded_text_key_referencing_unbounded_text_warns() {
    let mut case = course_registration();
    attribute(&mut case, "enrollments", "enrollments.course").data_type = DataType::varchar(8);
    attribute(&mut case, "enrollments", "enrollments.course").foreign_key =
        Some(ForeignKeyRef::new("courses", "courses.code"));
    case.relationships[1].to_attribute_id = "courses.code".to_owned();

    let report = verify(&case);

    assert_eq!(codes(&report), vec![code::TYPE_MISMATCH]);
    assert_eq!(report.diagnostics[0].severity, Severity::Warning);
}

#[test]
fn a_relationship_with_no_foreign_key_behind_it_is_an_error() {
    let mut case = course_registration();
    attribute(&mut case, "enrollments", "enrollments.student").foreign_key = None;

    let report = verify(&case);

    assert_eq!(codes(&report), vec![code::RELATIONSHIP_WITHOUT_FOREIGN_KEY]);
    assert_eq!(report.diagnostics[0].element_ids, vec!["enrolls"]);
}

#[test]
fn a_many_to_many_relationship_needs_no_foreign_key() {
    let mut case = course_registration();
    attribute(&mut case, "enrollments", "enrollments.student").foreign_key = None;
    case.relationships[0].cardinality = Cardinality::ManyToMany;

    assert!(verify(&case).diagnostics.is_empty());
}

#[test]
fn a_one_to_one_on_a_column_that_is_not_unique_warns() {
    let mut case = course_registration();
    case.relationships[0].cardinality = Cardinality::OneToOne;

    let report = verify(&case);

    assert_eq!(codes(&report), vec![code::CARDINALITY_MISMATCH]);
    assert!(report.is_valid());
}

#[test]
fn a_one_to_many_on_a_unique_column_warns() {
    let mut case = course_registration();
    attribute(&mut case, "enrollments", "enrollments.student").unique = true;

    let report = verify(&case);

    assert_eq!(codes(&report), vec![code::CARDINALITY_MISMATCH]);
}

#[test]
fn tables_that_need_each_other_and_allow_null_only_warn() {
    let mut case = course_registration();
    entity(&mut case, "students").attributes.push(
        Attribute::new(
            "students.first",
            "first_enrollment_id",
            DataType::simple(DataTypeKind::Uuid),
        )
        .referencing(ForeignKeyRef::new("enrollments", "enrollments.id"))
        .with_description("The first enrollment, filled in after it exists"),
    );

    let report = verify(&case);

    assert_eq!(codes(&report), vec![code::CIRCULAR_DEPENDENCY]);
    assert_eq!(report.diagnostics[0].severity, Severity::Warning);
    assert_eq!(
        report.diagnostics[0].element_ids,
        vec!["students", "enrollments"]
    );
}

#[test]
fn a_cycle_of_not_null_keys_is_an_error_naming_every_table() {
    let mut case = course_registration();
    entity(&mut case, "students").attributes.push(reference(
        "students.course",
        "home_course_id",
        "courses",
        "courses.id",
    ));
    entity(&mut case, "courses").attributes.push(reference(
        "courses.enrollment",
        "sample_enrollment_id",
        "enrollments",
        "enrollments.id",
    ));

    let report = verify(&case);

    assert_eq!(codes(&report), vec![code::CIRCULAR_DEPENDENCY]);
    assert_eq!(report.diagnostics[0].severity, Severity::Error);
    assert_eq!(
        report.diagnostics[0].element_ids,
        vec!["students", "courses", "enrollments"]
    );
}

#[test]
fn a_missing_description_is_one_warning_per_table() {
    let mut case = course_registration();
    entity(&mut case, "students").description = String::new();
    attribute(&mut case, "students", "students.email").description = String::new();
    attribute(&mut case, "students", "students.name").description = String::new();
    case.relationships[1].description = String::new();

    let report = verify(&case);

    assert_eq!(
        codes(&report),
        vec![code::MISSING_DESCRIPTION, code::MISSING_DESCRIPTION]
    );
    assert_eq!(
        report.diagnostics[0].element_ids,
        vec!["students", "students.name", "students.email"]
    );
    assert_eq!(report.diagnostics[1].element_ids, vec!["fills"]);
    assert!(
        report.is_valid(),
        "undocumented is not the same as incorrect"
    );
}

#[test]
fn a_diagnostic_about_a_table_carries_its_position() {
    let mut case = course_registration();
    let students = entity(&mut case, "students");
    students.position = crate::module::schema::core::domain::Position::new(120.0, 40.0);
    students.attributes[0].primary_key = false;
    students.attributes[2].unique = false;
    attribute(&mut case, "enrollments", "enrollments.student").foreign_key =
        Some(ForeignKeyRef::new("students", "students.id"));

    let report = verify(&case);
    let missing = report
        .diagnostics
        .iter()
        .find(|d| d.code == code::MISSING_PRIMARY_KEY)
        .expect("the missing key should be reported");

    assert_eq!(
        missing.location,
        Some(crate::module::schema::core::domain::Position::new(
            120.0, 40.0
        ))
    );
}
