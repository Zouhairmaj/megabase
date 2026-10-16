-- Ported from supabase/supabase docker/dev/data.sql (Apache-2.0), pin v1.26.08.
--
-- The supabase/postgres init scripts create the empty `supabase_realtime`
-- publication on the reference database; postgres_changes streams the
-- tables added to it. Guarded so an existing publication (and the tables
-- already added to it) is kept.

do $$
begin
  if not exists (select 1 from pg_publication where pubname = 'supabase_realtime') then
    create publication supabase_realtime;
  end if;
end
$$;
