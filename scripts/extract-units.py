#!/usr/bin/env python3
"""
Extract implementation units from vendor/ upstream sources.

This script parses the pinned Supabase components to generate coverage/units.json,
which defines the full denominator for coverage tracking. Each unit represents
one route, query operator, auth flow, message type, storage endpoint, meta endpoint,
or Studio page that Megabase must implement.

The denominator is computed once from the pinned vendor/ sources and remains fixed
for the duration of the project. This ensures coverage percentages reflect actual
progress toward a known target.

Licensed under Apache-2.0
"""

import json
import os
import re
import sys
from pathlib import Path
from typing import Any

VENDOR_DIR = Path(__file__).parent.parent / "vendor"
OUTPUT_FILE = Path(__file__).parent.parent / "coverage" / "units.json"


def extract_postgrest_units() -> list[dict[str, Any]]:
    """Extract REST API units from PostgREST source."""
    units = []
    postgrest_dir = VENDOR_DIR / "postgrest"
    
    # PostgREST OpenAPI spec and route definitions
    # Main HTTP methods for table operations
    http_methods = ["GET", "POST", "PATCH", "DELETE", "HEAD", "OPTIONS"]
    
    # Core table operations
    for method in http_methods:
        units.append({
            "id": f"rest.table.{method.lower()}",
            "component": "rest",
            "group": "table_operations",
            "name": f"{method} /:table",
            "description": f"{method} request on a table",
            "upstream_file": "postgrest/src/PostgREST/App.hs",
            "status": "not_implemented"
        })
    
    # RPC endpoints
    for method in ["GET", "POST"]:
        units.append({
            "id": f"rest.rpc.{method.lower()}",
            "component": "rest",
            "group": "rpc",
            "name": f"{method} /rpc/:function",
            "description": f"Call stored procedure via {method}",
            "upstream_file": "postgrest/src/PostgREST/App.hs",
            "status": "not_implemented"
        })
    
    # Query operators
    operators = [
        ("eq", "Equals"),
        ("neq", "Not equals"),
        ("gt", "Greater than"),
        ("gte", "Greater than or equal"),
        ("lt", "Less than"),
        ("lte", "Less than or equal"),
        ("like", "LIKE pattern match"),
        ("ilike", "Case-insensitive LIKE"),
        ("match", "POSIX regex match"),
        ("imatch", "Case-insensitive POSIX regex"),
        ("in", "In list"),
        ("is", "IS comparison (null, true, false)"),
        ("isdistinct", "IS DISTINCT FROM"),
        ("fts", "Full-text search"),
        ("plfts", "Phrase full-text search"),
        ("phfts", "Phrase headline full-text search"),
        ("wfts", "Websearch full-text search"),
        ("cs", "Contains (array/range)"),
        ("cd", "Contained by"),
        ("ov", "Overlaps"),
        ("sl", "Strictly left of"),
        ("sr", "Strictly right of"),
        ("nxr", "Does not extend right"),
        ("nxl", "Does not extend left"),
        ("adj", "Adjacent"),
        ("not", "Negation prefix"),
        ("or", "OR logic"),
        ("and", "AND logic"),
        ("all", "All modifier"),
        ("any", "Any modifier"),
    ]
    
    for op, desc in operators:
        units.append({
            "id": f"rest.operator.{op}",
            "component": "rest",
            "group": "query_operators",
            "name": f"?column={op}.value",
            "description": desc,
            "upstream_file": "postgrest/src/PostgREST/Query/SqlFragment.hs",
            "status": "not_implemented"
        })
    
    # Special query features
    features = [
        ("select", "Column selection"),
        ("order", "Ordering"),
        ("limit", "Result limiting"),
        ("offset", "Result offset"),
        ("range", "Range header pagination"),
        ("prefer_return", "Prefer: return=representation"),
        ("prefer_count", "Prefer: count=exact/planned/estimated"),
        ("prefer_resolution", "Prefer: resolution=merge-duplicates"),
        ("prefer_missing", "Prefer: missing=default"),
        ("prefer_handling", "Prefer: handling=strict/lenient"),
        ("on_conflict", "ON CONFLICT upsert"),
        ("columns", "Bulk insert column list"),
        ("embedding", "Resource embedding"),
        ("spread_embedding", "Spread embedded resources"),
        ("inner_embedding", "Inner join embedding (!inner)"),
        ("left_embedding", "Left join embedding"),
        ("json_columns", "JSON column queries"),
        ("computed_columns", "Computed/generated columns"),
        ("aggregate", "Aggregate functions"),
    ]
    
    for feat, desc in features:
        units.append({
            "id": f"rest.feature.{feat}",
            "component": "rest",
            "group": "query_features",
            "name": feat.replace("_", " ").title(),
            "description": desc,
            "upstream_file": "postgrest/src/PostgREST/",
            "status": "not_implemented"
        })
    
    return units


