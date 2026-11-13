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
    Access, Collaborator, DomainError, Grant, Invitation, Schema, SchemaDraft, SchemaSummary,
    Share, ShareRole, SharedSchema,
};
use schemaforge_backend::module::schema::core::ports::{
    InvitationDraft, SchemaRepository, ShareRepository, UserDirectory,
};
use schemaforge_backend::module::{auth, health, iam, schema};
use schemaforge_backend::package::auth::{Auth, AuthenticationTokens, Identity, Registration};
use schemaforge_backend::package::emailer::{Emailer, EmailerError};
use schemaforge_backend::package::errdef::Error;
use schemaforge_backend::package::pagination::{Data, ListRequest};
use schemaforge_backend::package::rbac::{Engine, Permission as GrantedPermission};
use schemaforge_backend::rpc::v1::auth_service_client::AuthServiceClient;
use schemaforge_backend::rpc::v1::health_service_client::HealthServiceClient;
use schemaforge_backend::rpc::v1::schema_service_client::SchemaServiceClient;
use schemaforge_backend::server::middlewares::rpc_auth::AuthLayer;
use schemaforge_backend::server::{self, Modules};
use std::net::SocketAddr;
use std::time::Duration;
use tokio::net::TcpListener;
use tokio::task::JoinHandle;
use tonic::service::interceptor::InterceptedService;
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

/// A sign up is answered with a token that names the new user, so the
/// identity lookup can hand the same person back on the next call.
const SIGNED_UP: &str = "signed-up:";

pub struct FakeAuth {
    pub user: Mutex<Identity>,
    pub signed_up: Mutex<Vec<Identity>>,
}

impl FakeAuth {
    pub fn for_user(user: Identity) -> Self {
        Self {
            user: Mutex::new(user),
            signed_up: Mutex::default(),
        }
    }

    /// What Google would answer for a code: email, name, picture.
    fn google_account(&self, code: &str) -> Option<(String, &'static str, &'static str)> {
        match code {
            "a-valid-google-code" => Some((
                self.user.lock().unwrap().email.clone(),
                "Admin From Google",
                "https://photo.test/google-admin",
            )),
            "a-google-code-for-a-stranger" => Some((
                " Stranger@Example.com ".to_owned(),
                "Sam Stranger",
                "https://photo.test/sam",
            )),
            _ => None,
        }
    }
}

fn split_name(name: &str) -> (String, String) {
    name.split_once(' ')
        .map(|(first, last)| (first.to_owned(), last.to_owned()))
        .unwrap_or((name.to_owned(), String::new()))
}

fn a_sign_up(email: &str, first_name: String, last_name: String) -> Identity {
    Identity {
        id: Uuid::new_v4(),
        email: email.to_owned(),
        user_name: email.to_owned(),
        first_name,
        last_name,
        avatar_url: None,
        password: String::new(),
        roles: vec!["Developer".to_owned()],
    }
}

fn signed_up_tokens(email: &str) -> AuthenticationTokens {
    AuthenticationTokens {
        token: format!("{SIGNED_UP}{email}"),
        refresh_token: "a-valid-refresh-token".to_owned(),
    }
}

#[async_trait]
impl Auth for FakeAuth {
    async fn generate_token(
        &self,
        email: &str,
        password: &str,
    ) -> Result<AuthenticationTokens, Error> {
        if email == self.user.lock().unwrap().email && password == "password" {
            return Ok(AuthenticationTokens {
                token: VALID_TOKEN.to_owned(),
                refresh_token: "a-valid-refresh-token".to_owned(),
            });
        }

        Err(Error::unauthorized("invalid username or password"))
    }

