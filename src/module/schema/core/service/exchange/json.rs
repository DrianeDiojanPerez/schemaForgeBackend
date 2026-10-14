use serde::Serialize;
use serde_json::{Map, Value};
use uuid::Uuid;

use crate::module::schema::core::domain::{
    Attribute, Cardinality, DataType, Entity, ForeignKeyRef, Position, Relationship, Schema,
    SchemaDraft,
};
use crate::package::errdef::Error;

use super::names::{cardinality_from_name, kind_from_name};
use super::read::{
    array, coordinate, count, flag, object, optional_text, refuse, required, required_text,
};
use super::{cardinality_name, grid_slot, kind_name};

pub(super) const CONTENT_TYPE: &str = "application/json";
pub(super) const EXTENSION: &str = "json";

/// Bumped when a reader would get the file wrong without knowing which shape
/// it is looking at.
const VERSION: u32 = 1;

/// The file is the proto3 JSON mapping of the canonical model: camelCase
/// names and the contract's own enum spellings, so anything generated from
/// the same proto can read it. The three fields the store assigns are left
/// out, because an exported file describes a design rather than a row.
pub(super) fn write(schema: &Schema, include_positions: bool) -> String {
    let document = Document {
        schemaforge: VERSION,
        name: &schema.name,
        description: &schema.description,
        entities: schema
            .entities
            .iter()
            .map(|entity| entity_doc(entity, include_positions))
            .collect(),
        relationships: schema.relationships.iter().map(relationship_doc).collect(),
    };

    serde_json::to_string_pretty(&document).expect("a document of owned strings serialises")
}

pub(super) fn file_name(schema_name: &str) -> String {
    let slug: String = schema_name
        .trim()
        .to_lowercase()
        .chars()
        .map(|character| {
            if character.is_whitespace() {
                '_'
            } else {
                character
            }
        })
        // A name is whatever somebody typed on the canvas, and it ends up in
        // a Content-Disposition header, so anything that is not plainly a
        // file name is dropped rather than escaped.
        .filter(|character| character.is_alphanumeric() || matches!(character, '_' | '-'))
        .collect();

    match slug.trim_matches('_') {
        "" => format!("schema.{EXTENSION}"),
        trimmed => format!("{trimmed}.{EXTENSION}"),
    }
}

fn entity_doc(entity: &Entity, include_positions: bool) -> EntityDoc<'_> {
    EntityDoc {
        id: &entity.id,
        name: &entity.name,
        description: &entity.description,
        attributes: entity.attributes.iter().map(attribute_doc).collect(),
        position: include_positions.then_some(PositionDoc {
            x: entity.position.x,
            y: entity.position.y,
        }),
    }
}

fn attribute_doc(attribute: &Attribute) -> AttributeDoc<'_> {
    AttributeDoc {
        id: &attribute.id,
        name: &attribute.name,
        description: &attribute.description,
        data_type: DataTypeDoc {
            kind: kind_name(attribute.data_type.kind),
            length: attribute.data_type.length,
            precision: attribute.data_type.precision,
            scale: attribute.data_type.scale,
        },
        nullable: attribute.nullable,
        primary_key: attribute.primary_key,
        unique: attribute.unique,
        foreign_key: attribute
            .foreign_key
            .as_ref()
            .map(|reference| ForeignKeyDoc {
                entity_id: &reference.entity_id,
                attribute_id: &reference.attribute_id,
            }),
        default_value: attribute.default_value.as_deref(),
    }
}

fn relationship_doc(relationship: &Relationship) -> RelationshipDoc<'_> {
    RelationshipDoc {
        id: &relationship.id,
        name: &relationship.name,
        description: &relationship.description,
        from_entity_id: &relationship.from_entity_id,
        from_attribute_id: &relationship.from_attribute_id,
        to_entity_id: &relationship.to_entity_id,
        to_attribute_id: &relationship.to_attribute_id,
        cardinality: cardinality_name(relationship.cardinality),
    }
}

#[derive(Serialize)]
struct Document<'a> {
    schemaforge: u32,
    name: &'a str,
    description: &'a str,
    entities: Vec<EntityDoc<'a>>,
    relationships: Vec<RelationshipDoc<'a>>,
}

