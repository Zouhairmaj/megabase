-- Ported from supabase/auth migrations/20220224000811_update_auth_functions.up.sql
-- (MIT), pin v2.197.0. First created in 20211124214934_update_auth_functions.up.sql.
-- Comment from 20220531120530_add_auth_jwt_function.up.sql.

create or replace function auth.email() 
returns text 
language sql stable
as $$
  select 
  coalesce(
    nullif(current_setting('request.jwt.claim.email', true), ''),
    (nullif(current_setting('request.jwt.claims', true), '')::jsonb ->> 'email')
  )::text
$$;

comment on function auth.email() is 'Deprecated. Use auth.jwt() -> ''email'' instead.';

GRANT EXECUTE ON FUNCTION auth.email() TO PUBLIC;
