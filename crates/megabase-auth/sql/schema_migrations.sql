-- Ported from supabase/auth (MIT), pin v2.197.0:
--   migrations/00_init_auth_schema.up.sql
--   migrations/20240612123726_enable_rls_update_grants.up.sql

CREATE TABLE IF NOT EXISTS auth.schema_migrations (
    version varchar(255) NOT NULL,
    CONSTRAINT schema_migrations_pkey PRIMARY KEY (version)
);

ALTER TABLE auth.schema_migrations ADD COLUMN IF NOT EXISTS version varchar(255);

ALTER TABLE auth.schema_migrations ALTER COLUMN version SET NOT NULL;

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1
        FROM pg_constraint
        WHERE conname = 'schema_migrations_pkey'
          AND conrelid = 'auth.schema_migrations'::regclass
    ) THEN
        ALTER TABLE auth.schema_migrations
            ADD CONSTRAINT schema_migrations_pkey PRIMARY KEY (version);
    END IF;
END $$;

COMMENT ON TABLE auth.schema_migrations IS 'Auth: Manages updates to the auth system.';

ALTER TABLE auth.schema_migrations ENABLE ROW LEVEL SECURITY;

DO $$
BEGIN
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'postgres') THEN
        GRANT SELECT ON auth.schema_migrations TO postgres WITH GRANT OPTION;
    END IF;
END $$;
