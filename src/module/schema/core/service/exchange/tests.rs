use serde_json::Value;

use crate::module::schema::core::domain::{
    diagnostic_code, Cardinality, DataType, DataTypeKind, Entity, ExchangeFormat, Position, Report,
    SchemaDraft, Severity,
};
use crate::module::schema::core::ports::{GenerateOptions, Generator};
use crate::module::schema::core::service::fixtures::course_registration;
use crate::module::schema::core::service::PostgresGenerator;
use crate::package::errdef::code;

use super::{export, import};

fn exported(include_positions: bool) -> Value {
    let file = export(
        &course_registration(),
        ExchangeFormat::Json,
        include_positions,
    )
    .expect("json export should succeed");

    assert_eq!(file.content_type, "application/json");

    serde_json::from_str(&file.content).expect("the export should be json")
}

#[test]
fn the_file_carries_the_marker_and_leaves_out_what_the_store_assigns() {
    let document = exported(true);

    assert_eq!(document["schemaforge"], 1);
    assert_eq!(document["name"], "Course Registration");
    assert_eq!(document["description"], "The academic case study");
    assert!(document.get("id").is_none(), "{document}");
    assert!(document.get("createdAt").is_none(), "{document}");
    assert!(document.get("updatedAt").is_none(), "{document}");
}

#[test]
fn an_entity_keeps_its_ids_because_the_references_are_built_on_them() {
    let document = exported(true);
    let students = &document["entities"][0];

    assert_eq!(students["id"], "e.students");
    assert_eq!(students["name"], "students");
    assert_eq!(students["attributes"][0]["id"], "a.id");
}

#[test]
fn a_column_is_written_in_the_spelling_the_contract_uses() {
    let document = exported(true);
    let id = &document["entities"][0]["attributes"][0];

    assert_eq!(id["dataType"]["kind"], "DATA_TYPE_KIND_UUID");
    assert_eq!(id["primaryKey"], true);
    assert_eq!(id["nullable"], false);
    assert_eq!(id["unique"], true);
    assert_eq!(id["description"], "Surrogate key");
}

#[test]
fn a_type_only_carries_the_parameters_that_apply_to_it() {
    let document = exported(true);
    let columns = &document["entities"][1]["attributes"];

    assert_eq!(columns[2]["dataType"]["precision"], 4);
    assert_eq!(columns[2]["dataType"]["scale"], 2);
    assert!(columns[2]["dataType"].get("length").is_none());

    assert_eq!(columns[3]["dataType"]["length"], 255);
    assert!(columns[3]["dataType"].get("precision").is_none());
}

#[test]
fn an_unset_field_is_left_out_rather_than_written_as_null() {
    let document = exported(true);
    let columns = &document["entities"][1]["attributes"];

    assert_eq!(columns[1]["foreignKey"]["entityId"], "e.students");
    assert_eq!(columns[1]["foreignKey"]["attributeId"], "a.id");
    assert!(columns[0].get("foreignKey").is_none(), "{columns}");

    assert_eq!(columns[2]["defaultValue"], "0.00");
    assert!(columns[0].get("defaultValue").is_none(), "{columns}");
}

#[test]
fn a_relationship_is_written_in_the_spelling_the_contract_uses() {
    let document = exported(true);
    let relationship = &document["relationships"][0];

    assert_eq!(relationship["cardinality"], "CARDINALITY_ONE_TO_MANY");
    assert_eq!(relationship["fromEntityId"], "e.students");
    assert_eq!(relationship["fromAttributeId"], "a.id");
    assert_eq!(relationship["toEntityId"], "e.enrollments");
    assert_eq!(relationship["toAttributeId"], "a.student_id");
}

#[test]
fn the_layout_comes_along_unless_it_is_turned_off() {
    let with = exported(true);

    assert_eq!(with["entities"][0]["position"]["x"], 40.0);
    assert_eq!(with["entities"][0]["position"]["y"], 80.0);

    let without = exported(false);

    assert!(without["entities"][0].get("position").is_none());
    assert!(without["entities"][1].get("position").is_none());
}

