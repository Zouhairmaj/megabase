-- Ported from supabase/auth (MIT), pin v2.197.0:
--   migrations/20250804100000_add_oauth_authorizations_consents.up.sql
--   migrations/20251104100000_add_nonce_to_oauth_authorizations.up.sql
-- Uses auth.code_challenge_method from the flow_state install.

DO $$ BEGIN
    CREATE TYPE auth.oauth_authorization_status AS ENUM ('pending', 'approved', 'denied', 'expired');
EXCEPTION
    WHEN duplicate_object THEN NULL;
END $$;

DO $$ BEGIN
    CREATE TYPE auth.oauth_response_type AS ENUM ('code');
EXCEPTION
    WHEN duplicate_object THEN NULL;
END $$;

CREATE TABLE IF NOT EXISTS auth.oauth_authorizations (
    id uuid NOT NULL,
    authorization_id text NOT NULL,
    client_id uuid NOT NULL REFERENCES auth.oauth_clients(id) ON DELETE CASCADE,
    user_id uuid NULL REFERENCES auth.users(id) ON DELETE CASCADE,
    redirect_uri text NOT NULL,
    scope text NOT NULL,
    state text NULL,
    resource text NULL,
    code_challenge text NULL,
    code_challenge_method auth.code_challenge_method NULL,
    response_type auth.oauth_response_type NOT NULL DEFAULT 'code',
    status auth.oauth_authorization_status NOT NULL DEFAULT 'pending',
    authorization_code text NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    expires_at timestamptz NOT NULL DEFAULT (now() + interval '3 minutes'),
    approved_at timestamptz NULL,
    nonce text NULL,
    CONSTRAINT oauth_authorizations_pkey PRIMARY KEY (id),
    CONSTRAINT oauth_authorizations_authorization_id_key UNIQUE (authorization_id),
    CONSTRAINT oauth_authorizations_authorization_code_key UNIQUE (authorization_code),
    CONSTRAINT oauth_authorizations_redirect_uri_length CHECK (char_length(redirect_uri) <= 2048),
    CONSTRAINT oauth_authorizations_scope_length CHECK (char_length(scope) <= 4096),
    CONSTRAINT oauth_authorizations_state_length CHECK (char_length(state) <= 4096),
    CONSTRAINT oauth_authorizations_resource_length CHECK (char_length(resource) <= 2048),
    CONSTRAINT oauth_authorizations_code_challenge_length CHECK (char_length(code_challenge) <= 128),
    CONSTRAINT oauth_authorizations_authorization_code_length CHECK (char_length(authorization_code) <= 255),
    CONSTRAINT oauth_authorizations_expires_at_future CHECK (expires_at > created_at),
    CONSTRAINT oauth_authorizations_nonce_length CHECK (char_length(nonce) <= 255)
);

ALTER TABLE auth.oauth_authorizations ADD COLUMN IF NOT EXISTS authorization_id text;
ALTER TABLE auth.oauth_authorizations ADD COLUMN IF NOT EXISTS client_id uuid;
ALTER TABLE auth.oauth_authorizations ADD COLUMN IF NOT EXISTS user_id uuid;
ALTER TABLE auth.oauth_authorizations ADD COLUMN IF NOT EXISTS redirect_uri text;
ALTER TABLE auth.oauth_authorizations ADD COLUMN IF NOT EXISTS scope text;
ALTER TABLE auth.oauth_authorizations ADD COLUMN IF NOT EXISTS state text;
ALTER TABLE auth.oauth_authorizations ADD COLUMN IF NOT EXISTS resource text;
ALTER TABLE auth.oauth_authorizations ADD COLUMN IF NOT EXISTS code_challenge text;
ALTER TABLE auth.oauth_authorizations ADD COLUMN IF NOT EXISTS code_challenge_method auth.code_challenge_method;
ALTER TABLE auth.oauth_authorizations ADD COLUMN IF NOT EXISTS response_type auth.oauth_response_type DEFAULT 'code';
ALTER TABLE auth.oauth_authorizations ADD COLUMN IF NOT EXISTS status auth.oauth_authorization_status DEFAULT 'pending';
ALTER TABLE auth.oauth_authorizations ADD COLUMN IF NOT EXISTS authorization_code text;
ALTER TABLE auth.oauth_authorizations ADD COLUMN IF NOT EXISTS created_at timestamptz DEFAULT now();
ALTER TABLE auth.oauth_authorizations ADD COLUMN IF NOT EXISTS expires_at timestamptz DEFAULT (now() + interval '3 minutes');
ALTER TABLE auth.oauth_authorizations ADD COLUMN IF NOT EXISTS approved_at timestamptz;
ALTER TABLE auth.oauth_authorizations ADD COLUMN IF NOT EXISTS nonce text;