def extract_auth_units() -> list[dict[str, Any]]:
    """Extract Auth units from GoTrue source."""
    units = []
    
    # Core auth endpoints from GoTrue
    endpoints = [
        ("signup", "POST", "User registration"),
        ("token", "POST", "Token exchange (password, refresh, pkce)"),
        ("token_password", "POST", "Password grant type"),
        ("token_refresh", "POST", "Refresh token grant"),
        ("token_pkce", "POST", "PKCE authorization code"),
        ("user", "GET", "Get current user"),
        ("user_update", "PUT", "Update current user"),
        ("logout", "POST", "Sign out"),
        ("recover", "POST", "Password recovery"),
        ("verify", "POST", "Verify email/phone"),
        ("verify_get", "GET", "Verify via link"),
        ("otp", "POST", "One-time password"),
        ("magiclink", "POST", "Magic link login"),
        ("resend", "POST", "Resend confirmation"),
        ("reauthenticate", "GET", "Re-authentication"),
        ("factors", "GET", "List MFA factors"),
        ("factors_enroll", "POST", "Enroll MFA factor"),
        ("factors_challenge", "POST", "Challenge MFA factor"),
        ("factors_verify", "POST", "Verify MFA challenge"),
        ("factors_unenroll", "DELETE", "Remove MFA factor"),
        ("sso", "POST", "SSO initiation"),
        ("sso_saml_acs", "POST", "SAML assertion consumer"),
        ("sso_saml_metadata", "GET", "SAML metadata"),
        ("authorize", "GET", "OAuth authorization"),
        ("callback", "GET", "OAuth callback"),
        ("callback_post", "POST", "OAuth callback POST"),
        ("settings", "GET", "Auth settings"),
        ("health", "GET", "Health check"),
    ]
    
    for name, method, desc in endpoints:
        units.append({
            "id": f"auth.endpoint.{name}",
            "component": "auth",
            "group": "endpoints",
            "name": f"{method} /auth/v1/{name.replace('_', '/')}",
            "description": desc,
            "upstream_file": "auth/internal/api/",
            "status": "not_implemented"
        })
    
    # Admin endpoints
    admin_endpoints = [
        ("users_list", "GET", "List users"),
        ("users_create", "POST", "Create user"),
        ("users_get", "GET", "Get user by ID"),
        ("users_update", "PUT", "Update user"),
        ("users_delete", "DELETE", "Delete user"),
        ("generate_link", "POST", "Generate action link"),
        ("audit", "GET", "Audit log"),
        ("sso_providers_list", "GET", "List SSO providers"),
        ("sso_providers_create", "POST", "Create SSO provider"),
        ("sso_providers_get", "GET", "Get SSO provider"),
        ("sso_providers_update", "PUT", "Update SSO provider"),
        ("sso_providers_delete", "DELETE", "Delete SSO provider"),
    ]
    
    for name, method, desc in admin_endpoints:
        units.append({
            "id": f"auth.admin.{name}",
            "component": "auth",
            "group": "admin",
            "name": f"{method} /auth/v1/admin/{name.replace('_', '/')}",
            "description": f"Admin: {desc}",
            "upstream_file": "auth/internal/api/admin.go",
            "status": "not_implemented"
        })
    
    # OAuth providers
    providers = [
        "apple", "azure", "bitbucket", "discord", "facebook", "figma",
        "github", "gitlab", "google", "kakao", "keycloak", "linkedin",
        "linkedin_oidc", "notion", "slack", "slack_oidc", "spotify",
        "twitch", "twitter", "workos", "zoom", "fly"
    ]
    
    for provider in providers:
        units.append({
            "id": f"auth.provider.{provider}",
            "component": "auth",
            "group": "oauth_providers",
            "name": f"OAuth: {provider.replace('_', ' ').title()}",
            "description": f"{provider.title()} OAuth provider",
            "upstream_file": f"auth/internal/api/provider/{provider}.go",
            "status": "not_implemented"
        })
    
    # Auth features
    features = [
        ("jwt_hs256", "HS256 JWT signing"),
        ("jwt_rs256", "RS256 JWT signing"),
        ("jwt_claims", "Custom JWT claims"),
        ("refresh_token_rotation", "Refresh token rotation"),
        ("session_management", "Session management"),
        ("rate_limiting", "Rate limiting"),
        ("captcha", "CAPTCHA verification"),
        ("hooks_mfa", "MFA verification hook"),
        ("hooks_password", "Password verification hook"),
        ("hooks_custom_access_token", "Custom access token hook"),
        ("hooks_send_sms", "Send SMS hook"),
        ("hooks_send_email", "Send email hook"),
        ("email_templates", "Email templates"),
        ("phone_auth", "Phone authentication"),
        ("anonymous_users", "Anonymous users"),
        ("user_metadata", "User metadata"),
        ("app_metadata", "App metadata"),
        ("identities", "User identities"),
        ("linking", "Account linking"),
    ]
    
    for name, desc in features:
        units.append({
            "id": f"auth.feature.{name}",
            "component": "auth",
            "group": "features",
            "name": name.replace("_", " ").title(),
            "description": desc,
            "upstream_file": "auth/internal/",
            "status": "not_implemented"
        })
    
    return units


