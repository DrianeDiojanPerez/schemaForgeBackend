mod compare;
mod json;
mod names;
mod read;
mod sql;

pub use compare::compare;
pub(crate) use names::{cardinality_name, kind_name};

#[cfg(test)]
mod tests;

use crate::module::schema::core::domain::{ExchangeFormat, Position, Report, Schema, SchemaDraft};
use crate::module::schema::core::ports::Exported;
use crate::package::errdef::Error;

pub fn export(
    schema: &Schema,
    format: ExchangeFormat,
    include_positions: bool,
) -> Result<Exported, Error> {
    match format {
        ExchangeFormat::Json => Ok(Exported {
            file_name: json::file_name(&schema.name),
            content_type: json::CONTENT_TYPE.to_owned(),
            content: json::write(schema, include_positions),
        }),
        ExchangeFormat::Sql => Err(Error::unimplemented(
            "exporting as sql is not built yet, generate the ddl instead",
        )),
    }
}

const GRID_COLUMNS: usize = 4;
const GRID_WIDTH: f64 = 320.0;
const GRID_HEIGHT: f64 = 260.0;

/// Where a table goes when the file does not say. Four across keeps a small
/// schema on one screen, and the order is the file's own, so two imports of
/// the same file lay out the same way.
pub(super) fn grid_slot(index: usize) -> Position {
    Position::new(
        (index % GRID_COLUMNS) as f64 * GRID_WIDTH,
        (index / GRID_COLUMNS) as f64 * GRID_HEIGHT,
    )
}

pub fn import(
    content: &str,
    format: ExchangeFormat,
    include_positions: bool,
) -> Result<(SchemaDraft, Report), Error> {
    match format {
        ExchangeFormat::Json => Ok((json::read(content, include_positions)?, Report::default())),
        ExchangeFormat::Sql => sql::read(content, include_positions),
    }
}
