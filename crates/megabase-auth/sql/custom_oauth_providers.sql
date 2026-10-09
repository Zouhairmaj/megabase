-- Ported from supabase/auth (MIT), pin v2.197.0:
--   migrations/20260219120000_add_custom_oauth_providers.up.sql
--   migrations/20260625000000_add_custom_claims_allowlist.up.sql

CREATE TABLE IF NOT EXISTS auth.custom_oauth_providers (
    id uuid NOT NULL DEFAULT gen_random_uuid(),
    provider_type text NOT NULL CHECK (provider_type IN ('oauth2', 'oidc')),
    identifier text NOT NULL,
    name text NOT NULL,
    client_id text NOT NULL,
    client_secret text NOT NULL,
    acceptable_client_ids text[] NOT NULL DEFAULT '{}',
    scopes text[] NOT NULL DEFAULT '{}',
    pkce_enabled boolean NOT NULL DEFAULT true,
    attribute_mapping jsonb NOT NULL DEFAULT '{}',
    authorization_params jsonb NOT NULL DEFAULT '{}',
    enabled boolean NOT NULL DEFAULT true,
    email_optional boolean NOT NULL DEFAULT false,
    issuer text NULL,
    discovery_url text NULL,
    skip_nonce_check boolean NOT NULL DEFAULT false,
    cached_discovery jsonb NULL,
    discovery_cached_at timestamptz NULL,
    authorization_url text NULL,
    token_url text NULL,
    userinfo_url text NULL,
    jwks_uri text NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    custom_claims_allowlist text[] NOT NULL DEFAULT '{}',
    CONSTRAINT custom_oauth_providers_pkey PRIMARY KEY (id),
    CONSTRAINT custom_oauth_providers_identifier_key UNIQUE (identifier),
    CONSTRAINT custom_oauth_providers_oidc_requires_issuer CHECK (
        provider_type != 'oidc' OR issuer IS NOT NULL
    ),
    CONSTRAINT custom_oauth_providers_oidc_issuer_https CHECK (
        provider_type != 'oidc' OR issuer IS NULL OR issuer LIKE 'https://%'
    ),
    CONSTRAINT custom_oauth_providers_oidc_discovery_url_https CHECK (
        provider_type != 'oidc' OR discovery_url IS NULL OR discovery_url LIKE 'https://%'
    ),
    CONSTRAINT custom_oauth_providers_oauth2_requires_endpoints CHECK (
        provider_type != 'oauth2' OR (
            authorization_url IS NOT NULL AND
            token_url IS NOT NULL AND
            userinfo_url IS NOT NULL
        )
    ),
    CONSTRAINT custom_oauth_providers_authorization_url_https CHECK (
        authorization_url IS NULL OR authorization_url LIKE 'https://%'
    ),
    CONSTRAINT custom_oauth_providers_token_url_https CHECK (
        token_url IS NULL OR token_url LIKE 'https://%'
    ),
    CONSTRAINT custom_oauth_providers_userinfo_url_https CHECK (
        userinfo_url IS NULL OR userinfo_url LIKE 'https://%'
    ),
    CONSTRAINT custom_oauth_providers_jwks_uri_https CHECK (
        jwks_uri IS NULL OR jwks_uri LIKE 'https://%'
    ),
    CONSTRAINT custom_oauth_providers_identifier_format CHECK (
        identifier ~ '^[a-z0-9][a-z0-9:-]{0,48}[a-z0-9]$'
    ),
    CONSTRAINT custom_oauth_providers_name_length CHECK (
        char_length(name) >= 1 AND char_length(name) <= 100
    ),
    CONSTRAINT custom_oauth_providers_issuer_length CHECK (
        issuer IS NULL OR (char_length(issuer) >= 1 AND char_length(issuer) <= 2048)
    ),
    CONSTRAINT custom_oauth_providers_discovery_url_length CHECK (
        discovery_url IS NULL OR char_length(discovery_url) <= 2048
    ),
    CONSTRAINT custom_oauth_providers_authorization_url_length CHECK (
        authorization_url IS NULL OR char_length(authorization_url) <= 2048
    ),
    CONSTRAINT custom_oauth_providers_token_url_length CHECK (
        token_url IS NULL OR char_length(token_url) <= 2048
    ),
    CONSTRAINT custom_oauth_providers_userinfo_url_length CHECK (
        userinfo_url IS NULL OR char_length(userinfo_url) <= 2048
    ),
    CONSTRAINT custom_oauth_providers_jwks_uri_length CHECK (
        jwks_uri IS NULL OR char_length(jwks_uri) <= 2048
    ),
    CONSTRAINT custom_oauth_providers_client_id_length CHECK (
        char_length(client_id) >= 1 AND char_length(client_id) <= 512
    )
);

ALTER TABLE auth.custom_oauth_providers
    ADD COLUMN IF NOT EXISTS custom_claims_allowlist text[] NOT NULL DEFAULT '{}';

CREATE INDEX IF NOT EXISTS custom_oauth_providers_identifier_idx
    ON auth.custom_oauth_providers (identifier);
CREATE INDEX IF NOT EXISTS custom_oauth_providers_provider_type_idx
    ON auth.custom_oauth_providers (provider_type);
CREATE INDEX IF NOT EXISTS custom_oauth_providers_enabled_idx
    ON auth.custom_oauth_providers (enabled);
CREATE INDEX IF NOT EXISTS custom_oauth_providers_created_at_idx
    ON auth.custom_oauth_providers (created_at);
