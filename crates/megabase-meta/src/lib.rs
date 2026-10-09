// Megabase Meta - Postgres Meta-compatible database introspection API
// Ported from Postgres Meta (Apache-2.0 license) - see NOTICE
// Upstream: vendor/postgres-meta

use axum::{
    extract::Path,
    http::Method,
    routing::{any, get},
    Router,
};
use megabase_core::MegabaseNotImplemented;

pub fn router() -> Router {
    Router::new()
        .route("/", get(root_handler))
        .route("/health", get(health_handler))
        .route("/config", any(config_handler))
        .route("/config/version", get(version_handler))
        .route("/schemas", any(schemas_handler))
        .route("/schemas/{schema_name}", any(schema_handler))
        .route("/tables", any(tables_handler))
        .route("/tables/{table_id}", any(table_handler))
        .route("/columns", any(columns_handler))
        .route("/columns/{column_id}", any(column_handler))
        .route("/functions", any(functions_handler))
        .route("/functions/{function_id}", any(function_handler))
        .route("/policies", any(policies_handler))
        .route("/policies/{policy_id}", any(policy_handler))
        .route("/roles", any(roles_handler))
        .route("/roles/{role_id}", any(role_handler))
        .route("/types", any(types_handler))
        .route("/types/{type_id}", any(type_handler))
        .route("/triggers", any(triggers_handler))
        .route("/triggers/{trigger_id}", any(trigger_handler))
        .route("/extensions", any(extensions_handler))
        .route("/extensions/{extension_id}", any(extension_handler))
        .route("/publications", any(publications_handler))
        .route("/publications/{publication_id}", any(publication_handler))
        .route("/query", any(query_handler))
        .route("/generators/typescript", any(typescript_handler))
        .fallback(fallback_handler)
}

async fn root_handler() -> MegabaseNotImplemented {
    MegabaseNotImplemented::meta("GET /pg/")
}

async fn health_handler() -> MegabaseNotImplemented {
    MegabaseNotImplemented::meta("GET /pg/health")
}

async fn config_handler(method: Method) -> MegabaseNotImplemented {
    MegabaseNotImplemented::meta(format!("{} /pg/config", method))
}

async fn version_handler() -> MegabaseNotImplemented {
    MegabaseNotImplemented::meta("GET /pg/config/version")
}

async fn schemas_handler(method: Method) -> MegabaseNotImplemented {
    MegabaseNotImplemented::meta(format!("{} /pg/schemas", method))
}

async fn schema_handler(method: Method, Path(schema_name): Path<String>) -> MegabaseNotImplemented {
    MegabaseNotImplemented::meta(format!("{} /pg/schemas/{}", method, schema_name))
}

async fn tables_handler(method: Method) -> MegabaseNotImplemented {
    MegabaseNotImplemented::meta(format!("{} /pg/tables", method))
}

async fn table_handler(method: Method, Path(table_id): Path<String>) -> MegabaseNotImplemented {
    MegabaseNotImplemented::meta(format!("{} /pg/tables/{}", method, table_id))
}

async fn columns_handler(method: Method) -> MegabaseNotImplemented {
    MegabaseNotImplemented::meta(format!("{} /pg/columns", method))
}

async fn column_handler(method: Method, Path(column_id): Path<String>) -> MegabaseNotImplemented {
    MegabaseNotImplemented::meta(format!("{} /pg/columns/{}", method, column_id))
}

async fn functions_handler(method: Method) -> MegabaseNotImplemented {
    MegabaseNotImplemented::meta(format!("{} /pg/functions", method))
}

async fn function_handler(
    method: Method,
    Path(function_id): Path<String>,
) -> MegabaseNotImplemented {
    MegabaseNotImplemented::meta(format!("{} /pg/functions/{}", method, function_id))
}

async fn policies_handler(method: Method) -> MegabaseNotImplemented {
    MegabaseNotImplemented::meta(format!("{} /pg/policies", method))
}

async fn policy_handler(method: Method, Path(policy_id): Path<String>) -> MegabaseNotImplemented {
    MegabaseNotImplemented::meta(format!("{} /pg/policies/{}", method, policy_id))
}

async fn roles_handler(method: Method) -> MegabaseNotImplemented {
    MegabaseNotImplemented::meta(format!("{} /pg/roles", method))
}

async fn role_handler(method: Method, Path(role_id): Path<String>) -> MegabaseNotImplemented {
    MegabaseNotImplemented::meta(format!("{} /pg/roles/{}", method, role_id))
}

async fn types_handler(method: Method) -> MegabaseNotImplemented {
    MegabaseNotImplemented::meta(format!("{} /pg/types", method))
}

async fn type_handler(method: Method, Path(type_id): Path<String>) -> MegabaseNotImplemented {
    MegabaseNotImplemented::meta(format!("{} /pg/types/{}", method, type_id))
}

async fn triggers_handler(method: Method) -> MegabaseNotImplemented {
    MegabaseNotImplemented::meta(format!("{} /pg/triggers", method))
}

async fn trigger_handler(method: Method, Path(trigger_id): Path<String>) -> MegabaseNotImplemented {
    MegabaseNotImplemented::meta(format!("{} /pg/triggers/{}", method, trigger_id))
}

async fn extensions_handler(method: Method) -> MegabaseNotImplemented {
    MegabaseNotImplemented::meta(format!("{} /pg/extensions", method))
}

async fn extension_handler(
    method: Method,
    Path(extension_id): Path<String>,
) -> MegabaseNotImplemented {
    MegabaseNotImplemented::meta(format!("{} /pg/extensions/{}", method, extension_id))
}

async fn publications_handler(method: Method) -> MegabaseNotImplemented {
    MegabaseNotImplemented::meta(format!("{} /pg/publications", method))
}

async fn publication_handler(
    method: Method,
    Path(publication_id): Path<String>,
) -> MegabaseNotImplemented {
    MegabaseNotImplemented::meta(format!("{} /pg/publications/{}", method, publication_id))
}

async fn query_handler(method: Method) -> MegabaseNotImplemented {
    MegabaseNotImplemented::meta(format!("{} /pg/query", method))
}

async fn typescript_handler(method: Method) -> MegabaseNotImplemented {
    MegabaseNotImplemented::meta(format!("{} /pg/generators/typescript", method))
}

async fn fallback_handler(method: Method, uri: axum::http::Uri) -> MegabaseNotImplemented {
    MegabaseNotImplemented::meta(format!("{} /pg{}", method, uri.path()))
}
