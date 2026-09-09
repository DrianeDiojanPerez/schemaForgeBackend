use std::sync::Arc;

use tonic::{Request, Response, Status};

use crate::package::auth::Auth;
use crate::package::errdef::Error;
use crate::rpc::v1;

pub struct AuthHandler {
    service: Arc<dyn Auth>,
}

impl AuthHandler {
    pub fn new(service: Arc<dyn Auth>) -> Self {
        Self { service }
    }
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
}
