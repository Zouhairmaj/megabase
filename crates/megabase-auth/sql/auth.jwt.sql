-- Ported from supabase/auth migrations/20220531120530_add_auth_jwt_function.up.sql
-- (MIT), pin v2.197.0.

create or replace function auth.jwt()
returns jsonb
language sql stable
as $$
  select 
    coalesce(
        nullif(current_setting('request.jwt.claim', true), ''),
        nullif(current_setting('request.jwt.claims', true), '')
    )::jsonb
$$;

GRANT EXECUTE ON FUNCTION auth.jwt() TO PUBLIC;