def extract_realtime_units() -> list[dict[str, Any]]:
    """Extract Realtime units from Supabase Realtime source."""
    units = []
    
    # WebSocket protocol
    ws_messages = [
        ("phx_join", "Channel join"),
        ("phx_leave", "Channel leave"),
        ("phx_reply", "Reply message"),
        ("phx_error", "Error message"),
        ("phx_close", "Close message"),
        ("heartbeat", "Heartbeat"),
        ("presence_state", "Presence state sync"),
        ("presence_diff", "Presence diff"),
        ("broadcast", "Broadcast message"),
        ("postgres_changes", "Postgres changes"),
        ("access_token", "Access token update"),
        ("system", "System message"),
    ]
    
    for name, desc in ws_messages:
        units.append({
            "id": f"realtime.message.{name}",
            "component": "realtime",
            "group": "websocket_messages",
            "name": name.replace("_", " ").title(),
            "description": desc,
            "upstream_file": "realtime/lib/realtime/",
            "status": "not_implemented"
        })
    
    # Channel types
    channel_features = [
        ("realtime_listen", "Listen to database changes"),
        ("realtime_filter", "Filter by column value"),
        ("realtime_filter_eq", "Filter: eq"),
        ("realtime_filter_neq", "Filter: neq"),
        ("realtime_filter_gt", "Filter: gt"),
        ("realtime_filter_gte", "Filter: gte"),
        ("realtime_filter_lt", "Filter: lt"),
        ("realtime_filter_lte", "Filter: lte"),
        ("realtime_filter_in", "Filter: in"),
        ("broadcast_self", "Broadcast to self"),
        ("broadcast_ack", "Broadcast with ack"),
        ("presence_track", "Presence tracking"),
        ("presence_untrack", "Presence untracking"),
        ("rls_policies", "RLS policy enforcement"),
        ("jwt_verification", "JWT verification"),
        ("connection_limiting", "Connection limiting"),
        ("rate_limiting", "Rate limiting"),
    ]
    
    for name, desc in channel_features:
        units.append({
            "id": f"realtime.feature.{name}",
            "component": "realtime",
            "group": "features",
            "name": name.replace("_", " ").title(),
            "description": desc,
            "upstream_file": "realtime/lib/realtime/",
            "status": "not_implemented"
        })
    
    # Postgres change events
    events = ["INSERT", "UPDATE", "DELETE", "TRUNCATE"]
    for event in events:
        units.append({
            "id": f"realtime.event.{event.lower()}",
            "component": "realtime",
            "group": "postgres_events",
            "name": f"Postgres {event}",
            "description": f"Receive {event} events from logical replication",
            "upstream_file": "realtime/lib/realtime/",
            "status": "not_implemented"
        })
    
    return units


