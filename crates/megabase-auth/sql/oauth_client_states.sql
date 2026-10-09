-- Ported from supabase/auth
-- migrations/20251201000000_add_oauth_client_states_table.up.sql (MIT), pin v2.197.0.
-- No RLS in this pin.

CREATE TABLE IF NOT EXISTS auth.oauth_client_states (
    id uuid PRIMARY KEY,
    provider_type text NOT NULL,
    code_verifier text,
    created_at timestamptz NOT NULL
);

ALTER TABLE auth.oauth_client_states ADD COLUMN IF NOT EXISTS provider_type text;
ALTER TABLE auth.oauth_client_states ADD COLUMN IF NOT EXISTS code_verifier text;
ALTER TABLE auth.oauth_client_states ADD COLUMN IF NOT EXISTS created_at timestamptz;

ALTER TABLE auth.oauth_client_states ALTER COLUMN provider_type SET NOT NULL;
ALTER TABLE auth.oauth_client_states ALTER COLUMN created_at SET NOT NULL;

CREATE INDEX IF NOT EXISTS idx_oauth_client_states_created_at
    ON auth.oauth_client_states (created_at);

COMMENT ON TABLE auth.oauth_client_states IS
    'Stores OAuth states for third-party provider authentication flows where Supabase acts as the OAuth client.';
