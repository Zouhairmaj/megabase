-- Ported from supabase/supabase docker/volumes/db/webhooks.sql (Apache-2.0),
-- pin v1.26.08 (supabase_functions schema, hooks, http_request; it also
-- installs pg_net into the pre-existing extensions schema), and from
-- supabase/supabase docker/dev/data.sql (Apache-2.0), pin v1.26.08 (empty
-- supabase_realtime publication). The Supabase Postgres image's own init
-- scripts, which create extensions and the publication, are not vendored.
-- Spec: specs/core/platform-schemas.md.

BEGIN;
SELECT pg_advisory_xact_lock(hashtext('megabase.core.install_platform'));

-- extensions: where Supabase installs extensions, on the default
-- search_path. Usage is granted to the API roles so extension functions can
-- be called through the API.
CREATE SCHEMA IF NOT EXISTS extensions;

DO $$
DECLARE
    grantees text := 'anon, authenticated, service_role';
BEGIN
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'postgres') THEN
        grantees := 'postgres, ' || grantees;
    END IF;
    EXECUTE 'GRANT USAGE ON SCHEMA extensions TO ' || grantees;
END $$;

-- Extensions a fresh Supabase project has in the extensions schema. Each is
-- created only when the server ships it and it is not installed yet (an
-- extension installed in another schema is not moved). A failure, such as a
-- non-superuser creating an untrusted extension, is reported as a WARNING
-- and leaves the rest of the install in place. pg_net and supabase_vault
-- need the Supabase image (shared_preload_libraries); on a stock server they
-- are not available and are skipped (the vault schema is still created).
DO $$
DECLARE
    ext text;
BEGIN
    FOREACH ext IN ARRAY ARRAY['uuid-ossp', 'pgcrypto', 'pg_stat_statements', 'pg_net'] LOOP
        IF EXISTS (SELECT 1 FROM pg_available_extensions WHERE name = ext)
           AND NOT EXISTS (SELECT 1 FROM pg_extension WHERE extname = ext) THEN
            BEGIN
                EXECUTE format('CREATE EXTENSION %I WITH SCHEMA extensions', ext);
            EXCEPTION WHEN OTHERS THEN
                RAISE WARNING 'megabase: extension % not created: %', ext, SQLERRM;
            END;
        END IF;
    END LOOP;

    -- The vault schema exists even without supabase_vault, so dumps that
    -- name it (GRANT ... ON SCHEMA vault, COMMENT ON SCHEMA vault) restore.
    -- No stand-in vault.secrets or vault.decrypted_secrets is created: a
    -- plain table would keep secrets unencrypted, and existing objects would
    -- stop CREATE EXTENSION supabase_vault on a later server that ships it.
    CREATE SCHEMA IF NOT EXISTS vault;

    -- The extension creates vault.secrets and vault.decrypted_secrets.
    IF EXISTS (SELECT 1 FROM pg_available_extensions WHERE name = 'supabase_vault')
       AND NOT EXISTS (SELECT 1 FROM pg_extension WHERE extname = 'supabase_vault') THEN
        BEGIN
            CREATE EXTENSION supabase_vault WITH SCHEMA vault;
        EXCEPTION WHEN OTHERS THEN
            RAISE WARNING 'megabase: extension supabase_vault not created: %', SQLERRM;
        END;
    END IF;
END $$;

-- New sessions resolve unqualified extension objects (uuid_generate_v4(),
-- gen_random_uuid() from pgcrypto) as on Supabase. Set only when the
-- database has no search_path of its own, so an operator's choice stays.
DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1
        FROM pg_db_role_setting s
        JOIN pg_database d ON d.oid = s.setdatabase
        WHERE d.datname = current_database()
          AND s.setrole = 0
          AND EXISTS (SELECT 1 FROM unnest(s.setconfig) c WHERE c LIKE 'search_path=%')
    ) THEN
        EXECUTE format(
            'ALTER DATABASE %I SET search_path TO "$user", public, extensions',
            current_database()
        );
    END IF;
END $$;

-- supabase_functions: database webhooks. Created as a unit only when the
-- schema is missing, so a database the official stack initialised is left
-- as it is.
DO $outer$
DECLARE
    grantees text := 'anon, authenticated, service_role';
