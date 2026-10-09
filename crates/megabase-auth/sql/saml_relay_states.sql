-- Ported from supabase/auth (MIT), pin v2.197.0:
--   migrations/20221021082433_add_saml.up.sql
--   migrations/20230818113222_add_flow_state_to_relay_state.up.sql
--   migrations/20230508135423_add_cleanup_indexes.up.sql
--   migrations/20240115144230_remove_ip_address_from_saml_relay_state.up.sql
--   migrations/20240612123726_enable_rls_update_grants.up.sql
-- Desired state: from_ip_address is dropped.

CREATE TABLE IF NOT EXISTS auth.saml_relay_states (
    id uuid NOT NULL,
    sso_provider_id uuid NOT NULL,
    request_id text NOT NULL,
    for_email text NULL,
    redirect_to text NULL,
    created_at timestamptz NULL,
    updated_at timestamptz NULL,
    flow_state_id uuid NULL,
    PRIMARY KEY (id),
    FOREIGN KEY (sso_provider_id) REFERENCES auth.sso_providers (id) ON DELETE CASCADE,
    FOREIGN KEY (flow_state_id) REFERENCES auth.flow_state (id) ON DELETE CASCADE,
    CONSTRAINT "request_id not empty" CHECK (char_length(request_id) > 0)
);

ALTER TABLE auth.saml_relay_states ADD COLUMN IF NOT EXISTS sso_provider_id uuid;
ALTER TABLE auth.saml_relay_states ADD COLUMN IF NOT EXISTS request_id text;
ALTER TABLE auth.saml_relay_states ADD COLUMN IF NOT EXISTS for_email text;
ALTER TABLE auth.saml_relay_states ADD COLUMN IF NOT EXISTS redirect_to text;
ALTER TABLE auth.saml_relay_states ADD COLUMN IF NOT EXISTS created_at timestamptz;
ALTER TABLE auth.saml_relay_states ADD COLUMN IF NOT EXISTS updated_at timestamptz;
ALTER TABLE auth.saml_relay_states ADD COLUMN IF NOT EXISTS flow_state_id uuid;

ALTER TABLE auth.saml_relay_states ALTER COLUMN id SET NOT NULL;
ALTER TABLE auth.saml_relay_states ALTER COLUMN sso_provider_id SET NOT NULL;
ALTER TABLE auth.saml_relay_states ALTER COLUMN request_id SET NOT NULL;

DO $$
BEGIN
    IF EXISTS (
        SELECT 1
        FROM information_schema.columns
        WHERE table_schema = 'auth'
          AND table_name = 'saml_relay_states'
          AND column_name = 'from_ip_address'
    ) THEN
        ALTER TABLE auth.saml_relay_states DROP COLUMN from_ip_address;
    END IF;
END $$;

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1
        FROM pg_constraint
        WHERE conname = 'saml_relay_states_pkey'
          AND conrelid = 'auth.saml_relay_states'::regclass
    ) THEN
        ALTER TABLE auth.saml_relay_states ADD CONSTRAINT saml_relay_states_pkey PRIMARY KEY (id);
    END IF;
    IF NOT EXISTS (
        SELECT 1
        FROM pg_constraint
        WHERE conname = 'saml_relay_states_sso_provider_id_fkey'
          AND conrelid = 'auth.saml_relay_states'::regclass
    ) THEN
        ALTER TABLE auth.saml_relay_states
            ADD CONSTRAINT saml_relay_states_sso_provider_id_fkey
            FOREIGN KEY (sso_provider_id) REFERENCES auth.sso_providers (id) ON DELETE CASCADE;
    END IF;
    IF NOT EXISTS (
        SELECT 1
        FROM pg_constraint
        WHERE conname = 'saml_relay_states_flow_state_id_fkey'
          AND conrelid = 'auth.saml_relay_states'::regclass
    ) THEN
        ALTER TABLE auth.saml_relay_states
            ADD CONSTRAINT saml_relay_states_flow_state_id_fkey
            FOREIGN KEY (flow_state_id) REFERENCES auth.flow_state (id) ON DELETE CASCADE;
    END IF;
    IF NOT EXISTS (
        SELECT 1
        FROM pg_constraint
        WHERE conname = 'request_id not empty'
          AND conrelid = 'auth.saml_relay_states'::regclass
    ) THEN
        ALTER TABLE auth.saml_relay_states
            ADD CONSTRAINT "request_id not empty"
            CHECK (char_length(request_id) > 0);
    END IF;
END $$;

CREATE INDEX IF NOT EXISTS saml_relay_states_sso_provider_id_idx ON auth.saml_relay_states (sso_provider_id);
CREATE INDEX IF NOT EXISTS saml_relay_states_for_email_idx ON auth.saml_relay_states (for_email);
CREATE INDEX IF NOT EXISTS saml_relay_states_created_at_idx ON auth.saml_relay_states (created_at DESC);

COMMENT ON TABLE auth.saml_relay_states IS 'Auth: Contains SAML Relay State information for each Service Provider initiated login.';

ALTER TABLE auth.saml_relay_states ENABLE ROW LEVEL SECURITY;

DO $$
BEGIN
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'postgres') THEN
        GRANT SELECT ON auth.saml_relay_states TO postgres WITH GRANT OPTION;
    END IF;
END $$;
