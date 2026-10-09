-- Ported from supabase/auth (MIT), pin v2.197.0:
--   migrations/20221003041349_add_mfa_schema.up.sql
--   migrations/20221011041400_add_mfa_indexes.up.sql
--   migrations/20230914180801_add_mfa_factors_user_id_idx.up.sql
--   migrations/20240729123726_add_mfa_phone_config.up.sql
--   migrations/20240802193726_add_mfa_factors_column_last_challenged_at.up.sql
--   migrations/20240806073726_drop_uniqueness_constraint_on_phone.up.sql
--   migrations/20241009103726_add_web_authn.up.sql
--   migrations/20250925093508_add_last_webauthn_challenge_data.up.sql
--   migrations/20260824000000_add_recovery_codes_factor_type.up.sql
--   migrations/20240612123726_enable_rls_update_grants.up.sql
-- factor_type final labels: totp, webauthn, phone, recovery_code.

DO $$ BEGIN
    CREATE TYPE auth.factor_type AS ENUM ('totp', 'webauthn', 'phone', 'recovery_code');
EXCEPTION
    WHEN duplicate_object THEN NULL;
END $$;

ALTER TYPE auth.factor_type ADD VALUE IF NOT EXISTS 'phone';
ALTER TYPE auth.factor_type ADD VALUE IF NOT EXISTS 'recovery_code';

DO $$ BEGIN
    CREATE TYPE auth.factor_status AS ENUM ('unverified', 'verified');
EXCEPTION
    WHEN duplicate_object THEN NULL;
END $$;

DO $$ BEGIN
    CREATE TYPE auth.aal_level AS ENUM ('aal1', 'aal2', 'aal3');
EXCEPTION
    WHEN duplicate_object THEN NULL;
END $$;

CREATE TABLE IF NOT EXISTS auth.mfa_factors (
    id uuid NOT NULL,
    user_id uuid NOT NULL,
    friendly_name text NULL,
    factor_type auth.factor_type NOT NULL,
    status auth.factor_status NOT NULL,
    created_at timestamptz NOT NULL,
    updated_at timestamptz NOT NULL,
    secret text NULL,
    phone text NULL DEFAULT NULL,
    last_challenged_at timestamptz NULL DEFAULT NULL,
    web_authn_credential jsonb NULL,
    web_authn_aaguid uuid NULL,
    last_webauthn_challenge_data jsonb NULL,
    CONSTRAINT mfa_factors_pkey PRIMARY KEY (id),
    CONSTRAINT mfa_factors_user_id_fkey FOREIGN KEY (user_id) REFERENCES auth.users(id) ON DELETE CASCADE,
    CONSTRAINT mfa_factors_last_challenged_at_key UNIQUE (last_challenged_at)
);

ALTER TABLE auth.mfa_factors ADD COLUMN IF NOT EXISTS user_id uuid;
ALTER TABLE auth.mfa_factors ADD COLUMN IF NOT EXISTS friendly_name text;
ALTER TABLE auth.mfa_factors ADD COLUMN IF NOT EXISTS factor_type auth.factor_type;
ALTER TABLE auth.mfa_factors ADD COLUMN IF NOT EXISTS status auth.factor_status;
ALTER TABLE auth.mfa_factors ADD COLUMN IF NOT EXISTS created_at timestamptz;
ALTER TABLE auth.mfa_factors ADD COLUMN IF NOT EXISTS updated_at timestamptz;
ALTER TABLE auth.mfa_factors ADD COLUMN IF NOT EXISTS secret text;
ALTER TABLE auth.mfa_factors ADD COLUMN IF NOT EXISTS phone text DEFAULT NULL;
ALTER TABLE auth.mfa_factors ADD COLUMN IF NOT EXISTS last_challenged_at timestamptz DEFAULT NULL;
ALTER TABLE auth.mfa_factors ADD COLUMN IF NOT EXISTS web_authn_credential jsonb;
ALTER TABLE auth.mfa_factors ADD COLUMN IF NOT EXISTS web_authn_aaguid uuid;
ALTER TABLE auth.mfa_factors ADD COLUMN IF NOT EXISTS last_webauthn_challenge_data jsonb;

