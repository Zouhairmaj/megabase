-- Ported from supabase/storage migrations/tenant (Apache-2.0), pin v1.80.2:
-- 0002-storage-schema.sql, 0003-pathtoken-column.sql,
-- 0011-add-trigger-to-auto-update-updated_at-column.sql, 0016-add-version.sql,
-- 0017-drop-owner-foreign-key.sql, 0018-add_owner_id_column_deprecate_owner.sql,
-- 0019-alter-default-value-objects-id.sql, 0025-custom-metadata.sql,
-- 0046-buckets-objects-grants.sql, 0051-index-backward-compatible-search.sql,
-- 0052-drop-not-used-indexes-and-functions.sql (drops `level`),
-- 0055-prevent-direct-deletes.sql, 0062-object-versioning-core.sql,
-- 0065/0066/0067/0071 (indexes), 0072-drop-bucketid-objname-index.sql and
-- 0073-revoke-grants-to-unused-operations.sql.
--
-- `storage.objects` in its final shape (specs/storage/database.md). The
-- indexes are created without CONCURRENTLY: the install runs in one
-- transaction.

CREATE TABLE IF NOT EXISTS storage.objects (
    id uuid NOT NULL DEFAULT gen_random_uuid(),
    bucket_id text,
    name text,
    owner uuid,
    created_at timestamptz DEFAULT now(),
    updated_at timestamptz DEFAULT now(),
    last_accessed_at timestamptz DEFAULT now(),
    metadata jsonb,
    path_tokens text[] GENERATED ALWAYS AS (string_to_array(name, '/')) STORED,
    version text,
    owner_id text,
    user_metadata jsonb,
    archived_at timestamptz,
    is_delete_marker boolean NOT NULL DEFAULT false,
    is_versioned boolean NOT NULL DEFAULT false,
    CONSTRAINT "objects_bucketId_fkey" FOREIGN KEY (bucket_id) REFERENCES storage.buckets(id),
    PRIMARY KEY (id)
);

-- A table an older storage-api created gains the later columns, as the
-- upstream migrations add them.
ALTER TABLE storage.objects
    ADD COLUMN IF NOT EXISTS path_tokens text[] GENERATED ALWAYS AS (string_to_array(name, '/')) STORED,
    ADD COLUMN IF NOT EXISTS version text,
    ADD COLUMN IF NOT EXISTS owner_id text,
    ADD COLUMN IF NOT EXISTS user_metadata jsonb,
    ADD COLUMN IF NOT EXISTS archived_at timestamptz,
    ADD COLUMN IF NOT EXISTS is_delete_marker boolean NOT NULL DEFAULT false,
    ADD COLUMN IF NOT EXISTS is_versioned boolean NOT NULL DEFAULT false;

CREATE INDEX IF NOT EXISTS name_prefix_search ON storage.objects (name text_pattern_ops);
CREATE INDEX IF NOT EXISTS idx_objects_bucket_id_name_lower
    ON storage.objects (bucket_id, (lower(name) COLLATE "C"));
CREATE UNIQUE INDEX IF NOT EXISTS idx_objects_current_version
    ON storage.objects (bucket_id, name COLLATE "C") WHERE archived_at IS NULL;
CREATE UNIQUE INDEX IF NOT EXISTS idx_objects_null_version
    ON storage.objects (bucket_id, name COLLATE "C") WHERE NOT is_versioned;
CREATE INDEX IF NOT EXISTS idx_objects_delete_markers
    ON storage.objects (bucket_id, name COLLATE "C") WHERE is_delete_marker;

DO $$
BEGIN
    IF current_setting('server_version_num')::integer >= 150000 THEN
        EXECUTE 'CREATE UNIQUE INDEX IF NOT EXISTS objects_bucket_id_name_version_key
            ON storage.objects (bucket_id, name COLLATE "C", version) NULLS NOT DISTINCT';
    ELSE
        EXECUTE 'CREATE UNIQUE INDEX IF NOT EXISTS objects_bucket_id_name_version_key
            ON storage.objects (
                (bucket_id IS NULL), (COALESCE(bucket_id, '''')),
                (name IS NULL), (COALESCE(name, '''') COLLATE "C"),
                (version IS NULL), (COALESCE(version, ''''))
            )';
    END IF;
END$$;

COMMENT ON COLUMN storage.objects.owner IS 'Field is deprecated, use owner_id instead';

ALTER TABLE storage.objects ENABLE ROW LEVEL SECURITY;

GRANT ALL ON storage.objects TO service_role, authenticated, anon;
REVOKE TRUNCATE, REFERENCES, TRIGGER ON storage.objects FROM anon, authenticated;

DO $$
BEGIN
    IF current_setting('server_version_num')::int >= 170000 THEN
        EXECUTE 'REVOKE MAINTAIN ON storage.objects FROM anon, authenticated';
    END IF;
END$$;

CREATE OR REPLACE FUNCTION storage.update_updated_at_column()
RETURNS TRIGGER AS $$
BEGIN
    NEW.updated_at = now();
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE OR REPLACE TRIGGER update_objects_updated_at
    BEFORE UPDATE ON storage.objects
    FOR EACH ROW EXECUTE PROCEDURE storage.update_updated_at_column();

CREATE OR REPLACE FUNCTION storage.protect_delete()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
BEGIN
    -- Check if storage.allow_delete_query is set to 'true'
    IF COALESCE(current_setting('storage.allow_delete_query', true), 'false') != 'true' THEN
        RAISE EXCEPTION 'Direct deletion from storage tables is not allowed. Use the Storage API instead.'
            USING HINT = 'This prevents accidental data loss from orphaned objects.',
                  ERRCODE = '42501';
    END IF;
    RETURN NULL;
END;
$$;

CREATE OR REPLACE TRIGGER protect_buckets_delete
    BEFORE DELETE ON storage.buckets
    FOR EACH STATEMENT
    EXECUTE FUNCTION storage.protect_delete();

CREATE OR REPLACE TRIGGER protect_objects_delete
    BEFORE DELETE ON storage.objects
    FOR EACH STATEMENT
    EXECUTE FUNCTION storage.protect_delete();
