use std::fmt;
use std::str::FromStr;

/// A dialect-independent type. No PostgreSQL or MySQL spelling appears here:
/// the mapping to a concrete type is the generator's job, which is what lets a
/// second dialect be added without touching the model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DataTypeKind {
    Text,
    Varchar,
    Char,
    SmallInt,
    Integer,
    BigInt,
    Numeric,
    Real,
    DoublePrecision,
    Boolean,
    Date,
    Time,
    Timestamp,
    TimestampTz,
    Uuid,
    Json,
    Jsonb,
    Bytea,
}

impl DataTypeKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            DataTypeKind::Text => "text",
            DataTypeKind::Varchar => "varchar",
            DataTypeKind::Char => "char",
            DataTypeKind::SmallInt => "smallint",
            DataTypeKind::Integer => "integer",
            DataTypeKind::BigInt => "bigint",
            DataTypeKind::Numeric => "numeric",
            DataTypeKind::Real => "real",
            DataTypeKind::DoublePrecision => "double precision",
            DataTypeKind::Boolean => "boolean",
            DataTypeKind::Date => "date",
            DataTypeKind::Time => "time",
            DataTypeKind::Timestamp => "timestamp",
            DataTypeKind::TimestampTz => "timestamptz",
            DataTypeKind::Uuid => "uuid",
            DataTypeKind::Json => "json",
            DataTypeKind::Jsonb => "jsonb",
            DataTypeKind::Bytea => "bytea",
        }
    }

    pub const VARIANTS: [DataTypeKind; 18] = [
        DataTypeKind::Text,
        DataTypeKind::Varchar,
        DataTypeKind::Char,
        DataTypeKind::SmallInt,
        DataTypeKind::Integer,
        DataTypeKind::BigInt,
        DataTypeKind::Numeric,
        DataTypeKind::Real,
        DataTypeKind::DoublePrecision,
        DataTypeKind::Boolean,
        DataTypeKind::Date,
        DataTypeKind::Time,
        DataTypeKind::Timestamp,
        DataTypeKind::TimestampTz,
        DataTypeKind::Uuid,
        DataTypeKind::Json,
        DataTypeKind::Jsonb,
        DataTypeKind::Bytea,
    ];

    /// Whether the kind is parameterised by a length, as `varchar(n)` is.
    pub const fn takes_length(self) -> bool {
        matches!(self, DataTypeKind::Varchar | DataTypeKind::Char)
    }

    /// Whether the kind is parameterised by precision and scale, as
    /// `numeric(p, s)` is.
    pub const fn takes_precision(self) -> bool {
        matches!(self, DataTypeKind::Numeric)
    }

    /// Family membership drives foreign-key type compatibility. A foreign key
    /// on a `bigint` referencing an `integer` primary key is a defect the
    /// validation engine reports, but `varchar` referencing `text` is not.
    pub const fn family(self) -> TypeFamily {
        match self {
            DataTypeKind::Text | DataTypeKind::Varchar | DataTypeKind::Char => TypeFamily::Textual,
            DataTypeKind::SmallInt | DataTypeKind::Integer | DataTypeKind::BigInt => {
                TypeFamily::Integral
            }
            DataTypeKind::Numeric | DataTypeKind::Real | DataTypeKind::DoublePrecision => {
                TypeFamily::Decimal
            }
            DataTypeKind::Boolean => TypeFamily::Boolean,
            DataTypeKind::Date
            | DataTypeKind::Time
            | DataTypeKind::Timestamp
            | DataTypeKind::TimestampTz => TypeFamily::Temporal,
            DataTypeKind::Uuid => TypeFamily::Uuid,
            DataTypeKind::Json | DataTypeKind::Jsonb => TypeFamily::Document,
            DataTypeKind::Bytea => TypeFamily::Binary,
        }
    }
}

impl fmt::Display for DataTypeKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for DataTypeKind {
    type Err = ();

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let name = value.trim().to_ascii_lowercase();

        DataTypeKind::VARIANTS
            .into_iter()
            .find(|kind| kind.as_str() == name)
            .ok_or(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TypeFamily {
    Textual,
    Integral,
    Decimal,
    Boolean,
    Temporal,
    Uuid,
    Document,
    Binary,
}

/// A kind together with whatever parameters it carries. Parameters that do not
/// apply to the kind are held as `None` rather than defaulted, so a `varchar`
/// the user never gave a length is distinguishable from `varchar(0)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DataType {
    pub kind: DataTypeKind,
    pub length: Option<u32>,
    pub precision: Option<u32>,
    pub scale: Option<u32>,
}

impl DataType {
    pub const fn simple(kind: DataTypeKind) -> Self {
        Self {
            kind,
            length: None,
            precision: None,
            scale: None,
        }
    }

