use axum::Router;

use crate::module::{auth, health, iam, schema};
use crate::server::{docs, Modules};

pub fn mount(modules: &Modules) -> Router {
    Router::new()
        .merge(health::routes())
        .merge(docs::routes())
        .merge(auth::routes(modules.auth.clone()))
        .merge(iam::routes(
            &modules.iam,
            modules.auth.clone(),
            modules.rbac.clone(),
        ))
        .merge(schema::routes(&modules.schema))
}
