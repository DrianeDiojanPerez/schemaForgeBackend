//! Translation between the wire contract and the canonical model.
//!
//! Every conversion inward is fallible: proto3 gives every field a zero value,
//! so an unset enum arrives as `UNSPECIFIED` rather than as an absence, and
//! that has to be caught here rather than reaching the model. Conversions
//! outward cannot fail, because the model is already well typed.
//!
//! Inward mapping accumulates violations instead of returning on the first
//! one, so a caller fixing a malformed request sees every problem at once.

use crate::module::schema::core::domain::{
    Attribute, Cardinality, DataType, DataTypeKind, Dialect, Entity, ForeignKeyRef, Position,
    Relationship, Schema, SchemaDraft, SchemaSummary, Severity,
};
use crate::module::schema::core::domain::{Diagnostic, Report};
use crate::package::errdef::Error;
use crate::rpc::v1;

/// Collects field violations under dotted paths so a client can point at the
/// exact element that failed, for example `entities[2].attributes[0].data_type`.
#[derive(Default)]
pub struct Violations {
    error: Option<Error>,
}

impl Violations {
    fn add(&mut self, field: impl Into<String>, message: impl Into<String>) {
        self.error
            .get_or_insert_with(|| Error::validation("failed payload validation"))
            .push_violation(field, message);
    }

