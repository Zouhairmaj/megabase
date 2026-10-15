// Ported from supabase/auth internal/api/external.go and
// internal/api/external_oauth.go (MIT), pin v2.197.0.

//! `GET /authorize` (`ExternalProviderRedirect`), and `GET` and
//! `POST /callback` (`ExternalProviderCallback`) up to the provider token
//! exchange.
//!
//! The `loadFlowState` middleware resolves `state` to an `auth.flow_state`
//! row. When that fails GoTrue writes a 303 to the site URL with the error in
//! the query, then also writes its JSON error into the same response: the
//! status and headers are already sent, so only the body bytes land after the
//! redirect. Errors after that point go through `redirectErrors`: a 302 to the
//! callback redirect URL with the error in the query and in the fragment.
//! Exchanging the code with an enabled provider is not ported and stays 501.

use std::time::SystemTime;

use axum::body::Body;
use axum::extract::State;
use axum::http::{header, HeaderMap, Method, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use megabase_core::MegabaseNotImplemented;
use serde_json::json;
use uuid::Uuid;

use crate::admin_batch2::get_referrer;
use crate::error::{validation, GoTrueError};
use crate::routes::{read_body, validate_pkce};
use crate::state::AuthState;
use crate::store::FlowStateRecord;
use crate::user::{provider_decision, ProviderDecision};
use crate::verify::{html_escape, query_escape};
use crate::COMPONENT;

const UNIT_AUTHORIZE: &str = "auth:route:GET /auth/v1/authorize";
const UNIT_CALLBACK_GET: &str = "auth:route:GET /auth/v1/callback";
const UNIT_CALLBACK_POST: &str = "auth:route:POST /auth/v1/callback";

/// `reservedOAuthParams` (`custom_oauth_admin.go`). `/authorize` lets
/// `nonce` through.
const RESERVED_OAUTH_PARAMS: &[&str] = &[
    "client_id",
    "client_secret",
    "redirect_uri",
    "response_type",
    "state",
    "code_challenge",
    "code_challenge_method",
    "code_verifier",
    "nonce",
];

/// Go `url.Values` in insertion order.
pub(crate) type Values = Vec<(String, String)>;

/// `GET /authorize` (`ExternalProviderRedirect`).
///
/// An unknown, disabled or misconfigured provider is GoTrue's 400
/// `validation_failed` before `redirect_to` is read, so no redirect is ever
/// sent. A ported provider gets an `auth.flow_state` row whose id is the
/// OAuth `state`, then a 302 to the provider's authorize URL. Providers
/// without a ported constructor, `custom:` providers, `invite_token`, and a
/// redirect allow-list pattern the glob port does not cover stay 501.
pub(crate) async fn authorize(
    State(state): State<AuthState>,
    headers: HeaderMap,
    uri: Uri,
) -> Response {
    let not_ported = || MegabaseNotImplemented::new(COMPONENT, UNIT_AUTHORIZE).into_response();
    let query = parse_query(uri.query().unwrap_or(""));
    let provider_type = first(&query, "provider");
    let code_challenge = first(&query, "code_challenge");
    let code_challenge_method = first(&query, "code_challenge_method");
    let (config, email_optional) =
        match provider_decision(&state, provider_type, first(&query, "scopes")) {
            ProviderDecision::Reject(detail) => {
                return validation(format!("Unsupported provider: {detail}")).into_response()
            }
            ProviderDecision::Ready(None) => return not_ported(),
            ProviderDecision::Ready(Some(ready)) => ready,
        };
    // `FindUserByOneTimeToken` for an invite is not ported.
    if !first(&query, "invite_token").is_empty() {
        return not_ported();
    }
    let Ok(referrer) = get_referrer(&state, &headers, first(&query, "redirect_to")) else {
        return not_ported();
    };
    if let Err(error) = validate_pkce(code_challenge_method, code_challenge) {
        return error.into_response();
    }

    // Every other query parameter is passed on to the provider, first value
    // per key, except `scopes`, `provider` and the reserved OAuth names.
    let mut params = Values::new();
    let mut seen: Vec<&str> = Vec::new();
    for (key, value) in &query {
        if seen.contains(&key.as_str()) {
            continue;
        }
        seen.push(key);
        let lower = key.to_lowercase();
        if key == "scopes"
            || key == "provider"
            || (lower != "nonce" && RESERVED_OAUTH_PARAMS.contains(&lower.as_str()))
        {
            continue;
        }
        // See https://workos.com/docs/reference/sso/authorize/get
        let name = if key == "workos_provider" {
            "provider"
        } else {
            key
        };
        set(&mut params, name, value);
    }

    // `NewFlowState`: PKCE fields only with a code challenge.
    let (auth_code, challenge, method) = if code_challenge.is_empty() {
        (None, None, None)
    } else {
        let method = match code_challenge_method.to_lowercase().as_str() {
            "s256" => "s256",
            "plain" => "plain",
            _ => return validation("Invalid code_challenge_method").into_response(),
        };
        (
            Some(Uuid::new_v4().to_string()),
            Some(code_challenge.to_string()),
            Some(method.to_string()),
        )
    };
    let flow = FlowStateRecord {
        provider_type: provider_type.to_string(),
        auth_code,
        code_challenge: challenge,
        code_challenge_method: method,
        user_id: None,
        authentication_method: "oauth".into(),
        provider_access_token: String::new(),
        provider_refresh_token: String::new(),
        created_at: None,
        auth_code_issued_at: None,
        referrer: Some(referrer).filter(|referrer| !referrer.is_empty()),
        linking_target_id: None,
        email_optional,
    };
    let id = match state.backend.create_flow_state(&flow).await {
        Ok(id) => id,
        Err(error) => {
            tracing::error!(%error, "creating the OAuth flow state failed");
            return crate::error::unexpected("Error creating flow state").into_response();
        }
    };
    let location = config.auth_code_url(&id.to_string(), &params);
    // `http.Redirect` rewrites a URL without scheme or host against the
    // request path; that only happens with a scheme-less provider `URL`.
    if !has_scheme(&location) && !location.starts_with("//") {
        return not_ported();
    }
    redirect(&Method::GET, StatusCode::FOUND, &location, None)
}

/// `url.Parse` finds a scheme: `^[A-Za-z][A-Za-z0-9+.-]*:`.
fn has_scheme(url: &str) -> bool {
    let Some((scheme, _)) = url.split_once(':') else {
        return false;
    };
    let mut chars = scheme.chars();
    chars.next().is_some_and(|ch| ch.is_ascii_alphabetic())
        && chars.all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '+' | '.' | '-'))
}

