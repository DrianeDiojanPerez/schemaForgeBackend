use crate::module::schema::core::domain::Entity;

use super::quote;

/// Where the table sits on the canvas, written as a comment the database
/// throws away. A schema that leaves as SQL and comes back still opens with
/// the diagram the user arranged rather than a fresh pile of boxes.
pub(super) fn position(entity: &Entity) -> String {
    format!(
        "-- @table {} {} {}",
        quote(&entity.name),
        entity.position.x.round() as i64,
        entity.position.y.round() as i64
    )
}
