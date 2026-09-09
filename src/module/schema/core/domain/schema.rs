use chrono::{DateTime, Utc};

use super::attribute::Attribute;
use super::entity::Entity;
use super::relationship::Relationship;

/// The canonical schema model. Verification, validation, and generation all
/// read this one structure, which is why the diagram the user sees, the errors
/// reported, and the SQL produced cannot describe different schemas.
#[derive(Debug, Clone, PartialEq)]
pub struct Schema {
    pub id: String,
    pub name: String,
    pub description: String,
    pub entities: Vec<Entity>,
    pub relationships: Vec<Relationship>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// The parts of a schema a caller supplies. The id and the timestamps are the
/// store's to assign, so they are absent here rather than present and ignored.
#[derive(Debug, Clone, Default)]
pub struct SchemaDraft {
    pub name: String,
    pub description: String,
    pub entities: Vec<Entity>,
    pub relationships: Vec<Relationship>,
}

/// The summary form a listing returns, so listing many schemas does not ship
/// every entity and relationship over the wire.
#[derive(Debug, Clone, PartialEq)]
pub struct SchemaSummary {
    pub id: String,
    pub name: String,
    pub description: String,
    pub entity_count: u32,
    pub relationship_count: u32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Schema {
    pub fn new(id: impl Into<String>, draft: SchemaDraft, now: DateTime<Utc>) -> Self {
        Self {
            id: id.into(),
            name: draft.name,
            description: draft.description,
            entities: draft.entities,
            relationships: draft.relationships,
            created_at: now,
            updated_at: now,
        }
    }

    /// Replaces the drawn content while keeping identity and creation time. The
    /// canvas holds the authoritative picture, so it sends the whole picture
    /// rather than a diff the backend would have to reassemble.
    pub fn replace_with(&mut self, draft: SchemaDraft, now: DateTime<Utc>) {
        self.name = draft.name;
        self.description = draft.description;
        self.entities = draft.entities;
        self.relationships = draft.relationships;
        self.updated_at = now;
    }

    pub fn entity(&self, entity_id: &str) -> Option<&Entity> {
        self.entities.iter().find(|entity| entity.id == entity_id)
    }

    pub fn entity_by_name(&self, name: &str) -> Option<&Entity> {
        self.entities
            .iter()
            .find(|entity| entity.name.eq_ignore_ascii_case(name))
    }

    pub fn relationship(&self, relationship_id: &str) -> Option<&Relationship> {
        self.relationships
            .iter()
            .find(|relationship| relationship.id == relationship_id)
    }

    /// Resolves an attribute through the entity that owns it. Returning both
    /// halves saves every caller from looking the entity up a second time to
    /// report which table a defect is on.
    pub fn resolve(&self, entity_id: &str, attribute_id: &str) -> Option<(&Entity, &Attribute)> {
        let entity = self.entity(entity_id)?;
        let attribute = entity.attribute(attribute_id)?;

        Some((entity, attribute))
    }

    pub fn relationships_for<'a>(
        &'a self,
        entity_id: &'a str,
    ) -> impl Iterator<Item = &'a Relationship> {
        self.relationships
            .iter()
            .filter(move |relationship| relationship.touches(entity_id))
    }

    pub fn summary(&self) -> SchemaSummary {
        SchemaSummary {
            id: self.id.clone(),
            name: self.name.clone(),
            description: self.description.clone(),
            entity_count: self.entities.len() as u32,
            relationship_count: self.relationships.len() as u32,
            created_at: self.created_at,
            updated_at: self.updated_at,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.entities.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::super::attribute::Attribute;
    use super::super::data_type::{DataType, DataTypeKind};
    use super::super::entity::Entity;
    use super::super::relationship::Cardinality;
    use super::*;

    fn now() -> DateTime<Utc> {
        DateTime::from_timestamp(1_700_000_000, 0).expect("a fixed instant")
    }

    fn later() -> DateTime<Utc> {
        DateTime::from_timestamp(1_700_086_400, 0).expect("a fixed instant")
    }

    fn users() -> Entity {
        Entity::new("e1", "users").with_attributes(vec![Attribute::new(
            "a1",
            "id",
            DataType::simple(DataTypeKind::Uuid),
        )
        .as_primary_key()])
    }

    fn posts() -> Entity {
        Entity::new("e2", "posts").with_attributes(vec![
            Attribute::new("a2", "id", DataType::simple(DataTypeKind::Uuid)).as_primary_key(),
            Attribute::new("a3", "user_id", DataType::simple(DataTypeKind::Uuid)),
        ])
    }

    fn draft() -> SchemaDraft {
        SchemaDraft {
            name: "blog".to_owned(),
            description: "A blog".to_owned(),
            entities: vec![users(), posts()],
            relationships: vec![Relationship::new(
                "r1",
                ("e2", "a3"),
                ("e1", "a1"),
                Cardinality::OneToMany,
            )],
        }
    }

    #[test]
    fn a_new_schema_is_created_and_updated_at_the_same_instant() {
        let schema = Schema::new("s1", draft(), now());

        assert_eq!(schema.created_at, schema.updated_at);
    }

    #[test]
    fn replacing_the_content_keeps_identity_and_creation_time() {
        let mut schema = Schema::new("s1", draft(), now());

        schema.replace_with(
            SchemaDraft {
                name: "renamed".to_owned(),
                ..SchemaDraft::default()
            },
            later(),
        );

        assert_eq!(schema.id, "s1");
        assert_eq!(schema.created_at, now(), "creation time is not rewritten");
        assert_eq!(schema.updated_at, later());
        assert_eq!(schema.name, "renamed");
        assert!(schema.is_empty(), "the new draft drew no entities");
    }

    #[test]
    fn resolves_an_attribute_through_its_owning_entity() {
        let schema = Schema::new("s1", draft(), now());

        let (entity, attribute) = schema.resolve("e2", "a3").expect("the pair should resolve");

        assert_eq!(entity.name, "posts");
        assert_eq!(attribute.name, "user_id");
    }

    #[test]
    fn resolving_an_attribute_on_the_wrong_entity_fails() {
        let schema = Schema::new("s1", draft(), now());

        assert!(
            schema.resolve("e1", "a3").is_none(),
            "a3 belongs to posts, not users"
        );
    }

    #[test]
    fn finds_an_entity_by_name_regardless_of_case() {
        let schema = Schema::new("s1", draft(), now());

        assert!(schema.entity_by_name("USERS").is_some());
        assert!(schema.entity_by_name("comments").is_none());
    }

    #[test]
    fn collects_the_relationships_touching_an_entity() {
        let schema = Schema::new("s1", draft(), now());

        assert_eq!(schema.relationships_for("e1").count(), 1);
        assert_eq!(schema.relationships_for("e2").count(), 1);
        assert_eq!(schema.relationships_for("e3").count(), 0);
    }

    #[test]
    fn a_summary_counts_without_carrying_the_content() {
        let summary = Schema::new("s1", draft(), now()).summary();

        assert_eq!(summary.entity_count, 2);
        assert_eq!(summary.relationship_count, 1);
        assert_eq!(summary.name, "blog");
    }
}
