-- Ported from supabase/auth
-- migrations/20260302000000_add_passkeys.up.sql (MIT), pin v2.197.0.
-- Credentials half of that file. No later migration in this pin. No RLS.

CREATE TABLE IF NOT EXISTS auth.webauthn_credentials (
    id uuid NOT NULL DEFAULT gen_random_uuid(),
    user_id uuid NOT NULL,
    credential_id bytea NOT NULL,
    public_key bytea NOT NULL,
    attestation_type text NOT NULL DEFAULT '',
    aaguid uuid,
    sign_count bigint NOT NULL DEFAULT 0,
    transports jsonb NOT NULL DEFAULT '[]'::jsonb,
    backup_eligible boolean NOT NULL DEFAULT false,
    backed_up boolean NOT NULL DEFAULT false,
    friendly_name text NOT NULL DEFAULT '',
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    last_used_at timestamptz,
    CONSTRAINT webauthn_credentials_pkey PRIMARY KEY (id),
    CONSTRAINT webauthn_credentials_user_id_fkey
        FOREIGN KEY (user_id) REFERENCES auth.users (id) ON DELETE CASCADE
);

ALTER TABLE auth.webauthn_credentials ADD COLUMN IF NOT EXISTS user_id uuid;
ALTER TABLE auth.webauthn_credentials ADD COLUMN IF NOT EXISTS credential_id bytea;
ALTER TABLE auth.webauthn_credentials ADD COLUMN IF NOT EXISTS public_key bytea;
ALTER TABLE auth.webauthn_credentials ADD COLUMN IF NOT EXISTS attestation_type text DEFAULT '';
ALTER TABLE auth.webauthn_credentials ADD COLUMN IF NOT EXISTS aaguid uuid;
ALTER TABLE auth.webauthn_credentials ADD COLUMN IF NOT EXISTS sign_count bigint DEFAULT 0;
ALTER TABLE auth.webauthn_credentials ADD COLUMN IF NOT EXISTS transports jsonb DEFAULT '[]'::jsonb;
ALTER TABLE auth.webauthn_credentials ADD COLUMN IF NOT EXISTS backup_eligible boolean DEFAULT false;
ALTER TABLE auth.webauthn_credentials ADD COLUMN IF NOT EXISTS backed_up boolean DEFAULT false;
ALTER TABLE auth.webauthn_credentials ADD COLUMN IF NOT EXISTS friendly_name text DEFAULT '';
ALTER TABLE auth.webauthn_credentials ADD COLUMN IF NOT EXISTS created_at timestamptz DEFAULT now();
ALTER TABLE auth.webauthn_credentials ADD COLUMN IF NOT EXISTS updated_at timestamptz DEFAULT now();
ALTER TABLE auth.webauthn_credentials ADD COLUMN IF NOT EXISTS last_used_at timestamptz;

ALTER TABLE auth.webauthn_credentials ALTER COLUMN id SET DEFAULT gen_random_uuid();
ALTER TABLE auth.webauthn_credentials ALTER COLUMN attestation_type SET DEFAULT '';
ALTER TABLE auth.webauthn_credentials ALTER COLUMN sign_count SET DEFAULT 0;
ALTER TABLE auth.webauthn_credentials ALTER COLUMN transports SET DEFAULT '[]'::jsonb;
ALTER TABLE auth.webauthn_credentials ALTER COLUMN backup_eligible SET DEFAULT false;
ALTER TABLE auth.webauthn_credentials ALTER COLUMN backed_up SET DEFAULT false;
ALTER TABLE auth.webauthn_credentials ALTER COLUMN friendly_name SET DEFAULT '';
ALTER TABLE auth.webauthn_credentials ALTER COLUMN created_at SET DEFAULT now();
ALTER TABLE auth.webauthn_credentials ALTER COLUMN updated_at SET DEFAULT now();

ALTER TABLE auth.webauthn_credentials ALTER COLUMN id SET NOT NULL;
ALTER TABLE auth.webauthn_credentials ALTER COLUMN user_id SET NOT NULL;
ALTER TABLE auth.webauthn_credentials ALTER COLUMN credential_id SET NOT NULL;
ALTER TABLE auth.webauthn_credentials ALTER COLUMN public_key SET NOT NULL;
ALTER TABLE auth.webauthn_credentials ALTER COLUMN attestation_type SET NOT NULL;
ALTER TABLE auth.webauthn_credentials ALTER COLUMN sign_count SET NOT NULL;
ALTER TABLE auth.webauthn_credentials ALTER COLUMN transports SET NOT NULL;
ALTER TABLE auth.webauthn_credentials ALTER COLUMN backup_eligible SET NOT NULL;
ALTER TABLE auth.webauthn_credentials ALTER COLUMN backed_up SET NOT NULL;
ALTER TABLE auth.webauthn_credentials ALTER COLUMN friendly_name SET NOT NULL;
ALTER TABLE auth.webauthn_credentials ALTER COLUMN created_at SET NOT NULL;
ALTER TABLE auth.webauthn_credentials ALTER COLUMN updated_at SET NOT NULL;

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1
        FROM pg_constraint
        WHERE conname = 'webauthn_credentials_pkey'
          AND conrelid = 'auth.webauthn_credentials'::regclass
    ) THEN
        ALTER TABLE auth.webauthn_credentials
            ADD CONSTRAINT webauthn_credentials_pkey PRIMARY KEY (id);
    END IF;
    IF NOT EXISTS (
        SELECT 1
        FROM pg_constraint
        WHERE conname = 'webauthn_credentials_user_id_fkey'
          AND conrelid = 'auth.webauthn_credentials'::regclass
    ) THEN
        ALTER TABLE auth.webauthn_credentials
            ADD CONSTRAINT webauthn_credentials_user_id_fkey
            FOREIGN KEY (user_id) REFERENCES auth.users (id) ON DELETE CASCADE;
    END IF;
END $$;

CREATE UNIQUE INDEX IF NOT EXISTS webauthn_credentials_credential_id_key
    ON auth.webauthn_credentials (credential_id);
CREATE INDEX IF NOT EXISTS webauthn_credentials_user_id_idx
    ON auth.webauthn_credentials (user_id);
