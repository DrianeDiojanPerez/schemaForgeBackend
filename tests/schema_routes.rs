mod support;

use axum::http::StatusCode;
use serde_json::{json, Value};

use support::{authorized, TestApp};

fn primary_key(id: &str, name: &str) -> Value {
    json!({
        "id": id,
        "name": name,
        "description": "Surrogate key",
        "data_type": { "kind": "uuid" },
        "nullable": false,
        "primary_key": true,
    })
}

fn attribute(id: &str, name: &str, kind: &str) -> Value {
    json!({
        "id": id,
        "name": name,
        "data_type": { "kind": kind },
    })
}

fn entity(id: &str, name: &str, attributes: Vec<Value>) -> Value {
    json!({
        "id": id,
        "name": name,
        "description": format!("The {name} table"),
        "attributes": attributes,
        "position": { "x": 40.0, "y": 80.0 },
    })
}

fn course_registration() -> (Vec<Value>, Vec<Value>) {
    let students = entity(
        "e1",
        "students",
        vec![
            primary_key("a1", "id"),
            attribute("a2", "full_name", "text"),
        ],
    );

    let mut student_id = attribute("a4", "student_id", "uuid");
    student_id["foreign_key"] = json!({ "entity_id": "e1", "attribute_id": "a1" });

    let enrollments = entity(
        "e2",
        "enrollments",
        vec![primary_key("a3", "id"), student_id],
    );

    let relationship = json!({
        "id": "r1",
        "name": "enrolls",
        "description": "A student holds many enrollments",
        "from_entity_id": "e2",
        "from_attribute_id": "a4",
        "to_entity_id": "e1",
        "to_attribute_id": "a1",
        "cardinality": "1:N",
    });

    (vec![students, enrollments], vec![relationship])
}

fn a_schema(name: &str) -> Value {
    let (entities, relationships) = course_registration();

    json!({
        "name": name,
        "description": "A course registration schema",
        "entities": entities,
        "relationships": relationships,
    })
}

async fn created(app: &TestApp, name: &str) -> Value {
    let (status, body) = app
        .authorized("POST", "/v1/schemas", Some(a_schema(name)))
        .await;

    assert_eq!(status, StatusCode::OK, "creation should succeed: {body}");

    body["data"].clone()
}

#[tokio::test]
async fn a_schema_survives_a_round_trip_through_the_router() {
    let app = TestApp::new();

    let created = created(&app, "course registration").await;

    let (status, body) = app
        .get_authorized(&format!("/v1/schemas/{}", created["id"].as_str().unwrap()))
        .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["data"], created, "what was stored is what comes back");
    assert_eq!(body["data"]["entities"].as_array().map(Vec::len), Some(2));
    assert_eq!(
        body["data"]["relationships"].as_array().map(Vec::len),
        Some(1)
    );
}

#[tokio::test]
async fn the_meta_knowledge_travels_with_the_schema() {
    let app = TestApp::new();

    let created = created(&app, "documented").await;

    let students = created["entities"]
        .as_array()
        .expect("the entities should come back")
        .iter()
        .find(|entity| entity["name"] == "students")
        .expect("the students entity should be there")
        .clone();

    assert_eq!(students["description"], "The students table");
    assert_eq!(
        students["attributes"][0]["description"], "Surrogate key",
        "an attribute description is part of the model, not a frontend-only label"
    );
    assert_eq!(
        created["relationships"][0]["description"],
        "A student holds many enrollments"
    );
}

#[tokio::test]
async fn the_canvas_layout_is_part_of_the_model() {
    let app = TestApp::new();

    let created = created(&app, "positioned").await;

    assert_eq!(
        created["entities"][0]["position"],
        json!({ "x": 40.0, "y": 80.0 }),
        "a reloaded diagram must look the way it was left"
    );
}

#[tokio::test]
async fn a_foreign_key_reference_survives_the_round_trip() {
    let app = TestApp::new();

    let created = created(&app, "with a key").await;

    let enrollments = created["entities"]
        .as_array()
        .expect("the entities should come back")
        .iter()
        .find(|entity| entity["name"] == "enrollments")
        .expect("the enrollments entity should be there")
        .clone();

    let reference = enrollments["attributes"]
        .as_array()
        .expect("the attributes should come back")
        .iter()
        .find_map(|attribute| attribute.get("foreign_key").filter(|key| !key.is_null()))
        .expect("the foreign key should be there")
        .clone();

    assert_eq!(reference["entity_id"], "e1");
    assert_eq!(reference["attribute_id"], "a1");
}

