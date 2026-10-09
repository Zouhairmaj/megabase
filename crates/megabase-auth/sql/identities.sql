-- Ported from supabase/auth (MIT), pin v2.197.0:
--   migrations/20210909172000_create_identities_table.up.sql
--   migrations/20231117164230_add_id_pkey_identities.up.sql
--   migrations/20221215195800_add_identities_email_column.up.sql
--   migrations/20211122151130_create_user_id_idx.up.sql
--   migrations/20240612123726_enable_rls_update_grants.up.sql
-- Desired state only: do not RENAME id (that breaks an already-migrated UUID PK).

-- attnum order matches the pin: email was added before the uuid `id`
-- (`20221215195800` then `20231117164230`). ADD COLUMN IF NOT EXISTS
-- cannot reorder an existing table; drop an empty wrong-order table so
-- CREATE below can rebuild it. ACCESS EXCLUSIVE before COUNT so a
-- concurrent insert cannot land between the emptiness check and DROP.
DO $$
DECLARE
    email_att smallint;
    id_att smallint;
    n bigint;
BEGIN
    IF to_regclass('auth.identities') IS NULL THEN
        RETURN;
    END IF;
    LOCK TABLE auth.identities IN ACCESS EXCLUSIVE MODE;
    SELECT COUNT(*) INTO n FROM auth.identities;
    SELECT a.attnum INTO email_att
      FROM pg_attribute a
      JOIN pg_class c ON c.oid = a.attrelid
      JOIN pg_namespace ns ON ns.oid = c.relnamespace
     WHERE ns.nspname = 'auth' AND c.relname = 'identities'
       AND a.attname = 'email' AND NOT a.attisdropped AND a.attnum > 0;
    SELECT a.attnum INTO id_att
      FROM pg_attribute a
      JOIN pg_class c ON c.oid = a.attrelid
      JOIN pg_namespace ns ON ns.oid = c.relnamespace
     WHERE ns.nspname = 'auth' AND c.relname = 'identities'
       AND a.attname = 'id' AND NOT a.attisdropped AND a.attnum > 0;
    IF n = 0 AND email_att IS NOT NULL AND id_att IS NOT NULL AND email_att > id_att THEN
        DROP TABLE auth.identities;
    END IF;
END $$;

CREATE TABLE IF NOT EXISTS auth.identities (
    provider_id text NOT NULL,
    user_id uuid NOT NULL,
    identity_data jsonb NOT NULL,
    provider text NOT NULL,
    last_sign_in_at timestamptz NULL,
    created_at timestamptz NULL,
    updated_at timestamptz NULL,
    email text GENERATED ALWAYS AS (lower(identity_data->>'email')) STORED,
    id uuid NOT NULL DEFAULT gen_random_uuid(),
    CONSTRAINT identities_pkey PRIMARY KEY (id),
    CONSTRAINT identities_user_id_fkey FOREIGN KEY (user_id) REFERENCES auth.users(id) ON DELETE CASCADE,
    CONSTRAINT identities_provider_id_provider_unique UNIQUE (provider_id, provider)
);

ALTER TABLE auth.identities ADD COLUMN IF NOT EXISTS provider_id text;
ALTER TABLE auth.identities ADD COLUMN IF NOT EXISTS user_id uuid;
ALTER TABLE auth.identities ADD COLUMN IF NOT EXISTS identity_data jsonb;
ALTER TABLE auth.identities ADD COLUMN IF NOT EXISTS provider text;
ALTER TABLE auth.identities ADD COLUMN IF NOT EXISTS last_sign_in_at timestamptz;
ALTER TABLE auth.identities ADD COLUMN IF NOT EXISTS created_at timestamptz;
ALTER TABLE auth.identities ADD COLUMN IF NOT EXISTS updated_at timestamptz;

ALTER TABLE auth.identities ALTER COLUMN provider_id SET NOT NULL;
ALTER TABLE auth.identities ALTER COLUMN user_id SET NOT NULL;
ALTER TABLE auth.identities ALTER COLUMN identity_data SET NOT NULL;
ALTER TABLE auth.identities ALTER COLUMN provider SET NOT NULL;

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1
        FROM information_schema.columns
        WHERE table_schema = 'auth'
          AND table_name = 'identities'
          AND column_name = 'email'
    ) THEN
        ALTER TABLE auth.identities
            ADD COLUMN email text GENERATED ALWAYS AS (lower(identity_data->>'email')) STORED;
    END IF;
END $$;

ALTER TABLE auth.identities ADD COLUMN IF NOT EXISTS id uuid DEFAULT gen_random_uuid();
ALTER TABLE auth.identities ALTER COLUMN id SET NOT NULL;

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1
        FROM pg_constraint
        WHERE conname = 'identities_pkey'
          AND conrelid = 'auth.identities'::regclass
    ) THEN
        ALTER TABLE auth.identities ADD CONSTRAINT identities_pkey PRIMARY KEY (id);
    END IF;
    IF NOT EXISTS (
        SELECT 1
        FROM pg_constraint
        WHERE conname = 'identities_user_id_fkey'
          AND conrelid = 'auth.identities'::regclass
    ) THEN
        ALTER TABLE auth.identities
            ADD CONSTRAINT identities_user_id_fkey
            FOREIGN KEY (user_id) REFERENCES auth.users(id) ON DELETE CASCADE;
    END IF;
    IF NOT EXISTS (
        SELECT 1
        FROM pg_constraint
        WHERE conname = 'identities_provider_id_provider_unique'
          AND conrelid = 'auth.identities'::regclass
    ) THEN
        ALTER TABLE auth.identities
            ADD CONSTRAINT identities_provider_id_provider_unique UNIQUE (provider_id, provider);
    END IF;
END $$;

CREATE INDEX IF NOT EXISTS identities_user_id_idx ON auth.identities USING btree (user_id);
CREATE INDEX IF NOT EXISTS identities_email_idx ON auth.identities (email text_pattern_ops);

COMMENT ON TABLE auth.identities IS 'Auth: Stores identities associated to a user.';
COMMENT ON COLUMN auth.identities.email IS
    'Auth: Email is a generated column that references the optional email property in the identity_data';
COMMENT ON INDEX auth.identities_email_idx IS 'Auth: Ensures indexed queries on the email column';

ALTER TABLE auth.identities ENABLE ROW LEVEL SECURITY;

DO $$
BEGIN
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'postgres') THEN
        GRANT SELECT ON auth.identities TO postgres WITH GRANT OPTION;
    END IF;
END $$;
