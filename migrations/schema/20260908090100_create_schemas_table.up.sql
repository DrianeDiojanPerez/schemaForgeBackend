CREATE EXTENSION IF NOT EXISTS "uuid-ossp";

CREATE TABLE IF NOT EXISTS schemaforge.schemas(
    id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    name VARCHAR(120) NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    entities JSONB NOT NULL DEFAULT '[]'::JSONB,
    relationships JSONB NOT NULL DEFAULT '[]'::JSONB,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Names are matched without case everywhere else, so the uniqueness rule is
-- stated the same way or two spellings of one name would both be stored.
CREATE UNIQUE INDEX IF NOT EXISTS schemas_name_unique ON schemaforge.schemas (LOWER(name));

-- A listing orders by these two, and the id breaks ties so paging cannot
-- return the same schema twice.
CREATE INDEX IF NOT EXISTS schemas_created_at_index ON schemaforge.schemas (created_at, id);
