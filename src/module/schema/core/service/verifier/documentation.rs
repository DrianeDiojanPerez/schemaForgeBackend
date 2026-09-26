use crate::module::schema::core::domain::{diagnostic_code as code, Diagnostic, Report, Schema};

use super::{relationship, table};

/// One warning per table rather than one per column, so a new diagram does
/// not bury its real defects under a warning for every column on it.
pub(super) fn check_documentation(schema: &Schema, report: &mut Report) {
    for entity in &schema.entities {
        let undocumented: Vec<_> = entity
            .attributes
            .iter()
            .filter(|attribute| !attribute.is_documented())
            .collect();

        if entity.is_documented() && undocumented.is_empty() {
            continue;
        }

        let columns = undocumented
            .iter()
            .map(|attribute| format!("`{}`", attribute.name.trim()))
            .collect::<Vec<_>>()
            .join(", ");

        let message = match (entity.is_documented(), undocumented.is_empty()) {
            (false, true) => format!("{} has no description", table(entity)),
            (false, false) => format!(
                "{} has no description, and neither do these columns: {columns}",
                table(entity)
            ),
            (true, _) => format!(
                "{} has columns with no description: {columns}",
                table(entity)
            ),
        };

        let mut diagnostic = Diagnostic::warning(code::MISSING_DESCRIPTION, message)
            .about(&entity.id)
            .at(entity.position);

        for attribute in undocumented {
            diagnostic = diagnostic.about(&attribute.id);
        }

        report.push(diagnostic);
    }

    for link in &schema.relationships {
        if !link.is_documented() {
            report.push(
                Diagnostic::warning(
                    code::MISSING_DESCRIPTION,
                    format!("{} has no description", relationship(schema, link)),
                )
                .about(&link.id),
            );
        }
    }
}
