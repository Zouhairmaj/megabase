-- Ported from supabase/storage
-- migrations/tenant/0070-list-objects-with-versions.sql:1538 (Apache-2.0),
-- pin v1.80.2. Final signature. 0027's five-argument function and 0039's
-- eight-argument function are dropped first so an older install does not
-- keep an overload (0050:78, 0070:1538).
-- Spec: specs/storage/database.md.

DROP FUNCTION IF EXISTS storage.search_v2(text, text, integer, integer, text);

DROP FUNCTION IF EXISTS storage.search_v2(text, text, integer, integer, text, text, text, text);

CREATE OR REPLACE FUNCTION storage.search_v2(
    prefix text,
    bucket_name text,
    limits integer DEFAULT 100,
    levels integer DEFAULT 1,
    start_after text DEFAULT ''::text,
    sort_order text DEFAULT 'asc'::text,
    sort_column text DEFAULT 'name'::text,
    sort_column_after text DEFAULT ''::text,
    noncurrent_versions text DEFAULT 'exclude'::text,
    delete_markers text DEFAULT 'exclude'::text,
    start_after_archived_at timestamptz DEFAULT NULL,
    start_after_version text DEFAULT ''::text,
    start_after_is_continuation boolean DEFAULT false
)
 RETURNS TABLE(
    key text,
    name text,
    id uuid,
    updated_at timestamp with time zone,
    created_at timestamp with time zone,
    last_accessed_at timestamp with time zone,
    metadata jsonb, version text,
    archived_at timestamp with time zone,
    is_delete_marker boolean,
    is_versioned boolean
)
 LANGUAGE plpgsql
 STABLE
AS $function$
DECLARE
    v_sort_col text;
    v_sort_ord text;
    v_limit int;
BEGIN
    -- Cap limit to maximum of 1500 records
    v_limit := LEAST(coalesce(limits, 100), 1500);

    -- Validate and normalize sort_order
    v_sort_ord := lower(coalesce(sort_order, 'asc'));
    IF v_sort_ord NOT IN ('asc', 'desc') THEN
        v_sort_ord := 'asc';
    END IF;

    -- Validate and normalize sort_column
    v_sort_col := lower(coalesce(sort_column, 'name'));
    IF v_sort_col NOT IN ('name', 'updated_at', 'created_at') THEN
        v_sort_col := 'name';
    END IF;

    -- Route to appropriate implementation
    IF v_sort_col = 'name' THEN
        -- Use list_objects_with_delimiter for name sorting (most efficient: O(k * log n))
        RETURN QUERY
        SELECT
            split_part(l.name, '/', levels) AS key,
            l.name AS name,
            l.id,
            l.updated_at,
            l.created_at,
            l.last_accessed_at,
            l.metadata,
            l.version,
            l.archived_at,
            l.is_delete_marker,
            l.is_versioned
        FROM storage.list_objects_with_delimiter(
            bucket_name,
            coalesce(prefix, ''),
            '/',
            v_limit,
            CASE WHEN start_after_is_continuation THEN '' ELSE start_after END,
            CASE WHEN start_after_is_continuation THEN start_after ELSE '' END,
            v_sort_ord,
            noncurrent_versions,
            delete_markers,
            start_after_archived_at,
            start_after_version
        ) l;
    ELSE
        -- Use aggregation approach for timestamp sorting
        -- Not efficient for large datasets but supports correct pagination
        RETURN QUERY SELECT * FROM storage.search_by_timestamp(
            prefix, bucket_name, v_limit, levels, start_after,
            v_sort_ord, v_sort_col, sort_column_after,
            noncurrent_versions, delete_markers, start_after_version
        );
    END IF;
END;
$function$;
