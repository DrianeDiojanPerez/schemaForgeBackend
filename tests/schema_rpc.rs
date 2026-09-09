mod support;

use schemaforge_backend::rpc::v1;
use support::TestServer;
use tonic::Code;

fn data_type(kind: v1::DataTypeKind) -> Option<v1::DataType> {
    Some(v1::DataType {
        kind: kind as i32,
        length: None,
        precision: None,
        scale: None,
    })
}

fn primary_key(id: &str, name: &str) -> v1::Attribute {
    v1::Attribute {
        id: id.to_owned(),
        name: name.to_owned(),
        description: "Surrogate key".to_owned(),
        data_type: data_type(v1::DataTypeKind::Uuid),
        nullable: false,
        primary_key: true,
        unique: false,
        foreign_key: None,
        default_value: None,
    }
}

fn attribute(id: &str, name: &str, kind: v1::DataTypeKind) -> v1::Attribute {
    v1::Attribute {
        id: id.to_owned(),
        name: name.to_owned(),
        description: String::new(),
        data_type: data_type(kind),
        nullable: true,
        primary_key: false,
        unique: false,
        foreign_key: None,
        default_value: None,
    }
}

fn entity(id: &str, name: &str, attributes: Vec<v1::Attribute>) -> v1::Entity {
    v1::Entity {
        id: id.to_owned(),
        name: name.to_owned(),
        description: format!("The {name} table"),
        attributes,
        position: Some(v1::Position { x: 40.0, y: 80.0 }),
    }
}

/// The course-registration case study in miniature: two entities and the
/// relationship between them.
fn course_registration() -> (Vec<v1::Entity>, Vec<v1::Relationship>) {
    let students = entity(
        "e1",
        "students",
        vec![
            primary_key("a1", "id"),
            attribute("a2", "full_name", v1::DataTypeKind::Text),
        ],
    );

    let mut course_id = attribute("a4", "student_id", v1::DataTypeKind::Uuid);
    course_id.foreign_key = Some(v1::ForeignKeyRef {
        entity_id: "e1".to_owned(),
        attribute_id: "a1".to_owned(),
    });

    let enrollments = entity(
        "e2",
        "enrollments",
        vec![primary_key("a3", "id"), course_id],
    );

    let relationship = v1::Relationship {
        id: "r1".to_owned(),
        name: "enrolls".to_owned(),
        description: "A student holds many enrollments".to_owned(),
        from_entity_id: "e2".to_owned(),
        from_attribute_id: "a4".to_owned(),
        to_entity_id: "e1".to_owned(),
        to_attribute_id: "a1".to_owned(),
        cardinality: v1::Cardinality::OneToMany as i32,
    };

    (vec![students, enrollments], vec![relationship])
}

fn create_request(name: &str) -> v1::CreateSchemaRequest {
    let (entities, relationships) = course_registration();

    v1::CreateSchemaRequest {
        name: name.to_owned(),
        description: "A course registration schema".to_owned(),
        entities,
        relationships,
    }
}

#[tokio::test]
async fn the_health_check_answers_without_touching_the_store() {
    let server = TestServer::start().await;
    let mut client = server.health_client().await;

    let response = client
        .check(v1::CheckRequest {})
        .await
        .expect("the check should succeed")
        .into_inner();

    assert_eq!(response.status, "OK");
    assert!(!response.version.is_empty());
}

#[tokio::test]
async fn a_schema_survives_a_round_trip_through_the_wire() {
    let server = TestServer::start().await;
    let mut client = server.schema_client().await;

    let created = client
        .create_schema(create_request("course registration"))
        .await
        .expect("creation should succeed")
        .into_inner()
        .schema
        .expect("a created schema should come back");

    let fetched = client
        .get_schema(v1::GetSchemaRequest {
            id: created.id.clone(),
        })
        .await
        .expect("the fetch should succeed")
        .into_inner()
        .schema
        .expect("the stored schema should come back");

    assert_eq!(fetched, created, "what was stored is what comes back");
    assert_eq!(fetched.entities.len(), 2);
    assert_eq!(fetched.relationships.len(), 1);
}

