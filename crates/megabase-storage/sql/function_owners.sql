-- Ported from supabase/storage migrations/tenant/0002-storage-schema.sql:157
-- (Apache-2.0), pin v1.80.2, for the functions and tables this install adds.
--
-- On the reference stack the migration role owns these objects. That role
-- is `supabase_storage_admin` unless `storage.super_user` says otherwise.
-- Creating the role is issue #493; until the role exists the connecting
-- role keeps ownership and the install still succeeds.

DO $$
DECLARE
    super_user text = COALESCE(current_setting('storage.super_user', true), 'supabase_storage_admin');
    connect_is_super boolean;
BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = super_user) THEN
        RETURN;
    END IF;

    -- Transfer only when this role can both give the objects away and
    -- CREATE OR REPLACE them on the next startup. Membership is not
    -- enough: after ALTER OWNER, only a superuser or the owner role
    -- itself can replace the function. Issue #493 creates the role.
    SELECT rolsuper INTO connect_is_super FROM pg_roles WHERE rolname = current_user;
    IF NOT coalesce(connect_is_super, false)
       AND current_user IS DISTINCT FROM super_user THEN
        RETURN;
    END IF;

    BEGIN
        EXECUTE format('ALTER FUNCTION storage.operation() OWNER TO %I', super_user);
        EXECUTE format('ALTER FUNCTION storage.can_insert_object(text, text, uuid, jsonb) OWNER TO %I', super_user);
        EXECUTE format('ALTER FUNCTION storage.get_size_by_bucket(text, text) OWNER TO %I', super_user);
        EXECUTE format('ALTER FUNCTION storage.protect_bucket_control_columns() OWNER TO %I', super_user);
        EXECUTE format('ALTER FUNCTION storage.list_multipart_uploads_with_delimiter(text, text, text, integer, text, text, text) OWNER TO %I', super_user);
        EXECUTE format('ALTER FUNCTION storage.list_objects_with_delimiter(text, text, text, integer, text, text, text, text, text, timestamptz, text) OWNER TO %I', super_user);
        EXECUTE format('ALTER FUNCTION storage.search_by_timestamp(text, text, integer, integer, text, text, text, text, text, text, text) OWNER TO %I', super_user);
        EXECUTE format('ALTER FUNCTION storage.search_v2(text, text, integer, integer, text, text, text, text, text, text, timestamptz, text, boolean) OWNER TO %I', super_user);
        EXECUTE format('ALTER TABLE storage.s3_multipart_uploads OWNER TO %I', super_user);
        EXECUTE format('ALTER TABLE storage.s3_multipart_uploads_parts OWNER TO %I', super_user);
    EXCEPTION WHEN insufficient_privilege THEN
        NULL;
    END;
END;
$$;
