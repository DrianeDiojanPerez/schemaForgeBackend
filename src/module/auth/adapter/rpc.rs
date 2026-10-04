use std::sync::Arc;

use tonic::{Request, Response, Status};

use crate::package::auth::{Auth, AuthUser};
use crate::package::errdef::Error;
use crate::package::rbac::{Engine, Permission};
use crate::rpc::v1;

pub struct AuthHandler {
    service: Arc<dyn Auth>,
    rbac: Arc<dyn Engine>,
}

impl AuthHandler {
    pub fn new(service: Arc<dyn Auth>, rbac: Arc<dyn Engine>) -> Self {
        Self { service, rbac }
    }
}

/// Two resources in one module can carry the same permission name, and the
/// frontend asks for the module and the name only, so what reads as one
/// permission there is sent once.
fn granted(permissions: Vec<Permission>) -> Vec<v1::Permission> {
    let mut granted: Vec<v1::Permission> = permissions
        .into_iter()
        .map(|permission| v1::Permission {
            module: permission.module,
            name: permission.name,
        })
        .collect();

    granted.sort_by(|a, b| (&a.module, &a.name).cmp(&(&b.module, &b.name)));
    granted.dedup_by(|a, b| a.module == b.module && a.name == b.name);

    granted
}

/// The HTTP side gets this from `validator` on the request struct. Protobuf
/// has no such attribute, so the same rule is spelled out here.
fn required(field: &str, value: &str, error: &mut Error) {
    if value.trim().is_empty() {
        error.push_violation(field, "field is required and cannot be empty");
    }
}

#[tonic::async_trait]
impl v1::auth_service_server::AuthService for AuthHandler {
    #[tracing::instrument(name = "AuthService.Login", skip_all)]
    async fn login(
        &self,
        request: Request<v1::LoginRequest>,
    ) -> Result<Response<v1::LoginResponse>, Status> {
        let request = request.into_inner();

        let mut error = Error::validation("failed payload validation");
        required("email", &request.email, &mut error);
        required("password", &request.password, &mut error);

        if error.has_violations() {
            return Err(error.into());
        }

        let tokens = self
            .service
            .generate_token(&request.email, &request.password)
            .await?;

        Ok(Response::new(v1::LoginResponse {
            token: tokens.token,
            refresh_token: tokens.refresh_token,
        }))
    }

    #[tracing::instrument(name = "AuthService.RefreshToken", skip_all)]
    async fn refresh_token(
        &self,
        request: Request<v1::RefreshTokenRequest>,
    ) -> Result<Response<v1::RefreshTokenResponse>, Status> {
        let request = request.into_inner();

        let mut error = Error::validation("failed payload validation");
        required("refresh_token", &request.refresh_token, &mut error);

        if error.has_violations() {
            return Err(error.into());
        }

        let tokens = self.service.refresh_token(&request.refresh_token).await?;

        Ok(Response::new(v1::RefreshTokenResponse {
            token: tokens.token,
            refresh_token: tokens.refresh_token,
        }))
    }

    #[tracing::instrument(name = "AuthService.GoogleLoginUrl", skip_all)]
    async fn google_login_url(
        &self,
        request: Request<v1::GoogleLoginUrlRequest>,
    ) -> Result<Response<v1::GoogleLoginUrlResponse>, Status> {
        let request = request.into_inner();

        Ok(Response::new(v1::GoogleLoginUrlResponse {
            url: self.service.google_login_url(&request.state),
        }))
    }

    #[tracing::instrument(name = "AuthService.GetCurrentUser", skip_all)]
    async fn get_current_user(
        &self,
        request: Request<v1::GetCurrentUserRequest>,
    ) -> Result<Response<v1::GetCurrentUserResponse>, Status> {
        // The guard put the caller here after it read the token, so a request
        // without one never reaches this far.
        let user = request
            .extensions()
            .get::<AuthUser>()
            .ok_or_else(|| Status::from(Error::unauthorized("malformed or missing jwt token")))?;

        let identity = &user.0;

        Ok(Response::new(v1::GetCurrentUserResponse {
            id: identity.id.to_string(),
            email: identity.email.clone(),
            name: identity.display_name(),
            avatar_url: identity.avatar_url.clone().unwrap_or_default(),
            roles: identity.roles.clone(),
            permissions: granted(self.rbac.permissions_of(identity.id).await),
        }))
    }

    #[tracing::instrument(name = "AuthService.LoginWithGoogle", skip_all)]
    async fn login_with_google(
        &self,
        request: Request<v1::LoginWithGoogleRequest>,
    ) -> Result<Response<v1::LoginResponse>, Status> {
        let request = request.into_inner();

        let mut error = Error::validation("failed payload validation");
        required("code", &request.code, &mut error);

        if error.has_violations() {
            return Err(error.into());
        }

        let tokens = self.service.login_with_google(&request.code).await?;

        Ok(Response::new(v1::LoginResponse {
            token: tokens.token,
            refresh_token: tokens.refresh_token,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn permission(module: &str, resource: &str, name: &str) -> Permission {
        Permission {
            module: module.to_owned(),
            resource: resource.to_owned(),
            name: name.to_owned(),
        }
    }

    #[test]
    fn orders_the_permissions_by_module_then_name() {
        let granted = granted(vec![
            permission("Schema Module", "Schemas", "View All"),
            permission("Access & Identity Module", "Users", "Create"),
            permission("Schema Module", "Schemas", "Create"),
        ]);

        let read: Vec<_> = granted
            .iter()
            .map(|p| (p.module.as_str(), p.name.as_str()))
            .collect();

        assert_eq!(
            read,
            vec![
                ("Access & Identity Module", "Create"),
                ("Schema Module", "Create"),
                ("Schema Module", "View All"),
            ]
        );
    }

    #[test]
    fn the_same_name_in_one_module_is_sent_once() {
        let granted = granted(vec![
            permission("Access & Identity Module", "Users", "View All"),
            permission("Access & Identity Module", "Roles", "View All"),
        ]);

        assert_eq!(granted.len(), 1);
    }
}
