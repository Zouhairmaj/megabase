-- Ported from supabase/auth (MIT), pin v2.197.0:
--   migrations/20221021082433_add_saml.up.sql
--   migrations/20221215195900_remove_sso_sessions.up.sql
-- Final shape: the table does not exist. Create then drop so a database that
-- never ran the add-SAML migration still matches the pinned reference.

CREATE TABLE IF NOT EXISTS auth.sso_sessions (
    id uuid NOT NULL,
    session_id uuid NOT NULL,
    sso_provider_id uuid NULL,
    not_before timestamptz NULL,
    not_after timestamptz NULL,
    idp_initiated boolean DEFAULT false,
    created_at timestamptz NULL,
    updated_at timestamptz NULL,
    PRIMARY KEY (id),
    FOREIGN KEY (session_id) REFERENCES auth.sessions (id) ON DELETE CASCADE,
    FOREIGN KEY (sso_provider_id) REFERENCES auth.sso_providers (id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS sso_sessions_session_id_idx ON auth.sso_sessions (session_id);
CREATE INDEX IF NOT EXISTS sso_sessions_sso_provider_id_idx ON auth.sso_sessions (sso_provider_id);

COMMENT ON TABLE auth.sso_sessions IS 'Auth: A session initiated by an SSO Identity Provider';

-- sso_sessions is not used as all of the necessary data is in sessions
DROP TABLE IF EXISTS auth.sso_sessions;
