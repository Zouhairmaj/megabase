-- Ported from supabase/auth migrations/00_init_auth_schema.up.sql (MIT), pin v2.197.0.
-- RLS/grants: migrations/20240612123726_enable_rls_update_grants.up.sql.

CREATE TABLE IF NOT EXISTS auth.instances (
    id uuid NOT NULL,
    uuid uuid NULL,
    raw_base_config text NULL,
    created_at timestamptz NULL,
    updated_at timestamptz NULL,
    CONSTRAINT instances_pkey PRIMARY KEY (id)
);

COMMENT ON TABLE auth.instances IS 'Auth: Manages users across multiple sites.';

ALTER TABLE auth.instances ENABLE ROW LEVEL SECURITY;

DO $$
BEGIN
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'postgres') THEN
        GRANT SELECT ON auth.instances TO postgres WITH GRANT OPTION;
    END IF;
END $$;
