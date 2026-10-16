-- Ported from supabase/realtime priv/repo/tenant_schema/realtime/types/equality_op.sql (Apache-2.0), pin v2.143.3.

create type "realtime"."equality_op" as enum (
  'eq',
  'neq',
  'lt',
  'lte',
  'gt',
  'gte',
  'in',
  'like',
  'ilike',
  'is',
  'match',
  'imatch',
  'isdistinct'
);

alter type "realtime"."equality_op" owner to "supabase_realtime_admin";
