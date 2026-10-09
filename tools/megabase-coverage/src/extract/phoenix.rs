//! Route table of an Elixir Phoenix router (`scope ... do` blocks).

use regex::Regex;

use crate::scan::{clean, join_paths, normalize_path, ELIXIR};

pub struct PhoenixRoute {
    pub method: String,
    pub path: String,
    pub offset: usize,
}

pub fn routes(text: &str) -> Vec<PhoenixRoute> {
    let code = clean(text, ELIXIR).code;
    let scope = Regex::new(r#"^\s*scope\s*\(?\s*"([^"]*)""#).unwrap();
    let opens_block = Regex::new(r"\bdo\s*$").unwrap();
    let closes_block = Regex::new(r"^\s*end\b").unwrap();
    let verb = Regex::new(r#"^\s*(get|post|put|patch|delete|head|options|live)\s*\(?\s*"([^"]*)""#)
        .unwrap();
    let resources = Regex::new(r#"^\s*resources\s*\(?\s*"([^"]*)"(.*)$"#).unwrap();
    let param = Regex::new(r#"param:\s*"([a-z_]+)""#).unwrap();
    let except = Regex::new(r"except:\s*\[([^\]]*)\]").unwrap();
    let only = Regex::new(r"only:\s*\[([^\]]*)\]").unwrap();

    let mut stack: Vec<Option<String>> = Vec::new();
    let mut out = Vec::new();
    let mut offset = 0;
    for line in code.split_inclusive('\n') {
        let start = offset;
        offset += line.len();
        let line = line.trim_end_matches('\n');
        let prefix = |stack: &[Option<String>], path: &str| {
            let mut parts: Vec<&str> = stack.iter().flatten().map(String::as_str).collect();
            parts.push(path);
            normalize_path(&join_paths(&parts))
        };
        if let Some(c) = verb.captures(line) {
            let method = if &c[1] == "live" {
                "GET".to_string()
            } else {
                c[1].to_uppercase()
            };
            out.push(PhoenixRoute {
                method,
                path: prefix(&stack, &c[2]),
                offset: start,
            });
        } else if let Some(c) = resources.captures(line) {
            let base = c[1].to_string();
            let options = c[2].to_string();
            let id = param
                .captures(&options)
                .map_or("id".to_string(), |p| p[1].to_string());
            let listed = |re: &Regex| -> Option<Vec<String>> {
                re.captures(&options).map(|l| {
                    l[1].split(',')
                        .map(|a| a.trim().trim_start_matches(':').to_string())
                        .collect()
                })
            };
            let excluded = listed(&except).unwrap_or_default();
            let included = listed(&only);
            let member = format!("{base}/:{id}");
            let actions = [
                ("index", "GET", base.as_str()),
                ("edit", "GET", ""),
                ("new", "GET", ""),
                ("show", "GET", member.as_str()),
                ("create", "POST", base.as_str()),
                ("update", "PATCH", member.as_str()),
                ("update", "PUT", member.as_str()),
                ("delete", "DELETE", member.as_str()),
            ];
            for (action, method, path) in actions {
                let wanted = included
                    .as_ref()
                    .is_none_or(|o| o.iter().any(|a| a == action));
                if path.is_empty() || !wanted || excluded.iter().any(|a| a == action) {
                    continue;
                }
                out.push(PhoenixRoute {
                    method: method.into(),
                    path: prefix(&stack, path),
                    offset: start,
                });
            }
        }
        if let Some(c) = scope.captures(line) {
            if opens_block.is_match(line) {
                stack.push(Some(c[1].to_string()));
            }
        } else if opens_block.is_match(line) {
            stack.push(None);
        } else if closes_block.is_match(line) {
            stack.pop();
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nests_scopes_and_expands_resources() {
        let src = r#"
  scope "/api", Web do
    pipe_through(:api)
    # get("/commented", C, :x)
    resources("/tenants", TenantController, param: "tenant_id", except: [:edit, :new])
    get("/ping", PingController, :ping)
  end
  get "/top", C, :top
"#;
        let found: Vec<_> = routes(src)
            .into_iter()
            .map(|r| format!("{} {}", r.method, r.path))
            .collect();
        assert_eq!(
            found,
            [
                "GET /api/tenants",
                "GET /api/tenants/{tenant_id}",
                "POST /api/tenants",
                "PATCH /api/tenants/{tenant_id}",
                "PUT /api/tenants/{tenant_id}",
                "DELETE /api/tenants/{tenant_id}",
                "GET /api/ping",
                "GET /top",
            ]
        );
    }
}
