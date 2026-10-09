-- Ported from supabase/auth migrations/20260824000001_add_recovery_codes_tables.up.sql
-- (MIT), pin v2.197.0. No RLS in this pin.

CREATE TABLE IF NOT EXISTS auth.mfa_recovery_code_sets (
    id uuid PRIMARY KEY,
    user_id uuid NOT NULL UNIQUE REFERENCES auth.users (id) ON DELETE CASCADE,
    mfa_factor_id uuid NOT NULL UNIQUE REFERENCES auth.mfa_factors (id) ON DELETE CASCADE,
    failed_verification_count integer NOT NULL DEFAULT 0 CHECK (failed_verification_count >= 0),
    verification_locked_until timestamptz,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now()
);

ALTER TABLE auth.mfa_recovery_code_sets ADD COLUMN IF NOT EXISTS user_id uuid;
ALTER TABLE auth.mfa_recovery_code_sets ADD COLUMN IF NOT EXISTS mfa_factor_id uuid;
ALTER TABLE auth.mfa_recovery_code_sets ADD COLUMN IF NOT EXISTS failed_verification_count integer DEFAULT 0;
ALTER TABLE auth.mfa_recovery_code_sets ADD COLUMN IF NOT EXISTS verification_locked_until timestamptz;
ALTER TABLE auth.mfa_recovery_code_sets ADD COLUMN IF NOT EXISTS created_at timestamptz DEFAULT now();
ALTER TABLE auth.mfa_recovery_code_sets ADD COLUMN IF NOT EXISTS updated_at timestamptz DEFAULT now();

ALTER TABLE auth.mfa_recovery_code_sets ALTER COLUMN user_id SET NOT NULL;
ALTER TABLE auth.mfa_recovery_code_sets ALTER COLUMN mfa_factor_id SET NOT NULL;
ALTER TABLE auth.mfa_recovery_code_sets ALTER COLUMN failed_verification_count SET NOT NULL;
ALTER TABLE auth.mfa_recovery_code_sets ALTER COLUMN created_at SET NOT NULL;
ALTER TABLE auth.mfa_recovery_code_sets ALTER COLUMN updated_at SET NOT NULL;

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1
        FROM pg_constraint
        WHERE conname = 'mfa_recovery_code_sets_user_id_key'
          AND conrelid = 'auth.mfa_recovery_code_sets'::regclass
    ) THEN
        ALTER TABLE auth.mfa_recovery_code_sets
            ADD CONSTRAINT mfa_recovery_code_sets_user_id_key UNIQUE (user_id);
    END IF;
    IF NOT EXISTS (
        SELECT 1
        FROM pg_constraint
        WHERE conname = 'mfa_recovery_code_sets_mfa_factor_id_key'
          AND conrelid = 'auth.mfa_recovery_code_sets'::regclass
    ) THEN
        ALTER TABLE auth.mfa_recovery_code_sets
            ADD CONSTRAINT mfa_recovery_code_sets_mfa_factor_id_key UNIQUE (mfa_factor_id);
    END IF;
    IF NOT EXISTS (
        SELECT 1
        FROM pg_constraint
        WHERE conname = 'mfa_recovery_code_sets_user_id_fkey'
          AND conrelid = 'auth.mfa_recovery_code_sets'::regclass
    ) THEN
        ALTER TABLE auth.mfa_recovery_code_sets
            ADD CONSTRAINT mfa_recovery_code_sets_user_id_fkey
            FOREIGN KEY (user_id) REFERENCES auth.users(id) ON DELETE CASCADE;
    END IF;
    IF NOT EXISTS (
        SELECT 1
        FROM pg_constraint
        WHERE conname = 'mfa_recovery_code_sets_mfa_factor_id_fkey'
          AND conrelid = 'auth.mfa_recovery_code_sets'::regclass
    ) THEN
        ALTER TABLE auth.mfa_recovery_code_sets
            ADD CONSTRAINT mfa_recovery_code_sets_mfa_factor_id_fkey
            FOREIGN KEY (mfa_factor_id) REFERENCES auth.mfa_factors(id) ON DELETE CASCADE;
    END IF;
    IF NOT EXISTS (
        SELECT 1
        FROM pg_constraint
        WHERE conname = 'mfa_recovery_code_sets_failed_verification_count_check'
          AND conrelid = 'auth.mfa_recovery_code_sets'::regclass
    ) THEN
        ALTER TABLE auth.mfa_recovery_code_sets
            ADD CONSTRAINT mfa_recovery_code_sets_failed_verification_count_check
            CHECK (failed_verification_count >= 0);
    END IF;
END $$;
