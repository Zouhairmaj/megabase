-- Ported from supabase/storage migrations/tenant/0024-operation-function.sql
-- (Apache-2.0), pin v1.80.2.
--
-- Final definition (specs/storage/database.md). The operation helpers are
-- `LANGUAGE sql` and PostgreSQL resolves `storage.operation()` when it
-- creates them, so this file is installed first.

CREATE OR REPLACE FUNCTION storage.operation()
    RETURNS text AS $$
BEGIN
    RETURN current_setting('storage.operation', true);
END;
$$ LANGUAGE plpgsql STABLE;
