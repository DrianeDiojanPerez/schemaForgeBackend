use async_trait::async_trait;

use crate::module::schema::core::domain::{
    Dialect, DomainError, Report, Schema, SchemaDraft, SchemaSummary,
};
use crate::package::errdef::Error;
use crate::package::pagination::{Data, ListRequest};

#[async_trait]
pub trait SchemaRepository: Send + Sync {
    async fn index(&self, request: &ListRequest) -> Result<(Vec<SchemaSummary>, i64), DomainError>;
    async fn create(&self, draft: SchemaDraft) -> Result<Schema, DomainError>;
    async fn find_by_id(&self, id: &str) -> Result<Option<Schema>, DomainError>;
    async fn find_by_name(&self, name: &str) -> Result<Option<Schema>, DomainError>;
    async fn replace(&self, id: &str, draft: SchemaDraft) -> Result<Schema, DomainError>;
    async fn delete(&self, id: &str) -> Result<(), DomainError>;
}

pub trait Verifier: Send + Sync {
    fn verify(&self, schema: &Schema) -> Report;
}

pub trait Generator: Send + Sync {
    fn dialect(&self) -> Dialect;
    fn generate(&self, schema: &Schema, include_comments: bool) -> String;
}

#[async_trait]
pub trait SchemaService: Send + Sync {
    async fn index(&self, request: ListRequest) -> Result<Data<SchemaSummary>, Error>;
    async fn create(&self, draft: SchemaDraft) -> Result<Schema, Error>;
    async fn find_by_id(&self, id: &str) -> Result<Schema, Error>;
    async fn replace(&self, id: &str, draft: SchemaDraft) -> Result<Schema, Error>;
    async fn delete(&self, id: &str) -> Result<(), Error>;

    /// Validates a stored schema, or an unsaved draft the canvas is holding.
    async fn validate(&self, target: ValidationTarget) -> Result<Report, Error>;

    /// Milestone M3: lands in weeks 7-11.
    async fn generate_ddl(&self, request: GenerateRequest) -> Result<Generated, Error>;
}

#[derive(Debug, Clone)]
pub enum ValidationTarget {
    Stored(String),
    Draft(Box<Schema>),
}

#[derive(Debug, Clone)]
pub struct GenerateRequest {
    pub target: ValidationTarget,
    pub dialect: Dialect,
    pub include_comments: bool,
}

#[derive(Debug, Clone)]
pub struct Generated {
    pub ddl: String,
    /// Generation refuses to run on an invalid schema, so a caller that skipped
    /// validation still gets the reason rather than broken SQL.
    pub report: Report,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_validation_target_is_either_stored_or_drawn() {
        let stored = ValidationTarget::Stored("s1".to_owned());

        match stored {
            ValidationTarget::Stored(id) => assert_eq!(id, "s1"),
            ValidationTarget::Draft(_) => panic!("expected a stored target"),
        }
    }
}
