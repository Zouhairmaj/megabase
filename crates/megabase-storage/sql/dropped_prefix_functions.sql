-- Ported from supabase/storage (Apache-2.0), pin v1.80.2:
--   migrations/tenant/0052-drop-not-used-indexes-and-functions.sql
--   migrations/tenant/0059-drop-unused-functions.sql
--
-- Final shape: `storage.add_prefixes(text, text)`,
-- `storage.delete_prefix(text, text)`,
-- `storage.delete_prefix_hierarchy_trigger()` and
-- `storage.delete_leaf_prefixes(text[], text[])` do not exist. Drop them so a
-- database migrated by an older storage-api matches the pinned reference.
-- Failures are swallowed, as upstream does.

DO $$
BEGIN
    DROP FUNCTION IF EXISTS storage.delete_prefix_hierarchy_trigger();
EXCEPTION WHEN OTHERS THEN NULL;
END;
$$;

DO $$
BEGIN
    DROP FUNCTION IF EXISTS storage.delete_prefix(_bucket_id text, _name text);
EXCEPTION WHEN OTHERS THEN NULL;
END;
$$;

DO $$
BEGIN
    DROP FUNCTION IF EXISTS storage.add_prefixes(_bucket_id text, _name text);
EXCEPTION WHEN OTHERS THEN NULL;
END;
$$;

DROP FUNCTION IF EXISTS storage.delete_leaf_prefixes(text[],text[]);
