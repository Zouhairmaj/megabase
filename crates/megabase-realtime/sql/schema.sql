-- Ported from supabase/realtime priv/repo/tenant_schema/realtime/schema.sql (Apache-2.0), pin v2.143.3.

create schema "realtime" authorization "supabase_admin";

grant usage on schema "realtime" to "anon", "authenticated", "service_role";

grant create, usage on schema "realtime" to "supabase_realtime_admin";
