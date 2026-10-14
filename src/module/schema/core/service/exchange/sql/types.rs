use crate::module::schema::core::domain::DataTypeKind;

/// PostgreSQL spells most types several ways, and a hand-written script uses
/// whichever one its author knew. The canonical model holds one of each, so
/// the aliases all land on the same kind.
pub(super) fn kind_of(name: &str) -> Option<DataTypeKind> {
    match name {
        "text" => Some(DataTypeKind::Text),
        "varchar" | "character varying" => Some(DataTypeKind::Varchar),
        "char" | "character" | "bpchar" => Some(DataTypeKind::Char),
        "smallint" | "int2" | "smallserial" | "serial2" => Some(DataTypeKind::SmallInt),
        "int" | "integer" | "int4" | "serial" | "serial4" => Some(DataTypeKind::Integer),
        "bigint" | "int8" | "bigserial" | "serial8" => Some(DataTypeKind::BigInt),
        "numeric" | "decimal" => Some(DataTypeKind::Numeric),
        "real" | "float4" => Some(DataTypeKind::Real),
        // A bare `float` is double precision in PostgreSQL.
        "double precision" | "float8" | "float" => Some(DataTypeKind::DoublePrecision),
        "boolean" | "bool" => Some(DataTypeKind::Boolean),
        "date" => Some(DataTypeKind::Date),
        "time" | "time without time zone" => Some(DataTypeKind::Time),
        "timestamp" | "timestamp without time zone" => Some(DataTypeKind::Timestamp),
        "timestamptz" | "timestamp with time zone" => Some(DataTypeKind::TimestampTz),
        "uuid" => Some(DataTypeKind::Uuid),
        "json" => Some(DataTypeKind::Json),
        "jsonb" => Some(DataTypeKind::Jsonb),
        "bytea" => Some(DataTypeKind::Bytea),
        _ => None,
    }
}

/// A serial column is an integer that fills itself in, so the column it
/// becomes is one the database will never leave empty.
pub(super) fn is_serial(name: &str) -> bool {
    matches!(
        name,
        "smallserial" | "serial2" | "serial" | "serial4" | "bigserial" | "serial8"
    )
}
