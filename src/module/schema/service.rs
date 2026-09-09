use std::sync::Arc;

use crate::database::Database;
use crate::module::schema::adapter::repository::PgSchemaRepository;
use crate::module::schema::core::ports::{SchemaRepository, SchemaService};
use crate::module::schema::core::service::SchemaServiceImpl;

#[derive(Clone)]
pub struct Services {
    pub schema: Arc<dyn SchemaService>,
}

impl Services {
    pub fn new(db: Arc<Database>) -> Self {
        Self::with_repository(Arc::new(PgSchemaRepository::new(db)))
    }

    pub fn with_repository(repository: Arc<dyn SchemaRepository>) -> Self {
        Self {
            schema: Arc::new(SchemaServiceImpl::new(repository)),
        }
    }
}