ALTER TABLE auth.mfa_factors ALTER COLUMN id SET NOT NULL;
ALTER TABLE auth.mfa_factors ALTER COLUMN user_id SET NOT NULL;
ALTER TABLE auth.mfa_factors ALTER COLUMN factor_type SET NOT NULL;
ALTER TABLE auth.mfa_factors ALTER COLUMN status SET NOT NULL;
ALTER TABLE auth.mfa_factors ALTER COLUMN created_at SET NOT NULL;
ALTER TABLE auth.mfa_factors ALTER COLUMN updated_at SET NOT NULL;

ALTER TABLE auth.mfa_factors DROP CONSTRAINT IF EXISTS mfa_factors_phone_key;

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1
        FROM pg_constraint
        WHERE conname = 'mfa_factors_pkey'
          AND conrelid = 'auth.mfa_factors'::regclass
    ) THEN
        ALTER TABLE auth.mfa_factors ADD CONSTRAINT mfa_factors_pkey PRIMARY KEY (id);
    END IF;
    IF NOT EXISTS (
        SELECT 1
        FROM pg_constraint
        WHERE conname = 'mfa_factors_user_id_fkey'
          AND conrelid = 'auth.mfa_factors'::regclass
    ) THEN
        ALTER TABLE auth.mfa_factors
            ADD CONSTRAINT mfa_factors_user_id_fkey
            FOREIGN KEY (user_id) REFERENCES auth.users(id) ON DELETE CASCADE;
    END IF;
    IF NOT EXISTS (
        SELECT 1
        FROM pg_constraint
        WHERE conname = 'mfa_factors_last_challenged_at_key'
          AND conrelid = 'auth.mfa_factors'::regclass
    ) THEN
        ALTER TABLE auth.mfa_factors
            ADD CONSTRAINT mfa_factors_last_challenged_at_key UNIQUE (last_challenged_at);
    END IF;
END $$;

DO $$
BEGIN
    IF (
        SELECT count(*) = 2
        FROM pg_indexes
        WHERE indexname IN ('unique_verified_phone_factor', 'unique_phone_factor_per_user')
          AND schemaname = 'auth'
    ) THEN
        EXECUTE 'DROP INDEX auth.unique_verified_phone_factor';
    END IF;
    IF EXISTS (
        SELECT 1
        FROM pg_indexes
        WHERE indexname = 'unique_verified_phone_factor'
          AND schemaname = 'auth'
    ) THEN
        EXECUTE 'ALTER INDEX auth.unique_verified_phone_factor RENAME TO unique_phone_factor_per_user';
    END IF;
END $$;

CREATE UNIQUE INDEX IF NOT EXISTS mfa_factors_user_friendly_name_unique
    ON auth.mfa_factors (friendly_name, user_id)
    WHERE trim(friendly_name) <> '';
CREATE INDEX IF NOT EXISTS factor_id_created_at_idx
    ON auth.mfa_factors (user_id, created_at);
CREATE INDEX IF NOT EXISTS mfa_factors_user_id_idx
    ON auth.mfa_factors (user_id);
CREATE UNIQUE INDEX IF NOT EXISTS unique_phone_factor_per_user
    ON auth.mfa_factors (user_id, phone);

COMMENT ON TABLE auth.mfa_factors IS 'auth: stores metadata about factors';
COMMENT ON COLUMN auth.mfa_factors.last_webauthn_challenge_data IS
    'Stores the latest WebAuthn challenge data including attestation/assertion for customer verification';

ALTER TABLE auth.mfa_factors ENABLE ROW LEVEL SECURITY;

DO $$
BEGIN
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'postgres') THEN
        GRANT SELECT ON auth.mfa_factors TO postgres WITH GRANT OPTION;
    END IF;
END $$;