    fn into_result(self) -> Result<(), Error> {
        match self.error {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }
}

pub fn position_from(proto: Option<v1::Position>) -> Position {
    // An absent position is the origin rather than an error: a schema created
    // by a non-visual client has no layout to send, and auto-layout will place
    // it anyway.
    proto.map_or_else(Position::default, |p| Position::new(p.x, p.y))
}

pub fn position_to(position: Position) -> v1::Position {
    v1::Position {
        x: position.x,
        y: position.y,
    }
}

fn data_type_kind_from(kind: i32) -> Option<DataTypeKind> {
    match v1::DataTypeKind::try_from(kind).ok()? {
        v1::DataTypeKind::Unspecified => None,
        v1::DataTypeKind::Text => Some(DataTypeKind::Text),
        v1::DataTypeKind::Varchar => Some(DataTypeKind::Varchar),
        v1::DataTypeKind::Char => Some(DataTypeKind::Char),
        v1::DataTypeKind::SmallInt => Some(DataTypeKind::SmallInt),
        v1::DataTypeKind::Integer => Some(DataTypeKind::Integer),
        v1::DataTypeKind::BigInt => Some(DataTypeKind::BigInt),
        v1::DataTypeKind::Numeric => Some(DataTypeKind::Numeric),
        v1::DataTypeKind::Real => Some(DataTypeKind::Real),
        v1::DataTypeKind::DoublePrecision => Some(DataTypeKind::DoublePrecision),
        v1::DataTypeKind::Boolean => Some(DataTypeKind::Boolean),
        v1::DataTypeKind::Date => Some(DataTypeKind::Date),
        v1::DataTypeKind::Time => Some(DataTypeKind::Time),
        v1::DataTypeKind::Timestamp => Some(DataTypeKind::Timestamp),
        v1::DataTypeKind::Timestamptz => Some(DataTypeKind::TimestampTz),
        v1::DataTypeKind::Uuid => Some(DataTypeKind::Uuid),
        v1::DataTypeKind::Json => Some(DataTypeKind::Json),
        v1::DataTypeKind::Jsonb => Some(DataTypeKind::Jsonb),
        v1::DataTypeKind::Bytea => Some(DataTypeKind::Bytea),
    }
}

fn data_type_kind_to(kind: DataTypeKind) -> v1::DataTypeKind {
    match kind {
        DataTypeKind::Text => v1::DataTypeKind::Text,
        DataTypeKind::Varchar => v1::DataTypeKind::Varchar,
        DataTypeKind::Char => v1::DataTypeKind::Char,
        DataTypeKind::SmallInt => v1::DataTypeKind::SmallInt,
        DataTypeKind::Integer => v1::DataTypeKind::Integer,
        DataTypeKind::BigInt => v1::DataTypeKind::BigInt,
        DataTypeKind::Numeric => v1::DataTypeKind::Numeric,
        DataTypeKind::Real => v1::DataTypeKind::Real,
        DataTypeKind::DoublePrecision => v1::DataTypeKind::DoublePrecision,
        DataTypeKind::Boolean => v1::DataTypeKind::Boolean,
        DataTypeKind::Date => v1::DataTypeKind::Date,
        DataTypeKind::Time => v1::DataTypeKind::Time,
        DataTypeKind::Timestamp => v1::DataTypeKind::Timestamp,
        DataTypeKind::TimestampTz => v1::DataTypeKind::Timestamptz,
        DataTypeKind::Uuid => v1::DataTypeKind::Uuid,
        DataTypeKind::Json => v1::DataTypeKind::Json,
        DataTypeKind::Jsonb => v1::DataTypeKind::Jsonb,
        DataTypeKind::Bytea => v1::DataTypeKind::Bytea,
    }
}

fn data_type_from(
    proto: Option<v1::DataType>,
    path: &str,
    violations: &mut Violations,
) -> Option<DataType> {
    let Some(proto) = proto else {
        violations.add(path, "field is required");
        return None;
    };

    let Some(kind) = data_type_kind_from(proto.kind) else {
        violations.add(path, "field must name a known data type");
        return None;
    };

    // Parameters that do not apply to the kind are dropped rather than carried,
    // so `integer(11)` cannot reach the generator and be emitted.
    Some(DataType {
        kind,
        length: kind.takes_length().then_some(proto.length).flatten(),
        precision: kind.takes_precision().then_some(proto.precision).flatten(),
        scale: kind.takes_precision().then_some(proto.scale).flatten(),
    })
}

fn data_type_to(data_type: DataType) -> v1::DataType {
    v1::DataType {
        kind: data_type_kind_to(data_type.kind) as i32,
        length: data_type.length,
        precision: data_type.precision,
        scale: data_type.scale,
    }
}

fn cardinality_from(value: i32, path: &str, violations: &mut Violations) -> Option<Cardinality> {
    match v1::Cardinality::try_from(value) {
        Ok(v1::Cardinality::OneToOne) => Some(Cardinality::OneToOne),
        Ok(v1::Cardinality::OneToMany) => Some(Cardinality::OneToMany),
        Ok(v1::Cardinality::ManyToMany) => Some(Cardinality::ManyToMany),
        Ok(v1::Cardinality::Unspecified) | Err(_) => {
            violations.add(path, "field is required and must name a known cardinality");
            None
        }
    }
}

fn cardinality_to(cardinality: Cardinality) -> v1::Cardinality {
    match cardinality {
        Cardinality::OneToOne => v1::Cardinality::OneToOne,
        Cardinality::OneToMany => v1::Cardinality::OneToMany,
        Cardinality::ManyToMany => v1::Cardinality::ManyToMany,
    }
}

pub fn dialect_from(value: i32) -> Dialect {
    match v1::Dialect::try_from(value) {
        Ok(v1::Dialect::Mysql) => Dialect::MySql,
        // Postgres is the committed target, so an unset dialect means it
        // rather than an error.
        _ => Dialect::Postgres,
    }
}

fn attribute_from(
    proto: v1::Attribute,
    path: &str,
    violations: &mut Violations,
) -> Option<Attribute> {
    if proto.id.trim().is_empty() {
        violations.add(
            format!("{path}.id"),
            "field is required and cannot be empty",
        );
    }

    if proto.name.trim().is_empty() {
        violations.add(
            format!("{path}.name"),
            "field is required and cannot be empty",
        );
    }

    let data_type = data_type_from(proto.data_type, &format!("{path}.data_type"), violations)?;

    let foreign_key = proto.foreign_key.and_then(|reference| {
        if reference.entity_id.trim().is_empty() || reference.attribute_id.trim().is_empty() {
            violations.add(
                format!("{path}.foreign_key"),
                "a reference must name both an entity and an attribute",
            );
            return None;
        }

        Some(ForeignKeyRef::new(
            reference.entity_id,
            reference.attribute_id,
        ))
    });

    Some(Attribute {
        id: proto.id,
        name: proto.name,
        description: proto.description,
        data_type,
        // A primary key is never nullable, whatever the request claimed.
        nullable: proto.nullable && !proto.primary_key,
        primary_key: proto.primary_key,
        unique: proto.unique,
        foreign_key,
        default_value: proto.default_value,
    })
}

fn attribute_to(attribute: Attribute) -> v1::Attribute {
    v1::Attribute {
        id: attribute.id,
        name: attribute.name,
        description: attribute.description,
        data_type: Some(data_type_to(attribute.data_type)),
        nullable: attribute.nullable,
        primary_key: attribute.primary_key,
        unique: attribute.unique,
        foreign_key: attribute.foreign_key.map(|reference| v1::ForeignKeyRef {
            entity_id: reference.entity_id,
            attribute_id: reference.attribute_id,
        }),
        default_value: attribute.default_value,
    }
}

fn entity_from(proto: v1::Entity, path: &str, violations: &mut Violations) -> Option<Entity> {
    if proto.id.trim().is_empty() {
        violations.add(
            format!("{path}.id"),
            "field is required and cannot be empty",
        );
    }

    if proto.name.trim().is_empty() {
        violations.add(
            format!("{path}.name"),
            "field is required and cannot be empty",
        );
    }

    let attributes = proto
        .attributes
        .into_iter()
        .enumerate()
        .filter_map(|(index, attribute)| {
            attribute_from(
                attribute,
                &format!("{path}.attributes[{index}]"),
                violations,
            )
        })
        .collect();

    Some(Entity {
        id: proto.id,
        name: proto.name,
        description: proto.description,
        attributes,
        position: position_from(proto.position),
    })
}

fn entity_to(entity: Entity) -> v1::Entity {
    v1::Entity {
        id: entity.id,
        name: entity.name,
        description: entity.description,
        attributes: entity.attributes.into_iter().map(attribute_to).collect(),
        position: Some(position_to(entity.position)),
    }
}

fn relationship_from(
    proto: v1::Relationship,
    path: &str,
    violations: &mut Violations,
) -> Option<Relationship> {
    if proto.id.trim().is_empty() {
        violations.add(
            format!("{path}.id"),
            "field is required and cannot be empty",
        );
    }

    for (field, value) in [
        ("from_entity_id", &proto.from_entity_id),
        ("from_attribute_id", &proto.from_attribute_id),
        ("to_entity_id", &proto.to_entity_id),
        ("to_attribute_id", &proto.to_attribute_id),
    ] {
        if value.trim().is_empty() {
            violations.add(
                format!("{path}.{field}"),
                "field is required and cannot be empty",
            );
        }
    }

    let cardinality = cardinality_from(
        proto.cardinality,
        &format!("{path}.cardinality"),
        violations,
    )?;

    Some(Relationship {
        id: proto.id,
        name: proto.name,
        description: proto.description,
        from_entity_id: proto.from_entity_id,
        from_attribute_id: proto.from_attribute_id,
        to_entity_id: proto.to_entity_id,
        to_attribute_id: proto.to_attribute_id,
        cardinality,
    })
}

fn relationship_to(relationship: Relationship) -> v1::Relationship {
    v1::Relationship {
        id: relationship.id,
        name: relationship.name,
        description: relationship.description,
        from_entity_id: relationship.from_entity_id,
        from_attribute_id: relationship.from_attribute_id,
        to_entity_id: relationship.to_entity_id,
        to_attribute_id: relationship.to_attribute_id,
        cardinality: cardinality_to(relationship.cardinality) as i32,
    }
}

pub fn draft_from(
    name: String,
    description: String,
    entities: Vec<v1::Entity>,
    relationships: Vec<v1::Relationship>,
) -> Result<SchemaDraft, Error> {
    let mut violations = Violations::default();

    let entities = entities
        .into_iter()
        .enumerate()
        .filter_map(|(index, entity)| {
            entity_from(entity, &format!("entities[{index}]"), &mut violations)
        })
        .collect();

    let relationships = relationships
        .into_iter()
        .enumerate()
        .filter_map(|(index, relationship)| {
            relationship_from(
                relationship,
                &format!("relationships[{index}]"),
                &mut violations,
            )
        })
        .collect();

    violations.into_result()?;

    Ok(SchemaDraft {
        name,
        description,
        entities,
        relationships,
    })
}

/// An unsaved diagram the canvas is holding. It has no stored identity, so the
/// id and timestamps are placeholders the validator never reads.
pub fn draft_schema_from(proto: v1::Schema) -> Result<Schema, Error> {
    let draft = draft_from(
        proto.name,
        proto.description,
        proto.entities,
        proto.relationships,
    )?;

    Ok(Schema::new(proto.id, draft, chrono::Utc::now()))
}

pub fn schema_to(schema: Schema) -> v1::Schema {
    v1::Schema {
        id: schema.id,
        name: schema.name,
        description: schema.description,
        entities: schema.entities.into_iter().map(entity_to).collect(),
        relationships: schema
            .relationships
            .into_iter()
            .map(relationship_to)
            .collect(),
        created_at: schema.created_at.to_rfc3339(),
        updated_at: schema.updated_at.to_rfc3339(),
    }
}

pub fn summary_to(summary: SchemaSummary) -> v1::SchemaSummary {
    v1::SchemaSummary {
        id: summary.id,
        name: summary.name,
        description: summary.description,
        entity_count: summary.entity_count,
        relationship_count: summary.relationship_count,
        created_at: summary.created_at.to_rfc3339(),
        updated_at: summary.updated_at.to_rfc3339(),
    }
}

fn diagnostic_to(diagnostic: Diagnostic) -> v1::Diagnostic {
    v1::Diagnostic {
        code: diagnostic.code.to_owned(),
        severity: match diagnostic.severity {
            Severity::Warning => v1::Severity::Warning,
            Severity::Error => v1::Severity::Error,
        } as i32,
        message: diagnostic.message,
        element_ids: diagnostic.element_ids,
        location: diagnostic.location.map(position_to),
    }
}

pub fn report_to(report: Report) -> Vec<v1::Diagnostic> {
    report.diagnostics.into_iter().map(diagnostic_to).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::package::errdef::code;

    fn typed(kind: v1::DataTypeKind) -> Option<v1::DataType> {
        Some(v1::DataType {
            kind: kind as i32,
            length: None,
            precision: None,
            scale: None,
        })
    }

    fn attribute(id: &str, name: &str) -> v1::Attribute {
        v1::Attribute {
            id: id.to_owned(),
            name: name.to_owned(),
            description: String::new(),
            data_type: typed(v1::DataTypeKind::Uuid),
            nullable: true,
            primary_key: false,
            unique: false,
            foreign_key: None,
            default_value: None,
        }
    }

    fn entity(id: &str, name: &str, attributes: Vec<v1::Attribute>) -> v1::Entity {
        v1::Entity {
            id: id.to_owned(),
            name: name.to_owned(),
            description: String::new(),
            attributes,
            position: None,
        }
    }

    fn violations_of(error: &Error) -> Vec<String> {
        match error {
            Error::Validation(err) => err.field_violations.keys().cloned().collect(),
            Error::App(_) => Vec::new(),
        }
    }

    #[test]
    fn maps_a_well_formed_draft_inward() {
        let draft = draft_from(
            "blog".to_owned(),
            "A blog".to_owned(),
            vec![entity("e1", "users", vec![attribute("a1", "id")])],
            vec![],
        )
        .expect("the draft should map");

        assert_eq!(draft.name, "blog");
        assert_eq!(draft.entities.len(), 1);
        assert_eq!(draft.entities[0].attributes[0].name, "id");
    }

    #[test]
    fn an_unset_data_type_is_rejected() {
        let mut bare = attribute("a1", "id");
        bare.data_type = typed(v1::DataTypeKind::Unspecified);

        let error = draft_from(
            "blog".to_owned(),
            String::new(),
            vec![entity("e1", "users", vec![bare])],
            vec![],
        )
        .expect_err("the draft should not map");

        assert_eq!(error.app_code(), code::VALIDATION_FAILED);
        assert_eq!(
            violations_of(&error),
            vec!["entities[0].attributes[0].data_type"],
            "the path points at the exact attribute"
        );
    }

    #[test]
    fn a_missing_data_type_is_rejected() {
        let mut bare = attribute("a1", "id");
        bare.data_type = None;

        let error = draft_from(
            "blog".to_owned(),
            String::new(),
            vec![entity("e1", "users", vec![bare])],
            vec![],
        )
        .expect_err("the draft should not map");

        assert_eq!(error.app_code(), code::VALIDATION_FAILED);
    }

    #[test]
    fn every_violation_is_reported_not_only_the_first() {
        let error = draft_from(
            "blog".to_owned(),
            String::new(),
            vec![
                entity("", "users", vec![attribute("a1", "")]),
                entity("e2", "", vec![]),
            ],
            vec![],
        )
        .expect_err("the draft should not map");

        let paths = violations_of(&error);

        assert!(paths.contains(&"entities[0].id".to_owned()));
        assert!(paths.contains(&"entities[0].attributes[0].name".to_owned()));
        assert!(paths.contains(&"entities[1].name".to_owned()));
        assert_eq!(paths.len(), 3);
    }

    #[test]
    fn an_unset_cardinality_is_rejected() {
        let relationship = v1::Relationship {
            id: "r1".to_owned(),
            name: String::new(),
            description: String::new(),
            from_entity_id: "e2".to_owned(),
            from_attribute_id: "a3".to_owned(),
            to_entity_id: "e1".to_owned(),
            to_attribute_id: "a1".to_owned(),
            cardinality: v1::Cardinality::Unspecified as i32,
        };

        let error = draft_from("blog".to_owned(), String::new(), vec![], vec![relationship])
            .expect_err("the draft should not map");

        assert_eq!(
            violations_of(&error),
            vec!["relationships[0].cardinality"],
            "a line without a cardinality is not a relationship"
        );
    }

    #[test]
    fn a_relationship_missing_an_end_is_rejected() {
        let relationship = v1::Relationship {
            id: "r1".to_owned(),
            name: String::new(),
            description: String::new(),
            from_entity_id: "e2".to_owned(),
            from_attribute_id: String::new(),
            to_entity_id: String::new(),
            to_attribute_id: "a1".to_owned(),
            cardinality: v1::Cardinality::OneToMany as i32,
        };

        let error = draft_from("blog".to_owned(), String::new(), vec![], vec![relationship])
            .expect_err("the draft should not map");

        let paths = violations_of(&error);

        assert!(paths.contains(&"relationships[0].from_attribute_id".to_owned()));
        assert!(paths.contains(&"relationships[0].to_entity_id".to_owned()));
    }

    #[test]
    fn a_parameter_that_does_not_apply_to_the_kind_is_dropped() {
        let mut odd = attribute("a1", "count");
        odd.data_type = Some(v1::DataType {
            kind: v1::DataTypeKind::Integer as i32,
            length: Some(11),
            precision: None,
            scale: None,
        });

        let draft = draft_from(
            "blog".to_owned(),
            String::new(),
            vec![entity("e1", "stats", vec![odd])],
            vec![],
        )
        .expect("the draft should map");

        assert_eq!(
            draft.entities[0].attributes[0].data_type.length, None,
            "integer(11) must not reach the generator"
        );
    }

    #[test]
    fn a_parameter_that_does_apply_survives() {
        let mut sized = attribute("a1", "email");
        sized.data_type = Some(v1::DataType {
            kind: v1::DataTypeKind::Varchar as i32,
            length: Some(255),
            precision: None,
            scale: None,
        });

        let draft = draft_from(
            "blog".to_owned(),
            String::new(),
            vec![entity("e1", "users", vec![sized])],
            vec![],
        )
        .expect("the draft should map");

        assert_eq!(draft.entities[0].attributes[0].data_type.length, Some(255));
    }

    #[test]
    fn a_primary_key_arriving_as_nullable_is_corrected() {
        let mut key = attribute("a1", "id");
        key.primary_key = true;
        key.nullable = true;

        let draft = draft_from(
            "blog".to_owned(),
            String::new(),
            vec![entity("e1", "users", vec![key])],
            vec![],
        )
        .expect("the draft should map");

        assert!(!draft.entities[0].attributes[0].nullable);
    }

    #[test]
    fn a_half_written_foreign_key_is_rejected() {
        let mut dangling = attribute("a2", "user_id");
        dangling.foreign_key = Some(v1::ForeignKeyRef {
            entity_id: "e1".to_owned(),
            attribute_id: String::new(),
        });

        let error = draft_from(
            "blog".to_owned(),
            String::new(),
            vec![entity("e2", "posts", vec![dangling])],
            vec![],
        )
        .expect_err("the draft should not map");

        assert_eq!(
            violations_of(&error),
            vec!["entities[0].attributes[0].foreign_key"]
        );
    }

    #[test]
    fn an_absent_position_is_the_origin_not_an_error() {
        let draft = draft_from(
            "blog".to_owned(),
            String::new(),
            vec![entity("e1", "users", vec![])],
            vec![],
        )
        .expect("the draft should map");

        assert_eq!(draft.entities[0].position, Position::new(0.0, 0.0));
    }

    #[test]
    fn an_unset_dialect_is_the_committed_target() {
        assert_eq!(
            dialect_from(v1::Dialect::Unspecified as i32),
            Dialect::Postgres
        );
        assert_eq!(dialect_from(v1::Dialect::Mysql as i32), Dialect::MySql);
    }

    #[test]
    fn a_round_trip_through_the_wire_preserves_the_model() {
        let original = draft_from(
            "blog".to_owned(),
            "A blog".to_owned(),
            vec![entity(
                "e1",
                "users",
                vec![{
                    let mut key = attribute("a1", "id");
                    key.primary_key = true;
                    key.nullable = false;
                    key.description = "Surrogate key".to_owned();
                    key
                }],
            )],
            vec![],
        )
        .expect("the draft should map");

        let schema = Schema::new("s1", original.clone(), chrono::Utc::now());
        let returned = draft_schema_from(schema_to(schema.clone())).expect("it should map back");

        assert_eq!(returned.name, schema.name);
        assert_eq!(returned.entities, schema.entities);
        assert_eq!(returned.relationships, schema.relationships);
    }
}
