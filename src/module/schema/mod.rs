pub mod adapter;
pub mod core;
mod service;

pub use service::Services;

use std::sync::Arc;

use axum::routing::{delete, get, patch, post, put};
use axum::{middleware, Router};

use crate::module::schema::adapter::handler::{schema, sharing};
use crate::module::schema::adapter::rpc::SchemaHandler;
use crate::package::auth::Auth;
use crate::rpc::v1::schema_service_server::SchemaServiceServer;
use crate::server::middlewares::authentication::authenticate;

pub fn routes(services: &Services, auth: Arc<dyn Auth>) -> Router {
    Router::new()
        .merge(schema_routes(services))
        .merge(sharing_routes(services))
        .route_layer(middleware::from_fn_with_state(auth, authenticate))
}

fn schema_routes(services: &Services) -> Router {
    Router::new()
        .route("/v1/schemas", get(schema::index))
        .route("/v1/schemas", post(schema::create))
        .route("/v1/schemas/validate", post(schema::validate))
        .route("/v1/schemas/generate-ddl", post(schema::generate_ddl))
        .route("/v1/schemas/compare", post(schema::compare))
        .route("/v1/schemas/compare-sql", post(schema::compare_sql))
        .route("/v1/schemas/export", post(schema::export))
        .route("/v1/schemas/import", post(schema::import))
        .route("/v1/schemas/{schema-id}", get(schema::get))
        .route("/v1/schemas/{schema-id}", put(schema::replace))
        .route("/v1/schemas/{schema-id}", delete(schema::delete))
        .with_state(services.schema.clone())
}

fn sharing_routes(services: &Services) -> Router {
    Router::new()
        .route("/v1/schemas/shared-with-me", get(sharing::shared_with_me))
        .route("/v1/schemas/{schema-id}/people", get(sharing::people))
        .route("/v1/schemas/{schema-id}/shares", post(sharing::share))
        .route(
            "/v1/schemas/{schema-id}/shares/{user-id}",
            patch(sharing::change_role),
        )
        .route(
            "/v1/schemas/{schema-id}/shares/{user-id}",
            delete(sharing::revoke),
        )
        .route(
            "/v1/schemas/{schema-id}/invitations/{invitation-id}",
            delete(sharing::cancel_invitation),
        )
        .route(
            "/v1/invitations/{token}/accept",
            post(sharing::accept_invitation),
        )
        .with_state(services.sharing.clone())
}

pub fn service(services: &Services) -> SchemaServiceServer<SchemaHandler> {
    SchemaServiceServer::new(SchemaHandler::new(
        services.schema.clone(),
        services.sharing.clone(),
    ))
}
