-- Ported from supabase/storage migrations/tenant (Apache-2.0), pin v1.80.2:
-- 0021-s3-multipart-uploads.sql, 0022-s3-multipart-uploads-big-ints.sql,
-- 0025-custom-metadata.sql and 0057-s3-multipart-uploads-metadata.sql.
--
-- `storage.s3_multipart_uploads` and `storage.s3_multipart_uploads_parts` in
-- their final shape (specs/storage/database.md). Grants follow 0021: the
-- role names come from `storage.anon_role`, `storage.authenticated_role`
-- and `storage.service_role`, falling back to `anon`, `authenticated` and
-- `service_role`.

CREATE TABLE IF NOT EXISTS storage.s3_multipart_uploads (
    id text PRIMARY KEY,
    in_progress_size bigint NOT NULL DEFAULT 0,
    upload_signature text NOT NULL,
    bucket_id text NOT NULL REFERENCES storage.buckets(id),
    key text COLLATE "C" NOT NULL,
    version text NOT NULL,
    owner_id text NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    user_metadata jsonb NULL,
    metadata jsonb NULL
);

CREATE TABLE IF NOT EXISTS storage.s3_multipart_uploads_parts (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    upload_id text NOT NULL REFERENCES storage.s3_multipart_uploads(id) ON DELETE CASCADE,
    size bigint NOT NULL DEFAULT 0,
    part_number int NOT NULL,
    bucket_id text NOT NULL REFERENCES storage.buckets(id),
    key text COLLATE "C" NOT NULL,
    etag text NOT NULL,
    owner_id text NULL,
    version text NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now()
);

-- A table an older storage-api created gains the later columns and types.
ALTER TABLE storage.s3_multipart_uploads
    ADD COLUMN IF NOT EXISTS user_metadata jsonb NULL,
    ADD COLUMN IF NOT EXISTS metadata jsonb NULL;
ALTER TABLE storage.s3_multipart_uploads
    ALTER COLUMN in_progress_size TYPE bigint;
ALTER TABLE storage.s3_multipart_uploads_parts
    ALTER COLUMN size TYPE bigint;

CREATE INDEX IF NOT EXISTS idx_multipart_uploads_list
    ON storage.s3_multipart_uploads (bucket_id, (key COLLATE "C"), created_at ASC);

ALTER TABLE storage.s3_multipart_uploads ENABLE ROW LEVEL SECURITY;
ALTER TABLE storage.s3_multipart_uploads_parts ENABLE ROW LEVEL SECURITY;

DO $$
DECLARE
    anon_role text = COALESCE(current_setting('storage.anon_role', true), 'anon');
    authenticated_role text = COALESCE(current_setting('storage.authenticated_role', true), 'authenticated');
    service_role text = COALESCE(current_setting('storage.service_role', true), 'service_role');
BEGIN
    EXECUTE 'revoke all on storage.s3_multipart_uploads from ' || anon_role || ', ' || authenticated_role;
    EXECUTE 'revoke all on storage.s3_multipart_uploads_parts from ' || anon_role || ', ' || authenticated_role;
    EXECUTE 'GRANT ALL ON TABLE storage.s3_multipart_uploads TO ' || service_role;
    EXECUTE 'GRANT ALL ON TABLE storage.s3_multipart_uploads_parts TO ' || service_role;
    EXECUTE 'GRANT SELECT ON TABLE storage.s3_multipart_uploads TO ' || authenticated_role || ', ' || anon_role;
    EXECUTE 'GRANT SELECT ON TABLE storage.s3_multipart_uploads_parts TO ' || authenticated_role || ', ' || anon_role;
END$$;
