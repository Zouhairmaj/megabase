-- Ported from supabase/storage migrations/tenant (Apache-2.0), pin v1.80.2:
-- 0002-storage-schema.sql, 0007-add-rls-to-buckets.sql,
-- 0008-add-public-to-buckets.sql, 0012-add-automatic-avif-detection-flag.sql,
-- 0013-add-bucket-custom-limits.sql, 0014-use-bytes-for-max-size.sql,
-- 0018-add_owner_id_column_deprecate_owner.sql,
-- 0038-iceberg-catalog-flag-on-buckets.sql, 0044-vector-bucket-type.sql,
-- 0046-buckets-objects-grants.sql, 0062-object-versioning-core.sql,
-- 0068-bucket-lifecycle-configuration.sql and
-- 0073-revoke-grants-to-unused-operations.sql.
--
-- The `storage.buckets` columns the bucket routes read, in their final
-- shape. The triggers listed in specs/storage/database.md are installed by
-- their own files; the versioning and lifecycle CHECK constraints and every
-- other storage table are not installed (specs/storage/buckets.md).

CREATE SCHEMA IF NOT EXISTS storage;

GRANT USAGE ON SCHEMA storage TO anon, authenticated, service_role;

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_type t JOIN pg_namespace n ON n.oid = t.typnamespace
        WHERE n.nspname = 'storage' AND t.typname = 'buckettype'
    ) THEN
        CREATE TYPE storage.buckettype AS ENUM ('STANDARD', 'ANALYTICS', 'VECTOR');
    END IF;
END$$;

CREATE TABLE IF NOT EXISTS storage.buckets (
    id text NOT NULL,
    name text NOT NULL,
    owner uuid,
    created_at timestamptz DEFAULT now(),
    updated_at timestamptz DEFAULT now(),
    public boolean DEFAULT false,
    avif_autodetection boolean DEFAULT false,
    file_size_limit bigint,
    allowed_mime_types text[],
    owner_id text,
    type storage.buckettype NOT NULL DEFAULT 'STANDARD',
    versioning_status text NOT NULL DEFAULT 'DISABLED',
    lifecycle_configuration jsonb,
    lifecycle_configuration_generation uuid,
    PRIMARY KEY (id)
);

CREATE UNIQUE INDEX IF NOT EXISTS bname ON storage.buckets USING btree (name);

COMMENT ON COLUMN storage.buckets.owner IS 'Field is deprecated, use owner_id instead';

ALTER TABLE storage.buckets ENABLE ROW LEVEL SECURITY;

GRANT ALL ON storage.buckets TO service_role, authenticated, anon;
REVOKE TRUNCATE, REFERENCES, TRIGGER ON storage.buckets FROM anon, authenticated;

DO $$
BEGIN
    IF current_setting('server_version_num')::int >= 170000 THEN
        EXECUTE 'REVOKE MAINTAIN ON storage.buckets FROM anon, authenticated';
    END IF;
END$$;
