-- Ported from supabase/auth (MIT), pin v2.197.0:
--   migrations/20260821000000_add_scim_users.up.sql

CREATE TABLE IF NOT EXISTS auth.scim_users (
    id uuid NOT NULL,
    sso_provider_id uuid NOT NULL REFERENCES auth.sso_providers (id) ON DELETE CASCADE,
    user_id uuid REFERENCES auth.users (id) ON DELETE SET NULL,
    resource jsonb NOT NULL,
    user_name text NOT NULL GENERATED ALWAYS AS (lower(resource->>'userName')) STORED,
    external_id text GENERATED ALWAYS AS (resource->>'externalId') STORED,
    active boolean NOT NULL GENERATED ALWAYS AS (coalesce((resource->>'active')::boolean, true)) STORED,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    deleted_at timestamptz,
    CONSTRAINT scim_users_pkey PRIMARY KEY (id)
);

ALTER TABLE auth.scim_users ADD COLUMN IF NOT EXISTS sso_provider_id uuid;
ALTER TABLE auth.scim_users ADD COLUMN IF NOT EXISTS user_id uuid;
ALTER TABLE auth.scim_users ADD COLUMN IF NOT EXISTS resource jsonb;
ALTER TABLE auth.scim_users ADD COLUMN IF NOT EXISTS created_at timestamptz NOT NULL DEFAULT now();
ALTER TABLE auth.scim_users ADD COLUMN IF NOT EXISTS updated_at timestamptz NOT NULL DEFAULT now();
ALTER TABLE auth.scim_users ADD COLUMN IF NOT EXISTS deleted_at timestamptz;

ALTER TABLE auth.scim_users ALTER COLUMN id SET NOT NULL;
ALTER TABLE auth.scim_users ALTER COLUMN sso_provider_id SET NOT NULL;
ALTER TABLE auth.scim_users ALTER COLUMN resource SET NOT NULL;
ALTER TABLE auth.scim_users ALTER COLUMN created_at SET NOT NULL;
ALTER TABLE auth.scim_users ALTER COLUMN updated_at SET NOT NULL;

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1
        FROM information_schema.columns
        WHERE table_schema = 'auth'
          AND table_name = 'scim_users'
          AND column_name = 'user_name'
    ) THEN
        ALTER TABLE auth.scim_users
            ADD COLUMN user_name text NOT NULL GENERATED ALWAYS AS (lower(resource->>'userName')) STORED;
    END IF;
    IF NOT EXISTS (
        SELECT 1
        FROM information_schema.columns
        WHERE table_schema = 'auth'
          AND table_name = 'scim_users'
          AND column_name = 'external_id'
    ) THEN
        ALTER TABLE auth.scim_users
            ADD COLUMN external_id text GENERATED ALWAYS AS (resource->>'externalId') STORED;
    END IF;
    IF NOT EXISTS (
        SELECT 1
        FROM information_schema.columns
        WHERE table_schema = 'auth'
          AND table_name = 'scim_users'
          AND column_name = 'active'
    ) THEN
        ALTER TABLE auth.scim_users
            ADD COLUMN active boolean NOT NULL GENERATED ALWAYS AS (coalesce((resource->>'active')::boolean, true)) STORED;
    END IF;
END $$;

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1
        FROM pg_constraint
        WHERE conname = 'scim_users_pkey'
          AND conrelid = 'auth.scim_users'::regclass
    ) THEN
        ALTER TABLE auth.scim_users ADD CONSTRAINT scim_users_pkey PRIMARY KEY (id);
    END IF;
    IF NOT EXISTS (
        SELECT 1
        FROM pg_constraint
        WHERE conname = 'scim_users_sso_provider_id_fkey'
          AND conrelid = 'auth.scim_users'::regclass
    ) THEN
        ALTER TABLE auth.scim_users
            ADD CONSTRAINT scim_users_sso_provider_id_fkey
            FOREIGN KEY (sso_provider_id) REFERENCES auth.sso_providers (id) ON DELETE CASCADE;
    END IF;
    IF NOT EXISTS (
        SELECT 1
        FROM pg_constraint
        WHERE conname = 'scim_users_user_id_fkey'
          AND conrelid = 'auth.scim_users'::regclass
    ) THEN
        ALTER TABLE auth.scim_users
            ADD CONSTRAINT scim_users_user_id_fkey
            FOREIGN KEY (user_id) REFERENCES auth.users (id) ON DELETE SET NULL;
    END IF;
END $$;

CREATE UNIQUE INDEX IF NOT EXISTS scim_users_user_name_key
    ON auth.scim_users (sso_provider_id, user_name)
    WHERE deleted_at IS NULL;
CREATE UNIQUE INDEX IF NOT EXISTS scim_users_external_id_key
    ON auth.scim_users (sso_provider_id, external_id)
    WHERE external_id IS NOT NULL AND deleted_at IS NULL;
CREATE INDEX IF NOT EXISTS scim_users_user_id_idx ON auth.scim_users (user_id);
CREATE INDEX IF NOT EXISTS scim_users_id_idx
    ON auth.scim_users (sso_provider_id, id)
    WHERE deleted_at IS NULL;
CREATE INDEX IF NOT EXISTS scim_users_user_name_idx
    ON auth.scim_users (sso_provider_id, user_name COLLATE "C", id)
    WHERE deleted_at IS NULL;
CREATE INDEX IF NOT EXISTS scim_users_created_at_idx
    ON auth.scim_users (sso_provider_id, created_at, id)
    WHERE deleted_at IS NULL;
CREATE INDEX IF NOT EXISTS scim_users_updated_at_idx
    ON auth.scim_users (sso_provider_id, updated_at, id)
    WHERE deleted_at IS NULL;
CREATE INDEX IF NOT EXISTS scim_users_sso_provider_id_idx ON auth.scim_users (sso_provider_id);
CREATE INDEX IF NOT EXISTS scim_users_deleted_at_idx ON auth.scim_users (deleted_at);
