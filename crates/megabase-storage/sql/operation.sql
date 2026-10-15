-- Ported from supabase/storage migrations/tenant/0024-operation-function.sql
-- (Apache-2.0), pin v1.80.2.
--
-- Installed because the operation helpers below are `LANGUAGE sql` and
-- PostgreSQL resolves `storage.operation()` when it creates them.

CREATE OR REPLACE FUNCTION storage.operation()
    RETURNS text AS $$
BEGIN
    RETURN current_setting('storage.operation', true);
END;
$$ LANGUAGE plpgsql STABLE;
