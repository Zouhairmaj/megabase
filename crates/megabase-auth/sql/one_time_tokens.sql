-- Ported from supabase/auth (MIT), pin v2.197.0:
--   migrations/20240427152123_add_one_time_tokens_table.up.sql
--   migrations/20260831180000_add_expires_at_to_one_time_tokens.up.sql
--   migrations/20240612123726_enable_rls_update_grants.up.sql

DO $$ BEGIN
    CREATE TYPE auth.one_time_token_type AS ENUM (
        'confirmation_token',
        'reauthentication_token',
        'recovery_token',
        'email_change_token_new',
        'email_change_token_current',
        'phone_change_token'
    );
EXCEPTION
    WHEN duplicate_object THEN NULL;
END $$;

DO $$ BEGIN
    CREATE TABLE IF NOT EXISTS auth.one_time_tokens (
        id uuid PRIMARY KEY,
        user_id uuid NOT NULL REFERENCES auth.users ON DELETE CASCADE,
        token_type auth.one_time_token_type NOT NULL,
        token_hash text NOT NULL,
        relates_to text NOT NULL,
        created_at timestamp WITHOUT TIME ZONE NOT NULL DEFAULT now(),
        updated_at timestamp WITHOUT TIME ZONE NOT NULL DEFAULT now(),
        expires_at timestamptz,
        CHECK (char_length(token_hash) > 0)
    );

    BEGIN
        CREATE INDEX IF NOT EXISTS one_time_tokens_token_hash_hash_idx
            ON auth.one_time_tokens USING hash (token_hash);
        CREATE INDEX IF NOT EXISTS one_time_tokens_relates_to_hash_idx
            ON auth.one_time_tokens USING hash (relates_to);
    EXCEPTION
        WHEN OTHERS THEN
            CREATE INDEX IF NOT EXISTS one_time_tokens_token_hash_hash_idx
                ON auth.one_time_tokens USING btree (token_hash);
            CREATE INDEX IF NOT EXISTS one_time_tokens_relates_to_hash_idx
                ON auth.one_time_tokens USING btree (relates_to);
    END;

    CREATE UNIQUE INDEX IF NOT EXISTS one_time_tokens_user_id_token_type_key
        ON auth.one_time_tokens (user_id, token_type);
END $$;

ALTER TABLE auth.one_time_tokens ADD COLUMN IF NOT EXISTS user_id uuid;
ALTER TABLE auth.one_time_tokens ADD COLUMN IF NOT EXISTS token_type auth.one_time_token_type;
ALTER TABLE auth.one_time_tokens ADD COLUMN IF NOT EXISTS token_hash text;
ALTER TABLE auth.one_time_tokens ADD COLUMN IF NOT EXISTS relates_to text;
ALTER TABLE auth.one_time_tokens ADD COLUMN IF NOT EXISTS created_at timestamp WITHOUT TIME ZONE DEFAULT now();
ALTER TABLE auth.one_time_tokens ADD COLUMN IF NOT EXISTS updated_at timestamp WITHOUT TIME ZONE DEFAULT now();
ALTER TABLE auth.one_time_tokens ADD COLUMN IF NOT EXISTS expires_at timestamptz;

ALTER TABLE auth.one_time_tokens ALTER COLUMN user_id SET NOT NULL;
ALTER TABLE auth.one_time_tokens ALTER COLUMN token_type SET NOT NULL;
ALTER TABLE auth.one_time_tokens ALTER COLUMN token_hash SET NOT NULL;
ALTER TABLE auth.one_time_tokens ALTER COLUMN relates_to SET NOT NULL;
ALTER TABLE auth.one_time_tokens ALTER COLUMN created_at SET NOT NULL;
ALTER TABLE auth.one_time_tokens ALTER COLUMN updated_at SET NOT NULL;

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'one_time_tokens_user_id_fkey'
          AND conrelid = 'auth.one_time_tokens'::regclass
    ) THEN
        ALTER TABLE auth.one_time_tokens
            ADD CONSTRAINT one_time_tokens_user_id_fkey
            FOREIGN KEY (user_id) REFERENCES auth.users ON DELETE CASCADE;
    END IF;
END $$;

ALTER TABLE auth.one_time_tokens ENABLE ROW LEVEL SECURITY;

DO $$
BEGIN
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'postgres') THEN
        GRANT SELECT ON auth.one_time_tokens TO postgres WITH GRANT OPTION;
    END IF;
END $$;
