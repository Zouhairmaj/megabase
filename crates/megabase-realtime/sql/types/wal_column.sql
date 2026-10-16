-- Ported from supabase/realtime priv/repo/tenant_schema/realtime/types/wal_column.sql (Apache-2.0), pin v2.143.3.

create type "realtime"."wal_column" as (
  "name"          text,
  "type_name"     text,
  "type_oid"      oid,
  "value"         jsonb,
  "is_pkey"       boolean,
  "is_selectable" boolean
);

alter type "realtime"."wal_column" owner to "supabase_realtime_admin";