    async fn register(&self, registration: Registration) -> Result<AuthenticationTokens, Error> {
        if registration.password.chars().count() < 8 {
            return Err(Error::validation("password must be at least 8 characters")
                .add_violation("password", "field must be a minimum length of 8"));
        }

        let email = registration.email.trim().to_lowercase();
        let mut signed_up = self.signed_up.lock().unwrap();

        if email == self.user.lock().unwrap().email
            || signed_up.iter().any(|user| user.email == email)
        {
            return Err(Error::conflict("an account with this email already exists"));
        }

        let (first_name, last_name) = split_name(&registration.name);
        signed_up.push(a_sign_up(&email, first_name, last_name));

        Ok(signed_up_tokens(&email))
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

    fn google_login_url(&self, state: &str) -> String {
        format!("https://accounts.google.test/consent?state={state}")
    }

    async fn login_with_google(&self, code: &str) -> Result<AuthenticationTokens, Error> {
        let (email, name, picture) = self
            .google_account(code)
            .ok_or_else(|| Error::unauthorized("google rejected the sign in code"))?;

        let email = email.trim().to_lowercase();
        let (first_name, last_name) = split_name(name);

        let mut user = self.user.lock().unwrap();
        if user.email == email {
            user.first_name = first_name;
            user.last_name = last_name;
            user.avatar_url = Some(picture.to_owned());

            return Ok(AuthenticationTokens {
                token: VALID_TOKEN.to_owned(),
                refresh_token: "a-valid-refresh-token".to_owned(),
            });
        }

        let mut signed_up = self.signed_up.lock().unwrap();
        match signed_up.iter_mut().find(|user| user.email == email) {
            Some(user) => {
                user.first_name = first_name;
                user.last_name = last_name;
                user.avatar_url = Some(picture.to_owned());
            }
            None => {
                let mut user = a_sign_up(&email, first_name, last_name);
                user.avatar_url = Some(picture.to_owned());
                signed_up.push(user);
            }
        }

        Ok(signed_up_tokens(&email))
    }

    async fn get_identity(&self, access_token: &str) -> Result<Identity, Error> {
        if access_token == VALID_TOKEN {
            return Ok(self.user.lock().unwrap().clone());
        }

        if let Some(email) = access_token.strip_prefix(SIGNED_UP) {
            if let Some(user) = self
                .signed_up
                .lock()
                .unwrap()
                .iter()
                .find(|user| user.email == email)
            {
                return Ok(user.clone());
            }
        }

        Err(Error::unauthorized("invalid or malformed refresh token"))
    }

    async fn password_recovery(&self, email: &str, _callback_uri: &str) -> Result<(), Error> {
        if email == self.user.lock().unwrap().email {
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

    async fn permissions_of(&self, _user_id: Uuid) -> Vec<GrantedPermission> {
        self.allowed
            .iter()
            .filter_map(|allowed| allowed.split_once('.'))
            .map(|(resource, name)| GrantedPermission {
                module: format!("{resource} Module"),
                resource: resource.to_owned(),
                name: name.to_owned(),
            })
            .collect()
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
/// Like the real one it remembers who owns each schema and answers a caller
/// only about their own.
///
/// Persistence itself is covered in `tests/postgres.rs`.
#[derive(Default)]
pub struct FakeSchemaRepository {
    schemas: Mutex<HashMap<String, (Uuid, Schema)>>,
    shares: Mutex<Vec<Share>>,
    /// Each invitation beside the digest of its token.
    invitations: Mutex<Vec<(String, Invitation)>>,
}

impl FakeSchemaRepository {
    fn read(&self) -> std::sync::MutexGuard<'_, HashMap<String, (Uuid, Schema)>> {
        self.schemas.lock().expect("the store lock should be sound")
    }

    fn owner_of(&self, id: &str) -> Option<Uuid> {
        self.read().get(id).map(|(by, _)| *by)
    }

    fn share_of(&self, schema_id: &str, user_id: Uuid) -> Option<Share> {
        self.shares
            .lock()
            .unwrap()
            .iter()
            .find(|share| share.schema_id == schema_id && share.user_id == user_id)
            .cloned()
    }

    fn write_share(
        &self,
        schema_id: &str,
        user_id: Uuid,
        role: ShareRole,
        shared_by: Uuid,
    ) -> Share {
        let mut shares = self.shares.lock().unwrap();

        if let Some(share) = shares
            .iter_mut()
            .find(|share| share.schema_id == schema_id && share.user_id == user_id)
        {
            share.role = role;
            share.updated_at = Utc::now();
            return share.clone();
        }

        let share = Share {
            schema_id: schema_id.to_owned(),
            user_id,
            role,
            shared_by,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        shares.push(share.clone());

        share
    }

    fn take_invitation(&self, id: Uuid) -> Option<Invitation> {
        let mut invitations = self.invitations.lock().unwrap();
        let index = invitations
            .iter()
            .position(|(_, invitation)| invitation.id == id)?;

        Some(invitations.remove(index).1)
    }

    pub fn token_of(&self, invitation_id: Uuid) -> Option<String> {
        self.invitations
            .lock()
            .unwrap()
            .iter()
            .find(|(_, invitation)| invitation.id == invitation_id)
            .map(|(token, _)| token.clone())
    }

    fn owned_by(schemas: &HashMap<String, (Uuid, Schema)>, owner: Uuid) -> Vec<&Schema> {
        let mut mine: Vec<&Schema> = schemas
            .values()
            .filter(|(by, _)| *by == owner)
            .map(|(_, schema)| schema)
            .collect();
        mine.sort_by(|a, b| a.created_at.cmp(&b.created_at).then(a.id.cmp(&b.id)));
        mine
    }

    fn name_taken(
        schemas: &HashMap<String, (Uuid, Schema)>,
        owner: Uuid,
        name: &str,
        except: &str,
    ) -> bool {
        Self::owned_by(schemas, owner)
            .into_iter()
            .any(|schema| schema.id != except && schema.name.eq_ignore_ascii_case(name))
    }
}

#[async_trait]
impl SchemaRepository for FakeSchemaRepository {
    async fn index(
        &self,
        owner: Uuid,
        request: &ListRequest,
    ) -> Result<(Vec<SchemaSummary>, i64), DomainError> {
        let schemas = self.read();
        let mine = Self::owned_by(&schemas, owner);
        let total = mine.len() as i64;

        let page = mine
            .into_iter()
            .skip(request.offset() as usize)
            .take(request.per_page as usize)
            .map(Schema::summary)
            .collect();

        Ok((page, total))
    }

    async fn create(&self, owner: Uuid, draft: SchemaDraft) -> Result<Schema, DomainError> {
        let mut schemas = self.read();

        if Self::name_taken(&schemas, owner, &draft.name, "") {
            return Err(DomainError::DuplicateSchemaName(draft.name));
        }

        let schema = Schema::new(Uuid::new_v4().to_string(), draft, Utc::now());
        schemas.insert(schema.id.clone(), (owner, schema.clone()));

        Ok(schema)
    }

    async fn find_by_id(&self, owner: Uuid, id: &str) -> Result<Option<Schema>, DomainError> {
        Ok(self
            .read()
            .get(id)
            .filter(|(by, _)| *by == owner)
            .map(|(_, schema)| schema.clone()))
    }

    async fn find_by_name(&self, owner: Uuid, name: &str) -> Result<Option<Schema>, DomainError> {
        Ok(Self::owned_by(&self.read(), owner)
            .into_iter()
            .find(|schema| schema.name.eq_ignore_ascii_case(name))
            .cloned())
    }

    async fn replace(
        &self,
        owner: Uuid,
        id: &str,
        draft: SchemaDraft,
    ) -> Result<Schema, DomainError> {
        let mut schemas = self.read();

        if Self::name_taken(&schemas, owner, &draft.name, id) {
            return Err(DomainError::DuplicateSchemaName(draft.name));
        }

        let (_, schema) = schemas
            .get_mut(id)
            .filter(|(by, _)| *by == owner)
            .ok_or(DomainError::SchemaNotFound)?;
        schema.replace_with(draft, Utc::now());

        Ok(schema.clone())
    }

    async fn delete(&self, owner: Uuid, id: &str) -> Result<(), DomainError> {
        let mut schemas = self.read();

        if !schemas.get(id).is_some_and(|(by, _)| *by == owner) {
            return Err(DomainError::SchemaNotFound);
        }

        schemas.remove(id);

        Ok(())
    }
}

#[async_trait]
impl ShareRepository for FakeSchemaRepository {
    async fn grant(&self, caller: Uuid, schema_id: &str) -> Result<Option<Grant>, DomainError> {
        let Some(owner) = self.owner_of(schema_id) else {
            return Ok(None);
        };

        if owner == caller {
            return Ok(Some(Grant {
                owner,
                access: Access::Owner,
            }));
        }

        Ok(self.share_of(schema_id, caller).map(|share| Grant {
            owner,
            access: Access::Shared(share.role),
        }))
    }

    async fn shared_with(
        &self,
        caller: Uuid,
        request: &ListRequest,
    ) -> Result<(Vec<SharedSchema>, i64), DomainError> {
        let schemas = self.read();
        let mut mine: Vec<(Share, SharedSchema)> = self
            .shares
            .lock()
            .unwrap()
            .iter()
            .filter(|share| share.user_id == caller)
            .filter_map(|share| {
                let (owner, schema) = schemas.get(&share.schema_id)?;

                Some((
                    share.clone(),
                    SharedSchema {
                        summary: schema.summary(),
                        role: share.role,
                        owner_id: *owner,
                    },
                ))
            })
            .collect();
        mine.sort_by(|a, b| {
            a.0.created_at
                .cmp(&b.0.created_at)
                .then(a.0.schema_id.cmp(&b.0.schema_id))
        });
        let total = mine.len() as i64;

        Ok((
            mine.into_iter()
                .map(|(_, shared)| shared)
                .skip(request.offset() as usize)
                .take(request.per_page as usize)
                .collect(),
            total,
        ))
    }

    async fn shares(&self, schema_id: &str) -> Result<Vec<Share>, DomainError> {
        Ok(self
            .shares
            .lock()
            .unwrap()
            .iter()
            .filter(|share| share.schema_id == schema_id)
            .cloned()
            .collect())
    }

    async fn invitations(&self, schema_id: &str) -> Result<Vec<Invitation>, DomainError> {
        Ok(self
            .invitations
            .lock()
            .unwrap()
            .iter()
            .filter(|(_, invitation)| invitation.schema_id == schema_id)
            .map(|(_, invitation)| invitation.clone())
            .collect())
    }

    async fn create_share(
        &self,
        schema_id: &str,
        user_id: Uuid,
        role: ShareRole,
        shared_by: Uuid,
    ) -> Result<Share, DomainError> {
        if self.share_of(schema_id, user_id).is_some() {
            return Err(DomainError::AlreadyShared(user_id.to_string()));
        }

        Ok(self.write_share(schema_id, user_id, role, shared_by))
    }

    async fn update_share(
        &self,
        schema_id: &str,
        user_id: Uuid,
        role: ShareRole,
    ) -> Result<Share, DomainError> {
        let shared_by = self
            .share_of(schema_id, user_id)
            .ok_or(DomainError::ShareNotFound)?
            .shared_by;

        Ok(self.write_share(schema_id, user_id, role, shared_by))
    }

    async fn delete_share(&self, schema_id: &str, user_id: Uuid) -> Result<(), DomainError> {
        let mut shares = self.shares.lock().unwrap();
        let before = shares.len();
        shares.retain(|share| !(share.schema_id == schema_id && share.user_id == user_id));

        if shares.len() == before {
            return Err(DomainError::ShareNotFound);
        }

        Ok(())
    }

    async fn create_invitation(&self, draft: InvitationDraft) -> Result<Invitation, DomainError> {
        let mut invitations = self.invitations.lock().unwrap();

        if invitations.iter().any(|(_, invitation)| {
            invitation.schema_id == draft.schema_id && invitation.email == draft.email
        }) {
            return Err(DomainError::AlreadyInvited(draft.email));
        }

        let invitation = Invitation {
            id: Uuid::new_v4(),
            schema_id: draft.schema_id,
            email: draft.email,
            role: draft.role,
            invited_by: draft.invited_by,
            expires_at: draft.expires_at,
            created_at: Utc::now(),
        };
        invitations.push((draft.token_hash, invitation.clone()));

        Ok(invitation)
    }

    async fn delete_invitation(
        &self,
        schema_id: &str,
        invitation_id: Uuid,
    ) -> Result<(), DomainError> {
        let mut invitations = self.invitations.lock().unwrap();
        let before = invitations.len();
        invitations.retain(|(_, invitation)| {
            !(invitation.schema_id == schema_id && invitation.id == invitation_id)
        });

        if invitations.len() == before {
            return Err(DomainError::InvitationNotFound);
        }

        Ok(())
    }

    async fn find_invitation_by_token(
        &self,
        token_hash: &str,
    ) -> Result<Option<Invitation>, DomainError> {
        Ok(self
            .invitations
            .lock()
            .unwrap()
            .iter()
            .find(|(token, _)| token == token_hash)
            .map(|(_, invitation)| invitation.clone()))
    }

    async fn accept_invitation(
        &self,
        invitation_id: Uuid,
        user_id: Uuid,
    ) -> Result<Share, DomainError> {
        let invitation = self
            .take_invitation(invitation_id)
            .ok_or(DomainError::InvitationNotFound)?;

        Ok(self.write_share(
            &invitation.schema_id,
            user_id,
            invitation.role,
            invitation.invited_by,
        ))
    }

    async fn claim_invitations(
        &self,
        email: &str,
        user_id: Uuid,
        now: chrono::DateTime<Utc>,
    ) -> Result<Vec<Share>, DomainError> {
        let waiting: Vec<Invitation> = self
            .invitations
            .lock()
            .unwrap()
            .iter()
            .filter(|(_, invitation)| invitation.email.eq_ignore_ascii_case(email))
            .map(|(_, invitation)| invitation.clone())
            .collect();

        Ok(waiting
            .into_iter()
            .filter_map(|invitation| self.take_invitation(invitation.id))
            .filter(|invitation| !invitation.is_expired_at(now))
            .map(|invitation| {
                self.write_share(
                    &invitation.schema_id,
                    user_id,
                    invitation.role,
                    invitation.invited_by,
                )
            })
            .collect())
    }
}

/// Answered off the fake the sign ups land in, so a person who signed up
/// can be shared with by address.
pub struct FakeDirectory {
    auth: Arc<FakeAuth>,
}

impl FakeDirectory {
    fn everyone(&self) -> Vec<Collaborator> {
        let mut people = vec![self.auth.user.lock().unwrap().clone()];
        people.extend(self.auth.signed_up.lock().unwrap().iter().cloned());

        people
            .into_iter()
            .map(|identity| Collaborator {
                id: identity.id,
                email: identity.email.clone(),
                name: identity.display_name(),
                avatar_url: identity.avatar_url.clone(),
            })
            .collect()
    }
}

#[async_trait]
impl UserDirectory for FakeDirectory {
    async fn find_by_email(&self, email: &str) -> Result<Option<Collaborator>, DomainError> {
        Ok(self
            .everyone()
            .into_iter()
            .find(|person| person.email.eq_ignore_ascii_case(email)))
    }

    async fn find_by_ids(&self, ids: &[Uuid]) -> Result<Vec<Collaborator>, DomainError> {
        Ok(self
            .everyone()
            .into_iter()
            .filter(|person| ids.contains(&person.id))
            .collect())
    }
}

#[derive(Debug, Clone)]
pub struct SentMail {
    pub to: String,
    pub subject: String,
    pub template: String,
    pub data: HashMap<String, String>,
}

impl SentMail {
    pub fn token(&self, callback_uri: &str) -> String {
        self.data["callbackURI"]
            .strip_prefix(callback_uri)
            .expect("the link should start with the callback uri")
            .to_owned()
    }
}

#[derive(Default)]
pub struct RecordingMailer {
    pub sent: Mutex<Vec<SentMail>>,
}

impl RecordingMailer {
    pub fn last(&self) -> SentMail {
        self.sent
            .lock()
            .unwrap()
            .last()
            .cloned()
            .expect("a mail should have been sent")
    }
}

#[async_trait]
impl Emailer for RecordingMailer {
    async fn send_html(
        &self,
        to: &str,
        subject: &str,
        template_name: &str,
        data: HashMap<String, String>,
    ) -> Result<(), EmailerError> {
        self.sent.lock().unwrap().push(SentMail {
            to: to.to_owned(),
            subject: subject.to_owned(),
            template: template_name.to_owned(),
            data,
        });

        Ok(())
    }
}

pub fn fake_schema_services(auth: Arc<FakeAuth>, mailer: Arc<RecordingMailer>) -> schema::Services {
    let repository = Arc::new(FakeSchemaRepository::default());

    schema::Services::with_repositories(
        repository.clone(),
        repository,
        Arc::new(FakeDirectory { auth }),
        mailer,
    )
}

pub struct TestApp {
    pub router: Router,
    pub calls: Arc<Calls>,
    pub user: Identity,
    pub mail: Arc<RecordingMailer>,
    auth: Arc<FakeAuth>,
}

impl TestApp {
    pub fn with_permissions(allowed: &[&str]) -> Self {
        let user = a_domain_user();
        let identity = Identity {
            id: user.id,
            email: user.email.clone(),
            user_name: user.user_name.clone(),
            first_name: user.first_name.clone(),
            last_name: user.last_name.clone(),
            avatar_url: Some("https://photo.test/admin".to_owned()),
            password: String::new(),
            roles: vec!["Staff".to_owned()],
        };

        let calls = Arc::new(Calls::default());
        let auth = Arc::new(FakeAuth::for_user(identity.clone()));
        let mail = Arc::new(RecordingMailer::default());

        let modules = Modules {
            auth: auth.clone(),
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
                        module: "Access & Identity Module".to_owned(),
                    }],
                }),
            },
            schema: fake_schema_services(auth.clone(), mail.clone()),
        };

        Self {
            router: server::router(&modules),
            calls,
            user: identity,
            mail,
            auth,
        }
    }

