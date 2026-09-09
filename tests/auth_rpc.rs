mod support;

use schemaforge_backend::rpc::v1;
use support::TestServer;
use tonic::Code;

fn a_schema() -> v1::CreateSchemaRequest {
    v1::CreateSchemaRequest {
        name: "course registration".to_owned(),
        description: "A course registration schema".to_owned(),
        entities: vec![v1::Entity {
            id: "e1".to_owned(),
            name: "students".to_owned(),
            description: "The students table".to_owned(),
            attributes: vec![v1::Attribute {
                id: "a1".to_owned(),
                name: "id".to_owned(),
                description: "Surrogate key".to_owned(),
                data_type: Some(v1::DataType {
                    kind: v1::DataTypeKind::Uuid as i32,
                    length: None,
                    precision: None,
                    scale: None,
                }),
                nullable: false,
                primary_key: true,
                unique: false,
                foreign_key: None,
                default_value: None,
            }],
            position: None,
        }],
        relationships: vec![],
    }
}

#[tokio::test]
async fn the_frontend_server_can_trade_credentials_for_tokens() {
    let server = TestServer::start().await;
    let mut client = server.auth_client().await;

    let tokens = client
        .login(v1::LoginRequest {
            email: server.user.email.clone(),
            password: "password".to_owned(),
        })
        .await
        .expect("the login should succeed")
        .into_inner();

    assert!(!tokens.token.is_empty());
    assert!(!tokens.refresh_token.is_empty());

    let refreshed = client
        .refresh_token(v1::RefreshTokenRequest {
            refresh_token: tokens.refresh_token,
        })
        .await
        .expect("the refresh should succeed")
        .into_inner();

    assert_eq!(refreshed.token, tokens.token);
}

#[tokio::test]
async fn the_wrong_password_is_refused_the_same_way_the_api_refuses_it() {
    let server = TestServer::start().await;
    let mut client = server.auth_client().await;

    let status = client
        .login(v1::LoginRequest {
            email: server.user.email.clone(),
            password: "not the password".to_owned(),
        })
        .await
        .expect_err("the login should fail");

    assert_eq!(status.code(), Code::Unauthenticated);
    assert_eq!(
        status
            .metadata()
            .get("x-app-code")
            .expect("the app code should travel as metadata"),
        "1002"
    );
}

/// proto3 has no required fields, so an omitted email arrives as an empty
/// string rather than as nothing at all.
#[tokio::test]
async fn an_empty_credential_is_a_validation_failure_and_not_a_login_attempt() {
    let server = TestServer::start().await;
    let mut client = server.auth_client().await;

    let status = client
        .login(v1::LoginRequest {
            email: String::new(),
            password: String::new(),
        })
        .await
        .expect_err("the login should fail");

    assert_eq!(status.code(), Code::InvalidArgument);

    let violations = status
        .metadata()
        .get("x-validation-violations")
        .expect("the violations should travel as metadata")
        .to_str()
        .expect("the violations should be ascii");

    let parsed: serde_json::Value =
        serde_json::from_str(violations).expect("the violations should be json");

    assert!(parsed.get("email").is_some());
    assert!(parsed.get("password").is_some());
}

#[tokio::test]
async fn a_stale_refresh_token_is_refused() {
    let server = TestServer::start().await;
    let mut client = server.auth_client().await;

    let status = client
        .refresh_token(v1::RefreshTokenRequest {
            refresh_token: "an-expired-refresh-token".to_owned(),
        })
        .await
        .expect_err("the refresh should fail");

    assert_eq!(status.code(), Code::Unauthenticated);
}

#[tokio::test]
async fn a_schema_call_without_a_token_never_reaches_the_handler() {
    let server = TestServer::start().await;
    let mut client = server.anonymous_schema_client().await;

    let status = client
        .create_schema(a_schema())
        .await
        .expect_err("the create should fail");

    assert_eq!(status.code(), Code::Unauthenticated);
    assert!(
        server.calls.permission_checks.lock().unwrap().is_empty(),
        "an unidentified caller is turned away before any permission is looked up"
    );
}

#[tokio::test]
async fn a_token_the_auth_service_does_not_know_is_refused() {
    let server = TestServer::start().await;
    let mut client = server.anonymous_schema_client().await;

    let mut request = tonic::Request::new(a_schema());
    request.metadata_mut().insert(
        "authorization",
        "Bearer a-forged-token"
            .parse()
            .expect("the header should build"),
    );

    let status = client
        .create_schema(request)
        .await
        .expect_err("the create should fail");

    assert_eq!(status.code(), Code::Unauthenticated);
}

#[tokio::test]
async fn reading_a_schema_and_writing_one_ask_for_different_permissions() {
    let server = TestServer::with_permissions(&["Schemas.View All"]).await;
    let mut client = server.schema_client().await;

    client
        .list_schemas(v1::ListSchemasRequest {
            page: 1,
            per_page: 10,
        })
        .await
        .expect("the listing should be allowed");

    let status = client
        .create_schema(a_schema())
        .await
        .expect_err("the create should be refused");

    assert_eq!(status.code(), Code::Unauthenticated);

    let checks = server.calls.permission_checks.lock().unwrap();
    let asked: Vec<&str> = checks.iter().map(|(_, action)| action.as_str()).collect();

    assert_eq!(asked, vec!["Schemas.View All", "Schemas.Create"]);
    assert!(
        checks.iter().all(|(id, _)| *id == server.user.id),
        "the permission is looked up for the caller the token identified"
    );
}

#[tokio::test]
async fn the_health_check_stays_open() {
    let server = TestServer::start().await;
    let mut client = server.health_client().await;

    client
        .check(v1::CheckRequest {})
        .await
        .expect("the check should not need a token");
}
