use std::fmt;
use std::str::FromStr;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Cardinality {
    OneToOne,
    #[default]
    OneToMany,
    ManyToMany,
}

impl Cardinality {
    pub const fn as_str(self) -> &'static str {
        match self {
            Cardinality::OneToOne => "1:1",
            Cardinality::OneToMany => "1:N",
            Cardinality::ManyToMany => "N:M",
        }
    }

    pub const VARIANTS: [&'static str; 3] = ["1:1", "1:N", "N:M"];

    /// A many-to-many needs a join table that no entity on the canvas
    /// represents, so the generator has to synthesise one. Everything else
    /// becomes a foreign key on an existing table.
    pub const fn requires_join_table(self) -> bool {
        matches!(self, Cardinality::ManyToMany)
    }
}

impl fmt::Display for Cardinality {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for Cardinality {
    type Err = ();

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim().to_ascii_uppercase().as_str() {
            "1:1" | "ONE_TO_ONE" => Ok(Cardinality::OneToOne),
            "1:N" | "ONE_TO_MANY" => Ok(Cardinality::OneToMany),
            "N:M" | "MANY_TO_MANY" => Ok(Cardinality::ManyToMany),
            _ => Err(()),
        }
    }
}

/// A relationship connects exactly two attributes on two entities and carries
/// a cardinality. It is a first-class part of the model rather than a line
/// drawn between boxes, which is what lets the generator emit the constraint
/// and the verifier check that both ends exist.
#[derive(Debug, Clone, PartialEq)]
pub struct Relationship {
    pub id: String,
    pub name: String,
    pub description: String,
    pub from_entity_id: String,
    pub from_attribute_id: String,
    pub to_entity_id: String,
    pub to_attribute_id: String,
    pub cardinality: Cardinality,
}

impl Relationship {
    pub fn new(
        id: impl Into<String>,
        from: (&str, &str),
        to: (&str, &str),
        cardinality: Cardinality,
    ) -> Self {
        Self {
            id: id.into(),
            name: String::new(),
            description: String::new(),
            from_entity_id: from.0.to_owned(),
            from_attribute_id: from.1.to_owned(),
            to_entity_id: to.0.to_owned(),
            to_attribute_id: to.1.to_owned(),
            cardinality,
        }
    }

    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = name.into();
        self
    }

    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = description.into();
        self
    }

    /// A relationship whose two ends are the same entity. Legal and common
    /// (a category with a parent category), so the verifier must not treat it
    /// as a circular-dependency defect on its own.
    pub fn is_self_referencing(&self) -> bool {
        self.from_entity_id == self.to_entity_id
    }

    pub fn touches(&self, entity_id: &str) -> bool {
        self.from_entity_id == entity_id || self.to_entity_id == entity_id
    }

    pub fn is_documented(&self) -> bool {
        !self.description.trim().is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_both_the_symbol_and_the_name_spelling() {
        assert_eq!("1:1".parse(), Ok(Cardinality::OneToOne));
        assert_eq!("1:n".parse(), Ok(Cardinality::OneToMany));
        assert_eq!("MANY_TO_MANY".parse(), Ok(Cardinality::ManyToMany));
        assert!("1:many".parse::<Cardinality>().is_err());
    }

    #[test]
    fn renders_back_to_the_symbol_it_parsed_from() {
        for symbol in Cardinality::VARIANTS {
            let cardinality: Cardinality = symbol.parse().expect("the variant list should parse");

            assert_eq!(cardinality.to_string(), symbol);
        }
    }

    #[test]
    fn only_many_to_many_needs_a_join_table() {
        assert!(Cardinality::ManyToMany.requires_join_table());
        assert!(!Cardinality::OneToMany.requires_join_table());
        assert!(!Cardinality::OneToOne.requires_join_table());
    }

    #[test]
    fn recognises_a_self_reference() {
        let recursive = Relationship::new(
            "r1",
            ("categories", "parent_id"),
            ("categories", "id"),
            Cardinality::OneToMany,
        );

        assert!(recursive.is_self_referencing());
    }

    #[test]
    fn knows_which_entities_it_touches() {
        let relationship = Relationship::new(
            "r1",
            ("posts", "user_id"),
            ("users", "id"),
            Cardinality::OneToMany,
        );

        assert!(relationship.touches("posts"));
        assert!(relationship.touches("users"));
        assert!(!relationship.touches("comments"));
    }
}
