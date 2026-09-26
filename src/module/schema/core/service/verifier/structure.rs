use crate::module::schema::core::domain::{diagnostic_code as code, Diagnostic, Report, Schema};

use super::table;

/// PostgreSQL folds unquoted names to lower case, so `Users` and `users`
/// would be the same table once the DDL runs.
fn same_name_groups<T>(items: &[T], name: impl Fn(&T) -> &str) -> Vec<Vec<&T>> {
    let mut groups: Vec<(String, Vec<&T>)> = Vec::new();

    for item in items {
        let key = name(item).trim().to_lowercase();

        if key.is_empty() {
            continue;
        }

        match groups.iter_mut().find(|(existing, _)| *existing == key) {
            Some((_, members)) => members.push(item),
            None => groups.push((key, vec![item])),
        }
    }

    groups
        .into_iter()
        .map(|(_, members)| members)
        .filter(|members| members.len() > 1)
        .collect()
}

pub(super) fn check_duplicate_names(schema: &Schema, report: &mut Report) {
    for group in same_name_groups(&schema.entities, |entity| &entity.name) {
        let mut diagnostic = Diagnostic::error(
            code::DUPLICATE_ENTITY_NAME,
            format!(
                "{} tables are named `{}`",
                group.len(),
                group[0].name.trim()
            ),
        )
        .at(group[0].position);

        for entity in group {
            diagnostic = diagnostic.about(&entity.id);
        }

        report.push(diagnostic);
    }

    for entity in &schema.entities {
        for group in same_name_groups(&entity.attributes, |attribute| &attribute.name) {
            let mut diagnostic = Diagnostic::error(
                code::DUPLICATE_ATTRIBUTE_NAME,
                format!(
                    "{} has {} columns named `{}`",
                    table(entity),
                    group.len(),
                    group[0].name.trim()
                ),
            )
            .about(&entity.id)
            .at(entity.position);

            for attribute in group {
                diagnostic = diagnostic.about(&attribute.id);
            }

            report.push(diagnostic);
        }
    }
}

pub(super) fn check_primary_keys(schema: &Schema, report: &mut Report) {
    for entity in &schema.entities {
        if !entity.has_primary_key() {
            report.push(
                Diagnostic::error(
                    code::MISSING_PRIMARY_KEY,
                    format!("{} has no primary key", table(entity)),
                )
                .about(&entity.id)
                .at(entity.position),
            );
        }
    }
}