#[tokio::test]
async fn the_meta_knowledge_travels_with_the_schema() {
    let server = TestServer::start().await;
    let mut client = server.schema_client().await;

    let created = client
        .create_schema(create_request("documented"))
        .await
        .expect("creation should succeed")
        .into_inner()
        .schema
        .expect("a created schema should come back");

    let students = created
        .entities
        .iter()
        .find(|entity| entity.name == "students")
        .expect("the students entity should be there");

    assert_eq!(students.description, "The students table");
    assert_eq!(
        students.attributes[0].description, "Surrogate key",
        "an attribute description is part of the model, not a frontend-only label"
    );
    assert_eq!(
        created.relationships[0].description,
        "A student holds many enrollments"
    );
}

#[tokio::test]
async fn the_canvas_layout_is_part_of_the_model() {
    let server = TestServer::start().await;
    let mut client = server.schema_client().await;

    let created = client
        .create_schema(create_request("positioned"))
        .await
        .expect("creation should succeed")
        .into_inner()
        .schema
        .expect("a created schema should come back");

    assert_eq!(
        created.entities[0].position,
        Some(v1::Position { x: 40.0, y: 80.0 }),
        "a reloaded diagram must look the way it was left"
    );
}

#[tokio::test]
async fn a_foreign_key_reference_survives_the_round_trip() {
    let server = TestServer::start().await;
    let mut client = server.schema_client().await;

    let created = client
        .create_schema(create_request("with a key"))
        .await
        .expect("creation should succeed")
        .into_inner()
        .schema
        .expect("a created schema should come back");

    let enrollments = created
        .entities
        .iter()
        .find(|entity| entity.name == "enrollments")
        .expect("the enrollments entity should be there");

    let reference = enrollments
        .attributes
        .iter()
        .find_map(|attribute| attribute.foreign_key.as_ref())
        .expect("the foreign key should be there");

    assert_eq!(reference.entity_id, "e1");
    assert_eq!(reference.attribute_id, "a1");
}

#[tokio::test]
async fn a_missing_schema_is_a_not_found() {
    let server = TestServer::start().await;
    let mut client = server.schema_client().await;

    let status = client
        .get_schema(v1::GetSchemaRequest {
            id: "does-not-exist".to_owned(),
        })
        .await
        .expect_err("the fetch should fail");

    assert_eq!(status.code(), Code::NotFound);
}

#[tokio::test]
async fn a_duplicate_name_is_refused() {
    let server = TestServer::start().await;
    let mut client = server.schema_client().await;

    client
        .create_schema(create_request("blog"))
        .await
        .expect("the first create should succeed");

    let status = client
        .create_schema(create_request("blog"))
        .await
        .expect_err("the second create should fail");

    assert_eq!(status.code(), Code::AlreadyExists);
}

#[tokio::test]
async fn a_malformed_schema_is_refused_with_the_paths_that_failed() {
    let server = TestServer::start().await;
    let mut client = server.schema_client().await;

    let status = client
        .create_schema(v1::CreateSchemaRequest {
            name: "broken".to_owned(),
            description: String::new(),
            entities: vec![v1::Entity {
                id: "e1".to_owned(),
                name: String::new(),
                description: String::new(),
                attributes: vec![v1::Attribute {
                    id: "a1".to_owned(),
                    name: "id".to_owned(),
                    description: String::new(),
                    data_type: None,
                    nullable: false,
                    primary_key: true,
                    unique: false,
                    foreign_key: None,
                    default_value: None,
                }],
                position: None,
            }],
            relationships: vec![],
        })
        .await
        .expect_err("the create should fail");

    assert_eq!(status.code(), Code::InvalidArgument);

    let violations = status
        .metadata()
        .get("x-validation-violations")
        .expect("the violations should travel as metadata")
        .to_str()
        .expect("the violations should be ascii");

    let parsed: serde_json::Value =
        serde_json::from_str(violations).expect("the violations should be json");

    assert!(
        parsed.get("entities[0].name").is_some(),
        "the client is told which element failed, not just that something did"
    );
    assert!(parsed.get("entities[0].attributes[0].data_type").is_some());
}