#[tokio::test]
async fn a_call_without_a_token_is_turned_away() {
    let app = TestApp::new();

    let (status, body) = app.get("/v1/schemas").await;

    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body["error"]["message"], "malformed or missing jwt token");
}

#[tokio::test]
async fn a_missing_schema_is_a_not_found() {
    let app = TestApp::new();

    let (status, body) = app.get_authorized("/v1/schemas/does-not-exist").await;

    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["error"]["message"], "schema does not exist");
}

#[tokio::test]
async fn a_schema_is_only_seen_by_the_person_who_made_it() {
    let app = TestApp::new();

    let created = created(&app, "private").await;
    let id = created["id"].as_str().expect("an id should come back");
    let token = app.another_user().await;

    let (status, body) = app
        .send(authorized("GET", "/v1/schemas", &token, None))
        .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["pagination"]["total_count"], 0);

    let (status, body) = app
        .send(authorized(
            "GET",
            &format!("/v1/schemas/{id}"),
            &token,
            None,
        ))
        .await;

    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["error"]["message"], "schema does not exist");

    let (status, _) = app
        .send(authorized(
            "POST",
            "/v1/schemas",
            &token,
            Some(a_schema("private")),
        ))
        .await;

    assert_eq!(
        status,
        StatusCode::OK,
        "the same name is free for another person"
    );
}

#[tokio::test]
async fn a_duplicate_name_is_refused() {
    let app = TestApp::new();

    created(&app, "blog").await;

    let (status, body) = app
        .authorized("POST", "/v1/schemas", Some(a_schema("blog")))
        .await;

    assert_eq!(status, StatusCode::CONFLICT);
    assert!(body["error"]["message"]
        .as_str()
        .expect("a message should come back")
        .contains("blog"));
}

#[tokio::test]
async fn a_malformed_schema_is_refused_with_the_paths_that_failed() {
    let app = TestApp::new();

    let (status, body) = app
        .authorized(
            "POST",
            "/v1/schemas",
            Some(json!({
                "name": "broken",
                "entities": [{
                    "id": "e1",
                    "name": "",
                    "attributes": [{ "id": "a1", "name": "id", "primary_key": true }],
                }],
            })),
        )
        .await;

    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);

    let violations = &body["error"]["errors"];

    assert!(
        violations.get("entities[0].name").is_some(),
        "the client is told which element failed, not just that something did"
    );
    assert!(violations
        .get("entities[0].attributes[0].data_type")
        .is_some());
}

#[tokio::test]
async fn a_nameless_schema_is_refused() {
    let app = TestApp::new();

    let (status, body) = app
        .authorized("POST", "/v1/schemas", Some(json!({ "name": "  " })))
        .await;

    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(body["error"]["errors"].get("name").is_some());
}

#[tokio::test]
async fn an_update_replaces_the_drawing_and_keeps_the_identity() {
    let app = TestApp::new();

    let created = created(&app, "evolving").await;
    let id = created["id"].as_str().expect("an id should come back");

    let (status, body) = app
        .authorized(
            "PUT",
            &format!("/v1/schemas/{id}"),
            Some(json!({
                "name": "evolving",
                "description": "Now with one entity",
                "entities": [entity("e1", "students", vec![primary_key("a1", "id")])],
            })),
        )
        .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["data"]["id"], created["id"]);
    assert_eq!(
        body["data"]["created_at"], created["created_at"],
        "creation time is not rewritten"
    );
    assert_eq!(
        body["data"]["entities"].as_array().map(Vec::len),
        Some(1),
        "the drawing was replaced"
    );
    assert_eq!(
        body["data"]["relationships"].as_array().map(Vec::len),
        Some(0)
    );
}

#[tokio::test]
async fn a_listing_pages_and_reports_the_total() {
    let app = TestApp::new();

    for name in ["one", "two", "three"] {
        created(&app, name).await;
    }

    let (status, body) = app.get_authorized("/v1/schemas?page=1&per_page=2").await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["data"].as_array().map(Vec::len), Some(2));
    assert_eq!(body["pagination"]["total_count"], 3);
    assert_eq!(
        body["data"][0]["entity_count"], 2,
        "a summary counts without carrying the entities"
    );
    assert!(
        body["data"][0].get("entities").is_none(),
        "a listing does not ship every entity over the wire"
    );
}