#[test]
fn the_file_is_named_after_the_schema() {
    let file = export(&course_registration(), ExchangeFormat::Json, true)
        .expect("json export should succeed");

    assert_eq!(file.file_name, "course_registration.json");
}

#[test]
fn a_schema_whose_name_is_all_punctuation_still_gets_a_file_name() {
    let mut schema = course_registration();
    schema.name = " *** ".to_owned();

    let file = export(&schema, ExchangeFormat::Json, true).expect("json export should succeed");

    assert_eq!(file.file_name, "schema.json");
}

#[test]
fn exporting_as_sql_says_it_is_not_built_yet() {
    let error = export(&course_registration(), ExchangeFormat::Sql, true)
        .expect_err("sql export is not built");

    assert_eq!(error.app_code(), code::UNIMPLEMENTED);
}

fn imported(content: &str, include_positions: bool) -> SchemaDraft {
    import(content, ExchangeFormat::Json, include_positions)
        .expect("the file should load")
        .0
}

fn refused(content: &str) -> String {
    let error =
        import(content, ExchangeFormat::Json, true).expect_err("the file should be refused");

    assert_eq!(error.app_code(), code::VALIDATION_FAILED);

    error.to_string()
}

#[test]
fn a_bare_schema_without_the_marker_loads_too() {
    let draft = imported(
        r#"{
            "name": "blog",
            "entities": [
                {
                    "id": "e1",
                    "name": "users",
                    "attributes": [
                        { "id": "a1", "name": "id", "dataType": { "kind": "uuid" } }
                    ]
                }
            ]
        }"#,
        true,
    );

    assert_eq!(draft.name, "blog");
    assert_eq!(draft.entities.len(), 1);
    assert!(draft.relationships.is_empty());
}

#[test]
fn what_the_file_leaves_out_falls_back_to_the_models_own_defaults() {
    let draft = imported(
        r#"{
            "name": "blog",
            "entities": [
                {
                    "id": "e1",
                    "name": "users",
                    "attributes": [
                        { "id": "a1", "name": "nickname", "dataType": { "kind": "text" } }
                    ]
                }
            ]
        }"#,
        true,
    );

    let entity = &draft.entities[0];
    let attribute = &entity.attributes[0];

    assert_eq!(entity.description, "");
    assert!(attribute.nullable);
    assert!(!attribute.primary_key);
    assert!(!attribute.unique);
    assert_eq!(attribute.description, "");
    assert_eq!(attribute.default_value, None);
    assert_eq!(attribute.foreign_key, None);
}

#[test]
fn a_key_that_the_file_calls_nullable_is_still_not_nullable() {
    let draft = imported(
        r#"{
            "name": "blog",
            "entities": [{
                "id": "e1",
                "name": "users",
                "attributes": [{
                    "id": "a1",
                    "name": "id",
                    "dataType": { "kind": "uuid" },
                    "primaryKey": true,
                    "nullable": true
                }]
            }]
        }"#,
        true,
    );

    assert!(!draft.entities[0].attributes[0].nullable);
}

