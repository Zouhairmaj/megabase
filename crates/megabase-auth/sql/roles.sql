-- Ported from supabase/storage migrations/tenant/0002-storage-schema.sql
-- (Apache-2.0), pin v1.80.2 (role creation, idempotent form), and from
-- supabase/supavisor dev/postgres/00-setup.sql (Apache-2.0), pin v2.9.13
-- (public schema usage and default privileges). Both files repeat the
-- role setup of the Supabase Postgres image, which is not vendored.
-- Spec: specs/core/roles.md.

BEGIN;
SELECT pg_advisory_xact_lock(hashtext('megabase.core.install_roles'));

-- Roles are cluster-wide. Create each one only when it is missing so a
-- database the official stack already initialised is left as it is.
-- duplicate_object covers a second Megabase on another database of the
-- same cluster creating the role between the check and the CREATE.
DO $$
BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'anon') THEN
        BEGIN
            CREATE ROLE anon NOLOGIN NOINHERIT;
        EXCEPTION WHEN duplicate_object THEN NULL;
        END;
    END IF;

    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'authenticated') THEN
        BEGIN
            CREATE ROLE authenticated NOLOGIN NOINHERIT;
        EXCEPTION WHEN duplicate_object THEN NULL;
        END;
    END IF;

    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'service_role') THEN
        BEGIN
            CREATE ROLE service_role NOLOGIN NOINHERIT BYPASSRLS;
        EXCEPTION WHEN duplicate_object THEN NULL;
        END;
    END IF;

    -- No password: vendor/supabase/docker/volumes/db/roles.sql sets it from
    -- POSTGRES_PASSWORD on the official stack. Megabase does not use this
    -- role to connect, so it does not set one.
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'authenticator') THEN
        BEGIN
            CREATE ROLE authenticator LOGIN NOINHERIT;
        EXCEPTION WHEN duplicate_object THEN NULL;
        END;
    END IF;
END $$;

-- authenticator switches into the three API roles (PostgREST). The role
-- Megabase connects as does the same with set_config('role', ...), so it
-- needs the same memberships unless it is a superuser. Each GRANT is
-- skipped when the membership already exists so a cluster the official
-- stack initialised needs no privilege Megabase may lack.
DO $$
DECLARE
    api_role text;
    connect_role text := current_user;
    connect_is_super boolean;
BEGIN
    SELECT rolsuper INTO STRICT connect_is_super FROM pg_roles WHERE rolname = connect_role;

    FOREACH api_role IN ARRAY ARRAY['anon', 'authenticated', 'service_role'] LOOP
        IF NOT pg_has_role('authenticator', api_role, 'MEMBER') THEN
            EXECUTE format('GRANT %I TO authenticator', api_role);
        END IF;

        IF NOT connect_is_super
           AND NOT pg_has_role(connect_role, api_role, 'MEMBER') THEN
            EXECUTE format('GRANT %I TO %I', api_role, connect_role);
        END IF;
    END LOOP;
END $$;

-- Per-database: API roles may use public, and objects the connecting role
-- creates there later are granted to them, as on the official stack. The
-- postgres role is included when the cluster has one (the official image
-- always does; a stock cluster initialised under another name may not).
-- A database whose public schema was dropped is left alone.
DO $$
DECLARE
    grantees text := 'anon, authenticated, service_role';
BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_namespace WHERE nspname = 'public') THEN
        RETURN;
    END IF;

    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'postgres') THEN
        grantees := 'postgres, ' || grantees;
    END IF;

    EXECUTE 'GRANT USAGE ON SCHEMA public TO ' || grantees;
    EXECUTE 'ALTER DEFAULT PRIVILEGES IN SCHEMA public GRANT ALL ON TABLES TO ' || grantees;
    EXECUTE 'ALTER DEFAULT PRIVILEGES IN SCHEMA public GRANT ALL ON FUNCTIONS TO ' || grantees;
    EXECUTE 'ALTER DEFAULT PRIVILEGES IN SCHEMA public GRANT ALL ON SEQUENCES TO ' || grantees;
END $$;

COMMIT;