    pub fn new() -> Self {
        Self::with_permissions(&["Users.View All"])
    }

    /// Signs a second person up and hands back their token, for the tests
    /// about what one person can see of another's.
    pub async fn another_user(&self) -> String {
        another_user(&self.auth).await
    }

    pub async fn sign_up(&self, email: &str, name: &str) -> String {
        sign_up(&self.auth, email, name).await
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

async fn another_user(auth: &FakeAuth) -> String {
    sign_up(auth, "stranger@example.com", "Sam Stranger").await
}

async fn sign_up(auth: &FakeAuth, email: &str, name: &str) -> String {
    auth.register(Registration {
        name: name.to_owned(),
        email: email.to_owned(),
        password: "a-long-enough-password".to_owned(),
    })
    .await
    .expect("the sign up should succeed")
    .token
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
    pub calls: Arc<Calls>,
    pub user: Identity,
    pub mail: Arc<RecordingMailer>,
    auth: Arc<FakeAuth>,
}

pub type Authorized = InterceptedService<
    Channel,
    fn(tonic::Request<()>) -> Result<tonic::Request<()>, tonic::Status>,
>;

fn bearer(mut request: tonic::Request<()>) -> Result<tonic::Request<()>, tonic::Status> {
    request.metadata_mut().insert(
        "authorization",
        format!("Bearer {VALID_TOKEN}")
            .parse()
            .expect("the token should be a valid header value"),
    );

    Ok(request)
}

impl TestServer {
    pub async fn start() -> Self {
        Self::with_permissions(&[
            "Schemas.View All",
            "Schemas.Create",
            "Schemas.Update",
            "Schemas.Delete",
        ])
        .await
    }

    pub async fn with_permissions(allowed: &[&str]) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("a free port should be available");
        let addr = listener
            .local_addr()
            .expect("the listener should report its address");

        let user = a_domain_user();
        let identity = Identity {
            id: user.id,
            email: user.email.clone(),
            user_name: user.user_name.clone(),
            first_name: user.first_name.clone(),
            last_name: user.last_name.clone(),
            avatar_url: Some("https://photo.test/admin".to_owned()),
            password: String::new(),
            roles: vec!["Staff".to_owned()],
        };

        let calls = Arc::new(Calls::default());

        let fake_auth = Arc::new(FakeAuth::for_user(identity.clone()));
        let auth: Arc<dyn Auth> = fake_auth.clone();
        let rbac: Arc<dyn Engine> = Arc::new(FakeRbac {
            allowed: allowed.iter().map(|action| (*action).to_owned()).collect(),
            calls: calls.clone(),
        });

        let mail = Arc::new(RecordingMailer::default());
        let services = fake_schema_services(fake_auth.clone(), mail.clone());
        let layer = AuthLayer::new(auth.clone(), rbac.clone());

        let handle = tokio::spawn(async move {
            Server::builder()
                .layer(layer)
                .add_service(health::service())
                .add_service(auth::service(auth, rbac.clone()))
                .add_service(schema::service(&services))
                .serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(listener))
                .await
                .expect("the test server should serve");
        });

        Self {
            addr,
            handle,
            calls,
            user: identity,
            mail,
            auth: fake_auth,
        }
    }

