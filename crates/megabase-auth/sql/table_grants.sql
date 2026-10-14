-- Ported from supabase/supabase-js
-- packages/core/auth-js/infra/db/00-schema.sql (MIT), pin v2.117.3
-- (line 83: GRANT ALL PRIVILEGES ON ALL TABLES IN SCHEMA auth TO postgres).
-- That file repeats the auth setup of the Supabase Postgres image, whose
-- init scripts (which also grant dashboard_user ALL) are not vendored. The
-- reference stack decides the final ACLs (GOAL.md §2). Spec:
-- specs/auth/database.md "Table privileges".
--
-- auth.schema_migrations is left out: on the reference it carries only the
-- postgres SELECT grant from 20240612123726_enable_rls_update_grants.
-- GRANT ALL keeps the WITH GRANT OPTION that migration gives postgres on
-- SELECT. Roles missing on a stock cluster are skipped.

DO $$
DECLARE
    grantee text;
    tbl text;
BEGIN
    FOREACH grantee IN ARRAY ARRAY['postgres', 'dashboard_user'] LOOP
        IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = grantee) THEN
            CONTINUE;
        END IF;
        FOREACH tbl IN ARRAY ARRAY[
            'audit_log_entries',
            'custom_oauth_providers',
            'flow_state',
            'identities',
            'instances',
            'mfa_amr_claims',
            'mfa_challenges',
            'mfa_factors',
            'mfa_recovery_code_sets',
            'mfa_recovery_codes',
            'oauth_authorizations',
            'oauth_client_states',
            'oauth_clients',
            'oauth_consents',
            'one_time_tokens',
            'refresh_tokens',
            'saml_providers',
            'saml_relay_states',
            'scim_tokens',
            'scim_users',
            'sessions',
            'sso_domains',
            'sso_providers',
            'users',
            'webauthn_challenges',
            'webauthn_credentials'
        ] LOOP
            IF to_regclass(format('auth.%I', tbl)) IS NOT NULL THEN
                EXECUTE format('GRANT ALL ON TABLE auth.%I TO %I', tbl, grantee);
            END IF;
        END LOOP;
    END LOOP;
END $$;
