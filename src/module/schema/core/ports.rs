use async_trait::async_trait;
use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::module::schema::core::domain::{
    Collaborator, Comparison, Dialect, DomainError, ExchangeFormat, Grant, Invitation, Report,
    Schema, SchemaDraft, SchemaSummary, Share, ShareRole, SharedSchema,
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

/// The schema store is keyed by owner, so a caller's standing is settled
/// here first and the owner it names is what the store is then read with.
#[async_trait]
pub trait ShareRepository: Send + Sync {
    /// Nothing when the caller has no part in the schema, so one kept
    /// private reads as missing rather than as forbidden.
    async fn grant(&self, caller: Uuid, schema_id: &str) -> Result<Option<Grant>, DomainError>;
    async fn shared_with(
        &self,
        caller: Uuid,
        request: &ListRequest,
    ) -> Result<(Vec<SharedSchema>, i64), DomainError>;
    async fn shares(&self, schema_id: &str) -> Result<Vec<Share>, DomainError>;
    async fn invitations(&self, schema_id: &str) -> Result<Vec<Invitation>, DomainError>;
    async fn create_share(
        &self,
        schema_id: &str,
        user_id: Uuid,
        role: ShareRole,
        shared_by: Uuid,
    ) -> Result<Share, DomainError>;
    async fn update_share(
        &self,
        schema_id: &str,
        user_id: Uuid,
        role: ShareRole,
    ) -> Result<Share, DomainError>;
    async fn delete_share(&self, schema_id: &str, user_id: Uuid) -> Result<(), DomainError>;
    async fn create_invitation(&self, draft: InvitationDraft) -> Result<Invitation, DomainError>;
    async fn delete_invitation(
        &self,
        schema_id: &str,
        invitation_id: Uuid,
    ) -> Result<(), DomainError>;
    async fn find_invitation_by_token(
        &self,
        token_hash: &str,
    ) -> Result<Option<Invitation>, DomainError>;
    /// The invitation is gone afterwards, so a link only works once.
    async fn accept_invitation(
        &self,
        invitation_id: Uuid,
        user_id: Uuid,
    ) -> Result<Share, DomainError>;
    /// What a sign up with the address does to every invitation still open
    /// for it.
    async fn claim_invitations(
        &self,
        email: &str,
        user_id: Uuid,
        now: DateTime<Utc>,
    ) -> Result<Vec<Share>, DomainError>;
}

#[async_trait]
pub trait UserDirectory: Send + Sync {
    async fn find_by_email(&self, email: &str) -> Result<Option<Collaborator>, DomainError>;
    async fn find_by_ids(&self, ids: &[Uuid]) -> Result<Vec<Collaborator>, DomainError>;
}

#[derive(Debug, Clone)]
pub struct InvitationDraft {
    pub schema_id: String,
    pub email: String,
    pub role: ShareRole,
    /// The digest, never the token itself.
    pub token_hash: String,
    pub invited_by: Uuid,
    pub expires_at: DateTime<Utc>,
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
