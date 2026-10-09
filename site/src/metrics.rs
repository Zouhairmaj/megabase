use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use serde_json::Value;

pub const FALLBACK_STAGE: &str = "Phase 0 · bootstrap";
pub const FALLBACK_STAGE_SHORT: &str = "Phase 0";

/// Stable visual order, matching the Megabase crate layout.
pub const COMPONENT_ORDER: &[&str] = &[
    "rest",
    "auth",
    "realtime",
    "storage",
    "functions",
    "pooler",
    "meta",
    "studio",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnitStatus {
    NotStarted,
    Implemented,
    Tested,
    Conformant,
}

#[derive(Clone, Debug)]
pub struct ComponentBlock {
    pub id: String,
    pub label: &'static str,
    pub not_started: usize,
    pub implemented: usize,
    pub tested: usize,
    pub conformant: usize,
}

impl ComponentBlock {
    pub fn total(&self) -> usize {
        self.not_started + self.implemented + self.tested + self.conformant
    }
}

#[derive(Clone, Debug)]
pub struct Metrics {
    pub passing: Option<usize>,
    pub total: Option<usize>,
    pub coverage: Option<f64>,
    pub conformance: Option<f64>,
    pub components: Vec<ComponentBlock>,
    pub by_component: BTreeMap<String, f64>,
    pub source: String,
    pub stage: String,
    pub stage_short: String,
}

impl Metrics {
    pub fn placeholder() -> Self {
        Self {
            passing: None,
            total: None,
            coverage: None,
            conformance: None,
            components: Vec::new(),
            by_component: BTreeMap::new(),
            source: "placeholder".into(),
            stage: FALLBACK_STAGE.into(),
            stage_short: FALLBACK_STAGE_SHORT.into(),
        }
    }

    pub fn has_data(&self) -> bool {
        self.total.is_some()
    }

    pub fn passing_total_label(&self) -> String {
        match (self.passing, self.total) {
            (Some(passing), Some(total)) => format!("{} / {}", comma(passing), comma(total)),
            _ => "—".into(),
        }
    }

    pub fn coverage_label(&self) -> String {
        self.coverage.map(pct).unwrap_or_else(|| "—".into())
    }

    pub fn conformance_label(&self) -> String {
        self.conformance.map(pct).unwrap_or_else(|| "—".into())
    }

    pub fn coverage_conformance_label(&self) -> String {
        match (self.coverage, self.conformance) {
            (Some(a), Some(b)) => format!("{} · {}", pct(a), pct(b)),
            _ => "—".into(),
        }
    }

    pub fn component_coverage_label(&self, key: &str) -> String {
        if !self.has_data() {
            return "—".into();
        }
        compact_pct(self.by_component.get(key).copied().unwrap_or(0.0))
    }
}

pub fn load(repo_root: &Path) -> Metrics {
    let mut metrics = Metrics::placeholder();
    let summary_path = repo_root.join("coverage/summary.json");
    let units_path = repo_root.join("coverage/units.json");

    if summary_path.is_file() {
        if let Ok(text) = fs::read_to_string(&summary_path) {
            if let Ok(value) = serde_json::from_str::<Value>(&text) {
                apply_summary(&mut metrics, &value);
                metrics.source = "coverage/summary.json".into();
            }
        }
    }

    if units_path.is_file() {
        if let Ok(text) = fs::read_to_string(&units_path) {
            if let Ok(value) = serde_json::from_str::<Value>(&text) {
                if let Some(summary) = value.get("summary") {
                    if metrics.source == "placeholder" {
                        apply_summary(&mut metrics, summary);
                    }
                    merge_components(&mut metrics, summary);
                }
                if let Some(list) = value.get("units").and_then(Value::as_array) {
                    if !list.is_empty() {
                        apply_units(&mut metrics, list);
                        metrics.source = "coverage/units.json".into();
                    }
                }
            }
        }
    }

    metrics
}

fn apply_summary(metrics: &mut Metrics, value: &Value) {
    if let Some(n) = value.get("total_units").and_then(Value::as_u64) {
        metrics.total = Some(n as usize);
    }
    if let Some(n) = value.get("conformant").and_then(Value::as_u64) {
        metrics.passing = Some(n as usize);
    }
    if let Some(n) = value.get("coverage_percent").and_then(Value::as_f64) {
        metrics.coverage = Some(n);
    }
    if let Some(n) = value.get("conformance_percent").and_then(Value::as_f64) {
        metrics.conformance = Some(n);
    }
    merge_components(metrics, value);
    if metrics.components.is_empty() {
        blocks_from_summary(metrics, value);
    }
}

fn merge_components(metrics: &mut Metrics, value: &Value) {
    let Some(map) = value.get("by_component").and_then(Value::as_object) else {
        return;
    };
    for (name, stats) in map {
        if let Some(pct_val) = stats.get("coverage_percent").and_then(Value::as_f64) {
            metrics.by_component.insert(name.clone(), pct_val);
        } else if let (Some(total), Some(implemented)) = (
            stats.get("total").and_then(Value::as_u64),
            stats.get("implemented").and_then(Value::as_u64),
        ) {
            let pct_val = if total == 0 {
                0.0
            } else {
                100.0 * implemented as f64 / total as f64
            };
            metrics.by_component.insert(name.clone(), pct_val);
        }
    }
}

fn blocks_from_summary(metrics: &mut Metrics, value: &Value) {
    let Some(map) = value.get("by_component").and_then(Value::as_object) else {
        return;
    };
    let mut blocks = Vec::new();
    for id in COMPONENT_ORDER {
        let Some(stats) = map.get(*id) else {
            continue;
        };
        let total = stats.get("total").and_then(Value::as_u64).unwrap_or(0) as usize;
        let implemented = stats
            .get("implemented")
            .and_then(Value::as_u64)
            .unwrap_or(0) as usize;
        let conformant = stats.get("conformant").and_then(Value::as_u64).unwrap_or(0) as usize;
        let tested = stats.get("tested").and_then(Value::as_u64).unwrap_or(0) as usize;
        let implemented_only = implemented.saturating_sub(tested.max(conformant));
        let tested_only = tested.saturating_sub(conformant);
        let not_started = total.saturating_sub(implemented.max(tested).max(conformant));
        if total == 0 {
            continue;
        }
        blocks.push(ComponentBlock {
            id: (*id).into(),
            label: component_label(id),
            not_started,
            implemented: implemented_only,
            tested: tested_only,
            conformant,
        });
    }
    // Any extra components not in the canonical order.
    for (id, stats) in map {
        if COMPONENT_ORDER.contains(&id.as_str()) {
            continue;
        }
        let total = stats.get("total").and_then(Value::as_u64).unwrap_or(0) as usize;
        if total == 0 {
            continue;
        }
        blocks.push(ComponentBlock {
            id: id.clone(),
            label: component_label(id),
            not_started: total,
            implemented: 0,
            tested: 0,
            conformant: 0,
        });
    }
    metrics.components = blocks;
}

fn apply_units(metrics: &mut Metrics, list: &[Value]) {
    let mut counts: BTreeMap<String, [usize; 4]> = BTreeMap::new();
    let mut passing = 0usize;
    for item in list {
        let component = item
            .get("component")
            .and_then(Value::as_str)
            .unwrap_or("other")
            .to_string();
        let status = status_of(item);
        if status == UnitStatus::Conformant {
            passing += 1;
        }
        let slot = match status {
            UnitStatus::NotStarted => 0,
            UnitStatus::Implemented => 1,
            UnitStatus::Tested => 2,
            UnitStatus::Conformant => 3,
        };
        counts.entry(component).or_insert([0; 4])[slot] += 1;
    }

    let total = list.len();
    metrics.total = Some(total);
    metrics.passing = Some(passing);
    if metrics.coverage.is_none() && total > 0 {
        let implemented = counts.values().map(|c| c[1] + c[2] + c[3]).sum::<usize>();
        metrics.coverage = Some(100.0 * implemented as f64 / total as f64);
    }
    if metrics.conformance.is_none() && total > 0 {
        metrics.conformance = Some(100.0 * passing as f64 / total as f64);
    }

    let mut blocks = Vec::new();
    for id in COMPONENT_ORDER {
        let Some([ns, imp, tes, con]) = counts.get(*id).copied() else {
            continue;
        };
        blocks.push(ComponentBlock {
            id: (*id).into(),
            label: component_label(id),
            not_started: ns,
            implemented: imp,
            tested: tes,
            conformant: con,
        });
        let t = ns + imp + tes + con;
        if t > 0 {
            metrics
                .by_component
                .insert((*id).into(), 100.0 * (imp + tes + con) as f64 / t as f64);
        }
    }
    for (id, [ns, imp, tes, con]) in &counts {
        if COMPONENT_ORDER.contains(&id.as_str()) {
            continue;
        }
        blocks.push(ComponentBlock {
            id: id.clone(),
            label: component_label(id),
            not_started: *ns,
            implemented: *imp,
            tested: *tes,
            conformant: *con,
        });
    }
    metrics.components = blocks;
}

fn status_of(item: &Value) -> UnitStatus {
    if item.get("conformant").and_then(Value::as_bool) == Some(true) {
        return UnitStatus::Conformant;
    }
    let status = item
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_ascii_lowercase();
    match status.as_str() {
        "conformant" | "conforming" | "passing" | "passed" | "done" | "complete" | "completed" => {
            UnitStatus::Conformant
        }
        "tested" | "test_passed" | "judge_passed" => UnitStatus::Tested,
        "implemented" => UnitStatus::Implemented,
        _ => UnitStatus::NotStarted,
    }
}

pub fn component_label(id: &str) -> &'static str {
    match id {
        "rest" => "REST",
        "auth" => "Auth",
        "realtime" => "Realtime",
        "storage" => "Storage",
        "functions" => "Functions",
        "pooler" => "Pooler",
        "meta" => "Meta",
        "studio" => "Studio",
        "core" => "Core",
        _ => "Other",
    }
}

