// Every test binary compiles this module, so helpers only one of them uses
// would otherwise be reported as dead code.
#![allow(dead_code)]

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use axum::body::Body;
use axum::http::{Request, Response, StatusCode};
use axum::Router;
use chrono::Utc;
use http_body_util::BodyExt;
use serde_json::Value;
use tower::ServiceExt;
use uuid::Uuid;

use schemaforge_backend::module::iam::core::domain::{
    Company, CreateUser, Department, Permission, Role, Status, User,
};
use schemaforge_backend::module::iam::core::ports::{PermissionService, UpdateUser, UserService};
use schemaforge_backend::module::schema::core::domain::{
    DomainError, Schema, SchemaDraft, SchemaSummary,
};
use schemaforge_backend::module::schema::core::ports::SchemaRepository;
use schemaforge_backend::module::{health, iam, schema};
use schemaforge_backend::package::auth::{Auth, AuthenticationTokens, Identity};
use schemaforge_backend::package::errdef::Error;
use schemaforge_backend::package::pagination::{Data, ListRequest};
use schemaforge_backend::package::rbac::Engine;
use schemaforge_backend::rpc::v1::health_service_client::HealthServiceClient;
use schemaforge_backend::rpc::v1::schema_service_client::SchemaServiceClient;
use schemaforge_backend::server::{self, Modules};
use std::net::SocketAddr;
use std::time::Duration;
use tokio::net::TcpListener;
use tokio::task::JoinHandle;
use tonic::transport::{Channel, Server};

pub const VALID_TOKEN: &str = "a-valid-token";

#[derive(Default)]
pub struct Calls {
    pub list_requests: Mutex<Vec<ListRequest>>,
    pub created_users: Mutex<Vec<CreateUser>>,
    pub updates: Mutex<Vec<(Uuid, UpdateUser)>>,
    pub deleted: Mutex<Vec<Uuid>>,
    pub permission_checks: Mutex<Vec<(Uuid, String)>>,
}

pub struct FakeAuth {
    pub user: Identity,
}

#[async_trait]
impl Auth for FakeAuth {
    async fn generate_token(
        &self,
        email: &str,
        password: &str,
    ) -> Result<AuthenticationTokens, Error> {
        if email == self.user.email && password == "password" {
            return Ok(AuthenticationTokens {
                token: VALID_TOKEN.to_owned(),
                refresh_token: "a-valid-refresh-token".to_owned(),
            });
        }

        Err(Error::unauthorized("invalid username or password"))
    }

    async fn refresh_token(&self, refresh_token: &str) -> Result<AuthenticationTokens, Error> {
        if refresh_token == "a-valid-refresh-token" {
            return Ok(AuthenticationTokens {
                token: VALID_TOKEN.to_owned(),
                refresh_token: "a-valid-refresh-token".to_owned(),
            });
        }

        Err(Error::unauthorized("invalid or malformed refresh token"))
    }

    async fn get_identity(&self, access_token: &str) -> Result<Identity, Error> {
        if access_token == VALID_TOKEN {
            return Ok(self.user.clone());
        }

        Err(Error::unauthorized("invalid or malformed refresh token"))
    }

    async fn password_recovery(&self, email: &str, _callback_uri: &str) -> Result<(), Error> {
        if email == self.user.email {
            return Ok(());
        }

        Err(Error::not_found("invalid email address"))
    }

    async fn reset_password(&self, token: &str, _new_password: &str) -> Result<(), Error> {
        if token == "a-valid-reset-token" {
            return Ok(());
        }

        Err(Error::bad_request("invalid or expired token"))
    }
}

pub struct FakeRbac {
    pub allowed: Vec<String>,
    pub calls: Arc<Calls>,
}

#[async_trait]
impl Engine for FakeRbac {
    async fn can(&self, user_id: Uuid, action: &str) -> bool {
        self.calls
            .permission_checks
            .lock()
            .unwrap()
            .push((user_id, action.to_owned()));

        self.allowed.iter().any(|allowed| allowed == action)
    }

    async fn can_any(&self, user_id: Uuid, actions: &[&str]) -> bool {
        for action in actions {
            if self.can(user_id, action).await {
                return true;
            }
        }
        false
    }
}

pub struct FakeUserService {
    pub users: Vec<User>,
    pub calls: Arc<Calls>,
}

#[async_trait]
impl UserService for FakeUserService {
    async fn index(&self, request: ListRequest) -> Result<Data<User>, Error> {
        let page = request.page;
        let per_page = request.per_page;
        self.calls.list_requests.lock().unwrap().push(request);

        Ok(Data::new(
            self.users.clone(),
            self.users.len() as i64,
            page,
            per_page,
        ))
    }

