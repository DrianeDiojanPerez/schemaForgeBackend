mod json;
mod names;

pub(crate) use names::{cardinality_name, kind_name};

use crate::module::schema::core::domain::{ExchangeFormat, Schema};
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
