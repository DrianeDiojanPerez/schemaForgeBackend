use crate::module::schema::core::domain::Schema;

use super::quote;

pub const MARKER: &str = "-- @schemaforge layout 1";

/// Where the tables sit on the canvas, written as comments the database
/// throws away. A schema that leaves as SQL and comes back still opens with
/// the diagram the user arranged rather than a fresh pile of boxes.
pub(super) fn layout(schema: &Schema) -> String {
    let mut lines = vec![MARKER.to_owned()];

    for entity in &schema.entities {
        lines.push(format!(
            "-- @table {} {} {}",
            quote(&entity.name),
            entity.position.x.round() as i64,
            entity.position.y.round() as i64
        ));
    }

    lines.join("\n")
}
