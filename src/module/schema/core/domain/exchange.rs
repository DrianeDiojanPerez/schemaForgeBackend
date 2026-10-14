use std::fmt;
use std::str::FromStr;

/// How a schema is written to a file and read back. JSON carries the model
/// whole; SQL carries whatever a DDL script can say about it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ExchangeFormat {
    #[default]
    Json,
    Sql,
}

impl ExchangeFormat {
    pub const fn as_str(self) -> &'static str {
        match self {
            ExchangeFormat::Json => "json",
            ExchangeFormat::Sql => "sql",
        }
    }

    pub const VARIANTS: [&'static str; 2] = ["json", "sql"];
}

impl fmt::Display for ExchangeFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for ExchangeFormat {
    type Err = ();

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim().to_ascii_lowercase().as_str() {
            "json" => Ok(ExchangeFormat::Json),
            "sql" | "postgres" | "postgresql" => Ok(ExchangeFormat::Sql),
            _ => Err(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_is_the_format_a_caller_gets_without_asking() {
        assert_eq!(ExchangeFormat::default(), ExchangeFormat::Json);
    }

    #[test]
    fn renders_back_to_the_name_it_parsed_from() {
        for name in ExchangeFormat::VARIANTS {
            let format: ExchangeFormat = name.parse().expect("the variant list should parse");

            assert_eq!(format.to_string(), name);
        }
    }

    #[test]
    fn a_sql_file_may_be_asked_for_by_the_dialect_that_writes_it() {
        assert_eq!("postgres".parse(), Ok(ExchangeFormat::Sql));
        assert_eq!("PostgreSQL".parse(), Ok(ExchangeFormat::Sql));
    }

    #[test]
    fn rejects_a_format_nobody_can_write() {
        assert!("yaml".parse::<ExchangeFormat>().is_err());
        assert!("dbml".parse::<ExchangeFormat>().is_err());
    }
}
