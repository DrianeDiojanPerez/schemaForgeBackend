//! Translation between the JSON payloads and the canonical model.
//!
//! Every conversion inward is fallible: a payload names a data type or a
//! cardinality as a string, and a name the model cannot hold has to be caught
//! here rather than reaching the core. Conversions outward cannot fail,
//! because the model is already well typed.
//!
//! Inward mapping accumulates violations instead of returning on the first
//! one, so a caller fixing a malformed request sees every problem at once.

use serde::{Deserialize, Serialize};

use crate::module::schema::core::domain::{self as domain};
use crate::package::errdef::Error;

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

    pub fn into_result(self) -> Result<(), Error> {
        match self.error {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct Position {
    pub x: f64,
    pub y: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DataType {
    pub kind: String,
    #[serde(default)]
    pub length: Option<u32>,
    #[serde(default)]
    pub precision: Option<u32>,
    #[serde(default)]
    pub scale: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ForeignKeyRef {
    #[serde(default)]
    pub entity_id: String,
    #[serde(default)]
    pub attribute_id: String,
}

/// Ids and names default to empty rather than being required by serde, so a
/// payload that omits them collects a violation naming the element instead of
/// being rejected wholesale with one parse error.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Attribute {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub data_type: Option<DataType>,
    #[serde(default = "nullable_unless_told_otherwise")]
    pub nullable: bool,
    #[serde(default)]
    pub primary_key: bool,
    #[serde(default)]
    pub unique: bool,
    #[serde(default)]
    pub foreign_key: Option<ForeignKeyRef>,
    #[serde(default)]
    pub default_value: Option<String>,
}

const fn nullable_unless_told_otherwise() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entity {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub attributes: Vec<Attribute>,
    #[serde(default)]
    pub position: Option<Position>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Relationship {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub from_entity_id: String,
    #[serde(default)]
    pub from_attribute_id: String,
    #[serde(default)]
    pub to_entity_id: String,
    #[serde(default)]
    pub to_attribute_id: String,
    #[serde(default)]
    pub cardinality: String,
}

/// What a caller sends. The id is the store's to assign, so it is only read
/// when the payload stands in for an unsaved diagram the canvas is holding.
#[derive(Debug, Clone, Deserialize)]
pub struct SchemaPayload {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub entities: Vec<Entity>,
    #[serde(default)]
    pub relationships: Vec<Relationship>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Schema {
    pub id: String,
    pub name: String,
    pub description: String,
    pub entities: Vec<Entity>,
    pub relationships: Vec<Relationship>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct SchemaSummary {
    pub id: String,
    pub name: String,
    pub description: String,
    pub entity_count: u32,
    pub relationship_count: u32,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Diagnostic {
    pub code: String,
    pub severity: String,
    pub message: String,
    pub element_ids: Vec<String>,
    pub location: Option<Position>,
}

impl From<domain::Position> for Position {
    fn from(position: domain::Position) -> Self {
        Self {
            x: position.x,
            y: position.y,
        }
    }
}

impl From<domain::DataType> for DataType {
    fn from(data_type: domain::DataType) -> Self {
        Self {
            kind: data_type.kind.to_string(),
            length: data_type.length,
            precision: data_type.precision,
            scale: data_type.scale,
        }
    }
}

impl From<domain::ForeignKeyRef> for ForeignKeyRef {
    fn from(reference: domain::ForeignKeyRef) -> Self {
        Self {
            entity_id: reference.entity_id,
            attribute_id: reference.attribute_id,
        }
    }
}

impl From<domain::Attribute> for Attribute {
    fn from(attribute: domain::Attribute) -> Self {
        Self {
            id: attribute.id,
            name: attribute.name,
            description: attribute.description,
            data_type: Some(DataType::from(attribute.data_type)),
            nullable: attribute.nullable,
            primary_key: attribute.primary_key,
            unique: attribute.unique,
            foreign_key: attribute.foreign_key.map(ForeignKeyRef::from),
            default_value: attribute.default_value,
        }
    }
}

impl From<domain::Entity> for Entity {
    fn from(entity: domain::Entity) -> Self {
        Self {
            id: entity.id,
            name: entity.name,
            description: entity.description,
            attributes: entity.attributes.into_iter().map(Attribute::from).collect(),
            position: Some(Position::from(entity.position)),
        }
    }
}

impl From<domain::Relationship> for Relationship {
    fn from(relationship: domain::Relationship) -> Self {
        Self {
            id: relationship.id,
            name: relationship.name,
            description: relationship.description,
            from_entity_id: relationship.from_entity_id,
            from_attribute_id: relationship.from_attribute_id,
            to_entity_id: relationship.to_entity_id,
            to_attribute_id: relationship.to_attribute_id,
            cardinality: relationship.cardinality.to_string(),
        }
    }
}

impl From<domain::Schema> for Schema {
    fn from(schema: domain::Schema) -> Self {
        Self {
            id: schema.id,
            name: schema.name,
            description: schema.description,
            entities: schema.entities.into_iter().map(Entity::from).collect(),
            relationships: schema
                .relationships
                .into_iter()
                .map(Relationship::from)
                .collect(),
            created_at: schema.created_at.to_rfc3339(),
            updated_at: schema.updated_at.to_rfc3339(),
        }
    }
}

impl From<domain::SchemaSummary> for SchemaSummary {
    fn from(summary: domain::SchemaSummary) -> Self {
        Self {
            id: summary.id,
            name: summary.name,
            description: summary.description,
            entity_count: summary.entity_count,
            relationship_count: summary.relationship_count,
            created_at: summary.created_at.to_rfc3339(),
            updated_at: summary.updated_at.to_rfc3339(),
        }
    }
}

impl From<domain::Diagnostic> for Diagnostic {
    fn from(diagnostic: domain::Diagnostic) -> Self {
        Self {
            code: diagnostic.code.to_owned(),
            severity: diagnostic.severity.to_string(),
            message: diagnostic.message,
            element_ids: diagnostic.element_ids,
            location: diagnostic.location.map(Position::from),
        }
    }
}

pub fn report_to(report: domain::Report) -> Vec<Diagnostic> {
    report
        .diagnostics
        .into_iter()
        .map(Diagnostic::from)
        .collect()
}

fn position_from(payload: Option<Position>) -> domain::Position {
    // An absent position is the origin rather than an error: a schema created
    // by a non-visual client has no layout to send, and auto-layout will place
    // it anyway.
    payload.map_or_else(domain::Position::default, |position| {
        domain::Position::new(position.x, position.y)
    })
}

fn data_type_from(
    payload: Option<DataType>,
    path: &str,
    violations: &mut Violations,
) -> Option<domain::DataType> {
    let Some(payload) = payload else {
        violations.add(path, "field is required");
        return None;
    };

    let Ok(kind) = payload.kind.parse::<domain::DataTypeKind>() else {
        violations.add(path, "field must name a known data type");
        return None;
    };

    // Parameters that do not apply to the kind are dropped rather than carried,
    // so `integer(11)` cannot reach the generator and be emitted.
    Some(domain::DataType {
        kind,
        length: kind.takes_length().then_some(payload.length).flatten(),
        precision: kind
            .takes_precision()
            .then_some(payload.precision)
            .flatten(),
        scale: kind.takes_precision().then_some(payload.scale).flatten(),
    })
}

fn cardinality_from(
    value: &str,
    path: &str,
    violations: &mut Violations,
) -> Option<domain::Cardinality> {
    match value.parse() {
        Ok(cardinality) => Some(cardinality),
        Err(()) => {
            violations.add(path, "field is required and must name a known cardinality");
            None
        }
    }
}

pub fn dialect_from(
    value: Option<&str>,
    path: &str,
    violations: &mut Violations,
) -> Option<domain::Dialect> {
    // Postgres is the committed target, so an unset dialect means it rather
    // than an error. A named one the generator does not have is an error.
    let Some(value) = value else {
        return Some(domain::Dialect::default());
    };

    match value.parse() {
        Ok(dialect) => Some(dialect),
        Err(()) => {
            violations.add(path, "field must name a known dialect");
            None
        }
    }
}

fn attribute_from(
    payload: Attribute,
    path: &str,
    violations: &mut Violations,
) -> Option<domain::Attribute> {
    if payload.id.trim().is_empty() {
        violations.add(
            format!("{path}.id"),
            "field is required and cannot be empty",
        );
    }

    if payload.name.trim().is_empty() {
        violations.add(
            format!("{path}.name"),
            "field is required and cannot be empty",
        );
    }

    let data_type = data_type_from(payload.data_type, &format!("{path}.data_type"), violations)?;

    let foreign_key = payload.foreign_key.and_then(|reference| {
        if reference.entity_id.trim().is_empty() || reference.attribute_id.trim().is_empty() {
            violations.add(
                format!("{path}.foreign_key"),
                "a reference must name both an entity and an attribute",
            );
            return None;
        }

        Some(domain::ForeignKeyRef::new(
            reference.entity_id,
            reference.attribute_id,
        ))
    });

    Some(domain::Attribute {
        id: payload.id,
        name: payload.name,
        description: payload.description,
        data_type,
        // A primary key is never nullable, whatever the request claimed.
        nullable: payload.nullable && !payload.primary_key,
        primary_key: payload.primary_key,
        unique: payload.unique,
        foreign_key,
        default_value: payload.default_value,
    })
}

fn entity_from(payload: Entity, path: &str, violations: &mut Violations) -> Option<domain::Entity> {
    if payload.id.trim().is_empty() {
        violations.add(
            format!("{path}.id"),
            "field is required and cannot be empty",
        );
    }

    if payload.name.trim().is_empty() {
        violations.add(
            format!("{path}.name"),
            "field is required and cannot be empty",
        );
    }

    let attributes = payload
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

    Some(domain::Entity {
        id: payload.id,
        name: payload.name,
        description: payload.description,
        attributes,
        position: position_from(payload.position),
    })
}

fn relationship_from(
    payload: Relationship,
    path: &str,
    violations: &mut Violations,
) -> Option<domain::Relationship> {
    if payload.id.trim().is_empty() {
        violations.add(
            format!("{path}.id"),
            "field is required and cannot be empty",
        );
    }

    for (field, value) in [
        ("from_entity_id", &payload.from_entity_id),
        ("from_attribute_id", &payload.from_attribute_id),
        ("to_entity_id", &payload.to_entity_id),
        ("to_attribute_id", &payload.to_attribute_id),
    ] {
        if value.trim().is_empty() {
            violations.add(
                format!("{path}.{field}"),
                "field is required and cannot be empty",
            );
        }
    }

    let cardinality = cardinality_from(
        &payload.cardinality,
        &format!("{path}.cardinality"),
        violations,
    )?;

    Some(domain::Relationship {
        id: payload.id,
        name: payload.name,
        description: payload.description,
        from_entity_id: payload.from_entity_id,
        from_attribute_id: payload.from_attribute_id,
        to_entity_id: payload.to_entity_id,
        to_attribute_id: payload.to_attribute_id,
        cardinality,
    })
}

pub fn draft_from(payload: SchemaPayload) -> Result<domain::SchemaDraft, Error> {
    let mut violations = Violations::default();

    let draft = draft_with(payload, &mut violations);

    violations.into_result()?;

    Ok(draft)
}

/// An unsaved diagram the canvas is holding. It has no stored identity, so the
/// id and timestamps are placeholders the validator never reads.
pub fn draft_schema_from(payload: SchemaPayload) -> Result<domain::Schema, Error> {
    let id = payload.id.clone();
    let draft = draft_from(payload)?;

    Ok(domain::Schema::new(id, draft, chrono::Utc::now()))
}

fn draft_with(payload: SchemaPayload, violations: &mut Violations) -> domain::SchemaDraft {
    let entities = payload
        .entities
        .into_iter()
        .enumerate()
        .filter_map(|(index, entity)| {
            entity_from(entity, &format!("entities[{index}]"), violations)
        })
        .collect();

    let relationships = payload
        .relationships
        .into_iter()
        .enumerate()
        .filter_map(|(index, relationship)| {
            relationship_from(relationship, &format!("relationships[{index}]"), violations)
        })
        .collect();

    domain::SchemaDraft {
        name: payload.name,
        description: payload.description,
        entities,
        relationships,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::package::errdef::code;

    fn typed(kind: &str) -> Option<DataType> {
        Some(DataType {
            kind: kind.to_owned(),
            length: None,
            precision: None,
            scale: None,
        })
    }

    fn attribute(id: &str, name: &str) -> Attribute {
        Attribute {
            id: id.to_owned(),
            name: name.to_owned(),
            description: String::new(),
            data_type: typed("uuid"),
            nullable: true,
            primary_key: false,
            unique: false,
            foreign_key: None,
            default_value: None,
        }
    }

    fn entity(id: &str, name: &str, attributes: Vec<Attribute>) -> Entity {
        Entity {
            id: id.to_owned(),
            name: name.to_owned(),
            description: String::new(),
            attributes,
            position: None,
        }
    }

    fn payload(entities: Vec<Entity>, relationships: Vec<Relationship>) -> SchemaPayload {
        SchemaPayload {
            id: String::new(),
            name: "blog".to_owned(),
            description: String::new(),
            entities,
            relationships,
        }
    }

    fn relationship(cardinality: &str) -> Relationship {
        Relationship {
            id: "r1".to_owned(),
            name: String::new(),
            description: String::new(),
            from_entity_id: "e2".to_owned(),
            from_attribute_id: "a3".to_owned(),
            to_entity_id: "e1".to_owned(),
            to_attribute_id: "a1".to_owned(),
            cardinality: cardinality.to_owned(),
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
        let draft = draft_from(payload(
            vec![entity("e1", "users", vec![attribute("a1", "id")])],
            vec![],
        ))
        .expect("the draft should map");

        assert_eq!(draft.name, "blog");
        assert_eq!(draft.entities.len(), 1);
        assert_eq!(draft.entities[0].attributes[0].name, "id");
    }

    #[test]
    fn a_data_type_the_model_cannot_hold_is_rejected() {
        let mut unknown = attribute("a1", "id");
        unknown.data_type = typed("serial");

        let error = draft_from(payload(vec![entity("e1", "users", vec![unknown])], vec![]))
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

        let error = draft_from(payload(vec![entity("e1", "users", vec![bare])], vec![]))
            .expect_err("the draft should not map");

        assert_eq!(
            violations_of(&error),
            vec!["entities[0].attributes[0].data_type"]
        );
    }

    #[test]
    fn every_violation_is_reported_not_only_the_first() {
        let error = draft_from(payload(
            vec![
                entity("", "users", vec![attribute("a1", "")]),
                entity("e2", "", vec![]),
            ],
            vec![],
        ))
        .expect_err("the draft should not map");

        let paths = violations_of(&error);

        assert!(paths.contains(&"entities[0].id".to_owned()));
        assert!(paths.contains(&"entities[0].attributes[0].name".to_owned()));
        assert!(paths.contains(&"entities[1].name".to_owned()));
        assert_eq!(paths.len(), 3);
    }

    #[test]
    fn an_unnamed_cardinality_is_rejected() {
        let error = draft_from(payload(vec![], vec![relationship("")]))
            .expect_err("the draft should not map");

        assert_eq!(
            violations_of(&error),
            vec!["relationships[0].cardinality"],
            "a line without a cardinality is not a relationship"
        );
    }

    #[test]
    fn a_cardinality_is_read_by_symbol_or_by_name() {
        for spelling in ["1:N", "one_to_many"] {
            let draft = draft_from(payload(vec![], vec![relationship(spelling)]))
                .expect("the draft should map");

            assert_eq!(
                draft.relationships[0].cardinality,
                domain::Cardinality::OneToMany
            );
        }
    }

    #[test]
    fn a_relationship_missing_an_end_is_rejected() {
        let mut half_drawn = relationship("1:N");
        half_drawn.from_attribute_id = String::new();
        half_drawn.to_entity_id = String::new();

        let error =
            draft_from(payload(vec![], vec![half_drawn])).expect_err("the draft should not map");

        let paths = violations_of(&error);

        assert!(paths.contains(&"relationships[0].from_attribute_id".to_owned()));
        assert!(paths.contains(&"relationships[0].to_entity_id".to_owned()));
    }

    #[test]
    fn a_parameter_that_does_not_apply_to_the_kind_is_dropped() {
        let mut odd = attribute("a1", "count");
        odd.data_type = Some(DataType {
            kind: "integer".to_owned(),
            length: Some(11),
            precision: None,
            scale: None,
        });

        let draft = draft_from(payload(vec![entity("e1", "stats", vec![odd])], vec![]))
            .expect("the draft should map");

        assert_eq!(
            draft.entities[0].attributes[0].data_type.length, None,
            "integer(11) must not reach the generator"
        );
    }

    #[test]
    fn a_parameter_that_does_apply_survives() {
        let mut sized = attribute("a1", "email");
        sized.data_type = Some(DataType {
            kind: "varchar".to_owned(),
            length: Some(255),
            precision: None,
            scale: None,
        });

        let draft = draft_from(payload(vec![entity("e1", "users", vec![sized])], vec![]))
            .expect("the draft should map");

        assert_eq!(draft.entities[0].attributes[0].data_type.length, Some(255));
    }

    #[test]
    fn a_primary_key_arriving_as_nullable_is_corrected() {
        let mut key = attribute("a1", "id");
        key.primary_key = true;
        key.nullable = true;

        let draft = draft_from(payload(vec![entity("e1", "users", vec![key])], vec![]))
            .expect("the draft should map");

        assert!(!draft.entities[0].attributes[0].nullable);
    }

    #[test]
    fn a_half_written_foreign_key_is_rejected() {
        let mut dangling = attribute("a2", "user_id");
        dangling.foreign_key = Some(ForeignKeyRef {
            entity_id: "e1".to_owned(),
            attribute_id: String::new(),
        });

        let error = draft_from(payload(vec![entity("e2", "posts", vec![dangling])], vec![]))
            .expect_err("the draft should not map");

        assert_eq!(
            violations_of(&error),
            vec!["entities[0].attributes[0].foreign_key"]
        );
    }

    #[test]
    fn an_absent_position_is_the_origin_not_an_error() {
        let draft = draft_from(payload(vec![entity("e1", "users", vec![])], vec![]))
            .expect("the draft should map");

        assert_eq!(draft.entities[0].position, domain::Position::new(0.0, 0.0));
    }

    #[test]
    fn an_omitted_attribute_flag_takes_the_model_default() {
        let attribute: Attribute =
            serde_json::from_str(r#"{"id":"a1","name":"nickname","data_type":{"kind":"text"}}"#)
                .expect("the payload should parse");

        assert!(
            attribute.nullable,
            "an attribute is nullable until told otherwise"
        );
        assert!(!attribute.primary_key);
        assert!(!attribute.unique);
    }

    #[test]
    fn an_unset_dialect_is_the_committed_target() {
        let mut violations = Violations::default();

        assert_eq!(
            dialect_from(None, "dialect", &mut violations),
            Some(domain::Dialect::Postgres)
        );
        assert_eq!(
            dialect_from(Some("mysql"), "dialect", &mut violations),
            Some(domain::Dialect::MySql)
        );
        assert!(violations.into_result().is_ok());
    }

    #[test]
    fn a_dialect_with_no_generator_is_rejected() {
        let mut violations = Violations::default();

        assert_eq!(
            dialect_from(Some("sqlite"), "dialect", &mut violations),
            None
        );

        let error = violations
            .into_result()
            .expect_err("the dialect should not map");

        assert_eq!(violations_of(&error), vec!["dialect"]);
    }

    #[test]
    fn a_schema_renders_its_types_and_cardinalities_by_name() {
        let schema = domain::Schema::new(
            "s1",
            domain::SchemaDraft {
                name: "blog".to_owned(),
                description: String::new(),
                entities: vec![domain::Entity::new("e1", "users").with_attributes(vec![
                    domain::Attribute::new("a1", "id", domain::DataType::varchar(50)),
                ])],
                relationships: vec![domain::Relationship::new(
                    "r1",
                    ("e1", "a1"),
                    ("e1", "a1"),
                    domain::Cardinality::ManyToMany,
                )],
            },
            chrono::Utc::now(),
        );

        let dto = Schema::from(schema);

        assert_eq!(
            dto.entities[0].attributes[0]
                .data_type
                .as_ref()
                .map(|t| t.kind.as_str()),
            Some("varchar")
        );
        assert_eq!(
            dto.entities[0].attributes[0]
                .data_type
                .as_ref()
                .and_then(|t| t.length),
            Some(50)
        );
        assert_eq!(dto.relationships[0].cardinality, "N:M");
        assert!(dto.created_at.contains('T'), "timestamps are rfc 3339");
    }
}
