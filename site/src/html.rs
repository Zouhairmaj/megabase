//! Tiny HTML helpers shared by the generator.

use std::collections::BTreeMap;

pub fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

pub fn subst(tpl: &str, vars: &BTreeMap<String, String>) -> String {
    let mut out = tpl.to_string();
    let mut keys: Vec<_> = vars.keys().collect();
    keys.sort_by_key(|k| std::cmp::Reverse(k.len()));
    for key in keys {
        let needle = format!("{{{{{key}}}}}");
        if let Some(value) = vars.get(key) {
            out = out.replace(&needle, value);
        }
    }
    out
}

#[allow(dead_code)]
pub fn attr(s: &str) -> String {
    esc(s).replace('\'', "&#39;")
}
