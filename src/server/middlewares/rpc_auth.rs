//! Authentication and rbac for the gRPC port. Both go through the same
//! `Auth` and `Engine` the HTTP middleware uses, so a token and a permission
//! mean the same thing on either port, and a refused call is turned away
//! before the handler runs rather than inside it.

use std::sync::Arc;
use std::task::{Context, Poll};

use axum::http::{header, HeaderMap, Request, Response};
use tonic::Status;
use tower::{Layer, Service};

use crate::package::auth::{Auth, AuthUser};
use crate::package::errdef::Error;
use crate::package::rbac::Engine;

const BEARER: &str = "Bearer ";
const SCHEMA_SERVICE: &str = "/schemaforge.v1.SchemaService/";

enum Access {
    Open,
    Requires(&'static str),
    Denied,
}

/// gRPC has no route table to hang a guard on, so the method path is the
/// route and this is the table. The health check, the reflection service and
/// the two calls that mint tokens are all that is reachable without a token.
fn access_for(path: &str) -> Access {
    let Some(method) = path.strip_prefix(SCHEMA_SERVICE) else {
        return Access::Open;
    };

    match method {
        "GetSchema" | "ListSchemas" | "ValidateSchema" | "GenerateDdl" => {
            Access::Requires("Schemas.View All")
        }
        "CreateSchema" => Access::Requires("Schemas.Create"),
        "UpdateSchema" => Access::Requires("Schemas.Update"),
        "DeleteSchema" => Access::Requires("Schemas.Delete"),
        // A method added to the service and left off this list answers denied
        // rather than open.
        _ => Access::Denied,
    }
}

async fn identify(auth: &Arc<dyn Auth>, headers: &HeaderMap) -> Result<AuthUser, Error> {
    let token = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix(BEARER))
        .filter(|token| !token.is_empty())
        .ok_or_else(|| Error::unauthorized("malformed or missing jwt token"))?;

    auth.get_identity(token)
        .await
        .map(AuthUser)
        .map_err(|err| Error::unauthorized("malformed or missing jwt token").with_cause(err))
}

/// A refused call never reaches a handler, so the status is turned into the
/// same trailers-only response tonic writes when a handler returns one.
fn refuse<B: Default>(error: Error) -> Response<B> {
    Status::from(error).into_http()
}

#[derive(Clone)]
pub struct AuthLayer {
    auth: Arc<dyn Auth>,
    rbac: Arc<dyn Engine>,
}

impl AuthLayer {
    pub fn new(auth: Arc<dyn Auth>, rbac: Arc<dyn Engine>) -> Self {
        Self { auth, rbac }
    }
}

impl<S> Layer<S> for AuthLayer {
    type Service = AuthGuard<S>;

    fn layer(&self, inner: S) -> Self::Service {
        AuthGuard {
            inner,
            auth: self.auth.clone(),
            rbac: self.rbac.clone(),
        }
    }
}

#[derive(Clone)]
pub struct AuthGuard<S> {
    inner: S,
    auth: Arc<dyn Auth>,
    rbac: Arc<dyn Engine>,
}

impl<S, ReqBody, ResBody> Service<Request<ReqBody>> for AuthGuard<S>
where
    S: Service<Request<ReqBody>, Response = Response<ResBody>> + Clone + Send + 'static,
    S::Future: Send + 'static,
    ReqBody: Send + 'static,
    ResBody: Default + Send + 'static,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<Self::Response, Self::Error>> + Send>,
    >;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, mut request: Request<ReqBody>) -> Self::Future {
        let access = access_for(request.uri().path());

        let auth = self.auth.clone();
        let rbac = self.rbac.clone();

        // The clone dance is the standard tower workaround: `self.inner` is the
        // instance that was polled ready, so the ready one is what gets called.
        let clone = self.inner.clone();
        let mut inner = std::mem::replace(&mut self.inner, clone);

        Box::pin(async move {
            let action = match access {
                Access::Open => return inner.call(request).await,
                Access::Denied => {
                    return Ok(refuse(Error::unauthorized("unauthorized action")));
                }
                Access::Requires(action) => action,
            };

            let user = match identify(&auth, request.headers()).await {
                Ok(user) => user,
                Err(error) => return Ok(refuse(error)),
            };

            if !rbac.can(user.id(), action).await {
                return Ok(refuse(Error::unauthorized("unauthorized action")));
            }

            // tonic copies these onto the `Request` the handler receives, so a
            // handler can read the caller the same way an axum one does.
            request.extensions_mut().insert(user);

            inner.call(request).await
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn action(path: &str) -> Option<&'static str> {
        match access_for(path) {
            Access::Requires(action) => Some(action),
            _ => None,
        }
    }

    #[test]
    fn the_tokenless_calls_are_the_ones_that_hand_out_tokens() {
        assert!(matches!(
            access_for("/schemaforge.v1.HealthService/Check"),
            Access::Open
        ));
        assert!(matches!(
            access_for("/schemaforge.v1.AuthService/Login"),
            Access::Open
        ));
        assert!(matches!(
            access_for("/schemaforge.v1.AuthService/RefreshToken"),
            Access::Open
        ));
    }

    #[test]
    fn reading_a_schema_and_changing_one_are_not_the_same_permission() {
        assert_eq!(
            action("/schemaforge.v1.SchemaService/ListSchemas"),
            Some("Schemas.View All")
        );
        assert_eq!(
            action("/schemaforge.v1.SchemaService/GetSchema"),
            Some("Schemas.View All")
        );
        assert_eq!(
            action("/schemaforge.v1.SchemaService/ValidateSchema"),
            Some("Schemas.View All")
        );
        assert_eq!(
            action("/schemaforge.v1.SchemaService/GenerateDdl"),
            Some("Schemas.View All")
        );
        assert_eq!(
            action("/schemaforge.v1.SchemaService/CreateSchema"),
            Some("Schemas.Create")
        );
        assert_eq!(
            action("/schemaforge.v1.SchemaService/UpdateSchema"),
            Some("Schemas.Update")
        );
        assert_eq!(
            action("/schemaforge.v1.SchemaService/DeleteSchema"),
            Some("Schemas.Delete")
        );
    }

    #[test]
    fn an_unlisted_schema_method_is_closed() {
        assert!(matches!(
            access_for("/schemaforge.v1.SchemaService/RenameEverything"),
            Access::Denied
        ));
    }
}
