pub mod adapter;
pub mod core;
mod service;

pub use service::Services;

use std::sync::Arc;

use axum::routing::{delete, get, post, put};
use axum::{middleware, Router};

use crate::module::schema::adapter::handler::schema;
use crate::module::schema::adapter::rpc::SchemaHandler;
use crate::package::auth::Auth;
use crate::rpc::v1::schema_service_server::SchemaServiceServer;
use crate::server::middlewares::authentication::authenticate;

pub fn routes(services: &Services, auth: Arc<dyn Auth>) -> Router {
    Router::new()
        .route("/v1/schemas", get(schema::index))
        .route("/v1/schemas", post(schema::create))
        .route("/v1/schemas/validate", post(schema::validate))
        .route("/v1/schemas/generate-ddl", post(schema::generate_ddl))
        .route("/v1/schemas/export", post(schema::export))
        .route("/v1/schemas/import", post(schema::import))
        .route("/v1/schemas/{schema-id}", get(schema::get))
        .route("/v1/schemas/{schema-id}", put(schema::replace))
        .route("/v1/schemas/{schema-id}", delete(schema::delete))
        .route_layer(middleware::from_fn_with_state(auth, authenticate))
        .with_state(services.schema.clone())
}

pub fn service(services: &Services) -> SchemaServiceServer<SchemaHandler> {
    SchemaServiceServer::new(SchemaHandler::new(services.schema.clone()))
}
