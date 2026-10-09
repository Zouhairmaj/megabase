-- Ported from supabase/auth
-- migrations/20260302000000_add_passkeys.up.sql (MIT), pin v2.197.0.
-- Challenges half of that file. No later migration in this pin. No RLS.
-- user_id is nullable so signup challenges can exist before a user row.

CREATE TABLE IF NOT EXISTS auth.webauthn_challenges (
    id uuid NOT NULL DEFAULT gen_random_uuid(),
    user_id uuid,
    challenge_type text NOT NULL,
    session_data jsonb NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    expires_at timestamptz NOT NULL,
    CONSTRAINT webauthn_challenges_pkey PRIMARY KEY (id),
    CONSTRAINT webauthn_challenges_user_id_fkey
        FOREIGN KEY (user_id) REFERENCES auth.users (id) ON DELETE CASCADE,
    CONSTRAINT webauthn_challenges_challenge_type_check
        CHECK (challenge_type IN ('signup', 'registration', 'authentication'))
);

ALTER TABLE auth.webauthn_challenges ADD COLUMN IF NOT EXISTS user_id uuid;
ALTER TABLE auth.webauthn_challenges ALTER COLUMN user_id DROP NOT NULL;
ALTER TABLE auth.webauthn_challenges ADD COLUMN IF NOT EXISTS challenge_type text;
ALTER TABLE auth.webauthn_challenges ADD COLUMN IF NOT EXISTS session_data jsonb;
ALTER TABLE auth.webauthn_challenges ADD COLUMN IF NOT EXISTS created_at timestamptz DEFAULT now();
ALTER TABLE auth.webauthn_challenges ADD COLUMN IF NOT EXISTS expires_at timestamptz;

ALTER TABLE auth.webauthn_challenges ALTER COLUMN id SET DEFAULT gen_random_uuid();
ALTER TABLE auth.webauthn_challenges ALTER COLUMN created_at SET DEFAULT now();

ALTER TABLE auth.webauthn_challenges ALTER COLUMN id SET NOT NULL;
ALTER TABLE auth.webauthn_challenges ALTER COLUMN challenge_type SET NOT NULL;
ALTER TABLE auth.webauthn_challenges ALTER COLUMN session_data SET NOT NULL;
ALTER TABLE auth.webauthn_challenges ALTER COLUMN created_at SET NOT NULL;
ALTER TABLE auth.webauthn_challenges ALTER COLUMN expires_at SET NOT NULL;

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1
        FROM pg_constraint
        WHERE conname = 'webauthn_challenges_pkey'
          AND conrelid = 'auth.webauthn_challenges'::regclass
    ) THEN
        ALTER TABLE auth.webauthn_challenges
            ADD CONSTRAINT webauthn_challenges_pkey PRIMARY KEY (id);
    END IF;
    IF NOT EXISTS (
        SELECT 1
        FROM pg_constraint
        WHERE conname = 'webauthn_challenges_user_id_fkey'
          AND conrelid = 'auth.webauthn_challenges'::regclass
    ) THEN
        ALTER TABLE auth.webauthn_challenges
            ADD CONSTRAINT webauthn_challenges_user_id_fkey
            FOREIGN KEY (user_id) REFERENCES auth.users (id) ON DELETE CASCADE;
    END IF;
    IF NOT EXISTS (
        SELECT 1
        FROM pg_constraint
        WHERE conname = 'webauthn_challenges_challenge_type_check'
          AND conrelid = 'auth.webauthn_challenges'::regclass
    ) THEN
        ALTER TABLE auth.webauthn_challenges
            ADD CONSTRAINT webauthn_challenges_challenge_type_check
            CHECK (challenge_type IN ('signup', 'registration', 'authentication'));
    END IF;
END $$;

CREATE INDEX IF NOT EXISTS webauthn_challenges_user_id_idx
    ON auth.webauthn_challenges (user_id);
CREATE INDEX IF NOT EXISTS webauthn_challenges_expires_at_idx
    ON auth.webauthn_challenges (expires_at);