pub(crate) async fn callback_get(State(state): State<AuthState>, uri: Uri) -> Response {
    let form = parse_query(uri.query().unwrap_or(""));
    callback(&state, &Method::GET, &form).await
}

pub(crate) async fn callback_post(
    State(state): State<AuthState>,
    uri: Uri,
    headers: HeaderMap,
    body: Body,
) -> Response {
    let bytes = match read_body(body).await {
        Ok(bytes) => bytes,
        Err(error) => return error.into_response(),
    };
    // `r.ParseForm` puts the urlencoded body before the query string.
    let mut form = match media_type(&headers).as_str() {
        "application/x-www-form-urlencoded" => parse_query(&String::from_utf8_lossy(&bytes)),
        "multipart/form-data" => {
            return MegabaseNotImplemented::new(COMPONENT, UNIT_CALLBACK_POST).into_response()
        }
        _ => Values::new(),
    };
    form.extend(parse_query(uri.query().unwrap_or("")));
    callback(&state, &Method::POST, &form).await
}

async fn callback(state: &AuthState, method: &Method, form: &Values) -> Response {
    let flow = match load_flow_state(state, form).await {
        Ok(flow) => flow,
        Err(error) => return state_error(state, method, &error),
    };
    let config = &state.config;
    let base = if !config.external_redirect_url.is_empty() {
        config.external_redirect_url.as_str()
    } else {
        flow.referrer
            .as_deref()
            .filter(|referrer| !referrer.is_empty())
            .unwrap_or(&config.site_url)
    };
    let Err(failure) = provider_callback(state, &flow, form) else {
        let unit = if method == Method::GET {
            UNIT_CALLBACK_GET
        } else {
            UNIT_CALLBACK_POST
        };
        return MegabaseNotImplemented::new(COMPONENT, unit).into_response();
    };
    let mut pairs = parse_query(url_query(base));
    let mut hash = Values::new();
    match &failure {
        Failure::Http(error) => {
            let oauth = oauth_error(error);
            set(&mut pairs, "error", oauth);
            set(&mut pairs, "error_description", &error.message);
            set(&mut pairs, "error_code", error.error_code);
            set(&mut hash, "error", oauth);
            if !error.message.is_empty() {
                set(&mut hash, "error_description", &error.message);
            }
            set(&mut hash, "error_code", error.error_code);
        }
        Failure::OAuth { error, description } => {
            set(&mut pairs, "error", error);
            set(&mut pairs, "error_description", description);
            set(&mut hash, "error", error);
            if !description.is_empty() {
                set(&mut hash, "error_description", description);
            }
        }
    }
    set(&mut hash, "sb", "");
    let url = format!(
        "{}?{}#{}",
        fragment_base(base),
        encode(&pairs),
        escape_fragment(&encode(&hash))
    );
    redirect(method, StatusCode::FOUND, &url, None)
}

