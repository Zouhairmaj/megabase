-- Ported from supabase/auth
-- migrations/20250804100000_add_oauth_authorizations_consents.up.sql (MIT), pin v2.197.0.
-- No RLS in this pin.

CREATE TABLE IF NOT EXISTS auth.oauth_consents (
    id uuid NOT NULL,
    user_id uuid NOT NULL REFERENCES auth.users(id) ON DELETE CASCADE,
    client_id uuid NOT NULL REFERENCES auth.oauth_clients(id) ON DELETE CASCADE,
    scopes text NOT NULL,
    granted_at timestamptz NOT NULL DEFAULT now(),
    revoked_at timestamptz NULL,
    CONSTRAINT oauth_consents_pkey PRIMARY KEY (id),
    CONSTRAINT oauth_consents_user_client_unique UNIQUE (user_id, client_id),
    CONSTRAINT oauth_consents_scopes_length CHECK (char_length(scopes) <= 2048),
    CONSTRAINT oauth_consents_scopes_not_empty CHECK (char_length(trim(scopes)) > 0),
    CONSTRAINT oauth_consents_revoked_after_granted CHECK (revoked_at IS NULL OR revoked_at >= granted_at)
);

ALTER TABLE auth.oauth_consents ADD COLUMN IF NOT EXISTS user_id uuid;
ALTER TABLE auth.oauth_consents ADD COLUMN IF NOT EXISTS client_id uuid;
ALTER TABLE auth.oauth_consents ADD COLUMN IF NOT EXISTS scopes text;
ALTER TABLE auth.oauth_consents ADD COLUMN IF NOT EXISTS granted_at timestamptz DEFAULT now();
ALTER TABLE auth.oauth_consents ADD COLUMN IF NOT EXISTS revoked_at timestamptz;

ALTER TABLE auth.oauth_consents ALTER COLUMN user_id SET NOT NULL;
ALTER TABLE auth.oauth_consents ALTER COLUMN client_id SET NOT NULL;
ALTER TABLE auth.oauth_consents ALTER COLUMN scopes SET NOT NULL;
ALTER TABLE auth.oauth_consents ALTER COLUMN granted_at SET NOT NULL;

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'oauth_consents_pkey'
          AND conrelid = 'auth.oauth_consents'::regclass
    ) THEN
        ALTER TABLE auth.oauth_consents ADD CONSTRAINT oauth_consents_pkey PRIMARY KEY (id);
    END IF;
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'oauth_consents_user_client_unique'
          AND conrelid = 'auth.oauth_consents'::regclass
    ) THEN
        ALTER TABLE auth.oauth_consents
            ADD CONSTRAINT oauth_consents_user_client_unique UNIQUE (user_id, client_id);
    END IF;
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'oauth_consents_user_id_fkey'
          AND conrelid = 'auth.oauth_consents'::regclass
    ) THEN
        ALTER TABLE auth.oauth_consents
            ADD CONSTRAINT oauth_consents_user_id_fkey
            FOREIGN KEY (user_id) REFERENCES auth.users(id) ON DELETE CASCADE;
    END IF;
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'oauth_consents_client_id_fkey'
          AND conrelid = 'auth.oauth_consents'::regclass
    ) THEN
        ALTER TABLE auth.oauth_consents
            ADD CONSTRAINT oauth_consents_client_id_fkey
            FOREIGN KEY (client_id) REFERENCES auth.oauth_clients(id) ON DELETE CASCADE;
    END IF;
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'oauth_consents_scopes_length'
          AND conrelid = 'auth.oauth_consents'::regclass
    ) THEN
        ALTER TABLE auth.oauth_consents
            ADD CONSTRAINT oauth_consents_scopes_length
            CHECK (char_length(scopes) <= 2048);
    END IF;
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'oauth_consents_scopes_not_empty'
          AND conrelid = 'auth.oauth_consents'::regclass
    ) THEN
        ALTER TABLE auth.oauth_consents
            ADD CONSTRAINT oauth_consents_scopes_not_empty
            CHECK (char_length(trim(scopes)) > 0);
    END IF;
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'oauth_consents_revoked_after_granted'
          AND conrelid = 'auth.oauth_consents'::regclass
    ) THEN
        ALTER TABLE auth.oauth_consents
            ADD CONSTRAINT oauth_consents_revoked_after_granted
            CHECK (revoked_at IS NULL OR revoked_at >= granted_at);
    END IF;
END $$;

CREATE INDEX IF NOT EXISTS oauth_consents_active_user_client_idx
    ON auth.oauth_consents (user_id, client_id)
    WHERE revoked_at IS NULL;
CREATE INDEX IF NOT EXISTS oauth_consents_user_order_idx
    ON auth.oauth_consents (user_id, granted_at DESC);
CREATE INDEX IF NOT EXISTS oauth_consents_active_client_idx
    ON auth.oauth_consents (client_id)
    WHERE revoked_at IS NULL;
