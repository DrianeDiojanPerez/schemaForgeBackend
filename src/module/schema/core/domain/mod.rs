mod attribute;
mod data_type;
mod diagnostic;
mod dialect;
mod entity;
mod relationship;
mod schema;

pub use attribute::{Attribute, ForeignKeyRef};
pub use data_type::{DataType, DataTypeKind, TypeFamily};
pub use diagnostic::{code as diagnostic_code, Diagnostic, Report, Severity};
pub use dialect::Dialect;
pub use entity::{Entity, Position};
pub use relationship::{Cardinality, Relationship};
pub use schema::{Schema, SchemaDraft, SchemaSummary};

/// Failures the store can raise. Kept separate from the transport error so the
/// core never has to name an HTTP status.
#[derive(Debug, thiserror::Error)]
pub enum DomainError {
    #[error("schema does not exist")]
    SchemaNotFound,
    #[error("a schema named `{0}` already exists")]
    DuplicateSchemaName(String),
    #[error("storage failure: {0}")]
    Storage(String),
}