    /// Signs a second person up and hands back their token, for the tests
    /// about what one person can see of another's.
    pub async fn another_user(&self) -> String {
        another_user(&self.auth).await
    }

    pub async fn sign_up(&self, email: &str, name: &str) -> String {
        sign_up(&self.auth, email, name).await
    }

    async fn channel(&self) -> Channel {
        Channel::from_shared(format!("http://{}", self.addr))
            .expect("the address should be a valid endpoint")
            .connect_timeout(Duration::from_secs(5))
            .connect()
            .await
            .expect("the test server should accept a connection")
    }

    pub async fn schema_client(&self) -> SchemaServiceClient<Authorized> {
        SchemaServiceClient::with_interceptor(self.channel().await, bearer as _)
    }

    /// A schema client that signs with a token the server just handed out.
    pub async fn schema_client_with(
        &self,
        token: &str,
    ) -> SchemaServiceClient<
        InterceptedService<
            Channel,
            impl FnMut(tonic::Request<()>) -> Result<tonic::Request<()>, tonic::Status>,
        >,
    > {
        let value: tonic::metadata::MetadataValue<_> = format!("Bearer {token}")
            .parse()
            .expect("the header should build");

        SchemaServiceClient::with_interceptor(
            self.channel().await,
            move |mut request: tonic::Request<()>| {
                request
                    .metadata_mut()
                    .insert("authorization", value.clone());
                Ok(request)
            },
        )
    }

