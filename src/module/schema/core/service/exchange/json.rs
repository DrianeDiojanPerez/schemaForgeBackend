use serde::Serialize;

use crate::module::schema::core::domain::{
    Attribute, Entity, Relationship, Schema,
};

use super::{cardinality_name, kind_name};

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
