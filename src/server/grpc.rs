use std::net::SocketAddr;

use tonic::service::Routes;
use tonic::transport::Server;
use tower_http::trace::TraceLayer;

use crate::config::{AppConfig, Deployment};
use crate::module::{health, schema};
use crate::rpc::v1;
use crate::server::middlewares::rpc_context::RequestContextLayer;
use crate::server::Modules;

/// Every service the server answers on. Adding a module is one line here, the
/// same as adding a route group in the HTTP router.
fn mount(modules: &Modules) -> Routes {
    Routes::default()
        .add_service(health::service())
        .add_service(schema::service(&modules.schema))
}

/// Reflection lets grpcurl and a client generator read the contract off the
/// running server, which is worth having while the frontend is being written
/// and is not worth handing to whoever reaches the port in production.
fn serves_reflection(deployment: &Deployment) -> bool {
    !deployment.is_production()
}

pub async fn serve(
    config: &AppConfig,
    modules: &Modules,
    shutdown: impl std::future::Future<Output = ()>,
) -> anyhow::Result<()> {
    let addr = SocketAddr::from(([0, 0, 0, 0], config.server.grpc_port));

    let mut routes = mount(modules);

    if serves_reflection(&config.deployment) {
        let reflection = tonic_reflection::server::Builder::configure()
            .register_encoded_file_descriptor_set(v1::FILE_DESCRIPTOR_SET)
            .build_v1()?;

        routes = routes.add_service(reflection);

        tracing::info!(
            environment = %config.deployment.environment,
            "grpc reflection is enabled"
        );
    }

    tracing::info!(%addr, "grpc server listening");

    Server::builder()
        .layer(TraceLayer::new_for_grpc())
        .layer(RequestContextLayer)
        .add_routes(routes)
        .serve_with_shutdown(addr, shutdown)
        .await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::config::Environment;

    fn deployment(environment: Environment) -> Deployment {
        Deployment {
            name: "SchemaForgeBackend".to_owned(),
            environment,
            time_zone: "America/Belize".to_owned(),
        }
    }

    #[test]
    fn only_production_keeps_the_contract_to_itself() {
        assert!(serves_reflection(&deployment(Environment::Local)));
        assert!(serves_reflection(&deployment(Environment::Development)));
        assert!(!serves_reflection(&deployment(Environment::Production)));
    }

    /// The descriptor set is written by the build script and read back by
    /// path, so a rename in `build.rs` would only surface at start up.
    #[test]
    fn the_reflection_service_can_read_the_generated_descriptor() {
        tonic_reflection::server::Builder::configure()
            .register_encoded_file_descriptor_set(v1::FILE_DESCRIPTOR_SET)
            .build_v1()
            .expect("the descriptor set should register");
    }
}
