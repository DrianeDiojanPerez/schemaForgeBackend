use crate::module::schema::core::domain::Schema;

use super::quote;

/// Foreign keys are added once every table exists. Emitting them separately
/// means the tables can be created in the order they were drawn, without
/// having to sort the schema so no table references one that is not there yet.
pub(super) fn foreign_keys(schema: &Schema) -> Vec<String> {
    let mut statements = Vec::new();

    for entity in &schema.entities {
        for attribute in &entity.attributes {
            let Some(reference) = &attribute.foreign_key else {
                continue;
            };

            // A reference that does not resolve is already an error from the
            // verifier, and generation refuses to run on an invalid schema,
            // so there is nothing worth emitting for it here.
            let Some((target, key)) = schema.resolve(&reference.entity_id, &reference.attribute_id)
            else {
                continue;
            };

            statements.push(format!(
                "ALTER TABLE {}\n    ADD CONSTRAINT {} FOREIGN KEY ({}) REFERENCES {} ({});",
                quote(&entity.name),
                quote(&constraint_name(&entity.name, &attribute.name)),
                quote(&attribute.name),
                quote(&target.name),
                quote(&key.name)
            ));
        }
    }

    statements
}

const MAX_IDENTIFIER_BYTES: usize = 63;

/// PostgreSQL silently cuts an identifier down to 63 bytes. Cutting it here
/// instead means the name in the script is the name the database ends up
/// storing, so a generated schema still reads back the way it was written.
fn constraint_name(table: &str, column: &str) -> String {
    let mut name = format!("fk_{}_{}", table.trim(), column.trim());

    if name.len() > MAX_IDENTIFIER_BYTES {
        let end = (0..=MAX_IDENTIFIER_BYTES)
            .rev()
            .find(|at| name.is_char_boundary(*at))
            .unwrap_or(0);

        name.truncate(end);
    }

    name
}
