mod attribute;
mod comparison;
mod data_type;
mod diagnostic;
mod dialect;
mod entity;
mod exchange;
mod relationship;
mod schema;
mod share;

pub use attribute::{Attribute, ForeignKeyRef};
pub use comparison::{Category, Comparison, Counts, Difference, DifferenceKind};
pub use data_type::{DataType, DataTypeKind, TypeFamily};
pub use diagnostic::{code as diagnostic_code, Diagnostic, Report, Severity};
pub use dialect::Dialect;
pub use entity::{Entity, Position};
pub use exchange::ExchangeFormat;
pub use relationship::{Cardinality, Relationship};
pub use schema::{Schema, SchemaDraft, SchemaSummary};
pub use share::{Access, Collaborator, Grant, Invitation, Share, ShareRole, SharedSchema};

/// Failures the store can raise. Kept separate from the transport error so the
/// core never has to name an HTTP status.
#[derive(Debug, thiserror::Error)]
pub enum DomainError {
    #[error("schema does not exist")]
    SchemaNotFound,
    #[error("a schema named `{0}` already exists")]
    DuplicateSchemaName(String),
    #[error("`{0}` already has access to this schema")]
    AlreadyShared(String),
    #[error("`{0}` has already been invited to this schema")]
    AlreadyInvited(String),
    #[error("share does not exist")]
    ShareNotFound,
    #[error("invitation does not exist")]
    InvitationNotFound,
    #[error("storage failure: {0}")]
    Storage(String),
}