    async fn create(&self, new_user: CreateUser) -> Result<Uuid, Error> {
        let id = Uuid::new_v4();
        self.calls.created_users.lock().unwrap().push(new_user);
        Ok(id)
    }

    async fn find_by_id(&self, user_id: Uuid) -> Result<User, Error> {
        self.users
            .iter()
            .find(|user| user.id == user_id)
            .cloned()
            .ok_or_else(|| Error::not_found("user does not exists"))
    }

    async fn partial_update(&self, user_id: Uuid, fields: UpdateUser) -> Result<(), Error> {
        self.calls.updates.lock().unwrap().push((user_id, fields));
        Ok(())
    }

    async fn delete(&self, user_id: Uuid) -> Result<(), Error> {
        self.calls.deleted.lock().unwrap().push(user_id);
        Ok(())
    }
}

pub struct FakePermissionService {
    pub permissions: Vec<Permission>,
}

#[async_trait]
impl PermissionService for FakePermissionService {
    async fn list_all(&self) -> Result<Vec<Permission>, Error> {
        Ok(self.permissions.clone())
    }
}

/// The store the transport tests run against. The real one is PostgreSQL, and
/// these tests are about the JSON and protobuf mapping rather than about
/// persistence, so they keep their own store instead of needing a database.
///
/// Persistence itself is covered in `tests/postgres.rs`.
#[derive(Default)]
pub struct FakeSchemaRepository {
    schemas: Mutex<HashMap<String, Schema>>,
}

impl FakeSchemaRepository {
    fn read(&self) -> std::sync::MutexGuard<'_, HashMap<String, Schema>> {
        self.schemas.lock().expect("the store lock should be sound")
    }

    fn sorted(schemas: &HashMap<String, Schema>) -> Vec<&Schema> {
        let mut all: Vec<&Schema> = schemas.values().collect();
        all.sort_by(|a, b| a.created_at.cmp(&b.created_at).then(a.id.cmp(&b.id)));
        all
    }
}

#[async_trait]
impl SchemaRepository for FakeSchemaRepository {
    async fn index(&self, request: &ListRequest) -> Result<(Vec<SchemaSummary>, i64), DomainError> {
        let schemas = self.read();
        let total = schemas.len() as i64;

        let page = Self::sorted(&schemas)
            .into_iter()
            .skip(request.offset() as usize)
            .take(request.per_page as usize)
            .map(Schema::summary)
            .collect();

        Ok((page, total))
    }

    async fn create(&self, draft: SchemaDraft) -> Result<Schema, DomainError> {
        let mut schemas = self.read();

        if schemas
            .values()
            .any(|schema| schema.name.eq_ignore_ascii_case(&draft.name))
        {
            return Err(DomainError::DuplicateSchemaName(draft.name));
        }

        let schema = Schema::new(Uuid::new_v4().to_string(), draft, Utc::now());
        schemas.insert(schema.id.clone(), schema.clone());

        Ok(schema)
    }

    async fn find_by_id(&self, id: &str) -> Result<Option<Schema>, DomainError> {
        Ok(self.read().get(id).cloned())
    }

    async fn find_by_name(&self, name: &str) -> Result<Option<Schema>, DomainError> {
        Ok(self
            .read()
            .values()
            .find(|schema| schema.name.eq_ignore_ascii_case(name))
            .cloned())
    }

    async fn replace(&self, id: &str, draft: SchemaDraft) -> Result<Schema, DomainError> {
        let mut schemas = self.read();

        if schemas
            .values()
            .any(|schema| schema.id != id && schema.name.eq_ignore_ascii_case(&draft.name))
        {
            return Err(DomainError::DuplicateSchemaName(draft.name));
        }

        let schema = schemas.get_mut(id).ok_or(DomainError::SchemaNotFound)?;
        schema.replace_with(draft, Utc::now());

        Ok(schema.clone())
    }

    async fn delete(&self, id: &str) -> Result<(), DomainError> {
        self.read()
            .remove(id)
            .map(|_| ())
            .ok_or(DomainError::SchemaNotFound)
    }
}

pub fn fake_schema_services() -> schema::Services {
    schema::Services::with_repository(Arc::new(FakeSchemaRepository::default()))
}

pub struct TestApp {
    pub router: Router,
    pub calls: Arc<Calls>,
    pub user: Identity,
}

