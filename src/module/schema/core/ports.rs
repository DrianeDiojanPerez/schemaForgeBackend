use async_trait::async_trait;
use uuid::Uuid;

use crate::module::schema::core::domain::{
    Comparison, Dialect, DomainError, ExchangeFormat, Report, Schema, SchemaDraft, SchemaSummary,
};
use crate::package::errdef::Error;
use crate::package::pagination::{Data, ListRequest};

/// Every schema belongs to one user, and the store only ever answers about
/// that user's own. A schema someone else owns reads as missing rather than
/// as forbidden, so a caller cannot tell it exists.
#[async_trait]
pub trait SchemaRepository: Send + Sync {
    async fn index(
        &self,
        owner: Uuid,
        request: &ListRequest,
    ) -> Result<(Vec<SchemaSummary>, i64), DomainError>;
    async fn create(&self, owner: Uuid, draft: SchemaDraft) -> Result<Schema, DomainError>;
    async fn find_by_id(&self, owner: Uuid, id: &str) -> Result<Option<Schema>, DomainError>;
    async fn find_by_name(&self, owner: Uuid, name: &str) -> Result<Option<Schema>, DomainError>;
    async fn replace(
        &self,
        owner: Uuid,
        id: &str,
        draft: SchemaDraft,
    ) -> Result<Schema, DomainError>;
    async fn delete(&self, owner: Uuid, id: &str) -> Result<(), DomainError>;
}

pub trait Verifier: Send + Sync {
    fn verify(&self, schema: &Schema) -> Report;
}

pub trait Generator: Send + Sync {
    fn dialect(&self) -> Dialect;
    fn generate(&self, schema: &Schema, options: GenerateOptions) -> String;
}

#[derive(Debug, Clone, Copy)]
pub struct GenerateOptions {
    pub include_comments: bool,
    pub include_positions: bool,
}

/// A caller who says nothing gets both, because a script that drops the
/// descriptions and the layout quietly loses the two things a diagram holds
/// that a plain database schema does not.
impl Default for GenerateOptions {
    fn default() -> Self {
        Self {
            include_comments: true,
            include_positions: true,
        }
    }
}

#[async_trait]
pub trait SchemaService: Send + Sync {
    async fn index(&self, caller: Uuid, request: ListRequest)
        -> Result<Data<SchemaSummary>, Error>;
    async fn create(&self, caller: Uuid, draft: SchemaDraft) -> Result<Schema, Error>;
    async fn find_by_id(&self, caller: Uuid, id: &str) -> Result<Schema, Error>;
    async fn replace(&self, caller: Uuid, id: &str, draft: SchemaDraft) -> Result<Schema, Error>;
    async fn delete(&self, caller: Uuid, id: &str) -> Result<(), Error>;

    /// Validates a stored schema, or an unsaved draft the canvas is holding.
    async fn validate(&self, caller: Uuid, target: ValidationTarget) -> Result<Report, Error>;

    /// Verifies before it emits. An invalid schema comes back as diagnostics
    /// with an empty script rather than as a failure.
    async fn generate_ddl(
        &self,
        caller: Uuid,
        request: GenerateRequest,
    ) -> Result<Generated, Error>;

    /// Writes a schema out as a file the canvas can hand to the browser.
    async fn export(&self, caller: Uuid, request: ExportRequest) -> Result<Exported, Error>;

    /// Reads a file back into a draft. Nothing is stored: the canvas saves
    /// what comes back through the rpcs it already uses.
    async fn import(&self, request: ImportRequest) -> Result<Imported, Error>;

    /// Measures one schema against another, which is how a case study says
    /// what a round trip through SQL actually kept.
    async fn compare(&self, caller: Uuid, request: CompareRequest) -> Result<Comparison, Error>;
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
    pub options: GenerateOptions,
}

#[derive(Debug, Clone)]
pub struct ExportRequest {
    pub target: ValidationTarget,
    pub format: ExchangeFormat,
    pub include_positions: bool,
}

#[derive(Debug, Clone)]
pub struct Exported {
    pub file_name: String,
    pub content_type: String,
    pub content: String,
}

#[derive(Debug, Clone)]
pub struct ImportRequest {
    pub content: String,
    pub format: ExchangeFormat,
    pub include_positions: bool,
}

#[derive(Debug, Clone)]
pub struct CompareRequest {
    /// The schema as it was drawn.
    pub expected: ValidationTarget,
    /// What to hold against it, usually what came back from a round trip.
    pub actual: ValidationTarget,
    pub include_positions: bool,
}

#[derive(Debug, Clone)]
pub struct Imported {
    pub draft: Schema,
    /// What the reader had to skip. A file it could only partly read still
    /// opens, with the parts it gave up on named here.
    pub report: Report,
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
