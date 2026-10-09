-- Ported from supabase/auth (MIT), pin v2.197.0:
--   migrations/20221021082433_add_saml.up.sql
--   migrations/20240314092811_add_saml_name_id_format.up.sql
--   migrations/20240612123726_enable_rls_update_grants.up.sql

CREATE TABLE IF NOT EXISTS auth.saml_providers (
    id uuid NOT NULL,
    sso_provider_id uuid NOT NULL,
    entity_id text NOT NULL UNIQUE,
    metadata_xml text NOT NULL,
    metadata_url text NULL,
    attribute_mapping jsonb NULL,
    created_at timestamptz NULL,
    updated_at timestamptz NULL,
    name_id_format text NULL,
    PRIMARY KEY (id),
    FOREIGN KEY (sso_provider_id) REFERENCES auth.sso_providers (id) ON DELETE CASCADE,
    CONSTRAINT "metadata_xml not empty" CHECK (char_length(metadata_xml) > 0),
    CONSTRAINT "metadata_url not empty" CHECK (metadata_url = NULL OR char_length(metadata_url) > 0),
    CONSTRAINT "entity_id not empty" CHECK (char_length(entity_id) > 0)
);

ALTER TABLE auth.saml_providers ADD COLUMN IF NOT EXISTS sso_provider_id uuid;
ALTER TABLE auth.saml_providers ADD COLUMN IF NOT EXISTS entity_id text;
ALTER TABLE auth.saml_providers ADD COLUMN IF NOT EXISTS metadata_xml text;
ALTER TABLE auth.saml_providers ADD COLUMN IF NOT EXISTS metadata_url text;
ALTER TABLE auth.saml_providers ADD COLUMN IF NOT EXISTS attribute_mapping jsonb;
ALTER TABLE auth.saml_providers ADD COLUMN IF NOT EXISTS created_at timestamptz;
ALTER TABLE auth.saml_providers ADD COLUMN IF NOT EXISTS updated_at timestamptz;
ALTER TABLE auth.saml_providers ADD COLUMN IF NOT EXISTS name_id_format text;

ALTER TABLE auth.saml_providers ALTER COLUMN id SET NOT NULL;
ALTER TABLE auth.saml_providers ALTER COLUMN sso_provider_id SET NOT NULL;
ALTER TABLE auth.saml_providers ALTER COLUMN entity_id SET NOT NULL;
ALTER TABLE auth.saml_providers ALTER COLUMN metadata_xml SET NOT NULL;

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1
        FROM pg_constraint
        WHERE conname = 'saml_providers_pkey'
          AND conrelid = 'auth.saml_providers'::regclass
    ) THEN
        ALTER TABLE auth.saml_providers ADD CONSTRAINT saml_providers_pkey PRIMARY KEY (id);
    END IF;
    IF NOT EXISTS (
        SELECT 1
        FROM pg_constraint
        WHERE conname = 'saml_providers_sso_provider_id_fkey'
          AND conrelid = 'auth.saml_providers'::regclass
    ) THEN
        ALTER TABLE auth.saml_providers
            ADD CONSTRAINT saml_providers_sso_provider_id_fkey
            FOREIGN KEY (sso_provider_id) REFERENCES auth.sso_providers (id) ON DELETE CASCADE;
    END IF;
    IF NOT EXISTS (
        SELECT 1
        FROM pg_constraint
        WHERE conname = 'saml_providers_entity_id_key'
          AND conrelid = 'auth.saml_providers'::regclass
    ) THEN
        ALTER TABLE auth.saml_providers
            ADD CONSTRAINT saml_providers_entity_id_key UNIQUE (entity_id);
    END IF;
    IF NOT EXISTS (
        SELECT 1
        FROM pg_constraint
        WHERE conname = 'metadata_xml not empty'
          AND conrelid = 'auth.saml_providers'::regclass
    ) THEN
        ALTER TABLE auth.saml_providers
            ADD CONSTRAINT "metadata_xml not empty"
            CHECK (char_length(metadata_xml) > 0);
    END IF;
    IF NOT EXISTS (
        SELECT 1
        FROM pg_constraint
        WHERE conname = 'metadata_url not empty'
          AND conrelid = 'auth.saml_providers'::regclass
    ) THEN
        ALTER TABLE auth.saml_providers
            ADD CONSTRAINT "metadata_url not empty"
            CHECK (metadata_url = NULL OR char_length(metadata_url) > 0);
    END IF;
    IF NOT EXISTS (
        SELECT 1
        FROM pg_constraint
        WHERE conname = 'entity_id not empty'
          AND conrelid = 'auth.saml_providers'::regclass
    ) THEN
        ALTER TABLE auth.saml_providers
            ADD CONSTRAINT "entity_id not empty"
            CHECK (char_length(entity_id) > 0);
    END IF;
END $$;

CREATE INDEX IF NOT EXISTS saml_providers_sso_provider_id_idx ON auth.saml_providers (sso_provider_id);

COMMENT ON TABLE auth.saml_providers IS 'Auth: Manages SAML Identity Provider connections.';

ALTER TABLE auth.saml_providers ENABLE ROW LEVEL SECURITY;

DO $$
BEGIN
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'postgres') THEN
        GRANT SELECT ON auth.saml_providers TO postgres WITH GRANT OPTION;
    END IF;
END $$;