    pub async fn anonymous_schema_client(&self) -> SchemaServiceClient<Channel> {
        SchemaServiceClient::new(self.channel().await)
    }

    pub async fn health_client(&self) -> HealthServiceClient<Channel> {
        HealthServiceClient::new(self.channel().await)
    }

    pub async fn auth_client(&self) -> AuthServiceClient<Channel> {
        AuthServiceClient::new(self.channel().await)
    }

    pub async fn authorized_auth_client(&self) -> AuthServiceClient<Authorized> {
        AuthServiceClient::with_interceptor(self.channel().await, bearer as _)
    }

    /// A client that signs with a token the server just handed out, for the
    /// calls that follow a sign up.
    pub async fn auth_client_with(
        &self,
        token: &str,
    ) -> AuthServiceClient<
        InterceptedService<
            Channel,
            impl FnMut(tonic::Request<()>) -> Result<tonic::Request<()>, tonic::Status>,
        >,
    > {
        let value: tonic::metadata::MetadataValue<_> = format!("Bearer {token}")
            .parse()
            .expect("the header should build");

        AuthServiceClient::with_interceptor(
            self.channel().await,
            move |mut request: tonic::Request<()>| {
                request
                    .metadata_mut()
                    .insert("authorization", value.clone());
                Ok(request)
            },
        )
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        self.handle.abort();
    }
}
