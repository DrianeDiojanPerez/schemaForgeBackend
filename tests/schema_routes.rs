mod support;

use axum::http::StatusCode;
use serde_json::{json, Value};

use support::TestApp;

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
    let (status, body) = app.post("/v1/schemas", a_schema(name)).await;

    assert_eq!(status, StatusCode::OK, "creation should succeed: {body}");

    body["data"].clone()
}

#[tokio::test]
async fn a_schema_survives_a_round_trip_through_the_router() {
    let app = TestApp::new();

    let created = created(&app, "course registration").await;

    let (status, body) = app
        .get(&format!("/v1/schemas/{}", created["id"].as_str().unwrap()))
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
async fn a_missing_schema_is_a_not_found() {
    let app = TestApp::new();

    let (status, body) = app.get("/v1/schemas/does-not-exist").await;

    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["error"]["message"], "schema does not exist");
}

#[tokio::test]
async fn a_duplicate_name_is_refused() {
    let app = TestApp::new();

    created(&app, "blog").await;

    let (status, body) = app.post("/v1/schemas", a_schema("blog")).await;

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
        .post(
            "/v1/schemas",
            json!({
                "name": "broken",
                "entities": [{
                    "id": "e1",
                    "name": "",
                    "attributes": [{ "id": "a1", "name": "id", "primary_key": true }],
                }],
            }),
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

    let (status, body) = app.post("/v1/schemas", json!({ "name": "  " })).await;

    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(body["error"]["errors"].get("name").is_some());
}

#[tokio::test]
async fn an_update_replaces_the_drawing_and_keeps_the_identity() {
    let app = TestApp::new();

    let created = created(&app, "evolving").await;
    let id = created["id"].as_str().expect("an id should come back");

    let (status, body) = app
        .request(
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

    let (status, body) = app.get("/v1/schemas?page=1&per_page=2").await;

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
        .request("DELETE", &format!("/v1/schemas/{id}"), None)
        .await;

    assert_eq!(status, StatusCode::OK);

    let (status, _) = app.get(&format!("/v1/schemas/{id}")).await;

    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn validation_reports_the_milestone_it_waits_on_rather_than_passing() {
    let app = TestApp::new();

    let created = created(&app, "unvalidated").await;

    let (status, body) = app
        .post(
            "/v1/schemas/validate",
            json!({ "schema_id": created["id"] }),
        )
        .await;

    assert_eq!(
        status,
        StatusCode::NOT_IMPLEMENTED,
        "the frontend must not read a stub as a clean bill of health"
    );
    assert!(body["error"]["message"]
        .as_str()
        .expect("a message should come back")
        .contains("M2"));
}

#[tokio::test]
async fn generation_reports_the_milestone_it_waits_on() {
    let app = TestApp::new();

    let created = created(&app, "ungenerated").await;

    let (status, body) = app
        .post(
            "/v1/schemas/generate-ddl",
            json!({
                "schema_id": created["id"],
                "dialect": "postgres",
                "include_comments": true,
            }),
        )
        .await;

    assert_eq!(status, StatusCode::NOT_IMPLEMENTED);
    assert!(body["error"]["message"]
        .as_str()
        .expect("a message should come back")
        .contains("M3"));
}

#[tokio::test]
async fn an_unsaved_draft_can_be_validated_without_being_stored() {
    let app = TestApp::new();

    let (entities, relationships) = course_registration();

    let (status, _) = app
        .post(
            "/v1/schemas/validate",
            json!({
                "schema": {
                    "name": "a draft",
                    "entities": entities,
                    "relationships": relationships,
                },
            }),
        )
        .await;

    assert_eq!(
        status,
        StatusCode::NOT_IMPLEMENTED,
        "the draft mapped cleanly and reached the engine, which is what is missing"
    );

    let (_, body) = app.get("/v1/schemas").await;

    assert_eq!(
        body["pagination"]["total_count"], 0,
        "validating a draft must not store it"
    );
}

#[tokio::test]
async fn a_validation_without_a_target_is_refused() {
    let app = TestApp::new();

    let (status, body) = app.post("/v1/schemas/validate", json!({})).await;

    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(body["error"]["errors"].get("schema_id").is_some());
}
