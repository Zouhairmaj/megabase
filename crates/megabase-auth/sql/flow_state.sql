-- Ported from supabase/auth (MIT), pin v2.197.0:
--   migrations/20230322519590_add_flow_state_table.up.sql
--   migrations/20230402418590_add_authentication_method_to_flow_state_table.up.sql
--   migrations/20240306115329_add_issued_at_to_flow_state.up.sql
--   migrations/20260115000000_add_flow_state_oauth_context.up.sql
--   migrations/20230508135423_add_cleanup_indexes.up.sql
--   migrations/20240612123726_enable_rls_update_grants.up.sql

DO $$ BEGIN
    CREATE TYPE auth.code_challenge_method AS ENUM ('s256', 'plain');
EXCEPTION
    WHEN duplicate_object THEN NULL;
END $$;

CREATE TABLE IF NOT EXISTS auth.flow_state (
    id uuid PRIMARY KEY,
    user_id uuid NULL,
    auth_code text NULL,
    code_challenge_method auth.code_challenge_method NULL,
    code_challenge text NULL,
    provider_type text NOT NULL,
    provider_access_token text NULL,
    provider_refresh_token text NULL,
    created_at timestamptz NULL,
    updated_at timestamptz NULL,
    authentication_method text NOT NULL,
    auth_code_issued_at timestamptz NULL,
    invite_token text NULL,
    referrer text NULL,
    oauth_client_state_id uuid NULL,
    linking_target_id uuid NULL,
    email_optional boolean NOT NULL DEFAULT FALSE
);

ALTER TABLE auth.flow_state ADD COLUMN IF NOT EXISTS user_id uuid;
ALTER TABLE auth.flow_state ADD COLUMN IF NOT EXISTS auth_code text;
ALTER TABLE auth.flow_state ADD COLUMN IF NOT EXISTS code_challenge_method auth.code_challenge_method;
ALTER TABLE auth.flow_state ADD COLUMN IF NOT EXISTS code_challenge text;
ALTER TABLE auth.flow_state ADD COLUMN IF NOT EXISTS provider_type text;
ALTER TABLE auth.flow_state ADD COLUMN IF NOT EXISTS provider_access_token text;
ALTER TABLE auth.flow_state ADD COLUMN IF NOT EXISTS provider_refresh_token text;
ALTER TABLE auth.flow_state ADD COLUMN IF NOT EXISTS created_at timestamptz;
ALTER TABLE auth.flow_state ADD COLUMN IF NOT EXISTS updated_at timestamptz;
ALTER TABLE auth.flow_state ADD COLUMN IF NOT EXISTS authentication_method text;
ALTER TABLE auth.flow_state ADD COLUMN IF NOT EXISTS auth_code_issued_at timestamptz;
ALTER TABLE auth.flow_state ADD COLUMN IF NOT EXISTS invite_token text;
ALTER TABLE auth.flow_state ADD COLUMN IF NOT EXISTS referrer text;
ALTER TABLE auth.flow_state ADD COLUMN IF NOT EXISTS oauth_client_state_id uuid;
ALTER TABLE auth.flow_state ADD COLUMN IF NOT EXISTS linking_target_id uuid;
ALTER TABLE auth.flow_state ADD COLUMN IF NOT EXISTS email_optional boolean NOT NULL DEFAULT FALSE;

ALTER TABLE auth.flow_state ALTER COLUMN code_challenge DROP NOT NULL;
ALTER TABLE auth.flow_state ALTER COLUMN code_challenge_method DROP NOT NULL;
ALTER TABLE auth.flow_state ALTER COLUMN auth_code DROP NOT NULL;

CREATE INDEX IF NOT EXISTS idx_auth_code ON auth.flow_state (auth_code);
CREATE INDEX IF NOT EXISTS idx_user_id_auth_method ON auth.flow_state (user_id, authentication_method);
CREATE INDEX IF NOT EXISTS flow_state_created_at_idx ON auth.flow_state (created_at DESC);

COMMENT ON TABLE auth.flow_state IS 'Stores metadata for all OAuth/SSO login flows';

ALTER TABLE auth.flow_state ENABLE ROW LEVEL SECURITY;

DO $$
BEGIN
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'postgres') THEN
        GRANT SELECT ON auth.flow_state TO postgres WITH GRANT OPTION;
    END IF;
END $$;
