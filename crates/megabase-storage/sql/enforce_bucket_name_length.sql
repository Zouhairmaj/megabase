-- Ported from supabase/storage
-- migrations/tenant/0037-add-bucket-name-length-trigger.sql (Apache-2.0),
-- pin v1.80.2.
--
-- `CREATE OR REPLACE TRIGGER` instead of drop-and-create so a restart does
-- not take the table lock twice; the resulting trigger is the same.

create or replace function storage.enforce_bucket_name_length()
returns trigger as $$
begin
    if length(new.name) > 100 then
        raise exception 'bucket name "%" is too long (% characters). Max is 100.', new.name, length(new.name);
    end if;
    return new;
end;
$$ language plpgsql;

create or replace trigger enforce_bucket_name_length_trigger
before insert or update of name on storage.buckets
for each row execute function storage.enforce_bucket_name_length();
