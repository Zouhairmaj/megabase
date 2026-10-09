-- Ported from supabase/auth (MIT), pin v2.197.0:
--   migrations/20221003041349_add_mfa_schema.up.sql
--   migrations/20230523124323_add_mfa_challenge_cleanup_index.up.sql
--   migrations/20240729123726_add_mfa_phone_config.up.sql
--   migrations/20241009103726_add_web_authn.up.sql
--   migrations/20240612123726_enable_rls_update_grants.up.sql

CREATE TABLE IF NOT EXISTS auth.mfa_challenges (
    id uuid NOT NULL,
    factor_id uuid NOT NULL,
    created_at timestamptz NOT NULL,
    verified_at timestamptz NULL,
    ip_address inet NOT NULL,
    otp_code text NULL,
    web_authn_session_data jsonb NULL,
    CONSTRAINT mfa_challenges_pkey PRIMARY KEY (id),
    CONSTRAINT mfa_challenges_auth_factor_id_fkey
        FOREIGN KEY (factor_id) REFERENCES auth.mfa_factors(id) ON DELETE CASCADE
);

ALTER TABLE auth.mfa_challenges ADD COLUMN IF NOT EXISTS factor_id uuid;
ALTER TABLE auth.mfa_challenges ADD COLUMN IF NOT EXISTS created_at timestamptz;
ALTER TABLE auth.mfa_challenges ADD COLUMN IF NOT EXISTS verified_at timestamptz;
ALTER TABLE auth.mfa_challenges ADD COLUMN IF NOT EXISTS ip_address inet;
ALTER TABLE auth.mfa_challenges ADD COLUMN IF NOT EXISTS otp_code text;
ALTER TABLE auth.mfa_challenges ADD COLUMN IF NOT EXISTS web_authn_session_data jsonb;

ALTER TABLE auth.mfa_challenges ALTER COLUMN id SET NOT NULL;
ALTER TABLE auth.mfa_challenges ALTER COLUMN factor_id SET NOT NULL;
ALTER TABLE auth.mfa_challenges ALTER COLUMN created_at SET NOT NULL;
ALTER TABLE auth.mfa_challenges ALTER COLUMN ip_address SET NOT NULL;

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1
        FROM pg_constraint
        WHERE conname = 'mfa_challenges_pkey'
          AND conrelid = 'auth.mfa_challenges'::regclass
    ) THEN
        ALTER TABLE auth.mfa_challenges ADD CONSTRAINT mfa_challenges_pkey PRIMARY KEY (id);
    END IF;
    IF NOT EXISTS (
        SELECT 1
        FROM pg_constraint
        WHERE conname = 'mfa_challenges_auth_factor_id_fkey'
          AND conrelid = 'auth.mfa_challenges'::regclass
    ) THEN
        ALTER TABLE auth.mfa_challenges
            ADD CONSTRAINT mfa_challenges_auth_factor_id_fkey
            FOREIGN KEY (factor_id) REFERENCES auth.mfa_factors(id) ON DELETE CASCADE;
    END IF;
END $$;

CREATE INDEX IF NOT EXISTS mfa_challenge_created_at_idx
    ON auth.mfa_challenges (created_at DESC);

COMMENT ON TABLE auth.mfa_challenges IS 'auth: stores metadata about challenge requests made';

ALTER TABLE auth.mfa_challenges ENABLE ROW LEVEL SECURITY;

DO $$
BEGIN
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'postgres') THEN
        GRANT SELECT ON auth.mfa_challenges TO postgres WITH GRANT OPTION;
    END IF;
END $$;
