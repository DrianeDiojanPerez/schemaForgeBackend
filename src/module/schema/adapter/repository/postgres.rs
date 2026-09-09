//! The PostgreSQL store.
//!
//! Entities and relationships are held in two JSONB columns rather than in
//! tables of their own. The canvas always sends the whole picture and always
//! reads the whole picture back, so there is no query that would ever ask for
//! one attribute, and normalising would turn every save into a delete and
//! reinsert of every row.
//!
//! The records below are the stored shape, kept apart from the JSON the API
//! speaks. The two are free to move independently: renaming a field on the
//! wire is not a migration, and changing the storage does not break a client.

use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use sqlx::postgres::PgRow;
use sqlx::types::Json;
use sqlx::Row;
use uuid::Uuid;

use crate::database::{pg_error, Database};
use crate::module::schema::core::domain::{
    Attribute, Cardinality, DataType, DataTypeKind, DomainError, Entity, ForeignKeyRef, Position,
    Relationship, Schema, SchemaDraft, SchemaSummary,
};
use crate::module::schema::core::ports::SchemaRepository;
use crate::package::pagination::ListRequest;

const SELECT_SCHEMA: &str = "SELECT id, name, description, entities, relationships, \
     created_at, updated_at \
     FROM schemaforge.schemas";

const SELECT_SUMMARY: &str = "SELECT id, name, description, \
     jsonb_array_length(entities) AS entity_count, \
     jsonb_array_length(relationships) AS relationship_count, \
     created_at, updated_at \
     FROM schemaforge.schemas";

#[derive(Debug, Serialize, Deserialize)]
struct StoredPosition {
    x: f64,
    y: f64,
}

#[derive(Debug, Serialize, Deserialize)]
struct StoredDataType {
    kind: String,
    #[serde(default)]
    length: Option<u32>,
    #[serde(default)]
    precision: Option<u32>,
    #[serde(default)]
    scale: Option<u32>,
}