#[test]
fn a_table_with_no_position_is_dealt_into_a_grid() {
    let entities: Vec<String> = (0..6)
        .map(|index| format!(r#"{{ "id": "e{index}", "name": "t{index}", "attributes": [] }}"#))
        .collect();

    let draft = imported(
        &format!(
            r#"{{ "name": "grid", "entities": [{}] }}"#,
            entities.join(",")
        ),
        true,
    );

    let placed: Vec<(f64, f64)> = draft
        .entities
        .iter()
        .map(|entity| (entity.position.x, entity.position.y))
        .collect();

    assert_eq!(
        placed,
        vec![
            (0.0, 0.0),
            (320.0, 0.0),
            (640.0, 0.0),
            (960.0, 0.0),
            (0.0, 260.0),
            (320.0, 260.0),
        ]
    );
}

#[test]
fn turning_positions_off_lays_the_file_out_again_from_scratch() {
    let file =
        export(&course_registration(), ExchangeFormat::Json, true).expect("export should succeed");

    let draft = imported(&file.content, false);

    assert_eq!(draft.entities[0].position, Position::new(0.0, 0.0));
    assert_eq!(draft.entities[1].position, Position::new(320.0, 0.0));
}

#[test]
fn a_file_that_is_not_json_at_all_is_refused() {
    assert!(refused("not json").contains("`$`"));
}

#[test]
fn a_file_with_no_name_is_refused_and_says_so() {
    let message = refused(r#"{ "entities": [] }"#);

    assert!(message.contains("$.name"), "{message}");
}

#[test]
fn a_file_with_no_entities_is_refused_and_says_so() {
    let message = refused(r#"{ "name": "blog" }"#);

    assert!(message.contains("$.entities"), "{message}");
}

#[test]
fn the_refusal_names_the_first_bad_path_rather_than_the_file() {
    let message = refused(
        r#"{
            "name": "blog",
            "entities": [
                { "id": "e1", "name": "users", "attributes": [] },
                { "id": "e2", "attributes": [] }
            ]
        }"#,
    );

    assert!(message.contains("$.entities[1].name"), "{message}");
}

#[test]
fn a_column_with_a_type_nobody_knows_names_the_column_that_has_it() {
    let message = refused(
        r#"{
            "name": "blog",
            "entities": [{
                "id": "e1",
                "name": "users",
                "attributes": [{ "id": "a1", "name": "id", "dataType": { "kind": "money" } }]
            }]
        }"#,
    );

    assert!(
        message.contains("$.entities[0].attributes[0].dataType.kind"),
        "{message}"
    );
}

#[test]
fn a_relationship_pointing_at_a_table_the_file_does_not_hold_is_refused() {
    let message = refused(
        r#"{
            "name": "blog",
            "entities": [{
                "id": "e1",
                "name": "users",
                "attributes": [{ "id": "a1", "name": "id", "dataType": { "kind": "uuid" } }]
            }],
            "relationships": [{
                "id": "r1",
                "fromEntityId": "e9",
                "fromAttributeId": "a9",
                "toEntityId": "e1",
                "toAttributeId": "a1",
                "cardinality": "CARDINALITY_ONE_TO_MANY"
            }]
        }"#,
    );

    assert!(
        message.contains("$.relationships[0].fromEntityId"),
        "{message}"
    );
}

#[test]
fn a_file_that_names_no_ids_still_opens_with_ids_of_its_own() {
    let draft = imported(
        r#"{
            "name": "blog",
            "entities": [{
                "name": "users",
                "attributes": [{ "name": "id", "dataType": { "kind": "uuid" } }]
            }]
        }"#,
        true,
    );

    assert!(!draft.entities[0].id.is_empty());
    assert!(!draft.entities[0].attributes[0].id.is_empty());
}

fn from_sql(content: &str) -> (SchemaDraft, Report) {
    import(content, ExchangeFormat::Sql, true).expect("the script should load")
}

fn only_table(content: &str) -> Entity {
    let (draft, report) = from_sql(content);

    assert!(
        report.diagnostics.is_empty(),
        "nothing should be skipped: {:?}",
        report.diagnostics
    );

    draft.entities.into_iter().next().expect("one table")
}

#[test]
fn the_reference_comes_back_pointing_at_the_same_column() {
    let script = PostgresGenerator.generate(&course_registration(), GenerateOptions::default());
    let (draft, _) = from_sql(&script);

    let students = &draft.entities[0];
    let enrollments = &draft.entities[1];
    let reference = enrollments.attributes[1]
        .foreign_key
        .as_ref()
        .expect("student_id is a foreign key");

    assert_eq!(reference.entity_id, students.id);
    assert_eq!(reference.attribute_id, students.attributes[0].id);

    let relationship = &draft.relationships[0];

    assert_eq!(relationship.from_entity_id, students.id);
    assert_eq!(relationship.from_attribute_id, students.attributes[0].id);
    assert_eq!(relationship.to_entity_id, enrollments.id);
    assert_eq!(relationship.to_attribute_id, enrollments.attributes[1].id);
    assert_eq!(relationship.cardinality, Cardinality::OneToMany);
}

