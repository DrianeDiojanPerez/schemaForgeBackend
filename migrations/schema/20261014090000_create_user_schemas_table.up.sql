-- user_id is the id of a row in iam.users, held by value only. The schema
-- module does not reach into the iam tables, so there is no foreign key.
CREATE TABLE IF NOT EXISTS schemaforge.user_schemas(
    user_id UUID NOT NULL,
    schema_id UUID NOT NULL REFERENCES schemaforge.schemas(id) ON DELETE CASCADE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (schema_id)
);

-- A listing is one user's schemas in the order they were made, with the id
-- breaking ties so paging cannot return the same schema twice.
CREATE INDEX IF NOT EXISTS user_schemas_user_id_index
    ON schemaforge.user_schemas (user_id, created_at, schema_id);

-- A name only has to be unique among one user's schemas, and the owner lives
-- in the other table, so the repository enforces it instead of an index.
DROP INDEX IF EXISTS schemaforge.schemas_name_unique;
