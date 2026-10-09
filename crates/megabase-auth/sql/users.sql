-- Ported from supabase/auth (MIT), pin v2.197.0:
--   migrations/00_init_auth_schema.up.sql
--   migrations/20210710035447_alter_users.up.sql
--   migrations/20210722035447_adds_confirmed_at.up.sql
--   migrations/20210730183235_add_email_change_confirmed.up.sql
--   migrations/20220114185221_update_user_idx.up.sql
--   migrations/20220114185340_add_banned_until.up.sql
--   migrations/20220323170000_add_user_reauthentication.up.sql
--   migrations/20220429102000_add_unique_idx.up.sql
--   migrations/20221215195500_modify_users_email_unique_index.up.sql
--   migrations/20230116124310_alter_phone_type.up.sql
--   migrations/20230116124412_add_deleted_at.up.sql
--   migrations/20240214120130_add_is_anonymous_column.up.sql
--   migrations/20240612123726_enable_rls_update_grants.up.sql
-- Desired state only: do not RENAME confirmed_at / email_change_token when
-- the final column names already exist (that breaks an already-migrated table).

CREATE TABLE IF NOT EXISTS auth.users (
    instance_id uuid NULL,
    id uuid NOT NULL,
    aud varchar(255) NULL,
    role varchar(255) NULL,
    email varchar(255) NULL,
    encrypted_password varchar(255) NULL,
    email_confirmed_at timestamptz NULL,
    invited_at timestamptz NULL,
    confirmation_token varchar(255) NULL,
    confirmation_sent_at timestamptz NULL,
    recovery_token varchar(255) NULL,
    recovery_sent_at timestamptz NULL,
    email_change_token_new varchar(255) NULL,
    email_change varchar(255) NULL,
    email_change_sent_at timestamptz NULL,
    last_sign_in_at timestamptz NULL,
    raw_app_meta_data jsonb NULL,
    raw_user_meta_data jsonb NULL,
    is_super_admin bool NULL,
    created_at timestamptz NULL,
    updated_at timestamptz NULL,
    phone text NULL UNIQUE,
    phone_confirmed_at timestamptz NULL,
    phone_change text NULL DEFAULT '',
    phone_change_token varchar(255) NULL DEFAULT '',
    phone_change_sent_at timestamptz NULL,
    confirmed_at timestamptz GENERATED ALWAYS AS (LEAST (users.email_confirmed_at, users.phone_confirmed_at)) STORED,
    email_change_token_current varchar(255) NULL DEFAULT '',
    email_change_confirm_status smallint DEFAULT 0
        CHECK (email_change_confirm_status >= 0 AND email_change_confirm_status <= 2),
    banned_until timestamptz NULL,
    reauthentication_token varchar(255) NULL DEFAULT '',
    reauthentication_sent_at timestamptz NULL,
    deleted_at timestamptz NULL,
    is_sso_user boolean NOT NULL DEFAULT false,
    is_anonymous boolean NOT NULL DEFAULT false,
    CONSTRAINT users_pkey PRIMARY KEY (id)
);

-- Rename legacy confirmed_at / email_change_token before ADD COLUMN so
-- IF NOT EXISTS does not create empty replacements and skip the rename.
DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1
        FROM information_schema.columns
        WHERE table_schema = 'auth'
          AND table_name = 'users'
          AND column_name = 'email_confirmed_at'
    ) AND EXISTS (
        SELECT 1
        FROM information_schema.columns
        WHERE table_schema = 'auth'
          AND table_name = 'users'
          AND column_name = 'confirmed_at'
    ) AND EXISTS (
        SELECT 1
        FROM pg_attribute
        WHERE attrelid = 'auth.users'::regclass
          AND attname = 'confirmed_at'
          AND attgenerated = ''
    ) THEN
        ALTER TABLE auth.users RENAME COLUMN confirmed_at TO email_confirmed_at;
    END IF;
END $$;

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1
        FROM information_schema.columns
        WHERE table_schema = 'auth'
          AND table_name = 'users'
          AND column_name = 'email_change_token_new'
    ) AND EXISTS (
        SELECT 1
        FROM information_schema.columns
        WHERE table_schema = 'auth'
          AND table_name = 'users'
          AND column_name = 'email_change_token'
    ) THEN
        ALTER TABLE auth.users RENAME COLUMN email_change_token TO email_change_token_new;
    END IF;
END $$;