ALTER TABLE auth.oauth_authorizations ALTER COLUMN authorization_id SET NOT NULL;
ALTER TABLE auth.oauth_authorizations ALTER COLUMN client_id SET NOT NULL;
ALTER TABLE auth.oauth_authorizations ALTER COLUMN redirect_uri SET NOT NULL;
ALTER TABLE auth.oauth_authorizations ALTER COLUMN scope SET NOT NULL;
ALTER TABLE auth.oauth_authorizations ALTER COLUMN response_type SET NOT NULL;
ALTER TABLE auth.oauth_authorizations ALTER COLUMN status SET NOT NULL;
ALTER TABLE auth.oauth_authorizations ALTER COLUMN created_at SET NOT NULL;
ALTER TABLE auth.oauth_authorizations ALTER COLUMN expires_at SET NOT NULL;

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'oauth_authorizations_pkey'
          AND conrelid = 'auth.oauth_authorizations'::regclass
    ) THEN
        ALTER TABLE auth.oauth_authorizations ADD CONSTRAINT oauth_authorizations_pkey PRIMARY KEY (id);
    END IF;
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'oauth_authorizations_authorization_id_key'
          AND conrelid = 'auth.oauth_authorizations'::regclass
    ) THEN
        ALTER TABLE auth.oauth_authorizations
            ADD CONSTRAINT oauth_authorizations_authorization_id_key UNIQUE (authorization_id);
    END IF;
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'oauth_authorizations_authorization_code_key'
          AND conrelid = 'auth.oauth_authorizations'::regclass
    ) THEN
        ALTER TABLE auth.oauth_authorizations
            ADD CONSTRAINT oauth_authorizations_authorization_code_key UNIQUE (authorization_code);
    END IF;
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'oauth_authorizations_client_id_fkey'
          AND conrelid = 'auth.oauth_authorizations'::regclass
    ) THEN
        ALTER TABLE auth.oauth_authorizations
            ADD CONSTRAINT oauth_authorizations_client_id_fkey
            FOREIGN KEY (client_id) REFERENCES auth.oauth_clients(id) ON DELETE CASCADE;
    END IF;
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'oauth_authorizations_user_id_fkey'
          AND conrelid = 'auth.oauth_authorizations'::regclass
    ) THEN
        ALTER TABLE auth.oauth_authorizations
            ADD CONSTRAINT oauth_authorizations_user_id_fkey
            FOREIGN KEY (user_id) REFERENCES auth.users(id) ON DELETE CASCADE;
    END IF;
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'oauth_authorizations_nonce_length'
          AND conrelid = 'auth.oauth_authorizations'::regclass
    ) THEN
        ALTER TABLE auth.oauth_authorizations
            ADD CONSTRAINT oauth_authorizations_nonce_length
            CHECK (char_length(nonce) <= 255);
    END IF;
END $$;

CREATE INDEX IF NOT EXISTS oauth_auth_pending_exp_idx
    ON auth.oauth_authorizations (expires_at)
    WHERE status = 'pending';