    pub const fn varchar(length: u32) -> Self {
        Self {
            kind: DataTypeKind::Varchar,
            length: Some(length),
            precision: None,
            scale: None,
        }
    }

    pub const fn numeric(precision: u32, scale: u32) -> Self {
        Self {
            kind: DataTypeKind::Numeric,
            length: None,
            precision: Some(precision),
            scale: Some(scale),
        }
    }

    /// Two types a foreign key may span. Deliberately family-level rather than
    /// exact: requiring an exact match would reject `varchar(50)` referencing
    /// `varchar(100)`, which every real database accepts.
    pub fn is_compatible_with(&self, other: &Self) -> bool {
        self.kind.family() == other.kind.family()
    }
}

impl fmt::Display for DataType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match (self.kind.takes_length(), self.kind.takes_precision()) {
            (true, _) => match self.length {
                Some(length) => write!(f, "{}({})", self.kind, length),
                None => write!(f, "{}", self.kind),
            },
            (_, true) => match (self.precision, self.scale) {
                (Some(precision), Some(scale)) => {
                    write!(f, "{}({}, {})", self.kind, precision, scale)
                }
                (Some(precision), None) => write!(f, "{}({})", self.kind, precision),
                _ => write!(f, "{}", self.kind),
            },
            _ => write!(f, "{}", self.kind),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_a_parameterised_type_with_its_parameters() {
        assert_eq!(DataType::varchar(255).to_string(), "varchar(255)");
        assert_eq!(DataType::numeric(10, 2).to_string(), "numeric(10, 2)");
        assert_eq!(
            DataType::simple(DataTypeKind::TimestampTz).to_string(),
            "timestamptz"
        );
    }

    #[test]
    fn a_parameterised_type_without_parameters_renders_bare() {
        let unsized_varchar = DataType::simple(DataTypeKind::Varchar);

        assert_eq!(unsized_varchar.to_string(), "varchar");
    }

    #[test]
    fn a_length_on_a_type_that_takes_none_is_not_rendered() {
        let odd = DataType {
            kind: DataTypeKind::Integer,
            length: Some(11),
            precision: None,
            scale: None,
        };

        assert_eq!(odd.to_string(), "integer", "integer(11) is a MySQL-ism");
    }

    #[test]
    fn a_foreign_key_may_span_different_widths_in_one_family() {
        let integer = DataType::simple(DataTypeKind::Integer);
        let big_int = DataType::simple(DataTypeKind::BigInt);

        assert!(integer.is_compatible_with(&big_int));
    }

    #[test]
    fn a_foreign_key_may_not_span_families() {
        let integer = DataType::simple(DataTypeKind::Integer);
        let text = DataType::simple(DataTypeKind::Text);
        let uuid = DataType::simple(DataTypeKind::Uuid);

        assert!(!integer.is_compatible_with(&text));
        assert!(!uuid.is_compatible_with(&text), "a uuid is not a string");
    }

    #[test]
    fn text_and_varchar_are_one_family() {
        assert!(DataType::varchar(50).is_compatible_with(&DataType::simple(DataTypeKind::Text)));
    }

    #[test]
    fn parses_back_from_every_name_it_renders() {
        for kind in DataTypeKind::VARIANTS {
            assert_eq!(kind.as_str().parse(), Ok(kind));
        }

        assert_eq!("  TIMESTAMPTZ ".parse(), Ok(DataTypeKind::TimestampTz));
    }

    #[test]
    fn rejects_a_type_the_model_cannot_hold() {
        assert!("serial".parse::<DataTypeKind>().is_err());
        assert!("".parse::<DataTypeKind>().is_err());
    }

    #[test]
    fn knows_which_kinds_carry_which_parameters() {
        assert!(DataTypeKind::Varchar.takes_length());
        assert!(DataTypeKind::Char.takes_length());
        assert!(!DataTypeKind::Text.takes_length());

        assert!(DataTypeKind::Numeric.takes_precision());
        assert!(!DataTypeKind::Real.takes_precision());
    }
}
