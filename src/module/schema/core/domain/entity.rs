use super::attribute::Attribute;

/// Canvas placement. It lives in the model rather than in the frontend so a
/// schema that is saved, reloaded, or round-tripped through the database comes
/// back looking the way the user left it.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Position {
    pub x: f64,
    pub y: f64,
}

impl Position {
    pub const fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Entity {
    pub id: String,
    pub name: String,
    /// Meta-knowledge: what this table means, not how it is typed. Emitted as
    /// COMMENT ON TABLE so the documentation travels with the schema.
    pub description: String,
    pub attributes: Vec<Attribute>,
    pub position: Position,
}

impl Entity {
    pub fn new(id: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            description: String::new(),
            attributes: Vec::new(),
            position: Position::default(),
        }
    }

    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = description.into();
        self
    }

    pub fn with_position(mut self, position: Position) -> Self {
        self.position = position;
        self
    }

    pub fn with_attributes(mut self, attributes: Vec<Attribute>) -> Self {
        self.attributes = attributes;
        self
    }

    pub fn attribute(&self, attribute_id: &str) -> Option<&Attribute> {
        self.attributes
            .iter()
            .find(|attribute| attribute.id == attribute_id)
    }

    pub fn attribute_by_name(&self, name: &str) -> Option<&Attribute> {
        self.attributes
            .iter()
            .find(|attribute| attribute.name.eq_ignore_ascii_case(name))
    }

    /// Every attribute flagged as part of the primary key, in declaration
    /// order. A composite key is several; a table with no key is none, which is
    /// one of the defects the validation engine reports.
    pub fn primary_key(&self) -> Vec<&Attribute> {
        self.attributes
            .iter()
            .filter(|attribute| attribute.primary_key)
            .collect()
    }

    pub fn has_primary_key(&self) -> bool {
        self.attributes
            .iter()
            .any(|attribute| attribute.primary_key)
    }

    pub fn is_documented(&self) -> bool {
        !self.description.trim().is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::super::attribute::Attribute;
    use super::super::data_type::{DataType, DataTypeKind};
    use super::*;

    fn key(id: &str, name: &str) -> Attribute {
        Attribute::new(id, name, DataType::simple(DataTypeKind::Integer)).as_primary_key()
    }

    fn plain(id: &str, name: &str) -> Attribute {
        Attribute::new(id, name, DataType::simple(DataTypeKind::Text))
    }

    #[test]
    fn finds_an_attribute_by_id() {
        let entity = Entity::new("e1", "users").with_attributes(vec![key("a1", "id")]);

        assert_eq!(entity.attribute("a1").map(|a| a.name.as_str()), Some("id"));
        assert!(entity.attribute("missing").is_none());
    }

    #[test]
    fn finds_an_attribute_by_name_regardless_of_case() {
        let entity = Entity::new("e1", "users").with_attributes(vec![plain("a1", "email")]);

        assert!(entity.attribute_by_name("EMAIL").is_some());
    }

    #[test]
    fn reports_a_composite_key_in_declaration_order() {
        let entity = Entity::new("e1", "enrollments").with_attributes(vec![
            key("a1", "student_id"),
            key("a2", "course_id"),
            plain("a3", "grade"),
        ]);

        let names: Vec<_> = entity
            .primary_key()
            .iter()
            .map(|a| a.name.as_str())
            .collect();

        assert_eq!(names, vec!["student_id", "course_id"]);
    }

    #[test]
    fn an_entity_with_no_key_reports_none() {
        let entity = Entity::new("e1", "logs").with_attributes(vec![plain("a1", "line")]);

        assert!(!entity.has_primary_key());
        assert!(entity.primary_key().is_empty());
    }

    #[test]
    fn a_blank_description_does_not_count_as_documentation() {
        let entity = Entity::new("e1", "users").with_description("   ");

        assert!(!entity.is_documented());
        assert!(Entity::new("e2", "users")
            .with_description("A registered account")
            .is_documented());
    }
}
