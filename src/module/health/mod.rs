use axum::routing::get;
use axum::Router;
use serde::Serialize;
use tonic::{Request, Response, Status};

use crate::package::response;
use crate::rpc::v1;
use crate::rpc::v1::health_service_server::{HealthService, HealthServiceServer};

#[derive(Debug, Serialize)]
pub struct Health {
    pub status: &'static str,
    pub version: &'static str,
}

pub fn routes() -> Router {
    Router::new().route("/v1/healthcheck", get(healthcheck))
}

/// Liveness only. It answers as long as the process is serving, which is what
/// a container orchestrator restarts on. It deliberately does not touch the
/// database, so a slow query cannot get the whole service killed.
async fn healthcheck() -> axum::Json<response::Response<Health>> {
    response::ok(Health {
        status: "OK",
        version: env!("CARGO_PKG_VERSION"),
    })
}

pub struct HealthHandler;

#[tonic::async_trait]
impl HealthService for HealthHandler {
    async fn check(
        &self,
        _request: Request<v1::CheckRequest>,
    ) -> Result<Response<v1::CheckResponse>, Status> {
        Ok(Response::new(v1::CheckResponse {
            status: "OK".to_owned(),
            version: env!("CARGO_PKG_VERSION").to_owned(),
        }))
    }
}

pub fn service() -> HealthServiceServer<HealthHandler> {
    HealthServiceServer::new(HealthHandler)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn the_rpc_check_reports_ok_and_the_build_version() {
        let response = HealthHandler
            .check(Request::new(v1::CheckRequest {}))
            .await
            .expect("the check should succeed")
            .into_inner();

        assert_eq!(response.status, "OK");
        assert_eq!(response.version, env!("CARGO_PKG_VERSION"));
    }
}
