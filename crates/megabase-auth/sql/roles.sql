-- Ported from supabase/storage migrations/tenant/0002-storage-schema.sql
-- (Apache-2.0), pin v1.80.2 (role creation, idempotent form), and from
-- supabase/supavisor dev/postgres/00-setup.sql (Apache-2.0), pin v2.9.13
-- (public schema usage and default privileges). Both files repeat the
-- role setup of the Supabase Postgres image, which is not vendored.
--
-- Admin roles: attributes from the Supabase Postgres image 17.6.1.136
-- (vendor/supabase/docker/docker-compose.yml, the supabase pin v1.26.08)
-- and from supabase/realtime
-- lib/realtime/tenants/repo/migrations/20240401105812_create_realtime_admin_and_move_ownership.ex
-- (Apache-2.0), pin v2.143.3 (`supabase_realtime_admin`). The image's own
-- init scripts are not in vendor/. These roles are created NOLOGIN:
-- Megabase does not connect as them, and a login role would need a
-- password this process does not set. Spec: specs/core/roles.md.

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
        EXCEPTION WHEN duplicate_object OR unique_violation THEN NULL;
        END;
    END IF;

    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'authenticated') THEN
        BEGIN
            CREATE ROLE authenticated NOLOGIN NOINHERIT;
        EXCEPTION WHEN duplicate_object OR unique_violation THEN NULL;
        END;
    END IF;

    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'service_role') THEN
        BEGIN
            CREATE ROLE service_role NOLOGIN NOINHERIT BYPASSRLS;
        EXCEPTION WHEN duplicate_object OR unique_violation THEN NULL;
        END;
    END IF;

    -- No password: vendor/supabase/docker/volumes/db/roles.sql sets it from
    -- POSTGRES_PASSWORD on the official stack. Megabase does not use this
    -- role to connect, so it does not set one.
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'authenticator') THEN
        BEGIN
            CREATE ROLE authenticator LOGIN NOINHERIT;
        EXCEPTION WHEN duplicate_object OR unique_violation THEN NULL;
        END;
    END IF;
END $$;

-- Admin roles dumps, policies and the realtime schema name. Cluster-wide.
-- Created only when missing, so a cluster the official image initialised
-- keeps its LOGIN bit and every other attribute. NOLOGIN here: nothing in
-- this process connects as these roles.
--
-- supabase_admin is superuser-like (the image grants superuser, createdb,
-- createrole, replication and bypassrls, and it inherits). supabase_auth_admin
-- and supabase_storage_admin
-- are CREATEROLE NOINHERIT NOREPLICATION, matching
-- `CREATE USER ... NOINHERIT CREATEROLE LOGIN NOREPLICATION` with LOGIN
-- replaced by NOLOGIN. supabase_realtime_admin is NOINHERIT NOLOGIN
-- NOREPLICATION, the Realtime tenant migration.
--
-- A connecting role that may create roles but may not grant an attribute
-- (a non-superuser cannot grant SUPERUSER) still gets the role. Each
-- attribute it cannot grant is skipped and reported with a WARNING, which
-- startup logs. Creating the role itself still requires CREATEROLE.
-- duplicate_object is the check-then-create race; unique_violation is the
-- same race when it hits pg_authid_rolname_index instead.
DO $$
DECLARE
    admin_role text;
    attr text;
    base_sql text;
    full_sql text;
    extras text[];
BEGIN
    FOREACH admin_role IN ARRAY ARRAY[
        'supabase_admin',
        'supabase_auth_admin',
        'supabase_storage_admin',
        'supabase_realtime_admin'
    ]
    LOOP
        IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = admin_role) THEN
            CONTINUE;
        END IF;

        IF admin_role = 'supabase_admin' THEN
            base_sql := format('CREATE ROLE %I NOLOGIN INHERIT', admin_role);
            full_sql := format(
                'CREATE ROLE %I NOLOGIN SUPERUSER CREATEDB CREATEROLE INHERIT REPLICATION BYPASSRLS',
                admin_role
            );
            extras := ARRAY['SUPERUSER', 'CREATEDB', 'CREATEROLE', 'REPLICATION', 'BYPASSRLS'];
        ELSIF admin_role IN ('supabase_auth_admin', 'supabase_storage_admin') THEN
            base_sql := format('CREATE ROLE %I NOLOGIN NOINHERIT NOREPLICATION', admin_role);
            full_sql := format(
                'CREATE ROLE %I NOLOGIN NOINHERIT CREATEROLE NOREPLICATION',
                admin_role
            );
            extras := ARRAY['CREATEROLE'];
        ELSE
            base_sql := format('CREATE ROLE %I NOLOGIN NOINHERIT NOREPLICATION', admin_role);
            full_sql := base_sql;
            extras := ARRAY[]::text[];
        END IF;

        BEGIN
            EXECUTE full_sql;
        EXCEPTION
            WHEN duplicate_object OR unique_violation THEN
                CONTINUE;
            WHEN insufficient_privilege THEN
                BEGIN
                    EXECUTE base_sql;
                EXCEPTION
                    WHEN duplicate_object OR unique_violation THEN
                        CONTINUE;
                END;
                FOREACH attr IN ARRAY extras LOOP
                    BEGIN
                        EXECUTE format('ALTER ROLE %I %s', admin_role, attr);
                    EXCEPTION WHEN insufficient_privilege THEN
                        RAISE WARNING 'megabase: connecting role cannot grant % to %; role created without it',
                            attr, admin_role;
                    END;
                END LOOP;
        END;
    END LOOP;
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
    SELECT rolsuper INTO connect_is_super FROM pg_roles WHERE rolname = connect_role;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'connecting role "%" has no pg_roles row', connect_role;
    END IF;

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