/// `internalExternalProviderCallback` up to `GetOAuthToken`. `Ok` means the
/// request needs the unported provider exchange.
fn provider_callback(
    state: &AuthState,
    flow: &FlowStateRecord,
    form: &Values,
) -> Result<(), Failure> {
    // Twitter is OAuth 1.0 (`oAuth1Callback`), which reads other parameters.
    if flow.provider_type == "twitter" {
        return Ok(());
    }
    let error = first(form, "error");
    if !error.is_empty() {
        return Err(Failure::OAuth {
            error: error.to_string(),
            description: first(form, "error_description").to_string(),
        });
    }
    if first(form, "code").is_empty() {
        return Err(Failure::Http(GoTrueError::new(
            StatusCode::BAD_REQUEST,
            "bad_oauth_callback",
            "OAuth callback with missing authorization code missing",
        )));
    }
    match provider_decision(state, &flow.provider_type, "") {
        ProviderDecision::Reject(detail) => Err(Failure::Http(GoTrueError::new(
            StatusCode::BAD_REQUEST,
            "oauth_provider_not_supported",
            format!("Unsupported provider: {detail}"),
        ))),
        ProviderDecision::Ready(_) => Ok(()),
    }
}

enum Failure {
    Http(GoTrueError),
    OAuth { error: String, description: String },
}

/// `loadExternalState` and `loadExternalStateFromUUID`.
async fn load_flow_state(state: &AuthState, form: &Values) -> Result<FlowStateRecord, GoTrueError> {
    let raw = first(form, "state");
    if raw.is_empty() {
        return Err(GoTrueError::new(
            StatusCode::BAD_REQUEST,
            "bad_oauth_callback",
            "OAuth state parameter missing",
        ));
    }
    let Ok(id) = Uuid::parse_str(raw) else {
        return Err(bad_state("OAuth state parameter is invalid"));
    };
    let flow = match state.backend.find_flow_state(id).await {
        Ok(Some(flow)) => flow,
        Ok(None) => return Err(bad_state("OAuth state not found or expired")),
        Err(_) => return Err(crate::error::unexpected("Error loading flow state")),
    };
    if flow.is_expired(SystemTime::now()) {
        return Err(bad_state("OAuth state has expired"));
    }
    if flow.is_pkce() && flow.user_id.is_some() {
        return Err(GoTrueError::new(
            StatusCode::BAD_REQUEST,
            "flow_state_already_used",
            "State has already been used",
        ));
    }
    if let Some(target) = flow.linking_target_id {
        match state.backend.load_user(target).await {
            Ok(Some(_)) => {}
            Ok(None) => {
                return Err(GoTrueError::new(
                    StatusCode::UNPROCESSABLE_ENTITY,
                    "user_not_found",
                    "Linking target user not found",
                ))
            }
            Err(_) => return Err(crate::error::unexpected("Database error loading user")),
        }
    }
    Ok(flow)
}

fn bad_state(message: &str) -> GoTrueError {
    GoTrueError::new(StatusCode::BAD_REQUEST, "bad_oauth_state", message)
}

