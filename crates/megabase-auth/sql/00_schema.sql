-- Ported from supabase/auth migrations (MIT), pin v2.197.0.
-- Schema plus parent keys this issue's foreign keys need.
-- Full auth.flow_state / auth.oauth_clients definitions are other units;
-- do not mark them implemented here.

CREATE SCHEMA IF NOT EXISTS auth;
GRANT USAGE ON SCHEMA auth TO PUBLIC;

DO $$ BEGIN
    CREATE TYPE auth.aal_level AS ENUM ('aal1', 'aal2', 'aal3');
EXCEPTION
    WHEN duplicate_object THEN NULL;
END $$;

CREATE TABLE IF NOT EXISTS auth.flow_state (
    id uuid NOT NULL,
    CONSTRAINT flow_state_pkey PRIMARY KEY (id)
);

CREATE TABLE IF NOT EXISTS auth.oauth_clients (
    id uuid NOT NULL,
    CONSTRAINT oauth_clients_pkey PRIMARY KEY (id)
);
