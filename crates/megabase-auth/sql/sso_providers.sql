-- Ported from supabase/auth (MIT), pin v2.197.0:
--   migrations/20221021082433_add_saml.up.sql
--   migrations/20240612123726_enable_rls_update_grants.up.sql
--   migrations/20250717082212_add_disabled_to_sso_providers.up.sql

CREATE TABLE IF NOT EXISTS auth.sso_providers (
    id uuid NOT NULL,
    resource_id text NULL,
    created_at timestamptz NULL,
    updated_at timestamptz NULL,
    disabled boolean NULL,
    PRIMARY KEY (id),
    CONSTRAINT "resource_id not empty" CHECK (resource_id = NULL OR char_length(resource_id) > 0)
);

ALTER TABLE auth.sso_providers ADD COLUMN IF NOT EXISTS resource_id text;
ALTER TABLE auth.sso_providers ADD COLUMN IF NOT EXISTS created_at timestamptz;
ALTER TABLE auth.sso_providers ADD COLUMN IF NOT EXISTS updated_at timestamptz;
ALTER TABLE auth.sso_providers ADD COLUMN IF NOT EXISTS disabled boolean;

ALTER TABLE auth.sso_providers ALTER COLUMN id SET NOT NULL;

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1
        FROM pg_constraint
        WHERE conname = 'sso_providers_pkey'
          AND conrelid = 'auth.sso_providers'::regclass
    ) THEN
        ALTER TABLE auth.sso_providers ADD CONSTRAINT sso_providers_pkey PRIMARY KEY (id);
    END IF;
    IF NOT EXISTS (
        SELECT 1
        FROM pg_constraint
        WHERE conname = 'resource_id not empty'
          AND conrelid = 'auth.sso_providers'::regclass
    ) THEN
        ALTER TABLE auth.sso_providers
            ADD CONSTRAINT "resource_id not empty"
            CHECK (resource_id = NULL OR char_length(resource_id) > 0);
    END IF;
END $$;

CREATE UNIQUE INDEX IF NOT EXISTS sso_providers_resource_id_idx ON auth.sso_providers (lower(resource_id));
CREATE INDEX IF NOT EXISTS sso_providers_resource_id_pattern_idx ON auth.sso_providers (resource_id text_pattern_ops);

COMMENT ON TABLE auth.sso_providers IS 'Auth: Manages SSO identity provider information; see saml_providers for SAML.';
COMMENT ON COLUMN auth.sso_providers.resource_id IS 'Auth: Uniquely identifies a SSO provider according to a user-chosen resource ID (case insensitive), useful in infrastructure as code.';

ALTER TABLE auth.sso_providers ENABLE ROW LEVEL SECURITY;

DO $$
BEGIN
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'postgres') THEN
        GRANT SELECT ON auth.sso_providers TO postgres WITH GRANT OPTION;
    END IF;
END $$;