/// The `loadFlowState` failure: a 303 to the site URL, followed in the body
/// by the JSON error that `HandleResponseError` writes after the headers.
fn state_error(state: &AuthState, method: &Method, error: &GoTrueError) -> Response {
    let site = &state.config.site_url;
    if !(site.starts_with('/') || site.contains("://")) {
        return crate::error::unexpected("site url is improperly formatted").into_response();
    }
    // `url.ParseRequestURI` keeps any `#` in the query, so only `?` splits.
    let (base, query) = site.split_once('?').unwrap_or((site, ""));
    let mut pairs = parse_query(query);
    set(&mut pairs, "error", oauth_error(error));
    set(&mut pairs, "error_description", &error.message);
    set(&mut pairs, "error_code", error.error_code);
    let url = format!("{base}?{}", encode(&pairs));
    let body = json!({
        "code": error.status.as_u16(),
        "error_code": error.error_code,
        "msg": error.message,
    });
    redirect(method, StatusCode::SEE_OTHER, &url, Some(body.to_string()))
}

/// `getErrorQueryString` for an `HTTPError`.
fn oauth_error(error: &GoTrueError) -> &'static str {
    match error.error_code {
        "signup_disabled" | "user_banned" | "provider_email_needs_verification" => {
            return "access_denied"
        }
        _ => {}
    }
    match error.status.as_u16() {
        400 => "invalid_request",
        401 => "unauthorized_client",
        403 => "access_denied",
        500 => "server_error",
        503 => "temporarily_unavailable",
        _ => "server_error",
    }
}

/// Go `http.Redirect`: `GET` gets the short HTML body. `trailing` is written
/// after the headers were sent; Go sniffs `text/plain` for it on `POST`.
pub(crate) fn redirect(
    method: &Method,
    status: StatusCode,
    url: &str,
    trailing: Option<String>,
) -> Response {
    let mut builder = Response::builder()
        .status(status)
        .header(header::LOCATION, url);
    let mut body = String::new();
    if method == Method::GET {
        builder = builder.header(header::CONTENT_TYPE, "text/html; charset=utf-8");
        body = format!(
            "<a href=\"{}\">{}</a>.\n\n",
            html_escape(url),
            status.canonical_reason().unwrap_or_default()
        );
    }
    if let Some(trailing) = trailing {
        if method != Method::GET {
            builder = builder.header(header::CONTENT_TYPE, "text/plain; charset=utf-8");
        }
        body.push_str(&trailing);
    }
    builder
        .body(Body::from(body))
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

fn media_type(headers: &HeaderMap) -> String {
    headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase()
}

pub(crate) fn first<'a>(values: &'a Values, key: &str) -> &'a str {
    values
        .iter()
        .find(|(name, _)| name == key)
        .map_or("", |(_, value)| value.as_str())
}

/// `url.Values.Set`.
pub(crate) fn set(values: &mut Values, key: &str, value: &str) {
    values.retain(|(name, _)| name != key);
    values.push((key.to_string(), value.to_string()));
}

/// `url.Values.Encode`: keys sorted, values in order.
pub(crate) fn encode(values: &Values) -> String {
    let mut sorted: Vec<&(String, String)> = values.iter().collect();
    sorted.sort_by(|left, right| left.0.cmp(&right.0));
    sorted
        .into_iter()
        .map(|(key, value)| format!("{}={}", query_escape(key), query_escape(value)))
        .collect::<Vec<_>>()
        .join("&")
}

/// `url.ParseQuery`: pairs with a `;` or a bad escape are dropped.
pub(crate) fn parse_query(raw: &str) -> Values {
    raw.split('&')
        .filter(|pair| !pair.is_empty() && !pair.contains(';'))
        .filter_map(|pair| {
            let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
            Some((unescape(key)?, unescape(value)?))
        })
        .collect()
}

/// `url.QueryUnescape`.
fn unescape(input: &str) -> Option<String> {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'+' => out.push(b' '),
            b'%' => {
                let hex = input.get(index + 1..index + 3)?;
                out.push(u8::from_str_radix(hex, 16).ok()?);
                index += 2;
            }
            byte => out.push(byte),
        }
        index += 1;
    }
    Some(String::from_utf8_lossy(&out).into_owned())
}

/// Raw query of a URL, as `url.Parse` splits it off.
fn url_query(url: &str) -> &str {
    let rest = url.split_once('#').map_or(url, |(rest, _)| rest);
    rest.split_once('?').map_or("", |(_, query)| query)
}

/// The URL without its query and fragment.
fn fragment_base(url: &str) -> &str {
    let end = url.find(['?', '#']).unwrap_or(url.len());
    &url[..end]
}