-- ADD order matches pin attnums after the stub's instance_id, id.
ALTER TABLE auth.users ADD COLUMN IF NOT EXISTS instance_id uuid;
ALTER TABLE auth.users ADD COLUMN IF NOT EXISTS aud varchar(255);
ALTER TABLE auth.users ADD COLUMN IF NOT EXISTS role varchar(255);
ALTER TABLE auth.users ADD COLUMN IF NOT EXISTS email varchar(255);
ALTER TABLE auth.users ADD COLUMN IF NOT EXISTS encrypted_password varchar(255);
ALTER TABLE auth.users ADD COLUMN IF NOT EXISTS email_confirmed_at timestamptz;
ALTER TABLE auth.users ADD COLUMN IF NOT EXISTS invited_at timestamptz;
ALTER TABLE auth.users ADD COLUMN IF NOT EXISTS confirmation_token varchar(255);
ALTER TABLE auth.users ADD COLUMN IF NOT EXISTS confirmation_sent_at timestamptz;
ALTER TABLE auth.users ADD COLUMN IF NOT EXISTS recovery_token varchar(255);
ALTER TABLE auth.users ADD COLUMN IF NOT EXISTS recovery_sent_at timestamptz;
ALTER TABLE auth.users ADD COLUMN IF NOT EXISTS email_change_token_new varchar(255);
ALTER TABLE auth.users ADD COLUMN IF NOT EXISTS email_change varchar(255);
ALTER TABLE auth.users ADD COLUMN IF NOT EXISTS email_change_sent_at timestamptz;
ALTER TABLE auth.users ADD COLUMN IF NOT EXISTS last_sign_in_at timestamptz;
ALTER TABLE auth.users ADD COLUMN IF NOT EXISTS raw_app_meta_data jsonb;
ALTER TABLE auth.users ADD COLUMN IF NOT EXISTS raw_user_meta_data jsonb;
ALTER TABLE auth.users ADD COLUMN IF NOT EXISTS is_super_admin bool;
ALTER TABLE auth.users ADD COLUMN IF NOT EXISTS created_at timestamptz;
ALTER TABLE auth.users ADD COLUMN IF NOT EXISTS updated_at timestamptz;
ALTER TABLE auth.users ADD COLUMN IF NOT EXISTS phone text;
ALTER TABLE auth.users ADD COLUMN IF NOT EXISTS phone_confirmed_at timestamptz;
ALTER TABLE auth.users ADD COLUMN IF NOT EXISTS phone_change text DEFAULT '';
ALTER TABLE auth.users ADD COLUMN IF NOT EXISTS phone_change_token varchar(255) DEFAULT '';
ALTER TABLE auth.users ADD COLUMN IF NOT EXISTS phone_change_sent_at timestamptz;
ALTER TABLE auth.users ADD COLUMN IF NOT EXISTS confirmed_at timestamptz
    GENERATED ALWAYS AS (LEAST (users.email_confirmed_at, users.phone_confirmed_at)) STORED;
ALTER TABLE auth.users ADD COLUMN IF NOT EXISTS email_change_token_current varchar(255) DEFAULT '';
ALTER TABLE auth.users ADD COLUMN IF NOT EXISTS email_change_confirm_status smallint DEFAULT 0;
ALTER TABLE auth.users ADD COLUMN IF NOT EXISTS banned_until timestamptz;
ALTER TABLE auth.users ADD COLUMN IF NOT EXISTS reauthentication_token varchar(255) DEFAULT '';
ALTER TABLE auth.users ADD COLUMN IF NOT EXISTS reauthentication_sent_at timestamptz;
ALTER TABLE auth.users ADD COLUMN IF NOT EXISTS deleted_at timestamptz;
ALTER TABLE auth.users ADD COLUMN IF NOT EXISTS is_sso_user boolean NOT NULL DEFAULT false;
ALTER TABLE auth.users ADD COLUMN IF NOT EXISTS is_anonymous boolean NOT NULL DEFAULT false;

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1
        FROM information_schema.columns
        WHERE table_schema = 'auth'
          AND table_name = 'users'
          AND column_name = 'confirmed_at'
    ) THEN
        ALTER TABLE auth.users
            ADD COLUMN confirmed_at timestamptz
            GENERATED ALWAYS AS (LEAST (users.email_confirmed_at, users.phone_confirmed_at)) STORED;
    END IF;
END $$;

DO $$
BEGIN
    ALTER TABLE auth.users
        ALTER COLUMN phone TYPE text,
        ALTER COLUMN phone_change TYPE text;
