//! Extraction of the coverage denominator from the pinned upstream sources.
//!
//! Each component module reads its upstream repository in `vendor/` and
//! records units (routes, operators, flows, message types, pages...) with the
//! file and line they came from. Nothing here is a hand-maintained list: if
//! an upstream file moves or a pattern stops matching, extraction fails.

mod auth;
mod functions;
mod meta;
mod phoenix;
mod pooler;
mod realtime;
mod rest;
mod storage;
mod studio;

use std::path::Path;

use anyhow::{bail, Context, Result};
use regex::Regex;

use crate::kong::{Exposure, Kong};
use crate::model::{Collector, Pin, PinsFile, Source, UnitsFile};
use crate::scan::{normalize_path, Repo, TS};

pub struct Ctx {
    pub kong: Kong,
    pub out: Collector,
}

impl Ctx {
    /// Records an HTTP route served by `host` upstream, translated through
    /// the gateway; routes the gateway blocks or does not expose are recorded
    /// as exclusions instead.
    #[allow(clippy::too_many_arguments)]
    pub fn gateway_route(
        &mut self,
        component: &str,
        group: &str,
        host: &str,
        method: &str,
        upstream_path: &str,
        level: u8,
        source: Source,
    ) {
        let name = format!("{method} {upstream_path}");
        match self.kong.expose(host, upstream_path) {
            Exposure::Gateway(path) => self
                .out
                .route(component, group, method, &path, level, source),
            Exposure::Blocked(service) => self.out.exclude(
                component,
                name,
                &format!("blocked by the gateway (kong.yml service `{service}`)"),
                source,
            ),
            Exposure::NotExposed => self.out.exclude(
                component,
                name,
                "not exposed through the gateway (no kong.yml service)",
                source,
            ),
        }
    }
}

pub fn load_pins(root: &Path) -> Result<Vec<Pin>> {
    let text = std::fs::read_to_string(root.join("vendor.toml")).context("reading vendor.toml")?;
    let pins: PinsFile = toml::from_str(&text).context("parsing vendor.toml")?;
    Ok(pins.pin)
}

pub fn extract(root: &Path) -> Result<UnitsFile> {
    let pins = load_pins(root)?;
    let repo = |name: &str| -> Result<Repo> {
        let pin = pins
            .iter()
            .find(|p| p.name == name)
            .with_context(|| format!("vendor.toml has no pin named {name}"))?;
        let dir = root.join(&pin.path);
        if !dir.join(".git").exists()
            && std::fs::read_dir(&dir).is_ok_and(|mut d| d.next().is_none())
        {
            bail!(
                "{} is empty; run `git submodule update --init --depth 1 {}`",
                pin.path,
                pin.path
            );
        }
        Ok(Repo {
            name: name.to_string(),
            root: dir,
        })
    };

    let supabase = repo("supabase")?;
    let mut ctx = Ctx {
        kong: Kong::load(&supabase)?,
        out: Collector::default(),
    };
    rest::extract(&mut ctx, &repo("postgrest")?)?;
    auth::extract(&mut ctx, &repo("auth")?)?;
    realtime::extract(&mut ctx, &repo("realtime")?, &repo("supabase-js")?)?;
    storage::extract(&mut ctx, &repo("storage")?)?;
    functions::extract(&mut ctx, &repo("edge-runtime")?, &supabase)?;
    pooler::extract(&mut ctx, &repo("supavisor")?)?;
    meta::extract(&mut ctx, &repo("postgres-meta")?)?;
    studio::extract(&mut ctx, &supabase)?;
    for component in ["auth", "meta", "pooler", "studio"] {
        ctx.out.merge_small_route_groups(component, 3);
    }
    Ok(ctx.out.finish(pins))
}

/// Fails when a pattern that must match upstream source finds nothing, which
/// means the upstream layout changed and the extractor needs updating.
pub fn require<T>(items: Vec<T>, what: &str) -> Result<Vec<T>> {
    if items.is_empty() {
        bail!("extractor found no {what}; the upstream layout changed");
    }
    Ok(items)
}

pub struct TsRoute {
    pub method: String,
    pub path: String,
    pub offset: usize,
}