#[test]
fn a_late_foreign_key_runs_from_the_key_to_the_column_that_holds_it() {
    let (draft, _) = from_sql(
        "CREATE TABLE students (id UUID PRIMARY KEY);
         CREATE TABLE enrollments (id UUID PRIMARY KEY, student_id UUID NOT NULL);
         ALTER TABLE enrollments
             ADD CONSTRAINT fk_enrollments_student_id
             FOREIGN KEY (student_id) REFERENCES students (id);",
    );

    let students = &draft.entities[0];
    let enrollments = &draft.entities[1];
    let relationship = &draft.relationships[0];

    assert_eq!(relationship.from_entity_id, students.id);
    assert_eq!(relationship.from_attribute_id, students.attributes[0].id);
    assert_eq!(relationship.to_entity_id, enrollments.id);
    assert_eq!(relationship.to_attribute_id, enrollments.attributes[1].id);
}

#[test]
fn a_column_in_the_key_comes_back_unique() {
    let (draft, _) = from_sql(
        "CREATE TABLE students (id UUID NOT NULL, email TEXT, PRIMARY KEY (id));
         CREATE TABLE enrollments (
             student_id UUID NOT NULL,
             course_id UUID NOT NULL,
             PRIMARY KEY (student_id, course_id)
         );",
    );

    let students = &draft.entities[0];

    assert!(students.attributes[0].unique, "the key implies it");
    assert!(!students.attributes[1].unique);

    // Neither column of a composite key is unique on its own, but the model
    // has no way to say "part of the key" other than this flag, and SQL
    // cannot tell the two readings apart.
    let enrollments = &draft.entities[1];

    assert!(enrollments.attributes[0].unique);
    assert!(enrollments.attributes[1].unique);
}

#[test]
fn the_line_above_each_table_puts_it_back_where_it_was() {
    let script = PostgresGenerator.generate(&course_registration(), GenerateOptions::default());
    let (draft, _) = from_sql(&script);

    assert_eq!(draft.entities[0].position, Position::new(40.0, 80.0));
    assert_eq!(draft.entities[1].position, Position::new(400.0, 80.0));
}

/// Scripts saved before the positions moved above their tables still open
/// with the diagram they were saved with.
#[test]
fn a_layout_block_at_the_foot_of_an_older_script_still_counts() {
    let (draft, _) = from_sql(
        "CREATE TABLE \"users\" (\"id\" UUID NOT NULL, PRIMARY KEY (\"id\"));

         CREATE TABLE \"orders\" (\"id\" UUID NOT NULL, PRIMARY KEY (\"id\"));

         -- @schemaforge layout 1
         -- @table \"users\" 120 340
         -- @table \"orders\" 560 340",
    );

    assert_eq!(draft.entities[0].position, Position::new(120.0, 340.0));
    assert_eq!(draft.entities[1].position, Position::new(560.0, 340.0));
}

#[test]
fn the_line_above_the_table_beats_the_block_at_the_foot() {
    let (draft, _) = from_sql(
        "-- @table \"users\" 120 340
         CREATE TABLE \"users\" (\"id\" UUID NOT NULL, PRIMARY KEY (\"id\"));

         -- @schemaforge layout 1
         -- @table \"users\" 999 999",
    );

    assert_eq!(draft.entities[0].position, Position::new(120.0, 340.0));
}

#[test]
fn the_layout_can_be_ignored_in_favour_of_a_fresh_grid() {
    let script = PostgresGenerator.generate(&course_registration(), GenerateOptions::default());
    let (draft, _) = import(&script, ExchangeFormat::Sql, false).expect("the script should load");

    assert_eq!(draft.entities[0].position, Position::new(0.0, 0.0));
    assert_eq!(draft.entities[1].position, Position::new(320.0, 0.0));
}

