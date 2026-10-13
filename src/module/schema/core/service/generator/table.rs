use crate::module::schema::core::domain::{Attribute, Entity};

use super::quote;

pub(super) fn create_table(entity: &Entity) -> String {
    let mut lines: Vec<String> = entity.attributes.iter().map(column).collect();

    if let Some(constraint) = primary_key(entity) {
        lines.push(constraint);
    }

    if lines.is_empty() {
        return format!("CREATE TABLE {} ();", quote(&entity.name));
    }

    format!(
        "CREATE TABLE {} (\n    {}\n);",
        quote(&entity.name),
        lines.join(",\n    ")
    )
}

fn column(attribute: &Attribute) -> String {
    let mut line = format!(
        "{} {}",
        quote(&attribute.name),
        attribute.data_type.to_string().to_uppercase()
    );

    if !attribute.nullable {
        line.push_str(" NOT NULL");
    }

    // A default is whatever was typed into the inspector, and there is no way
    // to tell a literal from a call like now() by looking at it. It goes
    // through as written so the database has the last word on it.
    if let Some(default) = &attribute.default_value {
        line.push_str(&format!(" DEFAULT {default}"));
    }

    // The key is declared once at the foot of the table, so a column that is
    // only unique by virtue of being the key does not say so twice.
    if attribute.unique && !attribute.primary_key {
        line.push_str(" UNIQUE");
    }

    line
}

/// Written as a table constraint rather than inline, because a composite key
/// cannot be spelled on a single column and one form for both reads better
/// than two.
fn primary_key(entity: &Entity) -> Option<String> {
    let key = entity.primary_key();

    if key.is_empty() {
        return None;
    }

    let columns: Vec<String> = key.iter().map(|attribute| quote(&attribute.name)).collect();

    Some(format!("PRIMARY KEY ({})", columns.join(", ")))
}