BEGIN
    IF EXISTS (SELECT 1 FROM pg_namespace WHERE nspname = 'supabase_functions') THEN
        RETURN;
    END IF;

    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'postgres') THEN
        grantees := 'postgres, ' || grantees;
    END IF;

    CREATE SCHEMA supabase_functions;
    EXECUTE 'GRANT USAGE ON SCHEMA supabase_functions TO ' || grantees;
    EXECUTE 'ALTER DEFAULT PRIVILEGES IN SCHEMA supabase_functions GRANT ALL ON TABLES TO ' || grantees;
    EXECUTE 'ALTER DEFAULT PRIVILEGES IN SCHEMA supabase_functions GRANT ALL ON FUNCTIONS TO ' || grantees;
    EXECUTE 'ALTER DEFAULT PRIVILEGES IN SCHEMA supabase_functions GRANT ALL ON SEQUENCES TO ' || grantees;

    CREATE TABLE supabase_functions.migrations (
        version text PRIMARY KEY,
        inserted_at timestamptz NOT NULL DEFAULT NOW()
    );
    INSERT INTO supabase_functions.migrations (version) VALUES ('initial');

    CREATE TABLE supabase_functions.hooks (
        id bigserial PRIMARY KEY,
        hook_table_id integer NOT NULL,
        hook_name text NOT NULL,
        created_at timestamptz NOT NULL DEFAULT NOW(),
        request_id bigint
    );
    CREATE INDEX supabase_functions_hooks_request_id_idx ON supabase_functions.hooks USING btree (request_id);
    CREATE INDEX supabase_functions_hooks_h_table_id_h_name_idx ON supabase_functions.hooks USING btree (hook_table_id, hook_name);
    COMMENT ON TABLE supabase_functions.hooks IS 'Supabase Functions Hooks: Audit trail for triggered hooks.';

    -- The body calls net.http_get / net.http_post (pg_net). PL/pgSQL
    -- resolves them when the trigger fires, so the function and triggers
    -- that use it are created without pg_net; firing one without pg_net
    -- fails with "schema net does not exist".
    CREATE FUNCTION supabase_functions.http_request()
      RETURNS trigger
      LANGUAGE plpgsql
      AS $function$
      DECLARE
        request_id bigint;
        payload jsonb;
        url text := TG_ARGV[0]::text;
        method text := TG_ARGV[1]::text;
        headers jsonb DEFAULT '{}'::jsonb;
        params jsonb DEFAULT '{}'::jsonb;
        timeout_ms integer DEFAULT 1000;
      BEGIN
        IF url IS NULL OR url = 'null' THEN
          RAISE EXCEPTION 'url argument is missing';
        END IF;

        IF method IS NULL OR method = 'null' THEN
          RAISE EXCEPTION 'method argument is missing';
        END IF;

        IF TG_ARGV[2] IS NULL OR TG_ARGV[2] = 'null' THEN
          headers = '{"Content-Type": "application/json"}'::jsonb;
        ELSE
          headers = TG_ARGV[2]::jsonb;
        END IF;

        IF TG_ARGV[3] IS NULL OR TG_ARGV[3] = 'null' THEN
          params = '{}'::jsonb;
        ELSE
          params = TG_ARGV[3]::jsonb;
        END IF;

        IF TG_ARGV[4] IS NULL OR TG_ARGV[4] = 'null' THEN
          timeout_ms = 1000;
        ELSE
          timeout_ms = TG_ARGV[4]::integer;
        END IF;

        CASE
          WHEN method = 'GET' THEN
            SELECT http_get INTO request_id FROM net.http_get(
              url,
              params,
              headers,
              timeout_ms
            );
          WHEN method = 'POST' THEN
            payload = jsonb_build_object(
              'old_record', OLD,
              'record', NEW,
              'type', TG_OP,
              'table', TG_TABLE_NAME,
              'schema', TG_TABLE_SCHEMA
            );

            SELECT http_post INTO request_id FROM net.http_post(
              url,
              payload,
              params,
              headers,
              timeout_ms
            );
          ELSE
            RAISE EXCEPTION 'method argument % is invalid', method;
        END CASE;

        INSERT INTO supabase_functions.hooks
          (hook_table_id, hook_name, request_id)
        VALUES
          (TG_RELID, TG_NAME, request_id);

        RETURN NEW;
      END
    $function$;

    INSERT INTO supabase_functions.migrations (version) VALUES ('20210809183423_update_grants');
    ALTER FUNCTION supabase_functions.http_request() SECURITY DEFINER;
    ALTER FUNCTION supabase_functions.http_request() SET search_path = supabase_functions;
    REVOKE ALL ON FUNCTION supabase_functions.http_request() FROM PUBLIC;
    EXECUTE 'GRANT EXECUTE ON FUNCTION supabase_functions.http_request() TO ' || grantees;
END $outer$;

-- supabase_realtime: the publication Realtime streams from. Empty, so
-- `alter publication supabase_realtime add table ...` works, and dumps that
-- `drop publication if exists` and recreate it still do.
DO $$
BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_publication WHERE pubname = 'supabase_realtime') THEN
        CREATE PUBLICATION supabase_realtime;
    END IF;
END $$;

COMMIT;
