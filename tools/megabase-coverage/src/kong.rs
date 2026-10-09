//! The self-hosted gateway configuration
//! (`vendor/supabase/docker/volumes/api/kong.yml`), used to translate each
//! upstream service path into the path a client uses, and to drop paths the
//! gateway does not expose or explicitly blocks.

use anyhow::{bail, Result};
use regex::Regex;

use crate::scan::Repo;

pub const KONG_FILE: &str = "docker/volumes/api/kong.yml";

#[derive(Debug, Clone)]
pub struct Service {
    pub name: String,
    pub host: String,
    pub path: String,
    pub routes: Vec<String>,
    pub blocked: bool,
    pub line: usize,
}

pub struct Kong {
    pub services: Vec<Service>,
}

#[derive(Debug)]
pub enum Exposure {
    Gateway(String),
    Blocked(String),
    NotExposed,
}

impl Kong {
    pub fn load(supabase: &Repo) -> Result<Self> {
        let text = supabase.read(KONG_FILE)?;
        let service_re = Regex::new(r"^  - name: (\S+)").unwrap();
        let url_re = Regex::new(r#"^    url: "?https?://([^/"\s]+)(/[^"\s]*)?"?"#).unwrap();
        let path_re = Regex::new(r"^\s+- (/\S*)\s*$").unwrap();
        let mut services: Vec<Service> = Vec::new();
        let mut in_services = false;
        let mut in_paths = false;
        for (i, raw) in text.lines().enumerate() {
            let line = raw.split(" #").next().unwrap_or(raw);
            if line.trim_start().starts_with('#') {
                continue;
            }
            if !line.starts_with(' ') && !line.is_empty() {
                in_services = line.starts_with("services:");
                continue;
            }
            if !in_services {
                continue;
            }
            if let Some(c) = service_re.captures(line) {
                services.push(Service {
                    name: c[1].to_string(),
                    host: String::new(),
                    path: "/".into(),
                    routes: Vec::new(),
                    blocked: false,
                    line: i + 1,
                });
                in_paths = false;
                continue;
            }
            let Some(service) = services.last_mut() else {
                continue;
            };
            if let Some(c) = url_re.captures(line) {
                service.host = c[1].to_string();
                service.path = c.get(2).map_or("/", |m| m.as_str()).to_string();
            } else if line.trim() == "paths:" {
                in_paths = true;
            } else if in_paths && path_re.is_match(line) {
                service
                    .routes
                    .push(path_re.captures(line).unwrap()[1].to_string());
            } else if line.trim() == "- name: request-termination" {
                service.blocked = true;
            } else if !line.trim().is_empty() {
                in_paths = false;
            }
        }
        if services.iter().all(|s| s.routes.is_empty()) {
            bail!("no gateway routes found in {KONG_FILE}");
        }
        Ok(Self { services })
    }

    /// Where the upstream path `path` on `host` is reachable through the
    /// gateway. The service whose URL path is the longest prefix wins.
    pub fn expose(&self, host: &str, path: &str) -> Exposure {
        let matches = |prefix: &str| {
            prefix == "/"
                || path == prefix
                || path.starts_with(&format!("{}/", prefix.trim_end_matches('/')))
                || path.starts_with(&format!("{prefix}?"))
        };
        let best = self
            .services
            .iter()
            .filter(|s| s.host == host && !s.routes.is_empty() && matches(&s.path))
            .max_by_key(|s| s.path.len());
        let Some(service) = best else {
            return Exposure::NotExposed;
        };
        if service.blocked {
            return Exposure::Blocked(service.name.clone());
        }
        let rest = if service.path == "/" {
            path
        } else {
            &path[service.path.len()..]
        };
        let route = service.routes[0].trim_end_matches('/');
        let mut out = format!("{route}{rest}");
        if out.is_empty() {
            out.push('/');
        }
        Exposure::Gateway(out)
    }

    pub fn service(&self, name: &str) -> Option<&Service> {
        self.services.iter().find(|s| s.name == name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scan::TempTree;

    fn load(yml: &str) -> Kong {
        let tree = TempTree::new();
        tree.write(KONG_FILE, yml);
        Kong::load(&tree.repo("supabase")).unwrap()
    }

    const SAMPLE: &str = r#"
# comment
consumers:
  - username: anon
services:
  - name: rest-v1
    url: http://rest:3000/
    routes:
      - name: rest
        strip_path: true
        paths:
          - /rest/v1/
  - name: rest-rpc
    url: "http://rest:3000/rpc"
    routes:
      - name: rpc
        paths:
          - /rest/v1/rpc
  - name: blocked-mcp
    url: http://studio:3000/api/mcp
    routes:
      - name: mcp
        paths:
          - /mcp
    plugins:
      - name: request-termination
  - name: root-studio
    url: http://studio:3000/
    routes:
      - name: studio
        paths:
          - /
"#;

    #[test]
    fn expose_picks_longest_prefix_and_maps_gateway_path() {
        let kong = load(SAMPLE);
        match kong.expose("rest:3000", "/todos") {
            Exposure::Gateway(p) => assert_eq!(p, "/rest/v1/todos"),
            other => panic!("{other:?}"),
        }
        match kong.expose("rest:3000", "/rpc/graphql") {
            Exposure::Gateway(p) => assert_eq!(p, "/rest/v1/rpc/graphql"),
            other => panic!("{other:?}"),
        }
        match kong.expose("rest:3000", "/rpc") {
            Exposure::Gateway(p) => assert_eq!(p, "/rest/v1/rpc"),
            other => panic!("{other:?}"),
        }
        match kong.expose("studio:3000", "/api/mcp") {
            Exposure::Blocked(name) => assert_eq!(name, "blocked-mcp"),
            other => panic!("{other:?}"),
        }
        match kong.expose("studio:3000", "/") {
            Exposure::Gateway(p) => assert_eq!(p, "/"),
            other => panic!("{other:?}"),
        }
        match kong.expose("missing:1", "/x") {
            Exposure::NotExposed => {}
            other => panic!("{other:?}"),
        }
        assert_eq!(kong.service("rest-v1").unwrap().host, "rest:3000");
        assert!(kong.service("nope").is_none());
    }

    #[test]
    fn expose_matches_query_string_on_service_path() {
        let kong = load(
            r#"
services:
  - name: auth-verify
    url: http://auth:9999/verify
    routes:
      - name: v
        paths:
          - /auth/v1/verify
"#,
        );
        match kong.expose("auth:9999", "/verify?token=1") {
            Exposure::Gateway(p) => assert_eq!(p, "/auth/v1/verify?token=1"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn load_skips_comments_and_rejects_empty_routes() {
        let tree = TempTree::new();
        tree.write(KONG_FILE, "services:\n  - name: x\n");
        assert!(Kong::load(&tree.repo("supabase")).is_err());
    }
}
