-- Ported from supabase/storage
-- migrations/tenant/0015-add-can-insert-object-function.sql (Apache-2.0),
-- pin v1.80.2. The signature is unchanged after 0015.
-- Spec: specs/storage/database.md.

CREATE OR REPLACE FUNCTION storage.can_insert_object(bucketid text, name text, owner uuid, metadata jsonb)
RETURNS VOID
LANGUAGE plpgsql
AS $$
BEGIN
  INSERT INTO "storage"."objects" ("bucket_id", "name", "owner", "metadata") VALUES (bucketid, name, owner, metadata);
  -- hack to rollback the successful insert
  RAISE sqlstate 'PT200' using
  message = 'ROLLBACK',
  detail = 'rollback successful insert';
END
$$;