#[tokio::test]
async fn a_deleted_schema_is_gone() {
    let app = TestApp::new();

    let created = created(&app, "temporary").await;
    let id = created["id"].as_str().expect("an id should come back");

    let (status, _) = app
        .authorized("DELETE", &format!("/v1/schemas/{id}"), None)
        .await;

    assert_eq!(status, StatusCode::OK);

    let (status, _) = app.get_authorized(&format!("/v1/schemas/{id}")).await;

    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn a_stored_schema_is_validated_and_undocumented_columns_only_warn() {
    let app = TestApp::new();

    let created = created(&app, "validated").await;

    let (status, body) = app
        .authorized(
            "POST",
            "/v1/schemas/validate",
            Some(json!({ "schema_id": created["id"] })),
        )
        .await;

    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["data"]["valid"], true, "{body}");

    let diagnostics = body["data"]["diagnostics"]
        .as_array()
        .expect("diagnostics should come back");

    assert_eq!(diagnostics.len(), 2, "{body}");
    assert!(diagnostics.iter().all(|diagnostic| {
        diagnostic["code"] == "SF-DOC-MISSING" && diagnostic["severity"] == "warning"
    }));
}

async fn generated(app: &TestApp, schema_id: &Value, include_comments: bool) -> Value {
    let (status, body) = app
        .authorized(
            "POST",
            "/v1/schemas/generate-ddl",
            Some(json!({
                "schema_id": schema_id,
                "dialect": "postgres",
                "include_comments": include_comments,
            })),
        )
        .await;

    assert_eq!(status, StatusCode::OK, "generation should succeed: {body}");

    body["data"].clone()
}

