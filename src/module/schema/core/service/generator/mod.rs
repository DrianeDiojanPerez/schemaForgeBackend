mod comment;
mod constraint;
mod layout;
mod table;

#[cfg(test)]
mod tests;

use crate::module::schema::core::domain::{Dialect, Schema};
use crate::module::schema::core::ports::{GenerateOptions, Generator};

pub struct PostgresGenerator;

impl Generator for PostgresGenerator {
    fn dialect(&self) -> Dialect {
        Dialect::Postgres
    }

    fn generate(&self, schema: &Schema, options: GenerateOptions) -> String {
        let mut blocks: Vec<String> = schema.entities.iter().map(table::create_table).collect();

        blocks.extend(constraint::foreign_keys(schema));

        if options.include_comments {
            let documentation = comment::comments(schema);

            if !documentation.is_empty() {
                blocks.push(documentation.join("\n"));
            }
        }

        // A script with nothing in it stays empty, so a caller cannot mistake
        // a layout block on its own for a schema.
        if options.include_positions && !blocks.is_empty() {
            blocks.push(layout::layout(schema));
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
