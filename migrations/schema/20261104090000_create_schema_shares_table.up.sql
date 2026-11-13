-- One row per person a schema is shared with. user_id and shared_by are ids
-- of rows in iam.users, held by value only, the same as user_schemas.
CREATE TABLE IF NOT EXISTS schemaforge.schema_shares(
    schema_id UUID NOT NULL REFERENCES schemaforge.schemas(id) ON DELETE CASCADE,
    user_id UUID NOT NULL,
    role VARCHAR(20) NOT NULL CHECK (role IN ('viewer', 'editor')),
    shared_by UUID NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (schema_id, user_id)
);

-- "Shared with me" is one user's shares in the order they were given, with
-- the schema id breaking ties so paging cannot return the same one twice.
CREATE INDEX IF NOT EXISTS schema_shares_user_id_index
    ON schemaforge.schema_shares (user_id, created_at, schema_id);

-- A share waiting on someone who has no account yet. It turns into a row in
-- schema_shares the moment they sign up with this email, or open the link.
-- Only the digest of the token is stored; the raw token travels by email.
CREATE TABLE IF NOT EXISTS schemaforge.schema_invitations(
    id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    schema_id UUID NOT NULL REFERENCES schemaforge.schemas(id) ON DELETE CASCADE,
    email VARCHAR(100) NOT NULL,
    role VARCHAR(20) NOT NULL CHECK (role IN ('viewer', 'editor')),
    token VARCHAR(255) NOT NULL UNIQUE,
    invited_by UUID NOT NULL,
    expires_at TIMESTAMPTZ NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (schema_id, email)
);

CREATE INDEX IF NOT EXISTS schema_invitations_email_index
    ON schemaforge.schema_invitations (email);