#[test]
fn the_descriptions_come_back_off_the_comments() {
    let script = PostgresGenerator.generate(&course_registration(), GenerateOptions::default());
    let (draft, _) = from_sql(&script);

    assert_eq!(draft.entities[0].description, "Everyone enrolled");
    assert_eq!(draft.entities[0].attributes[0].description, "Surrogate key");
}

#[test]
fn a_hand_written_script_reads_without_any_quoting() {
    let entity = only_table(
        "create table users (
            id uuid primary key,
            email varchar(255) not null unique,
            nickname text,
            joined timestamptz default now()
        );",
    );

    assert_eq!(entity.name, "users");

    let names: Vec<&str> = entity
        .attributes
        .iter()
        .map(|attribute| attribute.name.as_str())
        .collect();

    assert_eq!(names, vec!["id", "email", "nickname", "joined"]);
    assert!(entity.attributes[0].primary_key);
    assert!(!entity.attributes[0].nullable);
    assert!(entity.attributes[1].unique);
    assert!(!entity.attributes[1].nullable);
    assert_eq!(entity.attributes[1].data_type, DataType::varchar(255));
    assert!(entity.attributes[2].nullable);
    assert_eq!(entity.attributes[3].default_value.as_deref(), Some("now()"));
}

#[test]
fn the_long_spellings_of_a_type_mean_the_same_thing() {
    let entity = only_table(
        "CREATE TABLE t (
            a CHARACTER VARYING(40),
            b DOUBLE PRECISION,
            c TIMESTAMP WITH TIME ZONE,
            d TIMESTAMP WITHOUT TIME ZONE,
            e DECIMAL(10, 2),
            f INT,
            g BOOL
        );",
    );

    let kinds: Vec<DataTypeKind> = entity
        .attributes
        .iter()
        .map(|attribute| attribute.data_type.kind)
        .collect();

    assert_eq!(
        kinds,
        vec![
            DataTypeKind::Varchar,
            DataTypeKind::DoublePrecision,
            DataTypeKind::TimestampTz,
            DataTypeKind::Timestamp,
            DataTypeKind::Numeric,
            DataTypeKind::Integer,
            DataTypeKind::Boolean,
        ]
    );
    assert_eq!(entity.attributes[0].data_type.length, Some(40));
    assert_eq!(entity.attributes[4].data_type.precision, Some(10));
    assert_eq!(entity.attributes[4].data_type.scale, Some(2));
}

#[test]
fn a_serial_key_is_an_integer_the_database_always_fills_in() {
    let entity = only_table("CREATE TABLE t (id SERIAL PRIMARY KEY, big BIGSERIAL);");

    assert_eq!(entity.attributes[0].data_type.kind, DataTypeKind::Integer);
    assert_eq!(entity.attributes[1].data_type.kind, DataTypeKind::BigInt);
    assert!(!entity.attributes[1].nullable);
}

#[test]
fn a_key_written_at_the_foot_of_the_table_marks_the_same_columns() {
    let entity = only_table(
        "CREATE TABLE enrollments (
            student_id uuid,
            course_id uuid,
            PRIMARY KEY (student_id, course_id)
        );",
    );

    assert_eq!(entity.primary_key().len(), 2);
    assert!(entity
        .attributes
        .iter()
        .all(|attribute| !attribute.nullable));
}

#[test]
fn an_inline_reference_becomes_a_relationship() {
    let (draft, report) = from_sql(
        "CREATE TABLE users (id uuid PRIMARY KEY);
         CREATE TABLE posts (id uuid PRIMARY KEY, author uuid REFERENCES users (id));",
    );

    assert!(report.diagnostics.is_empty(), "{:?}", report.diagnostics);
    assert_eq!(draft.relationships.len(), 1);
    assert!(draft.entities[1].attributes[1].foreign_key.is_some());
}

