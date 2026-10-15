-- Ported from supabase/storage
-- migrations/tenant/0060-optimize-existing-functions-again.sql (Apache-2.0),
-- pin v1.80.2. First defined in 0002-storage-schema.sql:109.

CREATE OR REPLACE FUNCTION storage.extension(name text)
    RETURNS text
    LANGUAGE plpgsql
    IMMUTABLE
AS $function$
DECLARE
    _parts text[];
    _filename text;
BEGIN
    -- Split on "/" to get path segments
    SELECT string_to_array(name, '/') INTO _parts;
    -- Get the last path segment (the actual filename)
    SELECT _parts[array_length(_parts, 1)] INTO _filename;
    -- Extract extension: reverse, split on '.', then reverse again
    RETURN reverse(split_part(reverse(_filename), '.', 1));
END
$function$;
