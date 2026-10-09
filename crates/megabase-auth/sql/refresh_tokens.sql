-- Ported from supabase/auth (MIT), pin v2.197.0:
--   migrations/00_init_auth_schema.up.sql
--   migrations/20210927181326_add_refresh_token_parent.up.sql
--   migrations/20220811173540_add_sessions_table.up.sql
--   migrations/20221021073300_add_refresh_tokens_session_id_revoked_index.up.sql
--   migrations/20221114143410_remove_parent_foreign_key_refresh_tokens.up.sql
--   migrations/20230411005111_remove_duplicate_idx.up.sql
--   migrations/20230508135423_add_cleanup_indexes.up.sql
--   migrations/20240612123726_enable_rls_update_grants.up.sql
-- parent column stays; parent_fkey and refresh_tokens_token_idx do not.

CREATE TABLE IF NOT EXISTS auth.refresh_tokens (
    instance_id uuid NULL,
    id bigserial NOT NULL,
    token varchar(255) NULL,
    user_id varchar(255) NULL,
    revoked bool NULL,
    created_at timestamptz NULL,
    updated_at timestamptz NULL,
    parent varchar(255) NULL,
    session_id uuid NULL,
    CONSTRAINT refresh_tokens_pkey PRIMARY KEY (id),
    CONSTRAINT refresh_tokens_token_unique UNIQUE (token),
    CONSTRAINT refresh_tokens_session_id_fkey
        FOREIGN KEY (session_id) REFERENCES auth.sessions(id) ON DELETE CASCADE
);

ALTER TABLE auth.refresh_tokens ADD COLUMN IF NOT EXISTS instance_id uuid;
ALTER TABLE auth.refresh_tokens ADD COLUMN IF NOT EXISTS token varchar(255);
ALTER TABLE auth.refresh_tokens ADD COLUMN IF NOT EXISTS user_id varchar(255);
ALTER TABLE auth.refresh_tokens ADD COLUMN IF NOT EXISTS revoked bool;
ALTER TABLE auth.refresh_tokens ADD COLUMN IF NOT EXISTS created_at timestamptz;
ALTER TABLE auth.refresh_tokens ADD COLUMN IF NOT EXISTS updated_at timestamptz;
ALTER TABLE auth.refresh_tokens ADD COLUMN IF NOT EXISTS parent varchar(255);
ALTER TABLE auth.refresh_tokens ADD COLUMN IF NOT EXISTS session_id uuid;

ALTER TABLE auth.refresh_tokens DROP CONSTRAINT IF EXISTS refresh_tokens_parent_fkey;
DROP INDEX IF EXISTS auth.refresh_tokens_token_idx;

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'refresh_tokens_pkey'
          AND conrelid = 'auth.refresh_tokens'::regclass
    ) THEN
        ALTER TABLE auth.refresh_tokens ADD CONSTRAINT refresh_tokens_pkey PRIMARY KEY (id);
    END IF;
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'refresh_tokens_token_unique'
          AND conrelid = 'auth.refresh_tokens'::regclass
    ) THEN
        ALTER TABLE auth.refresh_tokens
            ADD CONSTRAINT refresh_tokens_token_unique UNIQUE (token);
    END IF;
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'refresh_tokens_session_id_fkey'
          AND conrelid = 'auth.refresh_tokens'::regclass
    ) THEN
        ALTER TABLE auth.refresh_tokens
            ADD CONSTRAINT refresh_tokens_session_id_fkey
            FOREIGN KEY (session_id) REFERENCES auth.sessions(id) ON DELETE CASCADE;
    END IF;
END $$;

CREATE INDEX IF NOT EXISTS refresh_tokens_instance_id_idx
    ON auth.refresh_tokens USING btree (instance_id);
CREATE INDEX IF NOT EXISTS refresh_tokens_instance_id_user_id_idx
    ON auth.refresh_tokens USING btree (instance_id, user_id);
CREATE INDEX IF NOT EXISTS refresh_tokens_parent_idx
    ON auth.refresh_tokens USING btree (parent);
CREATE INDEX IF NOT EXISTS refresh_tokens_session_id_revoked_idx
    ON auth.refresh_tokens (session_id, revoked);
CREATE INDEX IF NOT EXISTS refresh_tokens_updated_at_idx
    ON auth.refresh_tokens (updated_at DESC);

COMMENT ON TABLE auth.refresh_tokens IS
    'Auth: Store of tokens used to refresh JWT tokens once they expire.';

ALTER TABLE auth.refresh_tokens ENABLE ROW LEVEL SECURITY;

DO $$
BEGIN
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'postgres') THEN
        GRANT SELECT ON auth.refresh_tokens TO postgres WITH GRANT OPTION;
    END IF;
END $$;
