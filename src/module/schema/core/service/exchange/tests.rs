use chrono::DateTime;
use serde_json::Value;

use crate::module::schema::core::domain::{
    Attribute, Cardinality, DataType, DataTypeKind, Entity, ExchangeFormat,
    ForeignKeyRef, Position, Relationship, Schema, SchemaDraft,
};
use crate::package::errdef::code;

use super::{export, import};

fn course_registration() -> Schema {
    let students = Entity::new("e1", "students")
        .with_description("Everyone enrolled")
        .with_position(Position::new(40.0, 80.0))
        .with_attributes(vec![Attribute::new(
            "a1",
            "id",
            DataType::simple(DataTypeKind::Uuid),
        )
        .as_primary_key()
        .with_description("Surrogate key")]);

    let enrollments = Entity::new("e2", "enrollments")
        .with_position(Position::new(400.0, 80.0))
        .with_attributes(vec![
            Attribute::new("a2", "id", DataType::simple(DataTypeKind::Uuid)).as_primary_key(),
            Attribute::new("a3", "student_id", DataType::simple(DataTypeKind::Uuid))
                .required()
                .referencing(ForeignKeyRef::new("e1", "a1")),
            Attribute::new("a4", "grade", DataType::numeric(4, 2)).with_default("0.00"),
            Attribute::new("a5", "note", DataType::varchar(255)),
        ]);

    Schema::new(
        "s1",
        SchemaDraft {
            name: "Course Registration".to_owned(),
            description: "The academic case study".to_owned(),
            entities: vec![students, enrollments],
            relationships: vec![Relationship::new(
                "r1",
                ("e2", "a3"),
                ("e1", "a1"),
                Cardinality::OneToMany,
            )
            .with_name("enrolls")],
        },
        DateTime::from_timestamp(1_700_000_000, 0).expect("a fixed instant"),
    )
}

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

    assert_eq!(students["id"], "e1");
    assert_eq!(students["name"], "students");
    assert_eq!(students["attributes"][0]["id"], "a1");
}

#[test]
fn a_column_is_written_in_the_spelling_the_contract_uses() {
    let document = exported(true);
    let id = &document["entities"][0]["attributes"][0];

    assert_eq!(id["dataType"]["kind"], "DATA_TYPE_KIND_UUID");
    assert_eq!(id["primaryKey"], true);
    assert_eq!(id["nullable"], false);
    assert_eq!(id["unique"], false);
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

    assert_eq!(columns[1]["foreignKey"]["entityId"], "e1");
    assert_eq!(columns[1]["foreignKey"]["attributeId"], "a1");
    assert!(columns[0].get("foreignKey").is_none(), "{columns}");

    assert_eq!(columns[2]["defaultValue"], "0.00");
    assert!(columns[0].get("defaultValue").is_none(), "{columns}");
}

#[test]
fn a_relationship_is_written_in_the_spelling_the_contract_uses() {
    let document = exported(true);
    let relationship = &document["relationships"][0];

    assert_eq!(relationship["cardinality"], "CARDINALITY_ONE_TO_MANY");
    assert_eq!(relationship["fromEntityId"], "e2");
    assert_eq!(relationship["fromAttributeId"], "a3");
    assert_eq!(relationship["toEntityId"], "e1");
    assert_eq!(relationship["toAttributeId"], "a1");
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
fn a_schema_survives_a_round_trip_through_the_file() {
    let original = course_registration();
    let file = export(&original, ExchangeFormat::Json, true).expect("export should succeed");
    let draft = imported(&file.content, true);

    assert_eq!(draft.name, original.name);
    assert_eq!(draft.description, original.description);
    assert_eq!(draft.entities, original.entities);
    assert_eq!(draft.relationships, original.relationships);
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