#[derive(Debug, Serialize, Deserialize)]
struct StoredForeignKey {
    entity_id: String,
    attribute_id: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct StoredAttribute {
    id: String,
    name: String,
    #[serde(default)]
    description: String,
    data_type: StoredDataType,
    nullable: bool,
    primary_key: bool,
    unique: bool,
    #[serde(default)]
    foreign_key: Option<StoredForeignKey>,
    #[serde(default)]
    default_value: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct StoredEntity {
    id: String,
    name: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    attributes: Vec<StoredAttribute>,
    position: StoredPosition,
}

#[derive(Debug, Serialize, Deserialize)]
struct StoredRelationship {
    id: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    description: String,
    from_entity_id: String,
    from_attribute_id: String,
    to_entity_id: String,
    to_attribute_id: String,
    cardinality: String,
}

impl From<Position> for StoredPosition {
    fn from(position: Position) -> Self {
        Self {
            x: position.x,
            y: position.y,
        }
    }
}

impl From<DataType> for StoredDataType {
    fn from(data_type: DataType) -> Self {
        Self {
            kind: data_type.kind.as_str().to_owned(),
            length: data_type.length,
            precision: data_type.precision,
            scale: data_type.scale,
        }
    }
}

impl From<ForeignKeyRef> for StoredForeignKey {
    fn from(reference: ForeignKeyRef) -> Self {
        Self {
            entity_id: reference.entity_id,
            attribute_id: reference.attribute_id,
        }
    }
}

impl From<Attribute> for StoredAttribute {
    fn from(attribute: Attribute) -> Self {
        Self {
            id: attribute.id,
            name: attribute.name,
            description: attribute.description,
            data_type: attribute.data_type.into(),
            nullable: attribute.nullable,
            primary_key: attribute.primary_key,
            unique: attribute.unique,
            foreign_key: attribute.foreign_key.map(StoredForeignKey::from),
            default_value: attribute.default_value,
        }
    }
}

impl From<Entity> for StoredEntity {
    fn from(entity: Entity) -> Self {
        Self {
            id: entity.id,
            name: entity.name,
            description: entity.description,
            attributes: entity.attributes.into_iter().map(Into::into).collect(),
            position: entity.position.into(),
        }
    }
}

impl From<Relationship> for StoredRelationship {
    fn from(relationship: Relationship) -> Self {
        Self {
            id: relationship.id,
            name: relationship.name,
            description: relationship.description,
            from_entity_id: relationship.from_entity_id,
            from_attribute_id: relationship.from_attribute_id,
            to_entity_id: relationship.to_entity_id,
            to_attribute_id: relationship.to_attribute_id,
            cardinality: relationship.cardinality.as_str().to_owned(),
        }
    }
}

impl TryFrom<StoredDataType> for DataType {
    type Error = DomainError;

    fn try_from(stored: StoredDataType) -> Result<Self, Self::Error> {
        let kind: DataTypeKind = stored.kind.parse().map_err(|_| {
            DomainError::Storage(format!("`{}` is not a known data type", stored.kind))
        })?;

        Ok(Self {
            kind,
            length: stored.length,
            precision: stored.precision,
            scale: stored.scale,
        })
    }
}

impl TryFrom<StoredAttribute> for Attribute {
    type Error = DomainError;

    fn try_from(stored: StoredAttribute) -> Result<Self, Self::Error> {
        Ok(Self {
            id: stored.id,
            name: stored.name,
            description: stored.description,
            data_type: stored.data_type.try_into()?,
            nullable: stored.nullable,
            primary_key: stored.primary_key,
            unique: stored.unique,
            foreign_key: stored
                .foreign_key
                .map(|reference| ForeignKeyRef::new(reference.entity_id, reference.attribute_id)),
            default_value: stored.default_value,
        })
    }
}

impl TryFrom<StoredEntity> for Entity {
    type Error = DomainError;

    fn try_from(stored: StoredEntity) -> Result<Self, Self::Error> {
        Ok(Self {
            id: stored.id,
            name: stored.name,
            description: stored.description,
            attributes: stored
                .attributes
                .into_iter()
                .map(Attribute::try_from)
                .collect::<Result<Vec<_>, _>>()?,
            position: Position::new(stored.position.x, stored.position.y),
        })
    }
}

impl TryFrom<StoredRelationship> for Relationship {
    type Error = DomainError;

    fn try_from(stored: StoredRelationship) -> Result<Self, Self::Error> {
        let cardinality: Cardinality = stored.cardinality.parse().map_err(|_| {
            DomainError::Storage(format!(
                "`{}` is not a known cardinality",
                stored.cardinality
            ))
        })?;

        Ok(Self {
            id: stored.id,
            name: stored.name,
            description: stored.description,
            from_entity_id: stored.from_entity_id,
            from_attribute_id: stored.from_attribute_id,
            to_entity_id: stored.to_entity_id,
            to_attribute_id: stored.to_attribute_id,
            cardinality,
        })
    }
}

pub struct PgSchemaRepository {
    db: Arc<Database>,
}

impl PgSchemaRepository {
    pub fn new(db: Arc<Database>) -> Self {
        Self { db }
    }

    /// The cause is only ever logged, so the database's own wording is kept
    /// rather than flattened into something the caller would see anyway.
    fn storage(err: sqlx::Error) -> DomainError {
        DomainError::Storage(err.to_string())
    }

    /// The unique index on the lowercased name is what enforces the rule, so
    /// two callers racing on the same name still get one schema and one
    /// conflict instead of both succeeding.
    fn conflict_or_storage(err: sqlx::Error, name: &str) -> DomainError {
        if pg_error::is(&err, pg_error::UNIQUE_VIOLATION) {
            return DomainError::DuplicateSchemaName(name.to_owned());
        }

        Self::storage(err)
    }

    /// An id that is not a UUID cannot name a stored row, so it reads as
    /// missing rather than as a malformed request the caller has to handle
    /// differently from a deleted schema.
    fn parse_id(id: &str) -> Option<Uuid> {
        Uuid::parse_str(id).ok()
    }

    fn map_schema(row: &PgRow) -> Result<Schema, DomainError> {
        let id: Uuid = row.try_get("id").map_err(Self::storage)?;
        let entities: Json<Vec<StoredEntity>> = row.try_get("entities").map_err(Self::storage)?;
        let relationships: Json<Vec<StoredRelationship>> =
            row.try_get("relationships").map_err(Self::storage)?;

        Ok(Schema {
            id: id.to_string(),
            name: row.try_get("name").map_err(Self::storage)?,
            description: row.try_get("description").map_err(Self::storage)?,
            entities: entities
                .0
                .into_iter()
                .map(Entity::try_from)
                .collect::<Result<Vec<_>, _>>()?,
            relationships: relationships
                .0
                .into_iter()
                .map(Relationship::try_from)
                .collect::<Result<Vec<_>, _>>()?,
            created_at: row.try_get("created_at").map_err(Self::storage)?,
            updated_at: row.try_get("updated_at").map_err(Self::storage)?,
        })
    }

    /// The counts come from the database rather than from a decoded document,
    /// so a listing never pays for parsing entities it does not return.
    fn map_summary(row: &PgRow) -> Result<SchemaSummary, DomainError> {
        let id: Uuid = row.try_get("id").map_err(Self::storage)?;
        let entity_count: i32 = row.try_get("entity_count").map_err(Self::storage)?;
        let relationship_count: i32 = row.try_get("relationship_count").map_err(Self::storage)?;

        Ok(SchemaSummary {
            id: id.to_string(),
            name: row.try_get("name").map_err(Self::storage)?,
            description: row.try_get("description").map_err(Self::storage)?,
            entity_count: entity_count.max(0) as u32,
            relationship_count: relationship_count.max(0) as u32,
            created_at: row.try_get("created_at").map_err(Self::storage)?,
            updated_at: row.try_get("updated_at").map_err(Self::storage)?,
        })
    }

    fn drawing(draft: SchemaDraft) -> (Vec<StoredEntity>, Vec<StoredRelationship>) {
        (
            draft.entities.into_iter().map(Into::into).collect(),
            draft.relationships.into_iter().map(Into::into).collect(),
        )
    }
}

#[async_trait]
impl SchemaRepository for PgSchemaRepository {
    async fn index(&self, request: &ListRequest) -> Result<(Vec<SchemaSummary>, i64), DomainError> {
        let total: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM schemaforge.schemas")
            .fetch_one(self.db.pool())
            .await
            .map_err(Self::storage)?;

        let rows = sqlx::query(&format!(
            "{SELECT_SUMMARY} ORDER BY created_at, id LIMIT $1 OFFSET $2"
        ))
        .bind(request.per_page)
        .bind(request.offset())
        .fetch_all(self.db.pool())
        .await
        .map_err(Self::storage)?;

        let summaries = rows
            .iter()
            .map(Self::map_summary)
            .collect::<Result<Vec<_>, _>>()?;

        Ok((summaries, total))
    }

    async fn create(&self, draft: SchemaDraft) -> Result<Schema, DomainError> {
        let name = draft.name.clone();
        let description = draft.description.clone();
        let (entities, relationships) = Self::drawing(draft);

        let row = sqlx::query(
            "INSERT INTO schemaforge.schemas (name, description, entities, relationships) \
             VALUES ($1, $2, $3, $4) \
             RETURNING id, name, description, entities, relationships, created_at, updated_at",
        )
        .bind(&name)
        .bind(&description)
        .bind(Json(&entities))
        .bind(Json(&relationships))
        .fetch_one(self.db.pool())
        .await
        .map_err(|err| Self::conflict_or_storage(err, &name))?;

        Self::map_schema(&row)
    }

    async fn find_by_id(&self, id: &str) -> Result<Option<Schema>, DomainError> {
        let Some(uuid) = Self::parse_id(id) else {
            return Ok(None);
        };

        let row = sqlx::query(&format!("{SELECT_SCHEMA} WHERE id = $1"))
            .bind(uuid)
            .fetch_optional(self.db.pool())
            .await
            .map_err(Self::storage)?;

        row.as_ref().map(Self::map_schema).transpose()
    }

    async fn find_by_name(&self, name: &str) -> Result<Option<Schema>, DomainError> {
        let row = sqlx::query(&format!("{SELECT_SCHEMA} WHERE LOWER(name) = LOWER($1)"))
            .bind(name)
            .fetch_optional(self.db.pool())
            .await
            .map_err(Self::storage)?;

        row.as_ref().map(Self::map_schema).transpose()
    }

    async fn replace(&self, id: &str, draft: SchemaDraft) -> Result<Schema, DomainError> {
        let uuid = Self::parse_id(id).ok_or(DomainError::SchemaNotFound)?;

        let name = draft.name.clone();
        let description = draft.description.clone();
        let (entities, relationships) = Self::drawing(draft);

        let row = sqlx::query(
            "UPDATE schemaforge.schemas \
             SET name = $2, description = $3, entities = $4, relationships = $5, \
                 updated_at = NOW() \
             WHERE id = $1 \
             RETURNING id, name, description, entities, relationships, created_at, updated_at",
        )
        .bind(uuid)
        .bind(&name)
        .bind(&description)
        .bind(Json(&entities))
        .bind(Json(&relationships))
        .fetch_optional(self.db.pool())
        .await
        .map_err(|err| Self::conflict_or_storage(err, &name))?
        .ok_or(DomainError::SchemaNotFound)?;

        Self::map_schema(&row)
    }

    async fn delete(&self, id: &str) -> Result<(), DomainError> {
        let uuid = Self::parse_id(id).ok_or(DomainError::SchemaNotFound)?;

        let result = sqlx::query("DELETE FROM schemaforge.schemas WHERE id = $1")
            .bind(uuid)
            .execute(self.db.pool())
            .await
            .map_err(Self::storage)?;

        if result.rows_affected() == 0 {
            return Err(DomainError::SchemaNotFound);
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn attribute() -> Attribute {
        Attribute::new("a1", "id", DataType::simple(DataTypeKind::Uuid))
            .as_primary_key()
            .unique()
            .with_description("Surrogate key")
            .with_default("gen_random_uuid()")
            .referencing(ForeignKeyRef::new("e2", "a9"))
    }

    fn entity() -> Entity {
        Entity::new("e1", "users")
            .with_description("A registered account")
            .with_position(Position::new(120.5, -40.0))
            .with_attributes(vec![attribute()])
    }

    fn relationship() -> Relationship {
        Relationship::new("r1", ("e1", "a1"), ("e2", "a9"), Cardinality::ManyToMany)
            .with_name("enrolments")
            .with_description("Who is taking what")
    }

    #[test]
    fn an_entity_survives_the_round_trip_through_storage() {
        let original = entity();
        let stored = StoredEntity::from(original.clone());

        let json = serde_json::to_string(&stored).expect("the record should serialise");
        let decoded: StoredEntity =
            serde_json::from_str(&json).expect("the record should deserialise");

        assert_eq!(
            Entity::try_from(decoded).expect("the stored entity should map back"),
            original
        );
    }

    #[test]
    fn a_relationship_survives_the_round_trip_through_storage() {
        let original = relationship();
        let stored = StoredRelationship::from(original.clone());

        let json = serde_json::to_string(&stored).expect("the record should serialise");
        let decoded: StoredRelationship =
            serde_json::from_str(&json).expect("the record should deserialise");

        assert_eq!(
            Relationship::try_from(decoded).expect("the stored relationship should map back"),
            original
        );
    }

    #[test]
    fn every_data_type_keeps_its_spelling_in_storage() {
        for kind in DataTypeKind::VARIANTS {
            let stored = StoredDataType::from(DataType::simple(kind));

            assert_eq!(
                DataType::try_from(stored)
                    .expect("a stored kind should map back")
                    .kind,
                kind
            );
        }
    }

    #[test]
    fn a_type_the_model_cannot_hold_is_a_storage_failure() {
        let stored = StoredDataType {
            kind: "money".to_owned(),
            length: None,
            precision: None,
            scale: None,
        };

        assert!(matches!(
            DataType::try_from(stored).expect_err("an unknown kind should fail"),
            DomainError::Storage(_)
        ));
    }

    #[test]
    fn a_cardinality_the_model_cannot_hold_is_a_storage_failure() {
        let mut stored = StoredRelationship::from(relationship());
        stored.cardinality = "1:MANY".to_owned();

        assert!(matches!(
            Relationship::try_from(stored).expect_err("an unknown cardinality should fail"),
            DomainError::Storage(_)
        ));
    }

    #[test]
    fn an_id_that_is_not_a_uuid_names_nothing() {
        assert!(PgSchemaRepository::parse_id("does-not-exist").is_none());
        assert!(PgSchemaRepository::parse_id(&Uuid::new_v4().to_string()).is_some());
    }
}
