use crate::module::schema::core::domain::Schema;

use super::quote;

/// What a table or column means is the one part of a diagram that normally
/// does not survive export. PostgreSQL can hold it, so it travels with the
/// schema rather than staying behind on the canvas.
pub(super) fn comments(schema: &Schema) -> Vec<String> {
    let mut statements = Vec::new();

    for entity in &schema.entities {
        if entity.is_documented() {
            statements.push(format!(
                "COMMENT ON TABLE {} IS {};",
                quote(&entity.name),
                literal(&entity.description)
            ));
        }

        for attribute in &entity.attributes {
            if attribute.is_documented() {
                statements.push(format!(
                    "COMMENT ON COLUMN {}.{} IS {};",
                    quote(&entity.name),
                    quote(&attribute.name),
                    literal(&attribute.description)
                ));
            }
        }
    }

    statements
}

/// A description is prose somebody typed, so an apostrophe in it has to be
/// doubled or it would close the string and leave the rest as stray SQL.
fn literal(value: &str) -> String {
    format!("'{}'", value.trim().replace('\'', "''"))
}
