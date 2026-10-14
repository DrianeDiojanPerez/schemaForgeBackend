use crate::module::schema::core::domain::{Cardinality, DataTypeKind};

/// The proto enum value names, spelled out rather than derived from the
/// domain's own short names, because the two disagree in places: the domain
/// says `smallint` where the contract says `DATA_TYPE_KIND_SMALL_INT`. A test
/// in the rpc mapper holds this table against the generated enum.
pub(crate) const fn kind_name(kind: DataTypeKind) -> &'static str {
    match kind {
        DataTypeKind::Text => "DATA_TYPE_KIND_TEXT",
        DataTypeKind::Varchar => "DATA_TYPE_KIND_VARCHAR",
        DataTypeKind::Char => "DATA_TYPE_KIND_CHAR",
        DataTypeKind::SmallInt => "DATA_TYPE_KIND_SMALL_INT",
        DataTypeKind::Integer => "DATA_TYPE_KIND_INTEGER",
        DataTypeKind::BigInt => "DATA_TYPE_KIND_BIG_INT",
        DataTypeKind::Numeric => "DATA_TYPE_KIND_NUMERIC",
        DataTypeKind::Real => "DATA_TYPE_KIND_REAL",
        DataTypeKind::DoublePrecision => "DATA_TYPE_KIND_DOUBLE_PRECISION",
        DataTypeKind::Boolean => "DATA_TYPE_KIND_BOOLEAN",
        DataTypeKind::Date => "DATA_TYPE_KIND_DATE",
        DataTypeKind::Time => "DATA_TYPE_KIND_TIME",
        DataTypeKind::Timestamp => "DATA_TYPE_KIND_TIMESTAMP",
        DataTypeKind::TimestampTz => "DATA_TYPE_KIND_TIMESTAMPTZ",
        DataTypeKind::Uuid => "DATA_TYPE_KIND_UUID",
        DataTypeKind::Json => "DATA_TYPE_KIND_JSON",
        DataTypeKind::Jsonb => "DATA_TYPE_KIND_JSONB",
        DataTypeKind::Bytea => "DATA_TYPE_KIND_BYTEA",
    }
}

/// Both spellings are read back, so a file written against the contract and
/// one typed by hand against the canvas both load.
pub(super) fn kind_from_name(name: &str) -> Option<DataTypeKind> {
    let wanted = name.trim();

    DataTypeKind::VARIANTS
        .into_iter()
        .find(|kind| kind_name(*kind) == wanted || kind.as_str().eq_ignore_ascii_case(wanted))
}

pub(crate) const fn cardinality_name(cardinality: Cardinality) -> &'static str {
    match cardinality {
        Cardinality::OneToOne => "CARDINALITY_ONE_TO_ONE",
        Cardinality::OneToMany => "CARDINALITY_ONE_TO_MANY",
        Cardinality::ManyToMany => "CARDINALITY_MANY_TO_MANY",
    }
}

pub(super) fn cardinality_from_name(name: &str) -> Option<Cardinality> {
    let wanted = name.trim();

    [
        Cardinality::OneToOne,
        Cardinality::OneToMany,
        Cardinality::ManyToMany,
    ]
    .into_iter()
    .find(|cardinality| {
        cardinality_name(*cardinality) == wanted
            || cardinality.as_str().eq_ignore_ascii_case(wanted)
    })
}
