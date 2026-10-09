-- Ported from supabase/auth migrations/00_init_auth_schema.up.sql (MIT), pin v2.197.0.
-- ip_address: migrations/20220614074223_add_ip_address_to_audit_log.postgres.up.sql.
-- RLS/grants: migrations/20240612123726_enable_rls_update_grants.up.sql.

CREATE TABLE IF NOT EXISTS auth.audit_log_entries (
    instance_id uuid NULL,
    id uuid NOT NULL,
    payload json NULL,
    created_at timestamptz NULL,
    ip_address varchar(64) NOT NULL DEFAULT '',
    CONSTRAINT audit_log_entries_pkey PRIMARY KEY (id)
);

ALTER TABLE auth.audit_log_entries
    ADD COLUMN IF NOT EXISTS ip_address VARCHAR(64) NOT NULL DEFAULT '';

CREATE INDEX IF NOT EXISTS audit_logs_instance_id_idx
    ON auth.audit_log_entries USING btree (instance_id);

COMMENT ON TABLE auth.audit_log_entries IS 'Auth: Audit trail for user actions.';

ALTER TABLE auth.audit_log_entries ENABLE ROW LEVEL SECURITY;

DO $$
BEGIN
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'postgres') THEN
        GRANT SELECT ON auth.audit_log_entries TO postgres WITH GRANT OPTION;
    END IF;
END $$;
