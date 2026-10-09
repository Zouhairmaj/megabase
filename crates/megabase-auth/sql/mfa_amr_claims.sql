-- Ported from supabase/auth (MIT), pin v2.197.0:
--   migrations/20221003041349_add_mfa_schema.up.sql
--   migrations/20221011041400_add_mfa_indexes.up.sql
--   migrations/20240612123726_enable_rls_update_grants.up.sql

CREATE TABLE IF NOT EXISTS auth.mfa_amr_claims (
    session_id uuid NOT NULL,
    created_at timestamptz NOT NULL,
    updated_at timestamptz NOT NULL,
    authentication_method text NOT NULL,
    id uuid NOT NULL,
    CONSTRAINT mfa_amr_claims_session_id_authentication_method_pkey
        UNIQUE (session_id, authentication_method),
    CONSTRAINT mfa_amr_claims_session_id_fkey
        FOREIGN KEY (session_id) REFERENCES auth.sessions(id) ON DELETE CASCADE,
    CONSTRAINT amr_id_pk PRIMARY KEY (id)
);

ALTER TABLE auth.mfa_amr_claims ADD COLUMN IF NOT EXISTS session_id uuid;
ALTER TABLE auth.mfa_amr_claims ADD COLUMN IF NOT EXISTS created_at timestamptz;
ALTER TABLE auth.mfa_amr_claims ADD COLUMN IF NOT EXISTS updated_at timestamptz;
ALTER TABLE auth.mfa_amr_claims ADD COLUMN IF NOT EXISTS authentication_method text;
ALTER TABLE auth.mfa_amr_claims ADD COLUMN IF NOT EXISTS id uuid;

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1
        FROM pg_constraint
        WHERE conname = 'amr_id_pk'
          AND conrelid = 'auth.mfa_amr_claims'::regclass
    ) THEN
        ALTER TABLE auth.mfa_amr_claims ADD CONSTRAINT amr_id_pk PRIMARY KEY (id);
    END IF;
    IF NOT EXISTS (
        SELECT 1
        FROM pg_constraint
        WHERE conname = 'mfa_amr_claims_session_id_authentication_method_pkey'
          AND conrelid = 'auth.mfa_amr_claims'::regclass
    ) THEN
        ALTER TABLE auth.mfa_amr_claims
            ADD CONSTRAINT mfa_amr_claims_session_id_authentication_method_pkey
            UNIQUE (session_id, authentication_method);
    END IF;
    IF NOT EXISTS (
        SELECT 1
        FROM pg_constraint
        WHERE conname = 'mfa_amr_claims_session_id_fkey'
          AND conrelid = 'auth.mfa_amr_claims'::regclass
    ) THEN
        ALTER TABLE auth.mfa_amr_claims
            ADD CONSTRAINT mfa_amr_claims_session_id_fkey
            FOREIGN KEY (session_id) REFERENCES auth.sessions(id) ON DELETE CASCADE;
    END IF;
END $$;

COMMENT ON TABLE auth.mfa_amr_claims IS
    'auth: stores authenticator method reference claims for multi factor authentication';

ALTER TABLE auth.mfa_amr_claims ENABLE ROW LEVEL SECURITY;

DO $$
BEGIN
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'postgres') THEN
        GRANT SELECT ON auth.mfa_amr_claims TO postgres WITH GRANT OPTION;
    END IF;
END $$;