#[test]
fn a_reference_with_no_column_named_points_at_the_other_key() {
    let (draft, _) = from_sql(
        "CREATE TABLE users (id uuid PRIMARY KEY);
         CREATE TABLE posts (id uuid PRIMARY KEY, author uuid REFERENCES users);",
    );

    let reference = draft.entities[1].attributes[1]
        .foreign_key
        .as_ref()
        .expect("author is a foreign key");

    assert_eq!(reference.attribute_id, draft.entities[0].attributes[0].id);
}

#[test]
fn a_schema_qualified_table_keeps_the_schema_in_its_name() {
    let (draft, _) = from_sql(
        "CREATE TABLE public.users (id uuid PRIMARY KEY);
         CREATE TABLE public.posts (
            id uuid PRIMARY KEY,
            author uuid REFERENCES public.users (id)
         );",
    );

    assert_eq!(draft.entities[0].name, "public.users");
    assert_eq!(draft.entities[1].name, "public.posts");
    assert_eq!(draft.relationships.len(), 1);
}

#[test]
fn a_statement_the_reader_does_not_handle_is_a_warning_naming_it() {
    let (draft, report) = from_sql(
        "CREATE TABLE users (id uuid PRIMARY KEY);
         CREATE INDEX users_id_idx ON users (id);",
    );

    assert_eq!(draft.entities.len(), 1, "the rest of the file still opens");

    let skipped = report
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == diagnostic_code::IMPORT_SKIPPED)
        .expect("the index should be reported");

    assert_eq!(skipped.severity, Severity::Warning);
    assert!(skipped.message.contains("CREATE INDEX"), "{skipped:?}");
}

#[test]
fn a_column_with_a_type_nobody_knows_is_dropped_and_named() {
    let (draft, report) = from_sql("CREATE TABLE t (id uuid PRIMARY KEY, price money);");

    assert_eq!(draft.entities[0].attributes.len(), 1);

    let skipped = report
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == diagnostic_code::IMPORT_SKIPPED)
        .expect("the column should be reported");

    assert!(skipped.message.contains("price money"), "{skipped:?}");
}

#[test]
fn a_check_constraint_is_dropped_and_the_table_still_opens() {
    let (draft, report) = from_sql(
        "CREATE TABLE t (id uuid PRIMARY KEY, age int CHECK (age > 0), CHECK (id IS NOT NULL));",
    );

    assert_eq!(draft.entities[0].attributes.len(), 2);
    assert!(report
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.message.contains("check constraint")));
}

#[test]
fn comments_and_apostrophes_survive_the_reading() {
    let entity = only_table(
        "CREATE TABLE students (id uuid PRIMARY KEY);
         COMMENT ON TABLE students IS 'The student''s record';",
    );

    assert_eq!(entity.description, "The student's record");
}

#[test]
fn a_comment_on_a_table_nobody_declared_is_a_warning() {
    let (_, report) = from_sql(
        "CREATE TABLE users (id uuid PRIMARY KEY);
         COMMENT ON TABLE ghosts IS 'nothing here';",
    );

    assert!(report
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.message.contains("ghosts")));
}

#[test]
fn a_block_comment_and_a_plain_line_comment_are_both_ignored() {
    let entity = only_table(
        "/* the accounts table /* nested */ still a comment */
         CREATE TABLE users ( -- the key
            id uuid PRIMARY KEY
         );",
    );

    assert_eq!(entity.name, "users");
    assert_eq!(entity.attributes.len(), 1);
}

#[test]
fn a_semicolon_inside_a_string_does_not_end_the_statement() {
    let entity = only_table("CREATE TABLE t (id uuid PRIMARY KEY, note text DEFAULT 'a; b');");

    assert_eq!(
        entity.attributes[1].default_value.as_deref(),
        Some("'a; b'")
    );
}

#[test]
fn a_file_with_nothing_the_canvas_could_draw_is_refused() {
    let error = import("SELECT 1;", ExchangeFormat::Sql, true).expect_err("nothing to draw");

    assert_eq!(error.app_code(), code::VALIDATION_FAILED);
}
