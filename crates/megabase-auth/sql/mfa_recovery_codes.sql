-- Ported from supabase/auth migrations/20260824000001_add_recovery_codes_tables.up.sql
-- (MIT), pin v2.197.0. No RLS in this pin.

CREATE TABLE IF NOT EXISTS auth.mfa_recovery_codes (
    id uuid PRIMARY KEY,
    mfa_recovery_code_set_id uuid NOT NULL REFERENCES auth.mfa_recovery_code_sets (id) ON DELETE CASCADE,
    code_hash text NOT NULL,
    consumed_at timestamptz,
    created_at timestamptz NOT NULL DEFAULT now()
);

ALTER TABLE auth.mfa_recovery_codes ADD COLUMN IF NOT EXISTS mfa_recovery_code_set_id uuid;
ALTER TABLE auth.mfa_recovery_codes ADD COLUMN IF NOT EXISTS code_hash text;
ALTER TABLE auth.mfa_recovery_codes ADD COLUMN IF NOT EXISTS consumed_at timestamptz;
ALTER TABLE auth.mfa_recovery_codes ADD COLUMN IF NOT EXISTS created_at timestamptz DEFAULT now();

ALTER TABLE auth.mfa_recovery_codes ALTER COLUMN mfa_recovery_code_set_id SET NOT NULL;
ALTER TABLE auth.mfa_recovery_codes ALTER COLUMN code_hash SET NOT NULL;
ALTER TABLE auth.mfa_recovery_codes ALTER COLUMN created_at SET NOT NULL;

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1
        FROM pg_constraint
        WHERE conname = 'mfa_recovery_codes_mfa_recovery_code_set_id_fkey'
          AND conrelid = 'auth.mfa_recovery_codes'::regclass
    ) THEN
        ALTER TABLE auth.mfa_recovery_codes
            ADD CONSTRAINT mfa_recovery_codes_mfa_recovery_code_set_id_fkey
            FOREIGN KEY (mfa_recovery_code_set_id)
            REFERENCES auth.mfa_recovery_code_sets(id) ON DELETE CASCADE;
    END IF;
END $$;

CREATE INDEX IF NOT EXISTS mfa_recovery_codes_set_id_idx
    ON auth.mfa_recovery_codes (mfa_recovery_code_set_id);
