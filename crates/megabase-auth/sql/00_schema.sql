-- Ported from supabase/auth migrations (MIT), pin v2.197.0.
-- Schema plus parent keys for foreign keys in this issue. Full
-- auth.users / auth.sessions definitions are later units.

CREATE SCHEMA IF NOT EXISTS auth;
GRANT USAGE ON SCHEMA auth TO PUBLIC;

CREATE TABLE IF NOT EXISTS auth.users (
    id uuid NOT NULL,
    CONSTRAINT users_pkey PRIMARY KEY (id)
);

CREATE TABLE IF NOT EXISTS auth.sessions (
    id uuid NOT NULL,
    user_id uuid NOT NULL,
    CONSTRAINT sessions_pkey PRIMARY KEY (id),
    CONSTRAINT sessions_user_id_fkey FOREIGN KEY (user_id) REFERENCES auth.users(id) ON DELETE CASCADE
);
