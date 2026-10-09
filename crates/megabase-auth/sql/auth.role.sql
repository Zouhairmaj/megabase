-- Ported from supabase/auth migrations/20220224000811_update_auth_functions.up.sql
-- (MIT), pin v2.197.0. Comment from 20220531120530_add_auth_jwt_function.up.sql.

create or replace function auth.role() 
returns text 
language sql stable
as $$
  select 
  coalesce(
    nullif(current_setting('request.jwt.claim.role', true), ''),
    (nullif(current_setting('request.jwt.claims', true), '')::jsonb ->> 'role')
  )::text
$$;

comment on function auth.role() is 'Deprecated. Use auth.jwt() -> ''role'' instead.';

GRANT EXECUTE ON FUNCTION auth.role() TO PUBLIC;