pub fn comma(n: usize) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, ch) in s.chars().rev().enumerate() {
        if i > 0 && i % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    out.chars().rev().collect()
}

pub fn pct(value: f64) -> String {
    format!("{value:.1}%")
}

pub fn compact_pct(value: f64) -> String {
    let rounded = value.round();
    if (value - rounded).abs() < 0.05 {
        format!("{}%", rounded as i64)
    } else {
        format!("{value:.1}%")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn placeholder_uses_em_dash_not_a_fake_total() {
        let metrics = Metrics::placeholder();
        assert_eq!(metrics.passing_total_label(), "—");
        assert_eq!(metrics.coverage_label(), "—");
        assert_eq!(metrics.conformance_label(), "—");
        assert_eq!(metrics.coverage_conformance_label(), "—");
        assert!(metrics.components.is_empty());
        assert_eq!(metrics.total, None);
    }

    #[test]
    fn units_array_length_is_the_denominator() {
        let list = vec![
            json!({"component": "rest", "status": "not_implemented"}),
            json!({"component": "rest", "status": "implemented"}),
            json!({"component": "auth", "status": "tested"}),
            json!({"component": "auth", "status": "conformant"}),
        ];
        let mut metrics = Metrics::placeholder();
        apply_units(&mut metrics, &list);
        assert_eq!(metrics.total, Some(4));
        assert_eq!(metrics.passing, Some(1));
        assert_eq!(metrics.passing_total_label(), "1 / 4");
        let rest = metrics
            .components
            .iter()
            .find(|c| c.id == "rest")
            .expect("rest");
        assert_eq!(rest.not_started, 1);
        assert_eq!(rest.implemented, 1);
        let auth = metrics
            .components
            .iter()
            .find(|c| c.id == "auth")
            .expect("auth");
        assert_eq!(auth.tested, 1);
        assert_eq!(auth.conformant, 1);
    }
}