#[tokio::test]
async fn a_stored_schema_generates_postgres_ddl() {
    let app = TestApp::new();

    let created = created(&app, "generated").await;
    let generated = generated(&app, &created["id"], true).await;
    let ddl = generated["ddl"]
        .as_str()
        .expect("a script should come back");

    assert!(ddl.contains(r#"CREATE TABLE "students" ("#), "{ddl}");
    assert!(ddl.contains(r#""id" UUID NOT NULL"#), "{ddl}");
    assert!(ddl.contains(r#"PRIMARY KEY ("id")"#), "{ddl}");
    assert!(
        ddl.contains(concat!(
            r#"ADD CONSTRAINT "fk_enrollments_student_id" FOREIGN KEY ("student_id") "#,
            r#"REFERENCES "students" ("id");"#
        )),
        "{ddl}"
    );
}

#[tokio::test]
async fn the_descriptions_travel_into_the_script_as_comments() {
    let app = TestApp::new();

    let created = created(&app, "documented").await;
    let with = generated(&app, &created["id"], true).await;
    let without = generated(&app, &created["id"], false).await;

    assert!(with["ddl"]
        .as_str()
        .expect("a script should come back")
        .contains(r#"COMMENT ON TABLE "students" IS 'The students table';"#));
    assert!(!without["ddl"]
        .as_str()
        .expect("a script should come back")
        .contains("COMMENT ON"));
}

#[tokio::test]
async fn an_undocumented_column_warns_without_blocking_generation() {
    let app = TestApp::new();

    let created = created(&app, "warned").await;
    let generated = generated(&app, &created["id"], true).await;

    let diagnostics = generated["diagnostics"]
        .as_array()
        .expect("diagnostics should come back");

    assert!(diagnostics
        .iter()
        .all(|diagnostic| diagnostic["severity"] == "warning"));
    assert!(!generated["ddl"]
        .as_str()
        .expect("a script should come back")
        .is_empty());
}

#[tokio::test]
async fn a_schema_with_a_defect_comes_back_as_diagnostics_rather_than_sql() {
    let app = TestApp::new();

    let (mut entities, relationships) = course_registration();
    entities.push(entity(
        "e3",
        "audit_log",
        vec![attribute("a9", "line", "text")],
    ));

    let (status, body) = app
        .authorized(
            "POST",
            "/v1/schemas",
            Some(json!({
                "name": "broken",
                "description": "A table with no key",
                "entities": entities,
                "relationships": relationships,
            })),
        )
        .await;

    assert_eq!(status, StatusCode::OK, "creation should succeed: {body}");

    let generated = generated(&app, &body["data"]["id"], true).await;

    assert_eq!(
        generated["ddl"].as_str(),
        Some(""),
        "a broken diagram produces no script: {generated}"
    );
    assert!(generated["diagnostics"]
        .as_array()
        .expect("diagnostics should come back")
        .iter()
        .any(|diagnostic| diagnostic["code"] == "SF-KEY-MISSING"));
}

#[tokio::test]
async fn a_dialect_with_no_generator_reports_what_is_missing() {
    let app = TestApp::new();

    let created = created(&app, "unsupported").await;

    let (status, body) = app
        .authorized(
            "POST",
            "/v1/schemas/generate-ddl",
            Some(json!({
                "schema_id": created["id"],
                "dialect": "mysql",
                "include_comments": false,
            })),
        )
        .await;

    assert_eq!(status, StatusCode::NOT_IMPLEMENTED);
    assert!(body["error"]["message"]
        .as_str()
        .expect("a message should come back")
        .contains("mysql"));
}

#[tokio::test]
async fn an_unsaved_draft_can_be_validated_without_being_stored() {
    let app = TestApp::new();

    let (mut entities, relationships) = course_registration();
    entities.push(entity(
        "e3",
        "audit_log",
        vec![attribute("a5", "message", "text")],
    ));

    let (status, body) = app
        .authorized(
            "POST",
            "/v1/schemas/validate",
            Some(json!({
                "schema": {
                    "name": "a draft",
                    "entities": entities,
                    "relationships": relationships,
                },
            })),
        )
        .await;

    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["data"]["valid"], false, "{body}");

    let missing_key = body["data"]["diagnostics"]
        .as_array()
        .expect("diagnostics should come back")
        .iter()
        .find(|diagnostic| diagnostic["code"] == "SF-KEY-MISSING")
        .expect("the table without a key should be reported");

    assert_eq!(missing_key["severity"], "error");
    assert_eq!(missing_key["element_ids"], json!(["e3"]));

    let (_, body) = app.get_authorized("/v1/schemas").await;

    assert_eq!(
        body["pagination"]["total_count"], 0,
        "validating a draft must not store it"
    );
}

#[tokio::test]
async fn a_validation_without_a_target_is_refused() {
    let app = TestApp::new();

    let (status, body) = app
        .authorized("POST", "/v1/schemas/validate", Some(json!({})))
        .await;

    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(body["error"]["errors"].get("schema_id").is_some());
}

#[tokio::test]
async fn a_client_that_says_nothing_about_positions_still_gets_the_layout() {
    let app = TestApp::new();

    let created = created(&app, "positioned").await;

    let (status, body) = app
        .authorized(
            "POST",
            "/v1/schemas/generate-ddl",
            Some(json!({ "schema_id": created["id"], "dialect": "postgres" })),
        )
        .await;

    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(
        body["data"]["ddl"]
            .as_str()
            .expect("a script should come back")
            .contains("-- @table \"students\" 40 80\nCREATE TABLE \"students\""),
        "{body}"
    );
}

#[tokio::test]
async fn positions_can_be_left_out_of_the_script() {
    let app = TestApp::new();

    let created = created(&app, "unpositioned").await;

    let (status, body) = app
        .authorized(
            "POST",
            "/v1/schemas/generate-ddl",
            Some(json!({
                "schema_id": created["id"],
                "dialect": "postgres",
                "include_positions": false,
            })),
        )
        .await;

    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(!body["data"]["ddl"]
        .as_str()
        .expect("a script should come back")
        .contains("-- @table"));
}

#[tokio::test]
async fn a_stored_schema_is_exported_as_a_json_file() {
    let app = TestApp::new();

    let created = created(&app, "course registration").await;

    let (status, body) = app
        .authorized(
            "POST",
            "/v1/schemas/export",
            Some(json!({ "schema_id": created["id"] })),
        )
        .await;

    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["data"]["file_name"], "course_registration.json");
    assert_eq!(body["data"]["content_type"], "application/json");

    let document: Value = serde_json::from_str(
        body["data"]["content"]
            .as_str()
            .expect("the file should come back"),
    )
    .expect("the file should be json");

    assert_eq!(document["schemaforge"], 1);
    assert_eq!(document["name"], "course registration");
    assert_eq!(document["entities"][0]["name"], "students");
    assert_eq!(document["entities"][0]["position"]["x"], 40.0);
    assert_eq!(
        document["relationships"][0]["cardinality"],
        "CARDINALITY_ONE_TO_MANY"
    );
    assert!(document.get("id").is_none(), "{document}");
}

#[tokio::test]
async fn an_export_can_leave_the_layout_behind() {
    let app = TestApp::new();

    let created = created(&app, "flat").await;

    let (status, body) = app
        .authorized(
            "POST",
            "/v1/schemas/export",
            Some(json!({ "schema_id": created["id"], "include_positions": false })),
        )
        .await;

    assert_eq!(status, StatusCode::OK, "{body}");

    let document: Value = serde_json::from_str(
        body["data"]["content"]
            .as_str()
            .expect("the file should come back"),
    )
    .expect("the file should be json");

    assert!(
        document["entities"][0].get("position").is_none(),
        "{document}"
    );
}

#[tokio::test]
async fn an_export_format_nobody_can_write_is_refused() {
    let app = TestApp::new();

    let created = created(&app, "unwritable").await;

    let (status, body) = app
        .authorized(
            "POST",
            "/v1/schemas/export",
            Some(json!({ "schema_id": created["id"], "format": "yaml" })),
        )
        .await;

    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert!(body["error"]["errors"].get("format").is_some(), "{body}");
}

#[tokio::test]
async fn exporting_as_sql_says_it_is_not_built_yet() {
    let app = TestApp::new();

    let created = created(&app, "sql export").await;

    let (status, _) = app
        .authorized(
            "POST",
            "/v1/schemas/export",
            Some(json!({ "schema_id": created["id"], "format": "sql" })),
        )
        .await;

    assert_eq!(status, StatusCode::NOT_IMPLEMENTED);
}

#[tokio::test]
async fn a_schema_goes_out_as_a_file_and_comes_back_as_a_draft() {
    let app = TestApp::new();

    let created = created(&app, "round trip").await;
    let exported = generated_file(&app, &created["id"]).await;

    let (status, body) = app
        .authorized(
            "POST",
            "/v1/schemas/import",
            Some(json!({ "content": exported["content"], "format": "json" })),
        )
        .await;

    assert_eq!(status, StatusCode::OK, "{body}");

    let draft = &body["data"]["draft"];

    assert_eq!(draft["name"], "round trip");
    assert_eq!(draft["id"], "", "nothing was stored");
    assert_eq!(draft["entities"][0]["name"], "students");
    assert_eq!(draft["entities"][0]["position"]["x"], 40.0);
    assert!(body["data"]["diagnostics"]
        .as_array()
        .expect("diagnostics should come back")
        .is_empty());
}

async fn generated_file(app: &TestApp, schema_id: &Value) -> Value {
    let (status, body) = app
        .authorized(
            "POST",
            "/v1/schemas/export",
            Some(json!({ "schema_id": schema_id })),
        )
        .await;

    assert_eq!(status, StatusCode::OK, "export should succeed: {body}");

    body["data"].clone()
}

#[tokio::test]
async fn a_ddl_script_is_read_back_into_a_draft() {
    let app = TestApp::new();

    let created = created(&app, "from sql").await;
    let (status, body) = app
        .authorized(
            "POST",
            "/v1/schemas/generate-ddl",
            Some(json!({ "schema_id": created["id"], "include_comments": true })),
        )
        .await;

    assert_eq!(status, StatusCode::OK, "{body}");

    let ddl = body["data"]["ddl"].clone();
    let (status, body) = app
        .authorized(
            "POST",
            "/v1/schemas/import",
            Some(json!({ "content": ddl, "format": "sql" })),
        )
        .await;

    assert_eq!(status, StatusCode::OK, "{body}");

    let draft = &body["data"]["draft"];

    assert_eq!(draft["entities"][0]["name"], "students");
    assert_eq!(draft["entities"][0]["description"], "The students table");
    assert_eq!(draft["entities"][0]["position"]["x"], 40.0);
    assert_eq!(draft["relationships"].as_array().map(Vec::len), Some(1));
}

#[tokio::test]
async fn an_import_with_nothing_in_it_is_refused() {
    let app = TestApp::new();

    let (status, body) = app
        .authorized(
            "POST",
            "/v1/schemas/import",
            Some(json!({ "content": "   " })),
        )
        .await;

    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(body["error"]["errors"].get("content").is_some(), "{body}");
}
