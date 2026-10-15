-- Ported from supabase/storage
-- migrations/tenant/0068-bucket-lifecycle-configuration.sql:115 and :155
-- (Apache-2.0), pin v1.80.2.
--
-- Only the AFTER UPDATE role check. The BEFORE triggers that run
-- `storage.protect_bucket_control_columns()` are a separate unit and are not
-- installed yet (specs/storage/database.md).

CREATE OR REPLACE FUNCTION storage.enforce_bucket_lifecycle_service_role()
RETURNS trigger
LANGUAGE plpgsql
SET search_path = pg_catalog
AS $$
BEGIN
  IF current_user::text IS DISTINCT FROM TG_ARGV[0]
     AND (
       OLD.lifecycle_configuration IS DISTINCT FROM NEW.lifecycle_configuration
       OR OLD.lifecycle_configuration_generation IS DISTINCT FROM NEW.lifecycle_configuration_generation
     ) THEN
    -- AFTER runs only after caller RLS has accepted the proposed row. The API
    -- recognizes this specific error after rolling back its permission probe;
    -- direct non-service writes still fail and cannot persist the change.
    RAISE EXCEPTION 'bucket control columns may only be changed by the configured storage service role'
      USING ERRCODE = 'PST01',
            SCHEMA = TG_TABLE_SCHEMA,
            TABLE = TG_TABLE_NAME,
            CONSTRAINT = TG_NAME;
  END IF;

  RETURN NULL;
END;
$$;

DO $$
DECLARE
  service_role text = COALESCE(current_setting('storage.service_role', true), 'service_role');
BEGIN
  EXECUTE format(
    'CREATE OR REPLACE TRIGGER protect_bucket_control_update_role AFTER UPDATE OF lifecycle_configuration, lifecycle_configuration_generation ON storage.buckets FOR EACH ROW EXECUTE FUNCTION storage.enforce_bucket_lifecycle_service_role(%L)',
    service_role
  );
END;
$$;