#[tokio::test]
async fn an_update_replaces_the_drawing_and_keeps_the_identity() {
    let server = TestServer::start().await;
    let mut client = server.schema_client().await;

    let created = client
        .create_schema(create_request("evolving"))
        .await
        .expect("creation should succeed")
        .into_inner()
        .schema
        .expect("a created schema should come back");

    let updated = client
        .update_schema(v1::UpdateSchemaRequest {
            id: created.id.clone(),
            name: "evolving".to_owned(),
            description: "Now with one entity".to_owned(),
            entities: vec![entity("e1", "students", vec![primary_key("a1", "id")])],
            relationships: vec![],
        })
        .await
        .expect("the update should succeed")
        .into_inner()
        .schema
        .expect("the updated schema should come back");

    assert_eq!(updated.id, created.id);
    assert_eq!(updated.created_at, created.created_at);
    assert_eq!(updated.entities.len(), 1, "the drawing was replaced");
    assert!(updated.relationships.is_empty());
}

#[tokio::test]
async fn a_listing_pages_and_reports_the_total() {
    let server = TestServer::start().await;
    let mut client = server.schema_client().await;

    for name in ["one", "two", "three"] {
        client
            .create_schema(create_request(name))
            .await
            .expect("creation should succeed");
    }

    let response = client
        .list_schemas(v1::ListSchemasRequest {
            page: 1,
            per_page: 2,
        })
        .await
        .expect("the listing should succeed")
        .into_inner();

    assert_eq!(response.schemas.len(), 2);
    assert_eq!(response.total, 3);
    assert_eq!(
        response.schemas[0].entity_count, 2,
        "a summary counts without carrying the entities"
    );
}

#[tokio::test]
async fn a_deleted_schema_is_gone() {
    let server = TestServer::start().await;
    let mut client = server.schema_client().await;

    let created = client
        .create_schema(create_request("temporary"))
        .await
        .expect("creation should succeed")
        .into_inner()
        .schema
        .expect("a created schema should come back");

    client
        .delete_schema(v1::DeleteSchemaRequest {
            id: created.id.clone(),
        })
        .await
        .expect("the delete should succeed");

    let status = client
        .get_schema(v1::GetSchemaRequest { id: created.id })
        .await
        .expect_err("the fetch should fail");

    assert_eq!(status.code(), Code::NotFound);
}

#[tokio::test]
async fn validation_reports_the_milestone_it_waits_on_rather_than_passing() {
    let server = TestServer::start().await;
    let mut client = server.schema_client().await;

    let created = client
        .create_schema(create_request("unvalidated"))
        .await
        .expect("creation should succeed")
        .into_inner()
        .schema
        .expect("a created schema should come back");

    let status = client
        .validate_schema(v1::ValidateSchemaRequest {
            target: Some(v1::validate_schema_request::Target::Id(created.id)),
        })
        .await
        .expect_err("the engine is not built yet");

    assert_eq!(
        status.code(),
        Code::Unimplemented,
        "the frontend must not read a stub as a clean bill of health"
    );
    assert!(status.message().contains("M2"));
}

#[tokio::test]
async fn generation_reports_the_milestone_it_waits_on() {
    let server = TestServer::start().await;
    let mut client = server.schema_client().await;

    let created = client
        .create_schema(create_request("ungenerated"))
        .await
        .expect("creation should succeed")
        .into_inner()
        .schema
        .expect("a created schema should come back");

    let status = client
        .generate_ddl(v1::GenerateDdlRequest {
            target: Some(v1::generate_ddl_request::Target::Id(created.id)),
            dialect: v1::Dialect::Postgres as i32,
            include_comments: true,
        })
        .await
        .expect_err("the generator is not built yet");

    assert_eq!(status.code(), Code::Unimplemented);
    assert!(status.message().contains("M3"));
}

#[tokio::test]
async fn an_unsaved_draft_can_be_validated_without_being_stored() {
    let server = TestServer::start().await;
    let mut client = server.schema_client().await;

    let (entities, relationships) = course_registration();

    let status = client
        .validate_schema(v1::ValidateSchemaRequest {
            target: Some(v1::validate_schema_request::Target::Draft(v1::Schema {
                id: String::new(),
                name: "a draft".to_owned(),
                description: String::new(),
                entities,
                relationships,
                created_at: String::new(),
                updated_at: String::new(),
            })),
        })
        .await
        .expect_err("the engine is not built yet");

    assert_eq!(
        status.code(),
        Code::Unimplemented,
        "the draft mapped cleanly and reached the engine, which is what is missing"
    );

    let listed = client
        .list_schemas(v1::ListSchemasRequest {
            page: 1,
            per_page: 10,
        })
        .await
        .expect("the listing should succeed")
        .into_inner();

    assert_eq!(listed.total, 0, "validating a draft must not store it");
}