#[derive(Serialize)]
struct EntityDoc<'a> {
    id: &'a str,
    name: &'a str,
    description: &'a str,
    attributes: Vec<AttributeDoc<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    position: Option<PositionDoc>,
}

#[derive(Serialize)]
struct PositionDoc {
    x: f64,
    y: f64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AttributeDoc<'a> {
    id: &'a str,
    name: &'a str,
    description: &'a str,
    data_type: DataTypeDoc,
    nullable: bool,
    primary_key: bool,
    unique: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    foreign_key: Option<ForeignKeyDoc<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    default_value: Option<&'a str>,
}

#[derive(Serialize)]
struct DataTypeDoc {
    kind: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    length: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    precision: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    scale: Option<u32>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ForeignKeyDoc<'a> {
    entity_id: &'a str,
    attribute_id: &'a str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RelationshipDoc<'a> {
    id: &'a str,
    name: &'a str,
    description: &'a str,
    from_entity_id: &'a str,
    from_attribute_id: &'a str,
    to_entity_id: &'a str,
    to_attribute_id: &'a str,
    cardinality: &'static str,
}

/// Reads back what `write` produced, and a bare `Schema` that never carried
/// the marker, because the obvious thing to paste in is whatever the gRPC
/// call last answered with.
pub(super) fn read(content: &str, include_positions: bool) -> Result<SchemaDraft, Error> {
    let document: Value = serde_json::from_str(content)
        .map_err(|err| refuse("$", &format!("invalid json: {err}")))?;

    let root = object(&document, "$")?;

    let mut draft = SchemaDraft {
        name: required_text(root, "name", "$.name")?,
        description: optional_text(root, "description", "$.description")?.unwrap_or_default(),
        entities: Vec::new(),
        relationships: Vec::new(),
    };

    for (index, value) in array(required(root, "entities", "$.entities")?, "$.entities")?
        .iter()
        .enumerate()
    {
        draft
            .entities
            .push(entity_from(value, index, include_positions)?);
    }

    if let Some(values) = root.get("relationships").filter(|value| !value.is_null()) {
        for (index, value) in array(values, "$.relationships")?.iter().enumerate() {
            draft
                .relationships
                .push(relationship_from(value, index, &draft.entities)?);
        }
    }

    Ok(draft)
}

fn entity_from(value: &Value, index: usize, include_positions: bool) -> Result<Entity, Error> {
    let path = format!("$.entities[{index}]");
    let source = object(value, &path)?;

    let mut entity = Entity::new(
        identity(source, "id", || format!("e{}", Uuid::new_v4().simple())),
        required_text(source, "name", &format!("{path}.name"))?,
    )
    .with_description(
        optional_text(source, "description", &format!("{path}.description"))?.unwrap_or_default(),
    )
    .with_position(position_from(source, index, include_positions, &path)?);

    if let Some(values) = source.get("attributes").filter(|value| !value.is_null()) {
        let attributes_path = format!("{path}.attributes");
        let mut attributes = Vec::new();

        for (at, attribute) in array(values, &attributes_path)?.iter().enumerate() {
            attributes.push(attribute_from(
                attribute,
                &format!("{attributes_path}[{at}]"),
            )?);
        }

        entity = entity.with_attributes(attributes);
    }

    Ok(entity)
}

fn position_from(
    source: &Map<String, Value>,
    index: usize,
    include_positions: bool,
    path: &str,
) -> Result<Position, Error> {
    if !include_positions {
        return Ok(grid_slot(index));
    }

    match source.get("position").filter(|value| !value.is_null()) {
        None => Ok(grid_slot(index)),
        Some(value) => {
            let position_path = format!("{path}.position");
            let position = object(value, &position_path)?;

            Ok(Position::new(
                coordinate(position, "x", &format!("{position_path}.x"))?,
                coordinate(position, "y", &format!("{position_path}.y"))?,
            ))
        }
    }
}

fn attribute_from(value: &Value, path: &str) -> Result<Attribute, Error> {
    let source = object(value, path)?;
    let mut attribute = Attribute::new(
        identity(source, "id", || format!("a{}", Uuid::new_v4().simple())),
        required_text(source, "name", &format!("{path}.name"))?,
        data_type_from(source, path)?,
    );

    attribute.description =
        optional_text(source, "description", &format!("{path}.description"))?.unwrap_or_default();
    attribute.nullable = flag(source, "nullable", &format!("{path}.nullable"), true)?;
    attribute.primary_key = flag(source, "primaryKey", &format!("{path}.primaryKey"), false)?;
    attribute.unique = flag(source, "unique", &format!("{path}.unique"), false)?;
    attribute.default_value =
        optional_text(source, "defaultValue", &format!("{path}.defaultValue"))?;

    if let Some(value) = source.get("foreignKey").filter(|value| !value.is_null()) {
        let reference_path = format!("{path}.foreignKey");
        let reference = object(value, &reference_path)?;

        attribute.foreign_key = Some(ForeignKeyRef::new(
            required_text(reference, "entityId", &format!("{reference_path}.entityId"))?,
            required_text(
                reference,
                "attributeId",
                &format!("{reference_path}.attributeId"),
            )?,
        ));
    }

    // A primary key is never nullable, and a file is free to say otherwise,
    // so the model's own rule wins over what was written down.
    if attribute.primary_key {
        attribute.nullable = false;
    }

    Ok(attribute)
}

fn data_type_from(source: &Map<String, Value>, path: &str) -> Result<DataType, Error> {
    let type_path = format!("{path}.dataType");
    let data_type = object(required(source, "dataType", &type_path)?, &type_path)?;
    let kind_path = format!("{type_path}.kind");
    let name = required_text(data_type, "kind", &kind_path)?;

    let Some(kind) = kind_from_name(&name) else {
        return Err(refuse(&kind_path, "field must name a known data type"));
    };

    // Parameters that do not apply to the kind are dropped rather than
    // carried, the same way they are at the gRPC boundary.
    Ok(DataType {
        kind,
        length: kind
            .takes_length()
            .then_some(count(data_type, "length", &format!("{type_path}.length"))?)
            .flatten(),
        precision: kind
            .takes_precision()
            .then_some(count(
                data_type,
                "precision",
                &format!("{type_path}.precision"),
            )?)
            .flatten(),
        scale: kind
            .takes_precision()
            .then_some(count(data_type, "scale", &format!("{type_path}.scale"))?)
            .flatten(),
    })
}

fn relationship_from(
    value: &Value,
    index: usize,
    entities: &[Entity],
) -> Result<Relationship, Error> {
    let path = format!("$.relationships[{index}]");
    let source = object(value, &path)?;
    let name = optional_text(source, "cardinality", &format!("{path}.cardinality"))?;

    let cardinality = match name {
        None => Cardinality::default(),
        Some(name) => cardinality_from_name(&name).ok_or_else(|| {
            refuse(
                &format!("{path}.cardinality"),
                "field must name a known cardinality",
            )
        })?,
    };

    let mut relationship = Relationship::new(
        identity(source, "id", || format!("r{}", Uuid::new_v4().simple())),
        (
            &required_text(source, "fromEntityId", &format!("{path}.fromEntityId"))?,
            &required_text(
                source,
                "fromAttributeId",
                &format!("{path}.fromAttributeId"),
            )?,
        ),
        (
            &required_text(source, "toEntityId", &format!("{path}.toEntityId"))?,
            &required_text(source, "toAttributeId", &format!("{path}.toAttributeId"))?,
        ),
        cardinality,
    );

    relationship.name = optional_text(source, "name", &format!("{path}.name"))?.unwrap_or_default();
    relationship.description =
        optional_text(source, "description", &format!("{path}.description"))?.unwrap_or_default();

    // A line drawn between tables that are not in the file would be a
    // dangling relationship the verifier reports on a diagram the user never
    // drew, so it is refused here where the path can be named.
    for (key, id) in [
        ("fromEntityId", &relationship.from_entity_id),
        ("toEntityId", &relationship.to_entity_id),
    ] {
        if !entities.iter().any(|entity| entity.id == *id) {
            return Err(refuse(
                &format!("{path}.{key}"),
                "field names an entity that is not in the file",
            ));
        }
    }

    Ok(relationship)
}

fn identity(source: &Map<String, Value>, key: &str, fresh: impl Fn() -> String) -> String {
    source
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map_or_else(fresh, str::to_owned)
}
