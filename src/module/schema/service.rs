use std::sync::Arc;

use crate::database::Database;
use crate::module::schema::adapter::repository::{
    PgSchemaRepository, PgShareRepository, PgUserDirectory,
};
use crate::module::schema::core::ports::{
    SchemaRepository, SchemaService, ShareRepository, SharingService, UserDirectory,
};
use crate::module::schema::core::service::{
    PostgresGenerator, SchemaServiceImpl, SchemaVerifier, SharingServiceImpl,
};
use crate::package::emailer::Emailer;

#[derive(Clone)]
pub struct Services {
    pub schema: Arc<dyn SchemaService>,
    pub sharing: Arc<dyn SharingService>,
}

impl Services {
    pub fn new(db: Arc<Database>, mailer: Arc<dyn Emailer>) -> Self {
        Self::with_repositories(
            Arc::new(PgSchemaRepository::new(db.clone())),
            Arc::new(PgShareRepository::new(db.clone())),
            Arc::new(PgUserDirectory::new(db)),
            mailer,
        )
    }

    pub fn with_repositories(
        repository: Arc<dyn SchemaRepository>,
        shares: Arc<dyn ShareRepository>,
        directory: Arc<dyn UserDirectory>,
        mailer: Arc<dyn Emailer>,
    ) -> Self {
        Self {
            schema: Arc::new(SchemaServiceImpl::new(
                repository.clone(),
                shares.clone(),
                Arc::new(SchemaVerifier),
                vec![Arc::new(PostgresGenerator)],
            )),
            sharing: Arc::new(SharingServiceImpl::new(
                repository, shares, directory, mailer,
            )),
        }
    }
}
