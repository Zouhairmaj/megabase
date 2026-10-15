-- Ported from supabase/storage migrations/tenant (Apache-2.0), pin v1.80.2:
-- storage.foldername() from 0060-optimize-existing-functions-again.sql:22,
-- storage.filename() from 0061-mark-filename-immutable.sql:7. RLS policies
-- on storage.objects call them on the object name.

CREATE OR REPLACE FUNCTION storage.foldername(name text)
    RETURNS text[]
    LANGUAGE plpgsql
    IMMUTABLE
AS $function$
DECLARE
    _parts text[];
BEGIN
    -- Split on "/" to get path segments
    SELECT string_to_array(name, '/') INTO _parts;
    -- Return everything except the last segment
    RETURN _parts[1 : array_length(_parts,1) - 1];
END
$function$;

CREATE OR REPLACE FUNCTION storage.filename(name text)
    RETURNS text
    LANGUAGE plpgsql
    IMMUTABLE
AS $function$
DECLARE
    _parts text[];
BEGIN
    SELECT string_to_array(name, '/') INTO _parts;
    RETURN _parts[array_length(_parts, 1)];
END
$function$;
