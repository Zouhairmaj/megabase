-- Ported from supabase/realtime priv/repo/tenant_schema/realtime/types/action.sql (Apache-2.0), pin v2.143.3.

create type "realtime"."action" as enum (
  'INSERT',
  'UPDATE',
  'DELETE',
  'TRUNCATE',
  'ERROR'
);

alter type "realtime"."action" owner to "supabase_realtime_admin";
