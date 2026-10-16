-- Ported from supabase/realtime priv/repo/tenant_schema/realtime/types/user_defined_filter.sql (Apache-2.0), pin v2.143.3.

create type "realtime"."user_defined_filter" as (
  "column_name" text,
  "op"          realtime.equality_op,
  "value"       text,
  "negate"      boolean
);

alter type "realtime"."user_defined_filter" owner to "supabase_realtime_admin";
