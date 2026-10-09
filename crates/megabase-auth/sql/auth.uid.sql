-- Ported from supabase/auth migrations/20220224000811_update_auth_functions.up.sql
-- (MIT), pin v2.197.0. Comment from 20220531120530_add_auth_jwt_function.up.sql.

create or replace function auth.uid() 
returns uuid 
language sql stable
as $$
  select 
  coalesce(
    nullif(current_setting('request.jwt.claim.sub', true), ''),
    (nullif(current_setting('request.jwt.claims', true), '')::jsonb ->> 'sub')
  )::uuid
$$;

comment on function auth.uid() is 'Deprecated. Use auth.jwt() -> ''sub'' instead.';

GRANT EXECUTE ON FUNCTION auth.uid() TO PUBLIC;