/// Finds `<receiver>.<method>(<path>, ...)` registrations in TypeScript
/// (fastify and storage's S3 router), skipping generic type arguments and
/// resolving a path given as a same-file `const`.
pub fn ts_routes(text: &str, receivers: &[&str]) -> Vec<TsRoute> {
    let cleaned = crate::scan::clean(text, TS);
    let code = &cleaned.code;
    let pattern = format!(
        r"\b(?:{})\.(get|post|put|patch|delete|head|options)\s*",
        receivers.join("|")
    );
    let call = Regex::new(&pattern).unwrap();
    let literal = Regex::new(r#"^\(\s*(['"`])([^'"`]*)['"`]"#).unwrap();
    let ident = Regex::new(r"^\(\s*([A-Za-z_][A-Za-z0-9_]*)\s*,").unwrap();
    let consts = crate::scan::string_consts(code);
    let mut routes = Vec::new();
    for m in call.captures_iter(code) {
        let whole = m.get(0).unwrap();
        let mut i = whole.end();
        let bytes = code.as_bytes();
        if bytes.get(i) == Some(&b'<') {
            let mut depth = 0i32;
            while i < bytes.len() {
                match bytes[i] {
                    b'<' => depth += 1,
                    b'>' => {
                        depth -= 1;
                        if depth == 0 {
                            i += 1;
                            break;
                        }
                    }
                    _ => {}
                }
                i += 1;
            }
        }
        let rest = &code[i..];
        let path = if let Some(c) = literal.captures(rest) {
            c[2].to_string()
        } else if let Some(c) = ident.captures(rest) {
            match consts.iter().rev().find(|(k, _)| k == &c[1]) {
                Some((_, v)) => v.clone(),
                None => continue,
            }
        } else {
            continue;
        };
        routes.push(TsRoute {
            method: m[1].to_uppercase(),
            path: normalize_path(&path),
            offset: whole.start(),
        });
    }
    routes
}

/// The `skip`-th segment of `path`, used as a feature-group name.
pub fn first_segment(path: &str, skip: usize) -> String {
    path.split('/')
        .filter(|s| !s.is_empty())
        .nth(skip)
        .map(|s| {
            s.trim_start_matches('{')
                .trim_end_matches('}')
                .trim_start_matches('.')
                .to_string()
        })
        .unwrap_or_else(|| "root".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kong::{Exposure, Kong, KONG_FILE};
    use crate::model::Source;
    use crate::scan::TempTree;

    fn src() -> Source {
        Source {
            repo: "t".into(),
            file: "f".into(),
            line: 1,
        }
    }

    fn kong(yml: &str) -> Kong {
        let tree = TempTree::new();
        tree.write(KONG_FILE, yml);
        Kong::load(&tree.repo("supabase")).unwrap()
    }

    #[test]
    fn require_rejects_empty() {
        assert!(require::<u8>(vec![], "things").is_err());
        assert_eq!(require(vec![1], "things").unwrap(), [1]);
    }

    #[test]
    fn first_segment_skips_and_strips_params() {
        assert_eq!(first_segment("/project/{ref}/sql", 2), "sql");
        assert_eq!(first_segment("/{.well-known}/jwks", 0), "well-known");
        assert_eq!(first_segment("/", 0), "root");
        assert_eq!(first_segment("/only", 5), "root");
    }

    #[test]
    fn ts_routes_reads_literals_generics_and_consts() {
        let src = r#"
const OBJECT = '/object'
fastify.get('/health', h)
fastify.post<Types>('/upload', h)
s3Router.put(OBJECT, h)
fastify.delete(UNKNOWN, h)
fastify.patch(123, h)
"#;
        let found: Vec<_> = ts_routes(src, &["fastify", "s3Router"])
            .into_iter()
            .map(|r| format!("{} {}", r.method, r.path))
            .collect();
        assert_eq!(found, ["GET /health", "POST /upload", "PUT /object",]);
    }

    #[test]
    fn gateway_route_records_exposed_blocked_and_hidden() {
        let kong = kong(
            r#"
services:
  - name: rest-v1
    url: http://rest:3000/
    routes:
      - name: rest
        paths:
          - /rest/v1/
  - name: blocked
    url: http://rest:3000/internal
    routes:
      - name: b
        paths:
          - /internal
    plugins:
      - name: request-termination
"#,
        );
        let mut ctx = Ctx {
            kong,
            out: crate::model::Collector::default(),
        };
        ctx.gateway_route("rest", "resources", "rest:3000", "GET", "/todos", 1, src());
        ctx.gateway_route(
            "rest",
            "resources",
            "rest:3000",
            "GET",
            "/internal/secret",
            1,
            src(),
        );
        ctx.gateway_route("rest", "resources", "other:1", "GET", "/nope", 1, src());
        let file = ctx.out.finish(vec![]);
        assert_eq!(file.units.len(), 1);
        assert_eq!(file.units[0].path.as_deref(), Some("/rest/v1/todos"));
        assert_eq!(file.excluded.len(), 2);
        assert!(file.excluded.iter().any(|e| e.reason.contains("blocked")));
        assert!(file
            .excluded
            .iter()
            .any(|e| e.reason.contains("not exposed")));
        match ctx.kong.expose("rest:3000", "/todos") {
            Exposure::Gateway(_) => {}
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn load_pins_reads_vendor_toml() {
        let tree = TempTree::new();
        tree.write(
            "vendor.toml",
            r#"
[[pin]]
name = "auth"
path = "vendor/auth"
repo = "https://example/auth"
tag = "v1"
commit = "abc"
license = "MIT"
"#,
        );
        let pins = load_pins(&tree.root).unwrap();
        assert_eq!(pins.len(), 1);
        assert_eq!(pins[0].name, "auth");
        assert_eq!(pins[0].commit, "abc");
    }

    fn write_min_vendor(tree: &TempTree) {
        tree.write(
            "vendor.toml",
            r#"
[[pin]]
name = "supabase"
path = "vendor/supabase"
repo = "https://example/supabase"
tag = "v1"
commit = "1"
license = "Apache-2.0"
[[pin]]
name = "postgrest"
path = "vendor/postgrest"
repo = "https://example/postgrest"
tag = "v1"
commit = "1"
license = "MIT"
[[pin]]
name = "auth"
path = "vendor/auth"
repo = "https://example/auth"
tag = "v1"
commit = "1"
license = "MIT"
[[pin]]
name = "realtime"
path = "vendor/realtime"
repo = "https://example/realtime"
tag = "v1"
commit = "1"
license = "Apache-2.0"
[[pin]]
name = "supabase-js"
path = "vendor/supabase-js"
repo = "https://example/js"
tag = "v1"
commit = "1"
license = "MIT"
[[pin]]
name = "storage"
path = "vendor/storage"
repo = "https://example/storage"
tag = "v1"
commit = "1"
license = "Apache-2.0"
[[pin]]
name = "edge-runtime"
path = "vendor/edge-runtime"
repo = "https://example/edge"
tag = "v1"
commit = "1"
license = "MIT"
[[pin]]
name = "supavisor"
path = "vendor/supavisor"
repo = "https://example/supavisor"
tag = "v1"
commit = "1"
license = "Apache-2.0"
[[pin]]
name = "postgres-meta"
path = "vendor/postgres-meta"
repo = "https://example/meta"
tag = "v1"
commit = "1"
license = "Apache-2.0"
"#,
        );
        tree.write(
            "vendor/supabase/docker/volumes/api/kong.yml",
            r#"
services:
  - name: rest-v1
    url: http://rest:3000/
    routes:
      - name: rest
        paths:
          - /rest/v1/
  - name: auth-v1
    url: http://auth:9999/
    routes:
      - name: auth
        paths:
          - /auth/v1/
  - name: realtime-v1-rest
    url: http://realtime-dev.supabase-realtime:4000/api
    routes:
      - name: rt
        paths:
          - /realtime/v1/api
  - name: realtime-v1
    url: http://realtime-dev.supabase-realtime:4000/socket
    routes:
      - name: sock
        paths:
          - /realtime/v1
  - name: storage-v1
    url: http://storage:5000/
    routes:
      - name: st
        paths:
          - /storage/v1/
  - name: meta
    url: http://meta:8080/
    routes:
      - name: meta
        paths:
          - /pg/
  - name: functions-v1
    url: http://functions:9000/
    routes:
      - name: fn
        paths:
          - /functions/v1/
  - name: studio
    url: http://studio:3000/
    routes:
      - name: studio
        paths:
          - /
"#,
        );
        for name in [
            "supabase",
            "postgrest",
            "auth",
            "realtime",
            "supabase-js",
            "storage",
            "edge-runtime",
            "supavisor",
            "postgres-meta",
        ] {
            tree.write(&format!("vendor/{name}/.git"), "gitdir: /tmp\n");
        }
        tree.write(
            "vendor/postgrest/src/library/PostgREST/ApiRequest.hs",
            r#"
(ResourceRelation, "GET")
(ResourceRoutine, "POST")
(ResourceSchema, "HEAD")
"#,
        );
        tree.write(
            "vendor/postgrest/src/library/PostgREST/ApiRequest/QueryParams.hs",
            r#"
lookupParam "select"
endingIn ["order"]
simpleOperator :: Parser
simpleOperator = string "eq"
pFieldSelect :: Parser
pFieldSelect = string "sum"
pEmbedParams :: Parser
pEmbedParams = string "inner"
pLogicTree :: Parser
pLogicTree = string "and"
pOrderTerm :: Parser
pOrderTerm = string "asc"
otherParser :: Parser
otherParser = string "foo"
"#,
        );
        tree.write(
            "vendor/postgrest/src/library/PostgREST/ApiRequest/Preferences.hs",
            r#"
toHeaderValue CountExact = "count=exact"
toHeaderValue Return = "return="
"#,
        );
        tree.write(
            "vendor/postgrest/src/library/PostgREST/MediaType.hs",
            r#"
toMime MTJSON = "application/json"
toMime MTObject = "application/vnd.pgrst.object+"
"#,
        );
        tree.write(
            "vendor/auth/internal/api/scim/server.go",
            r#"BasePath = "/scim""#,
        );
        tree.write(
            "vendor/auth/internal/api/api.go",
            r#"
func (a *API) handler() {
	r.Route("/admin", func(r *router) {
		r.Get("/users", a.adminUsers)
		r.Get("/sso/providers", a.sso)
	})
	r.Get("/health", a.health)
	r.Get("/signup", a.signup)
}
"#,
        );
        tree.write(
            "vendor/auth/internal/api/token.go",
            r#"
func (a *API) Token(w http.ResponseWriter, r *http.Request) error {
	switch grant {
	case "password":
	case "refresh_token":
	case "id_token":
	}
}
"#,
        );
        tree.write(
            "vendor/auth/internal/api/provider_constants.go",
            r#"GoogleProvider = "google""#,
        );
        tree.write(
            "vendor/auth/internal/api/external.go",
            r#"
func (a *API) Provider(ctx context.Context, name string) {
	switch name {
	case GoogleProvider:
	}
	if strings.HasPrefix(name, "custom:") {
	}
}
"#,
        );
        tree.write(
            "vendor/auth/internal/api/verify.go",
            r#"
SignupVerification = "signup"
MagicLinkVerification = "magiclink"
func (a *API) Verify(w http.ResponseWriter, r *http.Request) {
	switch params.Type {
	case SignupVerification:
	case MagicLinkVerification:
	}
}
"#,
        );
        tree.write(
            "vendor/auth/internal/mailer/mailer.go",
            r#"Invite = "invite""#,
        );
        tree.write(
            "vendor/auth/migrations/00.up.sql",
            r#"
CREATE OR REPLACE FUNCTION {{ db }}.uid() RETURNS uuid AS $$ SELECT 'x'::uuid $$ LANGUAGE sql;
CREATE TABLE IF NOT EXISTS {{ db }}.users (id uuid);
"#,
        );
        tree.write(
            "vendor/realtime/lib/realtime_web/router.ex",
            r#"
  scope "/api", RealtimeWeb do
    get("/tenants", TenantController, :index)
  end
  get("/healthcheck", PageController, :health)
"#,
        );
        tree.write(
            "vendor/realtime/lib/realtime_web/endpoint.ex",
            r#"
  socket "/socket", RealtimeWeb.UserSocket
"#,
        );
        tree.write(
            "vendor/realtime/lib/realtime_web/channels/realtime_channel.ex",
            r#"
  def handle_in("heartbeat", payload, socket) do
    push(socket, "phx_reply", %{})
  end
"#,
        );
        tree.write(
            "vendor/realtime/lib/extensions/postgres_cdc.ex",
            r#"
  event: "postgres_changes"
"#,
        );
        tree.write(
            "vendor/supabase-js/packages/core/realtime-js/src/lib/constants.ts",
            r#"
export const CHANNEL_EVENTS = {
  close: 'phx_close',
  error: 'phx_error',
}
"#,
        );
        tree.write(
            "vendor/storage/src/app.ts",
            r#"
  register(routes.bucket, { prefix: '/bucket' })
"#,
        );
        tree.write(
            "vendor/storage/src/http/routes/index.ts",
            r#"export { default as bucket } from './bucket'"#,
        );
        tree.write(
            "vendor/storage/src/http/routes/bucket/index.ts",
            r#"
import getBucket from './get'
register(getBucket, { prefix: '/get' })
"#,
        );
        tree.write(
            "vendor/storage/src/http/routes/bucket/get.ts",
            r#"
const SIGNED = '/sign'
fastify.get('/', handler)
fastify.get('/:id', handler)
fastify.post({ prefix: SIGNED }, handler)
"#,
        );
        tree.write(
            "vendor/storage/migrations/tenant/01.sql",
            r#"
CREATE FUNCTION storage.search(name text) RETURNS void AS $$ $$ LANGUAGE sql;
CREATE TABLE IF NOT EXISTS storage.objects (id uuid);
"#,
        );
        tree.write(
            "vendor/edge-runtime/ext/ai/ops.rs",
            r#"
#[op2(fast)]
fn op_ai_embed() {}
"#,
        );
        tree.write(
            "vendor/supavisor/lib/supavisor_web/router.ex",
            r#"
  scope "/api", SupavisorWeb do
    get("/tenants", TenantController, :index)
    get("/openapi", OpenApiController, :index)
  end
  get("/swaggerui", SwaggerController, :index)
"#,
        );
        tree.write(
            "vendor/supavisor/lib/supavisor/tenants/user.ex",
            r#"
  field(:mode_type, Ecto.Enum, values: [:transaction, :session, :statement])
"#,
        );
        tree.write(
            "vendor/postgres-meta/src/server/app.ts",
            r#"
app.get('/health', handler)
"#,
        );
        tree.write(
            "vendor/postgres-meta/src/server/routes/index.ts",
            r#"
import tables from './tables.js'
register(tables, { prefix: '/tables' })
"#,
        );
        tree.write(
            "vendor/postgres-meta/src/server/routes/tables.ts",
            r#"
fastify.get('/', handler)
fastify.get('/:id', handler)
"#,
        );
        tree.write(
            "vendor/supabase/apps/studio/pages/project/[ref]/index.tsx",
            "export default function Page() {}",
        );
        tree.write(
            "vendor/supabase/apps/studio/pages/project/[ref]/sql/[id].tsx",
            "export default function Sql() {}",
        );
        tree.write(
            "vendor/supabase/apps/studio/pages/sign-in.tsx",
            "export default function SignIn() {}",
        );
        tree.write(
            "vendor/supabase/apps/studio/pages/api/platform/profile.ts",
            r#"
switch (req.method) {
  case 'GET':
  case 'POST':
}
"#,
        );
        tree.write(
            "vendor/supabase/apps/studio/pages/api/ai/sql.ts",
            "export default function handler() {}",
        );
    }

    #[test]
    fn extract_from_fixture_tree_covers_every_component() {
        let tree = TempTree::new();
        write_min_vendor(&tree);
        let units = extract(&tree.root).unwrap();
        assert!(units.total >= 20, "total={}", units.total);
        for component in [
            "rest",
            "auth",
            "realtime",
            "storage",
            "functions",
            "pooler",
            "meta",
            "studio",
        ] {
            assert!(
                units.by_component.get(component).copied().unwrap_or(0) > 0,
                "missing {component}"
            );
        }
        assert!(units.units.iter().any(|u| u.kind == "filter-operator"));
        assert!(units.units.iter().any(|u| u.kind == "prefer"));
        assert!(units.units.iter().any(|u| u.kind == "media-type"));
        assert!(units.units.iter().any(|u| u.kind == "grant-type"));
        assert!(units.units.iter().any(|u| u.kind == "provider"));
        assert!(units.units.iter().any(|u| u.kind == "op"));
        assert!(units.units.iter().any(|u| u.kind == "pool-mode"));
        assert!(units
            .excluded
            .iter()
            .any(|e| e.reason.contains("hosted-platform")));
        assert!(units
            .excluded
            .iter()
            .any(|e| e.reason.contains("API documentation")));
    }

    #[test]
    fn extract_fails_on_empty_submodule_and_missing_pin() {
        let tree = TempTree::new();
        tree.write(
            "vendor.toml",
            r#"
[[pin]]
name = "supabase"
path = "vendor/supabase"
repo = "https://example/supabase"
tag = "v1"
commit = "1"
license = "Apache-2.0"
"#,
        );
        tree.mkdir("vendor/supabase");
        let err = extract(&tree.root).unwrap_err().to_string();
        assert!(
            err.contains("empty") || err.contains("no pin named"),
            "{err}"
        );
        tree.write("vendor/supabase/.git", "gitdir: /tmp\n");
        tree.write(
            "vendor/supabase/docker/volumes/api/kong.yml",
            "services:\n  - name: x\n    url: http://h/\n    routes:\n      - name: r\n        paths:\n          - /\n",
        );
        let err = extract(&tree.root).unwrap_err().to_string();
        assert!(err.contains("no pin named"), "{err}");
    }
}