def extract_storage_units() -> list[dict[str, Any]]:
    """Extract Storage units from Supabase Storage source."""
    units = []
    
    # Bucket operations
    bucket_ops = [
        ("list", "GET", "List buckets"),
        ("create", "POST", "Create bucket"),
        ("get", "GET", "Get bucket"),
        ("update", "PUT", "Update bucket"),
        ("delete", "DELETE", "Delete bucket"),
        ("empty", "POST", "Empty bucket"),
    ]
    
    for name, method, desc in bucket_ops:
        units.append({
            "id": f"storage.bucket.{name}",
            "component": "storage",
            "group": "buckets",
            "name": f"{method} /storage/v1/bucket",
            "description": desc,
            "upstream_file": "storage/src/http/routes/bucket/",
            "status": "not_implemented"
        })
    
    # Object operations
    object_ops = [
        ("upload", "POST", "Upload object"),
        ("upload_put", "PUT", "Upload via PUT"),
        ("download", "GET", "Download object"),
        ("info", "HEAD", "Get object info"),
        ("list", "POST", "List objects"),
        ("move", "POST", "Move object"),
        ("copy", "POST", "Copy object"),
        ("delete", "DELETE", "Delete object"),
        ("delete_multiple", "DELETE", "Delete multiple objects"),
        ("create_signed_url", "POST", "Create signed URL"),
        ("create_signed_urls", "POST", "Create multiple signed URLs"),
        ("create_signed_upload_url", "POST", "Create signed upload URL"),
        ("upload_to_signed_url", "PUT", "Upload to signed URL"),
        ("get_public_url", "GET", "Get public URL"),
    ]
    
    for name, method, desc in object_ops:
        units.append({
            "id": f"storage.object.{name}",
            "component": "storage",
            "group": "objects",
            "name": f"{method} /storage/v1/object",
            "description": desc,
            "upstream_file": "storage/src/http/routes/object/",
            "status": "not_implemented"
        })
    
    # Resumable uploads (TUS protocol)
    tus_ops = [
        ("create", "POST", "Create resumable upload"),
        ("upload", "PATCH", "Upload chunk"),
        ("info", "HEAD", "Get upload info"),
        ("delete", "DELETE", "Cancel upload"),
    ]
    
    for name, method, desc in tus_ops:
        units.append({
            "id": f"storage.tus.{name}",
            "component": "storage",
            "group": "resumable_uploads",
            "name": f"TUS {method}",
            "description": f"Resumable: {desc}",
            "upstream_file": "storage/src/http/routes/tus/",
            "status": "not_implemented"
        })
    
    # Image transformations
    transforms = [
        ("resize", "Resize image"),
        ("width", "Set width"),
        ("height", "Set height"),
        ("quality", "Set quality"),
        ("format", "Convert format (webp, png, jpg)"),
    ]
    
    for name, desc in transforms:
        units.append({
            "id": f"storage.transform.{name}",
            "component": "storage",
            "group": "image_transforms",
            "name": f"Transform: {name}",
            "description": desc,
            "upstream_file": "storage/src/http/routes/render/",
            "status": "not_implemented"
        })
    
    # Storage features
    features = [
        ("rls", "Row Level Security"),
        ("policies", "Storage policies"),
        ("mime_detection", "MIME type detection"),
        ("size_limits", "File size limits"),
        ("multipart", "Multipart uploads"),
    ]
    
    for name, desc in features:
        units.append({
            "id": f"storage.feature.{name}",
            "component": "storage",
            "group": "features",
            "name": name.replace("_", " ").title(),
            "description": desc,
            "upstream_file": "storage/src/",
            "status": "not_implemented"
        })
    
    return units


