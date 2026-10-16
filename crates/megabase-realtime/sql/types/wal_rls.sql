-- Ported from supabase/realtime priv/repo/tenant_schema/realtime/types/wal_rls.sql (Apache-2.0), pin v2.143.3.

create type "realtime"."wal_rls" as (
  "wal"              jsonb,
  "is_rls_enabled"   boolean,
  "subscription_ids" uuid[],
  "errors"           text[]
);

alter type "realtime"."wal_rls" owner to "supabase_realtime_admin";
