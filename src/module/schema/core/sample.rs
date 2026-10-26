use super::domain::{
    Attribute, Cardinality, DataType, DataTypeKind, Entity, ForeignKeyRef, Position, Relationship,
    SchemaDraft,
};

pub const SAMPLE_SCHEMA_NAME: &str = "Sample blog";

/// The schema every new account starts with, so the first thing a person
/// sees is a finished diagram rather than an empty canvas.
pub fn sample_schema() -> SchemaDraft {
    SchemaDraft {
        name: SAMPLE_SCHEMA_NAME.to_owned(),
        description: String::new(),
        entities: vec![users(), posts(), comments()],
        relationships: vec![
            Relationship::new(
                "rel-users-posts",
                ("users", "users.id"),
                ("posts", "posts.user_id"),
                Cardinality::OneToMany,
            ),
            Relationship::new(
                "rel-posts-comments",
                ("posts", "posts.id"),
                ("comments", "comments.post_id"),
                Cardinality::OneToMany,
            ),
            Relationship::new(
                "rel-users-comments",
                ("users", "users.id"),
                ("comments", "comments.user_id"),
                Cardinality::OneToMany,
            ),
        ],
    }
}

fn users() -> Entity {
    Entity::new("users", "users")
        .with_position(Position::new(100.0, 100.0))
        .with_attributes(vec![
            key("users.id"),
            varchar("users.email", "email").unique(),
            varchar("users.name", "name"),
            timestamp("users.created_at"),
        ])
}

fn posts() -> Entity {
    Entity::new("posts", "posts")
        .with_position(Position::new(500.0, 100.0))
        .with_attributes(vec![
            key("posts.id"),
            reference("posts.user_id", "user_id", "users"),
            varchar("posts.title", "title"),
            text("posts.content", "content"),
            timestamp("posts.created_at"),
        ])
}

fn comments() -> Entity {
    Entity::new("comments", "comments")
        .with_position(Position::new(300.0, 400.0))
        .with_attributes(vec![
            key("comments.id"),
            reference("comments.post_id", "post_id", "posts"),
            reference("comments.user_id", "user_id", "users"),
            text("comments.content", "content").required(),
            timestamp("comments.created_at"),
        ])
}

fn key(id: &str) -> Attribute {
    Attribute::new(id, "id", DataType::simple(DataTypeKind::Uuid))
        .as_primary_key()
        .unique()
}

fn reference(id: &str, name: &str, entity: &str) -> Attribute {
    Attribute::new(id, name, DataType::simple(DataTypeKind::Uuid))
        .required()
        .referencing(ForeignKeyRef::new(entity, format!("{entity}.id")))
}

fn varchar(id: &str, name: &str) -> Attribute {
    Attribute::new(id, name, DataType::simple(DataTypeKind::Varchar)).required()
}

fn text(id: &str, name: &str) -> Attribute {
    Attribute::new(id, name, DataType::simple(DataTypeKind::Text))
}

fn timestamp(id: &str) -> Attribute {
    Attribute::new(id, "created_at", DataType::simple(DataTypeKind::Timestamp)).required()
}

#[cfg(test)]
mod tests {
    use super::super::domain::Schema;
    use super::super::ports::Verifier;
    use super::super::service::SchemaVerifier;
    use super::*;

    #[test]
    fn the_sample_verifies_clean() {
        let now = chrono::DateTime::from_timestamp(1_700_000_000, 0).expect("a fixed instant");
        let report = SchemaVerifier.verify(&Schema::new("s1", sample_schema(), now));

        assert!(
            report.is_valid(),
            "{:?}",
            report.errors().collect::<Vec<_>>()
        );
    }

    #[test]
    fn the_sample_draws_three_tables_and_three_links() {
        let draft = sample_schema();

        assert_eq!(draft.name, "Sample blog");
        assert!(draft.description.is_empty());
        assert_eq!(draft.entities.len(), 3);
        assert_eq!(draft.relationships.len(), 3);
    }

    #[test]
    fn every_relationship_points_at_attributes_that_exist() {
        let draft = sample_schema();

        for relationship in &draft.relationships {
            for (entity_id, attribute_id) in [
                (
                    &relationship.from_entity_id,
                    &relationship.from_attribute_id,
                ),
                (&relationship.to_entity_id, &relationship.to_attribute_id),
            ] {
                let entity = draft
                    .entities
                    .iter()
                    .find(|entity| &entity.id == entity_id)
                    .unwrap_or_else(|| panic!("entity {entity_id} should exist"));

                assert!(
                    entity.attribute(attribute_id).is_some(),
                    "attribute {attribute_id} should exist on {entity_id}"
                );
            }
        }
    }

    #[test]
    fn every_foreign_key_targets_a_primary_key() {
        let draft = sample_schema();

        let keys: Vec<_> = draft
            .entities
            .iter()
            .flat_map(|entity| {
                entity
                    .attributes
                    .iter()
                    .filter_map(|a| a.foreign_key.as_ref())
            })
            .collect();

        assert_eq!(keys.len(), 3);

        for reference in keys {
            let entity = draft
                .entities
                .iter()
                .find(|entity| entity.id == reference.entity_id)
                .expect("the referenced entity should exist");
            let target = entity
                .attribute(&reference.attribute_id)
                .expect("the referenced attribute should exist");

            assert!(target.primary_key);
        }
    }
}