impl TestApp {
    pub fn with_permissions(allowed: &[&str]) -> Self {
        let user = a_domain_user();
        let identity = Identity {
            id: user.id,
            email: user.email.clone(),
            user_name: user.user_name.clone(),
            password: String::new(),
            roles: vec!["Staff".to_owned()],
        };

        let calls = Arc::new(Calls::default());

        let modules = Modules {
            auth: Arc::new(FakeAuth {
                user: identity.clone(),
            }),
            rbac: Arc::new(FakeRbac {
                allowed: allowed.iter().map(|a| (*a).to_owned()).collect(),
                calls: calls.clone(),
            }),
            iam: iam::Services {
                user: Arc::new(FakeUserService {
                    users: vec![user],
                    calls: calls.clone(),
                }),
                permission: Arc::new(FakePermissionService {
                    permissions: vec![Permission {
                        id: 1,
                        name: "View All".to_owned(),
                        resource: "Users".to_owned(),
                        module: "IAM Module".to_owned(),
                    }],
                }),
            },
            schema: fake_schema_services(),
        };

        Self {
            router: server::router(&modules),
            calls,
            user: identity,
        }
    }

    pub fn new() -> Self {
        Self::with_permissions(&["Users.View All"])
    }

    pub async fn send(&self, request: Request<Body>) -> (StatusCode, Value) {
        let response: Response<Body> = self
            .router
            .clone()
            .oneshot(request)
            .await
            .expect("the router should always respond");

        let status = response.status();
        let bytes = response
            .into_body()
            .collect()
            .await
            .expect("the body should be readable")
            .to_bytes();

        let body = if bytes.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&bytes).unwrap_or(Value::Null)
        };

        (status, body)
    }

    pub async fn get(&self, uri: &str) -> (StatusCode, Value) {
        self.send(
            Request::builder()
                .uri(uri)
                .body(Body::empty())
                .expect("the request should build"),
        )
        .await
    }

    pub async fn get_authorized(&self, uri: &str) -> (StatusCode, Value) {
        self.send(authorized("GET", uri, VALID_TOKEN, None)).await
    }

    pub async fn post(&self, uri: &str, body: Value) -> (StatusCode, Value) {
        self.send(
            Request::builder()
                .method("POST")
                .uri(uri)
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .expect("the request should build"),
        )
        .await
    }

    pub async fn authorized(
        &self,
        method: &str,
        uri: &str,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        self.send(authorized(method, uri, VALID_TOKEN, body)).await
    }

    /// Any method, no credentials, for the routes that are not behind the
    /// authentication layer.
    pub async fn request(
        &self,
        method: &str,
        uri: &str,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        let builder = Request::builder().method(method).uri(uri);

        let request = match body {
            Some(body) => builder
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .expect("the request should build"),
            None => builder
                .body(Body::empty())
                .expect("the request should build"),
        };

        self.send(request).await
    }
}

pub fn authorized(method: &str, uri: &str, token: &str, body: Option<Value>) -> Request<Body> {
    let builder = Request::builder()
        .method(method)
        .uri(uri)
        .header("authorization", format!("Bearer {token}"));

    match body {
        Some(body) => builder
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .expect("the request should build"),
        None => builder
            .body(Body::empty())
            .expect("the request should build"),
    }
}

pub fn a_domain_user() -> User {
    User {
        id: Uuid::new_v4(),
        user_name: "admin".to_owned(),
        avatar_id: String::new(),
        email: "admin@example.com".to_owned(),
        password: "$2a$10$averysecrethash".to_owned(),
        first_name: "App".to_owned(),
        last_name: "Admin".to_owned(),
        status: Status {
            id: 1,
            status: "Active".to_owned(),
        },
        department: Department {
            id: 1,
            name: "Administration".to_owned(),
            company: Company {
                id: 1,
                name: "Example Company Ltd".to_owned(),
            },
        },
        roles: vec![Role {
            role_id: 3,
            name: "Staff".to_owned(),
        }],
    }
}

/// A real server on a real socket, so the gRPC tests exercise the transport
/// the frontend server will speak rather than calling the handler in process.
///
/// Port 0 asks the OS for a free port, which is what lets tests run in
/// parallel without fighting over one.
pub struct TestServer {
    addr: SocketAddr,
    handle: JoinHandle<()>,
}

impl TestServer {
    pub async fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("a free port should be available");
        let addr = listener
            .local_addr()
            .expect("the listener should report its address");

        let services = fake_schema_services();

        let handle = tokio::spawn(async move {
            Server::builder()
                .add_service(health::service())
                .add_service(schema::service(&services))
                .serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(listener))
                .await
                .expect("the test server should serve");
        });

        Self { addr, handle }
    }

    async fn channel(&self) -> Channel {
        Channel::from_shared(format!("http://{}", self.addr))
            .expect("the address should be a valid endpoint")
            .connect_timeout(Duration::from_secs(5))
            .connect()
            .await
            .expect("the test server should accept a connection")
    }

    pub async fn schema_client(&self) -> SchemaServiceClient<Channel> {
        SchemaServiceClient::new(self.channel().await)
    }

    pub async fn health_client(&self) -> HealthServiceClient<Channel> {
        HealthServiceClient::new(self.channel().await)
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        self.handle.abort();
    }
}
