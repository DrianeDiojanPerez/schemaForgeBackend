use crate::module::schema::core::domain::{
    diagnostic_code as code, DataType, Diagnostic, Report, Schema,
};

use super::{column, relationship, table};

const MAX_LENGTH: u32 = 10_485_760;
const MAX_PRECISION: u32 = 1000;

pub(super) fn check_not_empty(schema: &Schema, report: &mut Report) {
    if schema.is_empty() {
        report.push(Diagnostic::warning(
            code::EMPTY_SCHEMA,
            "the schema has no tables yet",
        ));
    }
}

pub(super) fn check_names(schema: &Schema, report: &mut Report) {
    for entity in &schema.entities {
        if entity.name.trim().is_empty() {
            report.push(
                Diagnostic::error(code::EMPTY_NAME, "a table has no name")
                    .about(&entity.id)
                    .at(entity.position),
            );
        }

        for attribute in &entity.attributes {
            if attribute.name.trim().is_empty() {
                report.push(
                    Diagnostic::error(
                        code::EMPTY_NAME,
                        format!("a column in {} has no name", table(entity)),
                    )
                    .about(&entity.id)
                    .about(&attribute.id)
                    .at(entity.position),
                );
            }
        }
    }
}

pub(super) fn check_type_parameters(schema: &Schema, report: &mut Report) {
    for entity in &schema.entities {
        for attribute in &entity.attributes {
            let Some(problem) = parameter_problem(&attribute.data_type) else {
                continue;
            };

            report.push(
                Diagnostic::error(
                    code::INVALID_TYPE_PARAMETER,
                    format!(
                        "{} is {}, but {problem}",
                        column(entity, attribute),
                        attribute.data_type
                    ),
                )
                .about(&entity.id)
                .about(&attribute.id)
                .at(entity.position),
            );
        }
    }
}

// PostgreSQL 15 and later accept a scale larger than the precision, so
// numeric(2, 5) is not reported here even though older servers refuse it.
fn parameter_problem(data_type: &DataType) -> Option<String> {
    if data_type.kind.takes_length() {
        return match data_type.length {
            Some(0) => Some("a length has to be at least 1".to_owned()),
            Some(length) if length > MAX_LENGTH => Some(format!(
                "PostgreSQL allows a length of at most {MAX_LENGTH}"
            )),
            _ => None,
        };
    }

    if data_type.kind.takes_precision() {
        return match (data_type.precision, data_type.scale) {
            (None, Some(_)) => Some("a scale needs a precision".to_owned()),
            (Some(0), _) => Some("a precision has to be at least 1".to_owned()),
            (Some(precision), _) if precision > MAX_PRECISION => Some(format!(
                "PostgreSQL allows a precision of at most {MAX_PRECISION}"
            )),
            (_, Some(scale)) if scale > MAX_PRECISION => Some(format!(
                "PostgreSQL allows a scale of at most {MAX_PRECISION}"
            )),
            _ => None,
        };
    }

    None
}

pub(super) fn check_relationship_ends(schema: &Schema, report: &mut Report) {
    for link in &schema.relationships {
        let resolves = schema
            .resolve(&link.from_entity_id, &link.from_attribute_id)
            .is_some()
            && schema
                .resolve(&link.to_entity_id, &link.to_attribute_id)
                .is_some();

        if resolves {
            continue;
        }

        let mut diagnostic = Diagnostic::error(
            code::DANGLING_RELATIONSHIP,
            format!(
                "{} points at a table or column that does not exist",
                relationship(schema, link)
            ),
        )
        .about(&link.id);

        if let Some(entity) = schema
            .entity(&link.from_entity_id)
            .or_else(|| schema.entity(&link.to_entity_id))
        {
            diagnostic = diagnostic.at(entity.position);
        }

        report.push(diagnostic);
    }
}
