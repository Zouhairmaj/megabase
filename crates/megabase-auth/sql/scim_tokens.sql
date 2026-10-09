-- Ported from supabase/auth (MIT), pin v2.197.0:
--   migrations/20260821010000_add_scim_tokens.up.sql

CREATE TABLE IF NOT EXISTS auth.scim_tokens (
    id uuid NOT NULL,
    sso_provider_id uuid NOT NULL REFERENCES auth.sso_providers (id) ON DELETE CASCADE,
    token_hash text NOT NULL,
    prefix text NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    expires_at timestamptz,
    revoked_at timestamptz,
    last_used_at timestamptz,
    CONSTRAINT scim_tokens_pkey PRIMARY KEY (id),
    CONSTRAINT scim_tokens_token_hash_check CHECK (token_hash ~ '^[0-9a-f]{64}$'),
    CONSTRAINT scim_tokens_expires_at_future CHECK (expires_at IS NULL OR expires_at > created_at),
    CONSTRAINT scim_tokens_revoked_after_created CHECK (revoked_at IS NULL OR revoked_at >= created_at)
);

ALTER TABLE auth.scim_tokens ADD COLUMN IF NOT EXISTS sso_provider_id uuid;
ALTER TABLE auth.scim_tokens ADD COLUMN IF NOT EXISTS token_hash text;
ALTER TABLE auth.scim_tokens ADD COLUMN IF NOT EXISTS prefix text;
ALTER TABLE auth.scim_tokens ADD COLUMN IF NOT EXISTS created_at timestamptz NOT NULL DEFAULT now();
ALTER TABLE auth.scim_tokens ADD COLUMN IF NOT EXISTS expires_at timestamptz;
ALTER TABLE auth.scim_tokens ADD COLUMN IF NOT EXISTS revoked_at timestamptz;
ALTER TABLE auth.scim_tokens ADD COLUMN IF NOT EXISTS last_used_at timestamptz;

ALTER TABLE auth.scim_tokens ALTER COLUMN id SET NOT NULL;
ALTER TABLE auth.scim_tokens ALTER COLUMN sso_provider_id SET NOT NULL;
ALTER TABLE auth.scim_tokens ALTER COLUMN token_hash SET NOT NULL;
ALTER TABLE auth.scim_tokens ALTER COLUMN prefix SET NOT NULL;
ALTER TABLE auth.scim_tokens ALTER COLUMN created_at SET NOT NULL;

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1
        FROM pg_constraint
        WHERE conname = 'scim_tokens_pkey'
          AND conrelid = 'auth.scim_tokens'::regclass
    ) THEN
        ALTER TABLE auth.scim_tokens ADD CONSTRAINT scim_tokens_pkey PRIMARY KEY (id);
    END IF;
    IF NOT EXISTS (
        SELECT 1
        FROM pg_constraint
        WHERE conname = 'scim_tokens_sso_provider_id_fkey'
          AND conrelid = 'auth.scim_tokens'::regclass
    ) THEN
        ALTER TABLE auth.scim_tokens
            ADD CONSTRAINT scim_tokens_sso_provider_id_fkey
            FOREIGN KEY (sso_provider_id) REFERENCES auth.sso_providers (id) ON DELETE CASCADE;
    END IF;
    IF NOT EXISTS (
        SELECT 1
        FROM pg_constraint
        WHERE conname = 'scim_tokens_token_hash_check'
          AND conrelid = 'auth.scim_tokens'::regclass
    ) THEN
        ALTER TABLE auth.scim_tokens
            ADD CONSTRAINT scim_tokens_token_hash_check CHECK (token_hash ~ '^[0-9a-f]{64}$');
    END IF;
    IF NOT EXISTS (
        SELECT 1
        FROM pg_constraint
        WHERE conname = 'scim_tokens_expires_at_future'
          AND conrelid = 'auth.scim_tokens'::regclass
    ) THEN
        ALTER TABLE auth.scim_tokens
            ADD CONSTRAINT scim_tokens_expires_at_future
            CHECK (expires_at IS NULL OR expires_at > created_at);
    END IF;
    IF NOT EXISTS (
        SELECT 1
        FROM pg_constraint
        WHERE conname = 'scim_tokens_revoked_after_created'
          AND conrelid = 'auth.scim_tokens'::regclass
    ) THEN
        ALTER TABLE auth.scim_tokens
            ADD CONSTRAINT scim_tokens_revoked_after_created
            CHECK (revoked_at IS NULL OR revoked_at >= created_at);
    END IF;
END $$;

CREATE UNIQUE INDEX IF NOT EXISTS scim_tokens_token_hash_key ON auth.scim_tokens (token_hash);
CREATE INDEX IF NOT EXISTS scim_tokens_sso_provider_id_idx ON auth.scim_tokens (sso_provider_id);
CREATE INDEX IF NOT EXISTS scim_tokens_expires_at_idx ON auth.scim_tokens (expires_at);
CREATE INDEX IF NOT EXISTS scim_tokens_revoked_at_idx ON auth.scim_tokens (revoked_at);
