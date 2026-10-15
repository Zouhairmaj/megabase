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
--
-- A database that reached migration 0026 but not 0052 still has triggers on
-- `storage.objects` and `storage.prefixes` that depend on these functions, so
-- the drop would fail. Upstream removes those triggers first (0050 for
-- `storage.objects`, 0052 for `storage.prefixes`); the same statements run
-- here, guarded on the table existing.

DO $$
BEGIN
    IF to_regclass('storage.objects') IS NOT NULL THEN
        DROP TRIGGER IF EXISTS objects_insert_create_prefix ON storage.objects;
        DROP TRIGGER IF EXISTS objects_update_create_prefix ON storage.objects;
        DROP TRIGGER IF EXISTS objects_delete_delete_prefix ON storage.objects;
    END IF;
    IF to_regclass('storage.prefixes') IS NOT NULL THEN
        DROP TRIGGER IF EXISTS prefixes_create_hierarchy ON storage.prefixes;
        DROP TRIGGER IF EXISTS prefixes_delete_hierarchy ON storage.prefixes;
    END IF;
EXCEPTION WHEN OTHERS THEN NULL;
END;
$$;

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
