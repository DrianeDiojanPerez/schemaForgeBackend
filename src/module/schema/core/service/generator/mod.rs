mod comment;
mod constraint;
mod table;

#[cfg(test)]
mod tests;

use crate::module::schema::core::domain::{Dialect, Schema};
use crate::module::schema::core::ports::Generator;

pub struct PostgresGenerator;

impl Generator for PostgresGenerator {
    fn dialect(&self) -> Dialect {
        Dialect::Postgres
    }

    fn generate(&self, schema: &Schema, include_comments: bool) -> String {
        let mut blocks: Vec<String> = schema.entities.iter().map(table::create_table).collect();

        blocks.extend(constraint::foreign_keys(schema));

        if include_comments {
            let documentation = comment::comments(schema);

            if !documentation.is_empty() {
                blocks.push(documentation.join("\n"));
            }
        }

        blocks.join("\n\n")
    }
}

/// Every identifier is quoted. PostgreSQL folds an unquoted name to lower
/// case and refuses a reserved word like `order` outright, so quoting is what
/// keeps the table named the way the user drew it.
pub(super) fn quote(name: &str) -> String {
    format!("\"{}\"", name.trim().replace('"', "\"\""))
}
