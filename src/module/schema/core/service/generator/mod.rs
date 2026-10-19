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
        let mut blocks: Vec<String> = schema
            .entities
            .iter()
            .map(|entity| {
                let statement = table::create_table(entity);

                // The position rides directly above the table it belongs to,
                // so a hand edited script keeps the two together.
                match options.include_positions {
                    true => format!("{}\n{statement}", layout::position(entity)),
                    false => statement,
                }
            })
            .collect();

        blocks.extend(constraint::foreign_keys(schema));

        if options.include_comments {
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
