pub mod adapter;
pub mod core;
mod service;

pub use service::Services;

use axum::routing::{delete, get, post, put};
use axum::Router;

use crate::module::schema::adapter::handler::schema;
use crate::module::schema::adapter::rpc::SchemaHandler;
use crate::rpc::v1::schema_service_server::SchemaServiceServer;

pub fn routes(services: &Services) -> Router {
    Router::new()
        .route("/v1/schemas", get(schema::index))
        .route("/v1/schemas", post(schema::create))
        .route("/v1/schemas/validate", post(schema::validate))
        .route("/v1/schemas/generate-ddl", post(schema::generate_ddl))
        .route("/v1/schemas/{schema-id}", get(schema::get))
        .route("/v1/schemas/{schema-id}", put(schema::replace))
        .route("/v1/schemas/{schema-id}", delete(schema::delete))
        .with_state(services.schema.clone())
}

pub fn service(services: &Services) -> SchemaServiceServer<SchemaHandler> {
    SchemaServiceServer::new(SchemaHandler::new(services.schema.clone()))
}