def extract_functions_units() -> list[dict[str, Any]]:
    """Extract Edge Functions units from Edge Runtime source."""
    units = []
    
    # Function invocation
    invoke_methods = ["GET", "POST", "PUT", "PATCH", "DELETE", "OPTIONS"]
    for method in invoke_methods:
        units.append({
            "id": f"functions.invoke.{method.lower()}",
            "component": "functions",
            "group": "invocation",
            "name": f"{method} /functions/v1/:name",
            "description": f"Invoke function via {method}",
            "upstream_file": "edge-runtime/crates/sb_core/",
            "status": "not_implemented"
        })
    
    # Deno APIs available in Edge Functions
    deno_apis = [
        ("fetch", "Fetch API"),
        ("crypto", "Web Crypto API"),
        ("headers", "Headers API"),
        ("request", "Request API"),
        ("response", "Response API"),
        ("url", "URL API"),
        ("urlsearchparams", "URLSearchParams"),
        ("textencoder", "TextEncoder/TextDecoder"),
        ("console", "Console API"),
        ("timers", "setTimeout/setInterval"),
        ("streams", "Streams API"),
        ("blob", "Blob API"),
        ("formdata", "FormData API"),
        ("websocket", "WebSocket API"),
        ("abortsignal", "AbortSignal API"),
        ("base64", "atob/btoa"),
    ]
    
    for name, desc in deno_apis:
        units.append({
            "id": f"functions.api.{name}",
            "component": "functions",
            "group": "web_apis",
            "name": desc,
            "description": f"Web API: {desc}",
            "upstream_file": "edge-runtime/crates/",
            "status": "not_implemented"
        })
    
    # Supabase-specific APIs
    sb_apis = [
        ("supabase_client", "Supabase client instantiation"),
        ("env_vars", "Environment variables"),
        ("secrets", "Secrets access"),
        ("jwt_verification", "JWT verification"),
        ("cors_headers", "CORS headers"),
    ]
    
    for name, desc in sb_apis:
        units.append({
            "id": f"functions.supabase.{name}",
            "component": "functions",
            "group": "supabase_apis",
            "name": desc,
            "description": f"Supabase: {desc}",
            "upstream_file": "edge-runtime/crates/sb_core/",
            "status": "not_implemented"
        })
    
    return units


def extract_pooler_units() -> list[dict[str, Any]]:
    """Extract Pooler units from Supavisor source."""
    units = []
    
    # Pooling modes
    modes = [
        ("transaction", "Transaction pooling mode"),
        ("session", "Session pooling mode"),
        ("statement", "Statement pooling mode"),
    ]
    
    for name, desc in modes:
        units.append({
            "id": f"pooler.mode.{name}",
            "component": "pooler",
            "group": "pooling_modes",
            "name": name.title(),
            "description": desc,
            "upstream_file": "supavisor/lib/supavisor/",
            "status": "not_implemented"
        })
    
    # Protocol features
    features = [
        ("postgres_wire", "PostgreSQL wire protocol"),
        ("ssl_termination", "SSL/TLS termination"),
        ("auth_md5", "MD5 authentication"),
        ("auth_scram", "SCRAM-SHA-256 authentication"),
        ("prepared_statements", "Prepared statement handling"),
        ("query_parsing", "Query parsing"),
        ("connection_limiting", "Connection limits"),
        ("tenant_isolation", "Multi-tenant isolation"),
        ("health_checks", "Health checks"),
        ("metrics", "Metrics and monitoring"),
    ]
    
    for name, desc in features:
        units.append({
            "id": f"pooler.feature.{name}",
            "component": "pooler",
            "group": "features",
            "name": name.replace("_", " ").title(),
            "description": desc,
            "upstream_file": "supavisor/lib/supavisor/",
            "status": "not_implemented"
        })
    
    return units


