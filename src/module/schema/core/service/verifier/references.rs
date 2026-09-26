use crate::module::schema::core::domain::{
    diagnostic_code as code, Attribute, Cardinality, DataType, DataTypeKind, Diagnostic, Entity,
    Report, Schema, TypeFamily,
};

use super::{column, is_unique_key, relationship};

pub(super) fn check_foreign_keys(schema: &Schema, report: &mut Report) {
    for entity in &schema.entities {
        for attribute in &entity.attributes {
            let Some(reference) = &attribute.foreign_key else {
                continue;
            };

            let Some((target, key)) = schema.resolve(&reference.entity_id, &reference.attribute_id)
            else {
                report.push(
                    Diagnostic::error(
                        code::INVALID_FOREIGN_KEY,
                        format!(
                            "{} references a column that does not exist",
                            column(entity, attribute)
                        ),
                    )
                    .about(&entity.id)
                    .about(&attribute.id)
                    .at(entity.position),
                );
                continue;
            };

            if !is_unique_key(target, key) {
                report.push(
                    Diagnostic::error(
                        code::FOREIGN_KEY_NOT_A_KEY,
                        format!(
                            "{} references {}, which is neither the primary key nor unique",
                            column(entity, attribute),
                            column(target, key)
                        ),
                    )
                    .about(&entity.id)
                    .about(&attribute.id)
                    .about(&target.id)
                    .about(&key.id)
                    .at(entity.position),
                );
            }

            let mismatch = if !attribute.data_type.is_compatible_with(&key.data_type) {
                Some(Diagnostic::error(
                    code::TYPE_MISMATCH,
                    format!(
                        "{} is {} but references {}, which is {}",
                        column(entity, attribute),
                        attribute.data_type,
                        column(target, key),
                        key.data_type
                    ),
                ))
            } else if is_narrower(&attribute.data_type, &key.data_type) {
                Some(Diagnostic::warning(
                    code::TYPE_MISMATCH,
                    format!(
                        "{} is {} but references {}, which is {}, so some keys will not fit",
                        column(entity, attribute),
                        attribute.data_type,
                        column(target, key),
                        key.data_type
                    ),
                ))
            } else {
                None
            };

            if let Some(mismatch) = mismatch {
                report.push(
                    mismatch
                        .about(&entity.id)
                        .about(&attribute.id)
                        .about(&target.id)
                        .about(&key.id)
                        .at(entity.position),
                );
            }
        }
    }
}

fn is_narrower(column: &DataType, key: &DataType) -> bool {
    match column.kind.family() {
        TypeFamily::Integral => width(column.kind) < width(key.kind),
        TypeFamily::Textual => match (bound(column), bound(key)) {
            (Some(column), Some(key)) => column < key,
            (Some(_), None) => true,
            _ => false,
        },
        _ => false,
    }
}

fn width(kind: DataTypeKind) -> u8 {
    match kind {
        DataTypeKind::SmallInt => 2,
        DataTypeKind::Integer => 4,
        _ => 8,
    }
}

fn bound(data_type: &DataType) -> Option<u32> {
    match data_type.kind {
        DataTypeKind::Text => None,
        _ => data_type.length,
    }
}

fn points_at(attribute: &Attribute, entity: &Entity, key: &Attribute) -> bool {
    attribute.foreign_key.as_ref().is_some_and(|reference| {
        reference.entity_id == entity.id && reference.attribute_id == key.id
    })
}

/// Which end of a relationship is "from" differs between clients, so the
/// column holding the foreign key is found by looking at both ends rather
/// than trusting the direction the relationship was drawn in.
pub(super) fn check_relationships(schema: &Schema, report: &mut Report) {
    for link in &schema.relationships {
        // A many-to-many line becomes a join table, so neither end is meant
        // to hold the foreign key itself.
        if link.cardinality.requires_join_table() {
            continue;
        }

        let (Some((from_entity, from)), Some((to_entity, to))) = (
            schema.resolve(&link.from_entity_id, &link.from_attribute_id),
            schema.resolve(&link.to_entity_id, &link.to_attribute_id),
        ) else {
            continue;
        };

        let holder = if points_at(from, to_entity, to) {
            (from_entity, from)
        } else if points_at(to, from_entity, from) {
            (to_entity, to)
        } else {
            report.push(
                Diagnostic::error(
                    code::RELATIONSHIP_WITHOUT_FOREIGN_KEY,
                    format!(
                        "{} joins {} and {}, but neither column is a foreign key to the other",
                        relationship(schema, link),
                        column(from_entity, from),
                        column(to_entity, to)
                    ),
                )
                .about(&link.id)
                .at(from_entity.position),
            );
            continue;
        };

        let (holder_entity, holder_attribute) = holder;
        let unique = is_unique_key(holder_entity, holder_attribute);

        let message = match link.cardinality {
            Cardinality::OneToOne if !unique => format!(
                "{} is one-to-one, but {} is not unique, so many rows can point at the same row",
                relationship(schema, link),
                column(holder_entity, holder_attribute)
            ),
            Cardinality::OneToMany if unique => format!(
                "{} is one-to-many, but {} is unique, so each row can only be pointed at once",
                relationship(schema, link),
                column(holder_entity, holder_attribute)
            ),
            _ => continue,
        };

        report.push(
            Diagnostic::warning(code::CARDINALITY_MISMATCH, message)
                .about(&link.id)
                .about(&holder_entity.id)
                .about(&holder_attribute.id)
                .at(holder_entity.position),
        );
    }
}
