-- Ported from supabase/realtime priv/repo/tenant_schema/realtime/functions/wal2json_escape_identifier.sql (Apache-2.0), pin v2.143.3.

create or replace function realtime.wal2json_escape_identifier (
  name text
)
  returns text
  language sql
  immutable
  strict
  AS $function$
  -- Prefix `\`, `,`, `.`, and any whitespace with `\`
  SELECT regexp_replace(name, '([\\,.[:space:]])', '\\\1', 'g')
$function$;

alter function "realtime"."wal2json_escape_identifier"(text) owner to "supabase_realtime_admin";