/// `url.URL.EscapedFragment` for a fragment set from `url.Values.Encode`:
/// reserved characters and `!()*` stay, so in practice only `%` is escaped.
fn escape_fragment(fragment: &str) -> String {
    const HEX: &[u8] = b"0123456789ABCDEF";
    const KEEP: &[u8] = b"-_.~$&+,/:;=?@!()*";
    let mut out = String::with_capacity(fragment.len());
    for &byte in fragment.as_bytes() {
        if byte.is_ascii_alphanumeric() || KEEP.contains(&byte) {
            out.push(byte as char);
        } else {
            out.push('%');
            out.push(HEX[(byte >> 4) as usize] as char);
            out.push(HEX[(byte & 0x0f) as usize] as char);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AuthConfig;
    use crate::store::Backend;
    use axum::body::to_bytes;
    use axum::http::Request;
    use axum::Router;
    use std::time::Duration;
    use tower::ServiceExt;

    fn app(config: AuthConfig, backend: Backend) -> Router {
        crate::router_with_state(AuthState::new(config, None, backend))
    }

    async fn send(app: &Router, request: Request<Body>) -> (StatusCode, HeaderMap, String) {
        let response = app.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let headers = response.headers().clone();
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        (status, headers, String::from_utf8(bytes.to_vec()).unwrap())
    }

    async fn get(app: &Router, uri: &str) -> (StatusCode, HeaderMap, String) {
        send(app, Request::get(uri).body(Body::empty()).unwrap()).await
    }

    fn flow(provider: &str) -> FlowStateRecord {
        FlowStateRecord {
            provider_type: provider.into(),
            auth_code: None,
            code_challenge: None,
            code_challenge_method: None,
            user_id: None,
            authentication_method: "oauth".into(),
            provider_access_token: String::new(),
            provider_refresh_token: String::new(),
            created_at: Some(SystemTime::now()),
            auth_code_issued_at: None,
            referrer: Some("http://localhost:3000/after".into()),
            linking_target_id: None,
            email_optional: false,
        }
    }

    #[tokio::test]
    async fn missing_state_redirects_to_the_site_url_with_the_json_error_appended() {
        let app = app(AuthConfig::reference_defaults(), Backend::memory());
        let (status, headers, body) = get(&app, "/auth/v1/callback").await;
        assert_eq!(status, StatusCode::SEE_OTHER);
        let location = "http://localhost:3000?error=invalid_request&error_code=bad_oauth_callback&error_description=OAuth+state+parameter+missing";
        assert_eq!(headers[header::LOCATION], location);
        assert_eq!(headers[header::CONTENT_TYPE], "text/html; charset=utf-8");
        assert!(headers.get("x-sb-error-code").is_none());
        assert_eq!(
            body,
            format!(
                "<a href=\"{}\">See Other</a>.\n\n{}",
                html_escape(location),
                r#"{"code":400,"error_code":"bad_oauth_callback","msg":"OAuth state parameter missing"}"#
            )
        );

        let request = Request::post("/auth/v1/callback")
            .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
            .body(Body::from("state=not-a-uuid"))
            .unwrap();
        let (status, headers, body) = send(&app, request).await;
        assert_eq!(status, StatusCode::SEE_OTHER);
        assert_eq!(headers[header::CONTENT_TYPE], "text/plain; charset=utf-8");
        assert!(headers[header::LOCATION].to_str().unwrap().ends_with(
            "error_code=bad_oauth_state&error_description=OAuth+state+parameter+is+invalid"
        ));
        assert!(body.starts_with(r#"{"code":400,"error_code":"bad_oauth_state""#));
    }

    #[tokio::test]
    async fn unknown_expired_and_used_states_are_rejected() {
        let backend = Backend::memory();
        let expired = Uuid::new_v4();
        let mut old = flow("github");
        old.created_at = Some(SystemTime::now() - Duration::from_secs(301));
        backend.plant_flow_state_for_test(expired, old).await;
        let used = Uuid::new_v4();
        let mut claimed = flow("github");
        claimed.code_challenge = Some("challenge".into());
        claimed.user_id = Some(Uuid::new_v4());
        backend.plant_flow_state_for_test(used, claimed).await;
        let orphan = Uuid::new_v4();
        let mut linking = flow("github");
        linking.linking_target_id = Some(Uuid::new_v4());
        backend.plant_flow_state_for_test(orphan, linking).await;
        let app = app(AuthConfig::reference_defaults(), backend);
        for (id, fragment) in [
            (
                Uuid::new_v4(),
                "error_description=OAuth+state+not+found+or+expired",
            ),
            (expired, "error_description=OAuth+state+has+expired"),
            (used, "error_code=flow_state_already_used"),
            (orphan, "error=server_error&error_code=user_not_found"),
        ] {
            let (status, headers, _) = get(&app, &format!("/auth/v1/callback?state={id}")).await;
            assert_eq!(status, StatusCode::SEE_OTHER, "{id}");
            let location = headers[header::LOCATION].to_str().unwrap();
            assert!(location.contains(fragment), "{location}");
        }
    }

    #[tokio::test]
    async fn callback_errors_redirect_to_the_referrer_with_query_and_fragment() {
        let backend = Backend::memory();
        let id = Uuid::new_v4();
        backend.plant_flow_state_for_test(id, flow("github")).await;
        let app = app(AuthConfig::reference_defaults(), backend);

        let (status, headers, body) = get(&app, &format!("/auth/v1/callback?state={id}")).await;
        assert_eq!(status, StatusCode::FOUND);
        let location = "http://localhost:3000/after?error=invalid_request&error_code=bad_oauth_callback&error_description=OAuth+callback+with+missing+authorization+code+missing#error=invalid_request&error_code=bad_oauth_callback&error_description=OAuth+callback+with+missing+authorization+code+missing&sb=";
        assert_eq!(headers[header::LOCATION], location);
        assert!(body.contains("Found</a>"));

        let (_, headers, _) = get(&app, &format!("/auth/v1/callback?state={id}&code=x")).await;
        assert_eq!(
            headers[header::LOCATION],
            "http://localhost:3000/after?error=invalid_request&error_code=oauth_provider_not_supported&error_description=Unsupported+provider%3A+provider+is+not+enabled#error=invalid_request&error_code=oauth_provider_not_supported&error_description=Unsupported+provider%253A+provider+is+not+enabled&sb="
        );

        let request = Request::post(format!("/auth/v1/callback?state={id}"))
            .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
            .body(Body::from("error=access_denied&error_description=no"))
            .unwrap();
        let (status, headers, body) = send(&app, request).await;
        assert_eq!(status, StatusCode::FOUND);
        assert!(headers.get(header::CONTENT_TYPE).is_none());
        assert!(body.is_empty());
        assert_eq!(
            headers[header::LOCATION],
            "http://localhost:3000/after?error=access_denied&error_description=no#error=access_denied&error_description=no&sb="
        );
    }

    #[tokio::test]
    async fn enabled_provider_exchange_stays_501() {
        let backend = Backend::memory();
        let id = Uuid::new_v4();
        backend.plant_flow_state_for_test(id, flow("github")).await;
        let mut config = AuthConfig::reference_defaults();
        config.external.github = true;
        let app = app(config, backend.clone());
        let (_, headers, _) = get(&app, &format!("/auth/v1/callback?state={id}&code=x")).await;
        assert!(headers[header::LOCATION]
            .to_str()
            .unwrap()
            .contains("Unsupported+provider%3A+missing+OAuth+client+ID"));
        let app = super::tests::app(github_config(), backend);
        let (status, _, body) = get(&app, &format!("/auth/v1/callback?state={id}&code=x")).await;
        assert_eq!(status, StatusCode::NOT_IMPLEMENTED);
        assert!(body.contains(UNIT_CALLBACK_GET), "{body}");
    }

    fn github_config() -> AuthConfig {
        let mut config = AuthConfig::reference_defaults();
        config.external.github = true;
        config.oauth_clients.set(
            "github",
            crate::config::OAuthClient {
                client_id: vec!["cid".into()],
                secret: "s".into(),
                redirect_uri: "http://localhost:8000/auth/v1/callback".into(),
                url: String::new(),
                email_optional: true,
            },
        );
        config
    }

    fn state_param(location: &str) -> Uuid {
        let query = parse_query(url_query(location));
        Uuid::parse_str(first(&query, "state")).unwrap()
    }

    #[tokio::test]
    async fn authorize_creates_a_flow_state_and_redirects_to_the_provider() {
        let backend = Backend::memory();
        let app = app(github_config(), backend.clone());
        let (status, headers, body) = get(
            &app,
            "/auth/v1/authorize?provider=GitHub&scopes=repo&redirect_to=http%3A%2F%2Flocalhost%3A3000%2Fdone&state=evil&Redirect_URI=evil&nonce=n&prompt=consent",
        )
        .await;
        assert_eq!(status, StatusCode::FOUND);
        let location = headers[header::LOCATION].to_str().unwrap();
        let id = state_param(location);
        assert_eq!(
            location,
            format!(
                "https://github.com/login/oauth/authorize?client_id=cid&nonce=n&prompt=consent&redirect_to=http%3A%2F%2Flocalhost%3A3000%2Fdone&redirect_uri=http%3A%2F%2Flocalhost%3A8000%2Fauth%2Fv1%2Fcallback&response_type=code&scope=user%3Aemail+repo&state={id}"
            )
        );
        assert_eq!(headers[header::CONTENT_TYPE], "text/html; charset=utf-8");
        assert!(body.ends_with("\">Found</a>.\n\n"), "{body}");

        let flow = backend.find_flow_state(id).await.unwrap().unwrap();
        assert_eq!(flow.provider_type, "GitHub");
        assert_eq!(flow.authentication_method, "oauth");
        assert_eq!(flow.referrer.as_deref(), Some("http://localhost:3000/done"));
        assert!(flow.email_optional);
        assert!(!flow.is_pkce());
        assert!(flow.auth_code.is_none());
    }

    #[tokio::test]
    async fn authorize_pkce_stores_the_challenge_and_rejects_bad_parameters() {
        let backend = Backend::memory();
        let app = app(github_config(), backend.clone());
        let challenge = "a".repeat(43);
        let (status, headers, _) = get(
            &app,
            &format!("/auth/v1/authorize?provider=github&code_challenge={challenge}&code_challenge_method=S256&redirect_to=https%3A%2F%2Fevil.example"),
        )
        .await;
        assert_eq!(status, StatusCode::FOUND);
        let location = headers[header::LOCATION].to_str().unwrap();
        assert!(!location.contains("code_challenge"), "{location}");
        let flow = backend
            .find_flow_state(state_param(location))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(flow.code_challenge.as_deref(), Some(challenge.as_str()));
        assert_eq!(flow.code_challenge_method.as_deref(), Some("s256"));
        assert!(flow.auth_code.is_some());
        // A disallowed `redirect_to` falls back to the site URL.
        assert_eq!(flow.referrer.as_deref(), Some("http://localhost:3000"));

        for (query, message) in [
            (
                "code_challenge_method=s256".to_string(),
                "PKCE flow requires code_challenge_method and code_challenge",
            ),
            (
                format!("code_challenge={challenge}&code_challenge_method=md5"),
                "Invalid code_challenge_method",
            ),
        ] {
            let (status, _, body) =
                get(&app, &format!("/auth/v1/authorize?provider=github&{query}")).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{query}");
            let body: serde_json::Value = serde_json::from_str(&body).unwrap();
            assert_eq!(body["error_code"], "validation_failed");
            assert_eq!(body["msg"], message);
        }
    }

    #[tokio::test]
    async fn authorize_leaves_unported_paths_at_501() {
        let mut config = github_config();
        let mut scheme_less = config.oauth_clients.get("github").unwrap().clone();
        scheme_less.url = "github.example.com".into();
        let app = app(config.clone(), Backend::memory());
        let (status, _, body) =
            get(&app, "/auth/v1/authorize?provider=github&invite_token=t").await;
        assert_eq!(status, StatusCode::NOT_IMPLEMENTED);
        assert!(body.contains(UNIT_AUTHORIZE), "{body}");

        config.oauth_clients.set("github", scheme_less);
        let app = super::tests::app(config, Backend::memory());
        let (status, _, _) = get(&app, "/auth/v1/authorize?provider=github").await;
        assert_eq!(status, StatusCode::NOT_IMPLEMENTED);
    }

    #[test]
    fn go_query_parsing_drops_bad_pairs() {
        let pairs = parse_query("a=1&b=%zz&c;=2&d=x+y&e");
        assert_eq!(
            pairs,
            vec![
                ("a".to_string(), "1".to_string()),
                ("d".to_string(), "x y".to_string()),
                ("e".to_string(), String::new()),
            ]
        );
    }
}
