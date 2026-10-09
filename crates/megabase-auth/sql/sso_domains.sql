-- Ported from supabase/auth (MIT), pin v2.197.0:
--   migrations/20221021082433_add_saml.up.sql
--   migrations/20240612123726_enable_rls_update_grants.up.sql

CREATE TABLE IF NOT EXISTS auth.sso_domains (
    id uuid NOT NULL,
    sso_provider_id uuid NOT NULL,
    domain text NOT NULL,
    created_at timestamptz NULL,
    updated_at timestamptz NULL,
    PRIMARY KEY (id),
    FOREIGN KEY (sso_provider_id) REFERENCES auth.sso_providers (id) ON DELETE CASCADE,
    CONSTRAINT "domain not empty" CHECK (char_length(domain) > 0)
);

ALTER TABLE auth.sso_domains ADD COLUMN IF NOT EXISTS sso_provider_id uuid;
ALTER TABLE auth.sso_domains ADD COLUMN IF NOT EXISTS domain text;
ALTER TABLE auth.sso_domains ADD COLUMN IF NOT EXISTS created_at timestamptz;
ALTER TABLE auth.sso_domains ADD COLUMN IF NOT EXISTS updated_at timestamptz;

ALTER TABLE auth.sso_domains ALTER COLUMN id SET NOT NULL;
ALTER TABLE auth.sso_domains ALTER COLUMN sso_provider_id SET NOT NULL;
ALTER TABLE auth.sso_domains ALTER COLUMN domain SET NOT NULL;

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1
        FROM pg_constraint
        WHERE conname = 'sso_domains_pkey'
          AND conrelid = 'auth.sso_domains'::regclass
    ) THEN
        ALTER TABLE auth.sso_domains ADD CONSTRAINT sso_domains_pkey PRIMARY KEY (id);
    END IF;
    IF NOT EXISTS (
        SELECT 1
        FROM pg_constraint
        WHERE conname = 'sso_domains_sso_provider_id_fkey'
          AND conrelid = 'auth.sso_domains'::regclass
    ) THEN
        ALTER TABLE auth.sso_domains
            ADD CONSTRAINT sso_domains_sso_provider_id_fkey
            FOREIGN KEY (sso_provider_id) REFERENCES auth.sso_providers (id) ON DELETE CASCADE;
    END IF;
    IF NOT EXISTS (
        SELECT 1
        FROM pg_constraint
        WHERE conname = 'domain not empty'
          AND conrelid = 'auth.sso_domains'::regclass
    ) THEN
        ALTER TABLE auth.sso_domains
            ADD CONSTRAINT "domain not empty"
            CHECK (char_length(domain) > 0);
    END IF;
END $$;

CREATE INDEX IF NOT EXISTS sso_domains_sso_provider_id_idx ON auth.sso_domains (sso_provider_id);
CREATE UNIQUE INDEX IF NOT EXISTS sso_domains_domain_idx ON auth.sso_domains (lower(domain));

COMMENT ON TABLE auth.sso_domains IS 'Auth: Manages SSO email address domain mapping to an SSO Identity Provider.';

ALTER TABLE auth.sso_domains ENABLE ROW LEVEL SECURITY;

DO $$
BEGIN
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'postgres') THEN
        GRANT SELECT ON auth.sso_domains TO postgres WITH GRANT OPTION;
    END IF;
END $$;
