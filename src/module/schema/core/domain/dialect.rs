use std::fmt;
use std::str::FromStr;

/// A generation target. The canonical model knows nothing about these; only the
/// generator does. Adding one is a matter of writing another generator against
/// the same model rather than re-plumbing the system.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Dialect {
    #[default]
    Postgres,
    MySql,
}

impl Dialect {
    pub const fn as_str(self) -> &'static str {
        match self {
            Dialect::Postgres => "postgres",
            Dialect::MySql => "mysql",
        }
    }

    pub const VARIANTS: [&'static str; 2] = ["postgres", "mysql"];

    /// Only PostgreSQL carries descriptions as COMMENT ON statements. MySQL
    /// spells them as an inline COMMENT clause, which is the generator's
    /// problem rather than the model's.
    pub const fn supports_comment_on(self) -> bool {
        matches!(self, Dialect::Postgres)
    }
}

impl fmt::Display for Dialect {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for Dialect {
    type Err = ();

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim().to_ascii_lowercase().as_str() {
            "postgres" | "postgresql" => Ok(Dialect::Postgres),
            "mysql" => Ok(Dialect::MySql),
            _ => Err(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_both_spellings_of_postgres() {
        assert_eq!("postgres".parse(), Ok(Dialect::Postgres));
        assert_eq!("PostgreSQL".parse(), Ok(Dialect::Postgres));
    }

    #[test]
    fn rejects_a_dialect_with_no_generator() {
        assert!("sqlite".parse::<Dialect>().is_err());
        assert!("oracle".parse::<Dialect>().is_err());
    }

    #[test]
    fn renders_back_to_the_name_it_parsed_from() {
        for name in Dialect::VARIANTS {
            let dialect: Dialect = name.parse().expect("the variant list should parse");

            assert_eq!(dialect.to_string(), name);
        }
    }

    #[test]
    fn postgres_is_the_committed_target() {
        assert_eq!(Dialect::default(), Dialect::Postgres);
        assert!(Dialect::Postgres.supports_comment_on());
        assert!(!Dialect::MySql.supports_comment_on());
    }
}