EXCEPTION
    WHEN SQLSTATE '0A000' THEN
        RAISE NOTICE 'Unable to change data type of phone, phone_change columns due to use by a view or rule';
    WHEN SQLSTATE '2BP01' THEN
        RAISE NOTICE 'Unable to change data type of phone, phone_change columns due to dependent objects';
    WHEN SQLSTATE 'XX000' THEN
        RAISE NOTICE 'Unable to change data type of phone, phone_change columns due to internal error (OrioleDB)';
END $$;

ALTER TABLE auth.users ALTER COLUMN id SET NOT NULL;
ALTER TABLE auth.users ALTER COLUMN is_sso_user SET NOT NULL;
ALTER TABLE auth.users ALTER COLUMN is_anonymous SET NOT NULL;

DO $$
BEGIN
    ALTER TABLE auth.users DROP CONSTRAINT IF EXISTS users_email_key;
EXCEPTION
    WHEN SQLSTATE '2BP01' THEN
        RAISE NOTICE 'Unable to drop users_email_key constraint due to dependent objects, please resolve this manually or SSO may not work';
END $$;

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1
        FROM pg_constraint
        WHERE conname = 'users_pkey'
          AND conrelid = 'auth.users'::regclass
    ) THEN
        ALTER TABLE auth.users ADD CONSTRAINT users_pkey PRIMARY KEY (id);
    END IF;
    IF NOT EXISTS (
        SELECT 1
        FROM pg_constraint
        WHERE conname = 'users_phone_key'
          AND conrelid = 'auth.users'::regclass
    ) THEN
        ALTER TABLE auth.users ADD CONSTRAINT users_phone_key UNIQUE (phone);
    END IF;
    IF NOT EXISTS (
        SELECT 1
        FROM pg_constraint
        WHERE conname = 'users_email_change_confirm_status_check'
          AND conrelid = 'auth.users'::regclass
    ) THEN
        ALTER TABLE auth.users
            ADD CONSTRAINT users_email_change_confirm_status_check
            CHECK (email_change_confirm_status >= 0 AND email_change_confirm_status <= 2);
    END IF;
END $$;

DO $$
BEGIN
    IF EXISTS (
        SELECT 1 FROM pg_indexes
        WHERE schemaname = 'auth'
          AND indexname = 'users_instance_id_email_idx'
          AND indexdef NOT LIKE '%lower(%'
    ) THEN
        DROP INDEX auth.users_instance_id_email_idx;
    END IF;
END $$;
CREATE INDEX IF NOT EXISTS users_instance_id_idx ON auth.users USING btree (instance_id);
CREATE INDEX IF NOT EXISTS users_instance_id_email_idx ON auth.users USING btree (instance_id, lower(email));
CREATE UNIQUE INDEX IF NOT EXISTS confirmation_token_idx ON auth.users USING btree (confirmation_token) WHERE confirmation_token !~ '^[0-9 ]*$';
CREATE UNIQUE INDEX IF NOT EXISTS recovery_token_idx ON auth.users USING btree (recovery_token) WHERE recovery_token !~ '^[0-9 ]*$';
CREATE UNIQUE INDEX IF NOT EXISTS email_change_token_current_idx ON auth.users USING btree (email_change_token_current) WHERE email_change_token_current !~ '^[0-9 ]*$';
CREATE UNIQUE INDEX IF NOT EXISTS email_change_token_new_idx ON auth.users USING btree (email_change_token_new) WHERE email_change_token_new !~ '^[0-9 ]*$';
CREATE UNIQUE INDEX IF NOT EXISTS reauthentication_token_idx ON auth.users USING btree (reauthentication_token) WHERE reauthentication_token !~ '^[0-9 ]*$';
CREATE UNIQUE INDEX IF NOT EXISTS users_email_partial_key ON auth.users (email) WHERE (is_sso_user = false);
CREATE INDEX IF NOT EXISTS users_is_anonymous_idx ON auth.users USING btree (is_anonymous);

COMMENT ON TABLE auth.users IS 'Auth: Stores user login data within a secure schema.';
COMMENT ON COLUMN auth.users.is_sso_user IS 'Auth: Set this column to true when the account comes from SSO. These accounts can have duplicate emails.';
COMMENT ON INDEX auth.users_email_partial_key IS 'Auth: A partial unique index that applies only when is_sso_user is false';

ALTER TABLE auth.users ENABLE ROW LEVEL SECURITY;

DO $$
BEGIN
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'postgres') THEN
        GRANT SELECT ON auth.users TO postgres WITH GRANT OPTION;
    END IF;
END $$;
