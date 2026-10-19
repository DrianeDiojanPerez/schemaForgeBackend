DROP TABLE IF EXISTS schemaforge.user_schemas;

CREATE UNIQUE INDEX IF NOT EXISTS schemas_name_unique ON schemaforge.schemas (LOWER(name));
