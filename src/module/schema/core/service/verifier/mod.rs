mod cycles;
mod documentation;
mod references;
mod structure;
mod syntax;

#[cfg(test)]
mod tests;

use crate::module::schema::core::domain::{Attribute, Entity, Relationship, Report, Schema};
use crate::module::schema::core::ports::Verifier;

type Check = fn(&Schema, &mut Report);

// Syntax runs before structure and references so the first thing a user
// reads about a half-drawn diagram is what is missing from the drawing.
const CHECKS: [Check; 10] = [
    syntax::check_not_empty,
    syntax::check_names,
    syntax::check_type_parameters,
    syntax::check_relationship_ends,
    structure::check_duplicate_names,
    structure::check_primary_keys,
    references::check_foreign_keys,
    references::check_relationships,
    cycles::check_cycles,
    documentation::check_documentation,
];

pub struct SchemaVerifier;

impl Verifier for SchemaVerifier {
    fn verify(&self, schema: &Schema) -> Report {
        let mut report = Report::default();

        for check in CHECKS {
            check(schema, &mut report);
        }

        report
    }
}

fn table(entity: &Entity) -> String {
    match entity.name.trim() {
        "" => "an unnamed table".to_owned(),
        name => format!("`{name}`"),
    }
}

fn column(entity: &Entity, attribute: &Attribute) -> String {
    format!("`{}.{}`", entity.name.trim(), attribute.name.trim())
}

fn relationship(schema: &Schema, relationship: &Relationship) -> String {
    if !relationship.name.trim().is_empty() {
        return format!("relationship `{}`", relationship.name.trim());
    }

    match (
        schema.entity(&relationship.from_entity_id),
        schema.entity(&relationship.to_entity_id),
    ) {
        (Some(from), Some(to)) => {
            format!("the relationship between {} and {}", table(from), table(to))
        }
        _ => "a relationship".to_owned(),
    }
}

/// PostgreSQL only lets a foreign key point at columns with a unique
/// constraint. One column of a composite primary key is not unique on its
/// own, so it only counts when it is the whole key.
fn is_unique_key(entity: &Entity, attribute: &Attribute) -> bool {
    attribute.unique || (attribute.primary_key && entity.primary_key().len() == 1)
}
