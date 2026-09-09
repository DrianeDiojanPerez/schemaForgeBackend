use super::data_type::DataType;

/// A reference to the attribute a foreign key points at. It names ids rather
/// than names, so renaming the referenced column cannot silently break the
/// reference.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ForeignKeyRef {
    pub entity_id: String,
    pub attribute_id: String,
}

impl ForeignKeyRef {
    pub fn new(entity_id: impl Into<String>, attribute_id: impl Into<String>) -> Self {
        Self {
            entity_id: entity_id.into(),
            attribute_id: attribute_id.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Attribute {
    pub id: String,
    pub name: String,
    /// Emitted as COMMENT ON COLUMN.
    pub description: String,
    pub data_type: DataType,
    pub nullable: bool,
    pub primary_key: bool,
    pub unique: bool,
    pub foreign_key: Option<ForeignKeyRef>,
    pub default_value: Option<String>,
}

impl Attribute {
    pub fn new(id: impl Into<String>, name: impl Into<String>, data_type: DataType) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            description: String::new(),
            data_type,
            nullable: true,
            primary_key: false,
            unique: false,
            foreign_key: None,
            default_value: None,
        }
    }

    /// A primary key is never nullable, so flagging one clears nullability
    /// rather than leaving the model in a state the generator would have to
    /// second-guess.
    pub fn as_primary_key(mut self) -> Self {
        self.primary_key = true;
        self.nullable = false;
        self
    }

    pub fn required(mut self) -> Self {
        self.nullable = false;
        self
    }

    pub fn unique(mut self) -> Self {
        self.unique = true;
        self
    }

    pub fn referencing(mut self, reference: ForeignKeyRef) -> Self {
        self.foreign_key = Some(reference);
        self
    }

    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = description.into();
        self
    }

    pub fn with_default(mut self, default_value: impl Into<String>) -> Self {
        self.default_value = Some(default_value.into());
        self
    }

    pub fn is_foreign_key(&self) -> bool {
        self.foreign_key.is_some()
    }

    pub fn is_documented(&self) -> bool {
        !self.description.trim().is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::super::data_type::DataTypeKind;
    use super::*;

    #[test]
    fn an_attribute_is_nullable_until_told_otherwise() {
        let attribute = Attribute::new("a1", "nickname", DataType::simple(DataTypeKind::Text));

        assert!(attribute.nullable);
        assert!(!attribute.primary_key);
        assert!(!attribute.unique);
        assert!(!attribute.is_foreign_key());
    }

    #[test]
    fn a_primary_key_is_not_nullable() {
        let attribute =
            Attribute::new("a1", "id", DataType::simple(DataTypeKind::Uuid)).as_primary_key();

        assert!(attribute.primary_key);
        assert!(
            !attribute.nullable,
            "a nullable primary key is not a thing a database will accept"
        );
    }

    #[test]
    fn a_foreign_key_names_the_ids_it_points_at() {
        let attribute = Attribute::new("a2", "user_id", DataType::simple(DataTypeKind::Uuid))
            .referencing(ForeignKeyRef::new("e1", "a1"));

        assert!(attribute.is_foreign_key());
        assert_eq!(
            attribute.foreign_key,
            Some(ForeignKeyRef::new("e1", "a1")),
            "renaming the target column must not break the reference"
        );
    }

    #[test]
    fn a_blank_description_does_not_count_as_documentation() {
        let bare = Attribute::new("a1", "id", DataType::simple(DataTypeKind::Uuid));
        let documented = Attribute::new("a2", "id", DataType::simple(DataTypeKind::Uuid))
            .with_description("Surrogate key");

        assert!(!bare.is_documented());
        assert!(documented.is_documented());
    }
}
