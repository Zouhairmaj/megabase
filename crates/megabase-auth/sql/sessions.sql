-- Ported from supabase/auth (MIT), pin v2.197.0:
--   migrations/20220811173540_add_sessions_table.up.sql
--   migrations/20221003041400_add_aal_and_factor_id_to_sessions.up.sql
--   migrations/20221020193600_add_sessions_user_id_index.up.sql
--   migrations/20221011041400_add_mfa_indexes.up.sql
--   migrations/20221114143122_add_session_not_after_column.up.sql
--   migrations/20230508135423_add_cleanup_indexes.up.sql
--   migrations/20231027141322_add_session_refresh_columns.up.sql
--   migrations/20231114161723_add_sessions_tag.up.sql
--   migrations/20240612123726_enable_rls_update_grants.up.sql
--   migrations/20250904133000_add_oauth_client_id_to_session.up.sql
--   migrations/20251007112900_add_session_refresh_token_columns.up.sql
--   migrations/20251111201300_add_scopes_to_sessions.up.sql

-- Pin attnum order is refresh_token_* then scopes. An empty table created
-- with scopes first is dropped so CREATE can rebuild it; inbound FKs are
-- dropped here and re-added later. ACCESS EXCLUSIVE then EXISTS so a
-- concurrent insert cannot land between the emptiness check and DROP.
DO $$
DECLARE
    hmac_att smallint;
    scopes_att smallint;
    has_rows boolean;
BEGIN
    IF to_regclass('auth.sessions') IS NULL THEN
        RETURN;
    END IF;
    LOCK TABLE auth.sessions IN ACCESS EXCLUSIVE MODE;
    SELECT EXISTS (SELECT 1 FROM auth.sessions) INTO has_rows;
    SELECT a.attnum INTO hmac_att
      FROM pg_attribute a
      JOIN pg_class c ON c.oid = a.attrelid
      JOIN pg_namespace ns ON ns.oid = c.relnamespace
     WHERE ns.nspname = 'auth' AND c.relname = 'sessions'
       AND a.attname = 'refresh_token_hmac_key'
       AND NOT a.attisdropped AND a.attnum > 0;
    SELECT a.attnum INTO scopes_att
      FROM pg_attribute a
      JOIN pg_class c ON c.oid = a.attrelid
      JOIN pg_namespace ns ON ns.oid = c.relnamespace
     WHERE ns.nspname = 'auth' AND c.relname = 'sessions'
       AND a.attname = 'scopes' AND NOT a.attisdropped AND a.attnum > 0;
    IF NOT has_rows AND hmac_att IS NOT NULL AND scopes_att IS NOT NULL
       AND scopes_att < hmac_att THEN
        ALTER TABLE IF EXISTS auth.refresh_tokens
            DROP CONSTRAINT IF EXISTS refresh_tokens_session_id_fkey;
        ALTER TABLE IF EXISTS auth.mfa_amr_claims
            DROP CONSTRAINT IF EXISTS mfa_amr_claims_session_id_fkey;
        DROP TABLE auth.sessions;
    END IF;
END $$;

CREATE TABLE IF NOT EXISTS auth.sessions (
    id uuid NOT NULL,
    user_id uuid NOT NULL,
    created_at timestamptz NULL,
    updated_at timestamptz NULL,
    factor_id uuid NULL,
    aal auth.aal_level NULL,
    not_after timestamptz NULL,
    refreshed_at timestamp WITHOUT TIME ZONE NULL,
    user_agent text NULL,
    ip inet NULL,
    tag text NULL,
    oauth_client_id uuid NULL,
    refresh_token_hmac_key text NULL,
    refresh_token_counter bigint NULL,
    scopes text NULL,
    CONSTRAINT sessions_pkey PRIMARY KEY (id),
    CONSTRAINT sessions_user_id_fkey FOREIGN KEY (user_id) REFERENCES auth.users(id) ON DELETE CASCADE,
    CONSTRAINT sessions_oauth_client_id_fkey FOREIGN KEY (oauth_client_id) REFERENCES auth.oauth_clients(id) ON DELETE CASCADE,
    CONSTRAINT sessions_scopes_length CHECK (char_length(scopes) <= 4096)
);

ALTER TABLE auth.sessions ADD COLUMN IF NOT EXISTS user_id uuid;
ALTER TABLE auth.sessions ADD COLUMN IF NOT EXISTS created_at timestamptz;
ALTER TABLE auth.sessions ADD COLUMN IF NOT EXISTS updated_at timestamptz;
ALTER TABLE auth.sessions ADD COLUMN IF NOT EXISTS factor_id uuid;
ALTER TABLE auth.sessions ADD COLUMN IF NOT EXISTS aal auth.aal_level;
ALTER TABLE auth.sessions ADD COLUMN IF NOT EXISTS not_after timestamptz;
ALTER TABLE auth.sessions ADD COLUMN IF NOT EXISTS refreshed_at timestamp WITHOUT TIME ZONE;
ALTER TABLE auth.sessions ADD COLUMN IF NOT EXISTS user_agent text;
ALTER TABLE auth.sessions ADD COLUMN IF NOT EXISTS ip inet;
ALTER TABLE auth.sessions ADD COLUMN IF NOT EXISTS tag text;
ALTER TABLE auth.sessions ADD COLUMN IF NOT EXISTS oauth_client_id uuid;
ALTER TABLE auth.sessions ADD COLUMN IF NOT EXISTS refresh_token_hmac_key text;
ALTER TABLE auth.sessions ADD COLUMN IF NOT EXISTS refresh_token_counter bigint;
ALTER TABLE auth.sessions ADD COLUMN IF NOT EXISTS scopes text;

