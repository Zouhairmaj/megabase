-- Ported from supabase/auth (MIT), pin v2.197.0:
--   migrations/20250731150234_add_oauth_clients_table.up.sql
--   migrations/20250901200500_add_oauth_client_type.up.sql
--   migrations/20250903112500_remove_oauth_client_id_column.up.sql
--   migrations/20260121000000_add_token_endpoint_auth_method.up.sql
-- Desired state: no client_id column; id is the public client identifier.

DO $$ BEGIN
    CREATE TYPE auth.oauth_registration_type AS ENUM ('dynamic', 'manual');
EXCEPTION
    WHEN duplicate_object THEN NULL;
END $$;

DO $$ BEGIN
    CREATE TYPE auth.oauth_client_type AS ENUM ('public', 'confidential');
EXCEPTION
    WHEN duplicate_object THEN NULL;
END $$;

CREATE TABLE IF NOT EXISTS auth.oauth_clients (
    id uuid NOT NULL,
    client_secret_hash text NULL,
    registration_type auth.oauth_registration_type NOT NULL,
    redirect_uris text NOT NULL,
    grant_types text NOT NULL,
    client_name text NULL,
    client_uri text NULL,
    logo_uri text NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    deleted_at timestamptz NULL,
    client_type auth.oauth_client_type NOT NULL DEFAULT 'confidential',
    token_endpoint_auth_method text NOT NULL,
    CONSTRAINT oauth_clients_pkey PRIMARY KEY (id),
    CONSTRAINT oauth_clients_client_name_length CHECK (char_length(client_name) <= 1024),
    CONSTRAINT oauth_clients_client_uri_length CHECK (char_length(client_uri) <= 2048),
    CONSTRAINT oauth_clients_logo_uri_length CHECK (char_length(logo_uri) <= 2048),
    CONSTRAINT oauth_clients_token_endpoint_auth_method_check CHECK (
        token_endpoint_auth_method IN ('client_secret_basic', 'client_secret_post', 'none')
    )
);

ALTER TABLE auth.oauth_clients ADD COLUMN IF NOT EXISTS client_secret_hash text;
ALTER TABLE auth.oauth_clients ADD COLUMN IF NOT EXISTS registration_type auth.oauth_registration_type;
ALTER TABLE auth.oauth_clients ADD COLUMN IF NOT EXISTS redirect_uris text;
ALTER TABLE auth.oauth_clients ADD COLUMN IF NOT EXISTS grant_types text;
ALTER TABLE auth.oauth_clients ADD COLUMN IF NOT EXISTS client_name text;
ALTER TABLE auth.oauth_clients ADD COLUMN IF NOT EXISTS client_uri text;
ALTER TABLE auth.oauth_clients ADD COLUMN IF NOT EXISTS logo_uri text;
ALTER TABLE auth.oauth_clients ADD COLUMN IF NOT EXISTS created_at timestamptz DEFAULT now();
ALTER TABLE auth.oauth_clients ADD COLUMN IF NOT EXISTS updated_at timestamptz DEFAULT now();
ALTER TABLE auth.oauth_clients ADD COLUMN IF NOT EXISTS deleted_at timestamptz;
ALTER TABLE auth.oauth_clients ADD COLUMN IF NOT EXISTS client_type auth.oauth_client_type DEFAULT 'confidential';
ALTER TABLE auth.oauth_clients ADD COLUMN IF NOT EXISTS token_endpoint_auth_method text;

ALTER TABLE auth.oauth_clients ALTER COLUMN client_secret_hash DROP NOT NULL;
ALTER TABLE auth.oauth_clients ALTER COLUMN registration_type SET NOT NULL;
ALTER TABLE auth.oauth_clients ALTER COLUMN redirect_uris SET NOT NULL;
ALTER TABLE auth.oauth_clients ALTER COLUMN grant_types SET NOT NULL;
ALTER TABLE auth.oauth_clients ALTER COLUMN created_at SET NOT NULL;
ALTER TABLE auth.oauth_clients ALTER COLUMN updated_at SET NOT NULL;
ALTER TABLE auth.oauth_clients ALTER COLUMN client_type SET NOT NULL;

UPDATE auth.oauth_clients
SET token_endpoint_auth_method = CASE
    WHEN client_type = 'public' THEN 'none'
    ELSE 'client_secret_basic'
END
WHERE token_endpoint_auth_method IS NULL;

ALTER TABLE auth.oauth_clients ALTER COLUMN token_endpoint_auth_method SET NOT NULL;

ALTER TABLE auth.oauth_clients DROP CONSTRAINT IF EXISTS oauth_clients_client_id_key;
DROP INDEX IF EXISTS auth.oauth_clients_client_id_idx;
ALTER TABLE auth.oauth_clients DROP COLUMN IF EXISTS client_id;

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1
        FROM pg_constraint
        WHERE conname = 'oauth_clients_pkey'
          AND conrelid = 'auth.oauth_clients'::regclass
    ) THEN
        ALTER TABLE auth.oauth_clients ADD CONSTRAINT oauth_clients_pkey PRIMARY KEY (id);
    END IF;
    IF NOT EXISTS (
        SELECT 1
        FROM pg_constraint
        WHERE conname = 'oauth_clients_client_name_length'
          AND conrelid = 'auth.oauth_clients'::regclass
    ) THEN
        ALTER TABLE auth.oauth_clients
            ADD CONSTRAINT oauth_clients_client_name_length
            CHECK (char_length(client_name) <= 1024);
    END IF;
    IF NOT EXISTS (
        SELECT 1
        FROM pg_constraint
        WHERE conname = 'oauth_clients_client_uri_length'
          AND conrelid = 'auth.oauth_clients'::regclass
    ) THEN
        ALTER TABLE auth.oauth_clients
            ADD CONSTRAINT oauth_clients_client_uri_length
            CHECK (char_length(client_uri) <= 2048);
    END IF;
    IF NOT EXISTS (
        SELECT 1
        FROM pg_constraint
        WHERE conname = 'oauth_clients_logo_uri_length'
          AND conrelid = 'auth.oauth_clients'::regclass
    ) THEN
        ALTER TABLE auth.oauth_clients
            ADD CONSTRAINT oauth_clients_logo_uri_length
            CHECK (char_length(logo_uri) <= 2048);
    END IF;
    IF NOT EXISTS (
        SELECT 1
        FROM pg_constraint
        WHERE conname = 'oauth_clients_token_endpoint_auth_method_check'
          AND conrelid = 'auth.oauth_clients'::regclass
    ) THEN
        ALTER TABLE auth.oauth_clients
            ADD CONSTRAINT oauth_clients_token_endpoint_auth_method_check
            CHECK (token_endpoint_auth_method IN ('client_secret_basic', 'client_secret_post', 'none'));
    END IF;
END $$;

CREATE INDEX IF NOT EXISTS oauth_clients_deleted_at_idx
    ON auth.oauth_clients (deleted_at);