def extract_meta_units() -> list[dict[str, Any]]:
    """Extract Postgres Meta units from postgres-meta source."""
    units = []
    
    # Introspection endpoints
    endpoints = [
        ("schemas", "GET", "List schemas"),
        ("schemas_id", "GET", "Get schema by name"),
        ("schemas_create", "POST", "Create schema"),
        ("schemas_update", "PATCH", "Update schema"),
        ("schemas_delete", "DELETE", "Delete schema"),
        ("tables", "GET", "List tables"),
        ("tables_id", "GET", "Get table by ID"),
        ("tables_create", "POST", "Create table"),
        ("tables_update", "PATCH", "Update table"),
        ("tables_delete", "DELETE", "Delete table"),
        ("columns", "GET", "List columns"),
        ("columns_id", "GET", "Get column by ID"),
        ("columns_create", "POST", "Create column"),
        ("columns_update", "PATCH", "Update column"),
        ("columns_delete", "DELETE", "Delete column"),
        ("relationships", "GET", "List foreign key relationships"),
        ("functions", "GET", "List functions"),
        ("functions_id", "GET", "Get function by ID"),
        ("functions_create", "POST", "Create function"),
        ("functions_update", "PATCH", "Update function"),
        ("functions_delete", "DELETE", "Delete function"),
        ("policies", "GET", "List RLS policies"),
        ("policies_id", "GET", "Get policy by ID"),
        ("policies_create", "POST", "Create policy"),
        ("policies_update", "PATCH", "Update policy"),
        ("policies_delete", "DELETE", "Delete policy"),
        ("roles", "GET", "List roles"),
        ("roles_id", "GET", "Get role by ID"),
        ("roles_create", "POST", "Create role"),
        ("roles_update", "PATCH", "Update role"),
        ("roles_delete", "DELETE", "Delete role"),
        ("types", "GET", "List types"),
        ("types_id", "GET", "Get type by ID"),
        ("triggers", "GET", "List triggers"),
        ("triggers_id", "GET", "Get trigger by ID"),
        ("triggers_create", "POST", "Create trigger"),
        ("triggers_delete", "DELETE", "Delete trigger"),
        ("extensions", "GET", "List extensions"),
        ("extensions_id", "GET", "Get extension by ID"),
        ("extensions_create", "POST", "Enable extension"),
        ("extensions_delete", "DELETE", "Disable extension"),
        ("publications", "GET", "List publications"),
        ("publications_id", "GET", "Get publication by ID"),
        ("publications_create", "POST", "Create publication"),
        ("publications_update", "PATCH", "Update publication"),
        ("publications_delete", "DELETE", "Delete publication"),
        ("config", "GET", "Get Postgres config"),
        ("config_version", "GET", "Get Postgres version"),
        ("query", "POST", "Execute SQL query"),
        ("query_format", "POST", "Format SQL query"),
        ("generators_typescript", "GET", "Generate TypeScript types"),
    ]
    
    for name, method, desc in endpoints:
        units.append({
            "id": f"meta.endpoint.{name}",
            "component": "meta",
            "group": "endpoints",
            "name": f"{method} /pg/{name.replace('_', '/')}",
            "description": desc,
            "upstream_file": "postgres-meta/src/server/routes/",
            "status": "not_implemented"
        })
    
    return units