ALTER TABLE auth.sessions ALTER COLUMN id SET NOT NULL;
ALTER TABLE auth.sessions ALTER COLUMN user_id SET NOT NULL;

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1
        FROM pg_constraint
        WHERE conname = 'sessions_pkey'
          AND conrelid = 'auth.sessions'::regclass
    ) THEN
        ALTER TABLE auth.sessions ADD CONSTRAINT sessions_pkey PRIMARY KEY (id);
    END IF;
    IF NOT EXISTS (
        SELECT 1
        FROM pg_constraint
        WHERE conname = 'sessions_user_id_fkey'
          AND conrelid = 'auth.sessions'::regclass
    ) THEN
        ALTER TABLE auth.sessions
            ADD CONSTRAINT sessions_user_id_fkey
            FOREIGN KEY (user_id) REFERENCES auth.users(id) ON DELETE CASCADE;
    END IF;
    IF NOT EXISTS (
        SELECT 1
        FROM pg_constraint
        WHERE conname = 'sessions_oauth_client_id_fkey'
          AND conrelid = 'auth.sessions'::regclass
    ) THEN
        ALTER TABLE auth.sessions
            ADD CONSTRAINT sessions_oauth_client_id_fkey
            FOREIGN KEY (oauth_client_id) REFERENCES auth.oauth_clients(id) ON DELETE CASCADE NOT VALID;
        ALTER TABLE auth.sessions VALIDATE CONSTRAINT sessions_oauth_client_id_fkey;
    END IF;
    IF NOT EXISTS (
        SELECT 1
        FROM pg_constraint
        WHERE conname = 'sessions_scopes_length'
          AND conrelid = 'auth.sessions'::regclass
    ) THEN
        ALTER TABLE auth.sessions
            ADD CONSTRAINT sessions_scopes_length CHECK (char_length(scopes) <= 4096);
    END IF;
END $$;

DO $$
BEGIN
    IF to_regclass('auth.refresh_tokens') IS NOT NULL
       AND NOT EXISTS (
           SELECT 1
           FROM pg_constraint
           WHERE conname = 'refresh_tokens_session_id_fkey'
             AND conrelid = to_regclass('auth.refresh_tokens')
       ) THEN
        ALTER TABLE auth.refresh_tokens
            ADD CONSTRAINT refresh_tokens_session_id_fkey
            FOREIGN KEY (session_id) REFERENCES auth.sessions(id) ON DELETE CASCADE;
    END IF;
    IF to_regclass('auth.mfa_amr_claims') IS NOT NULL
       AND NOT EXISTS (
           SELECT 1
           FROM pg_constraint
           WHERE conname = 'mfa_amr_claims_session_id_fkey'
             AND conrelid = to_regclass('auth.mfa_amr_claims')
       ) THEN
        ALTER TABLE auth.mfa_amr_claims
            ADD CONSTRAINT mfa_amr_claims_session_id_fkey
            FOREIGN KEY (session_id) REFERENCES auth.sessions(id) ON DELETE CASCADE;
    END IF;
END $$;

CREATE INDEX IF NOT EXISTS sessions_user_id_idx ON auth.sessions (user_id);
CREATE INDEX IF NOT EXISTS user_id_created_at_idx ON auth.sessions (user_id, created_at);
CREATE INDEX IF NOT EXISTS sessions_not_after_idx ON auth.sessions (not_after DESC);
CREATE INDEX IF NOT EXISTS sessions_oauth_client_id_idx ON auth.sessions (oauth_client_id);

COMMENT ON TABLE auth.sessions IS 'Auth: Stores session data associated to a user.';
COMMENT ON COLUMN auth.sessions.not_after IS 'Auth: Not after is a nullable column that contains a timestamp after which the session should be regarded as expired.';
COMMENT ON COLUMN auth.sessions.refresh_token_hmac_key IS 'Holds a HMAC-SHA256 key used to sign refresh tokens for this session.';
COMMENT ON COLUMN auth.sessions.refresh_token_counter IS 'Holds the ID (counter) of the last issued refresh token.';

ALTER TABLE auth.sessions ENABLE ROW LEVEL SECURITY;

DO $$
BEGIN
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'postgres') THEN
        GRANT SELECT ON auth.sessions TO postgres WITH GRANT OPTION;
    END IF;
END $$;
