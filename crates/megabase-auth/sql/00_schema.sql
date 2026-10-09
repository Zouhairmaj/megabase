-- Ported from supabase/auth migrations (MIT), pin v2.197.0.
-- Schema plus parent stubs. Full auth.users / auth.sessions column
-- lists are later units; do not mark the stubs as those units.

CREATE SCHEMA IF NOT EXISTS auth;
GRANT USAGE ON SCHEMA auth TO PUBLIC;

DO $$ BEGIN
    CREATE TYPE auth.aal_level AS ENUM ('aal1', 'aal2', 'aal3');
EXCEPTION
    WHEN duplicate_object THEN NULL;
END $$;

-- Pin attnum 1 is instance_id (00_init_auth_schema.up.sql). CREATE TABLE
-- IF NOT EXISTS cannot reorder an existing table; later ADD COLUMN
-- appends. Keep this stub to the FK key plus that leading column.
CREATE TABLE IF NOT EXISTS auth.users (
    instance_id uuid NULL,
    id uuid NOT NULL,
    CONSTRAINT users_pkey PRIMARY KEY (id)
);

CREATE TABLE IF NOT EXISTS auth.sessions (
    id uuid NOT NULL,
    user_id uuid NOT NULL,
    CONSTRAINT sessions_pkey PRIMARY KEY (id),
    CONSTRAINT sessions_user_id_fkey FOREIGN KEY (user_id) REFERENCES auth.users(id) ON DELETE CASCADE
);