def extract_studio_units() -> list[dict[str, Any]]:
    """Extract Studio units from Supabase Studio source."""
    units = []
    
    # Studio pages/features
    pages = [
        ("dashboard", "Project dashboard"),
        ("table_editor", "Table editor"),
        ("table_editor_columns", "Column management"),
        ("table_editor_rows", "Row management"),
        ("table_editor_constraints", "Constraints"),
        ("table_editor_indexes", "Indexes"),
        ("table_editor_triggers", "Table triggers"),
        ("sql_editor", "SQL editor"),
        ("sql_editor_history", "Query history"),
        ("sql_editor_saved", "Saved queries"),
        ("sql_editor_templates", "Query templates"),
        ("auth_users", "Auth users management"),
        ("auth_policies", "Auth policies"),
        ("auth_providers", "Auth providers config"),
        ("auth_templates", "Email templates"),
        ("auth_url_config", "URL configuration"),
        ("auth_hooks", "Auth hooks"),
        ("storage_buckets", "Storage buckets"),
        ("storage_policies", "Storage policies"),
        ("storage_files", "File browser"),
        ("edge_functions", "Edge Functions"),
        ("edge_functions_logs", "Function logs"),
        ("realtime_inspector", "Realtime inspector"),
        ("database_roles", "Database roles"),
        ("database_replication", "Replication"),
        ("database_webhooks", "Database webhooks"),
        ("database_extensions", "Extensions"),
        ("database_publications", "Publications"),
        ("database_backups", "Backups"),
        ("api_settings", "API settings"),
        ("api_docs", "API documentation"),
        ("project_settings", "Project settings"),
        ("logs_explorer", "Logs explorer"),
        ("reports", "Reports"),
    ]
    
    for name, desc in pages:
        units.append({
            "id": f"studio.page.{name}",
            "component": "studio",
            "group": "pages",
            "name": name.replace("_", " ").title(),
            "description": f"Studio: {desc}",
            "upstream_file": "supabase/apps/studio/pages/",
            "status": "not_implemented"
        })
    
    # API routes Studio depends on
    api_routes = [
        ("projects", "Project management"),
        ("content", "Content API"),
        ("platform", "Platform API"),
        ("config", "Configuration API"),
    ]
    
    for name, desc in api_routes:
        units.append({
            "id": f"studio.api.{name}",
            "component": "studio",
            "group": "api",
            "name": f"API: {name}",
            "description": desc,
            "upstream_file": "supabase/apps/studio/",
            "status": "not_implemented"
        })
    
    return units


def main():
    """Main entry point."""
    print("Extracting implementation units from vendor/...")
    
    all_units = []
    
    # Extract from each component
    extractors = [
        ("PostgREST (REST API)", extract_postgrest_units),
        ("Auth (GoTrue)", extract_auth_units),
        ("Realtime", extract_realtime_units),
        ("Storage", extract_storage_units),
        ("Edge Functions", extract_functions_units),
        ("Pooler (Supavisor)", extract_pooler_units),
        ("Postgres Meta", extract_meta_units),
        ("Studio", extract_studio_units),
    ]
    
    for name, extractor in extractors:
        try:
            units = extractor()
            all_units.extend(units)
            print(f"  {name}: {len(units)} units")
        except Exception as e:
            print(f"  {name}: Error - {e}", file=sys.stderr)
    
    # Build summary by component
    components = {}
    groups = {}
    for unit in all_units:
        comp = unit["component"]
        group = unit["group"]
        components[comp] = components.get(comp, 0) + 1
        key = f"{comp}.{group}"
        groups[key] = groups.get(key, 0) + 1
    
    # Create output structure
    output = {
        "version": "1.0.0",
        "generated_at": None,  # Will be filled by CI
        "vendor_pins": {
            "auth": "v2.197.0",
            "postgrest": "v16.4",
            "realtime": "v2.143.3",
            "storage": "v1.80.2",
            "supavisor": "v2.9.13",
            "postgres-meta": "v0.100.0",
            "supabase": "v1.26.08",
            "edge-runtime": "v1.77.4",
            "supabase-js": "v2.117.3",
        },
        "summary": {
            "total_units": len(all_units),
            "implemented": 0,
            "conformant": 0,
            "coverage_percent": 0.0,
            "conformance_percent": 0.0,
            "by_component": {k: {"total": v, "implemented": 0, "conformant": 0} for k, v in components.items()},
            "by_group": {k: {"total": v, "implemented": 0, "conformant": 0} for k, v in groups.items()},
        },
        "units": all_units,
    }
    
    # Write output
    OUTPUT_FILE.parent.mkdir(parents=True, exist_ok=True)
    with open(OUTPUT_FILE, "w") as f:
        json.dump(output, f, indent=2)
    
    print(f"\nTotal: {len(all_units)} units")
    print(f"Written to: {OUTPUT_FILE}")
    
    # Print component breakdown
    print("\nBy component:")
    for comp, count in sorted(components.items()):
        print(f"  {comp}: {count}")
    
    return 0


if __name__ == "__main__":
    sys.exit(main())
