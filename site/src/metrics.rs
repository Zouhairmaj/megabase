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
pub struct FeatureGroup {
    pub id: String,
    pub label: String,
    pub units: Vec<UnitStatus>,
}

impl FeatureGroup {
    pub fn total(&self) -> usize {
        self.units.len()
    }
}

#[derive(Clone, Debug)]
pub struct ComponentBlock {
    pub id: String,
    pub label: &'static str,
    pub not_started: usize,
    pub implemented: usize,
    pub tested: usize,
    pub conformant: usize,
    pub units: Vec<UnitStatus>,
    pub groups: Vec<FeatureGroup>,
}

impl ComponentBlock {
    pub fn from_counts(
        id: impl Into<String>,
        label: &'static str,
        not_started: usize,
        implemented: usize,
        tested: usize,
        conformant: usize,
    ) -> Self {
        let mut units = Vec::with_capacity(not_started + implemented + tested + conformant);
        units.extend(std::iter::repeat_n(UnitStatus::NotStarted, not_started));
        units.extend(std::iter::repeat_n(UnitStatus::Implemented, implemented));
        units.extend(std::iter::repeat_n(UnitStatus::Tested, tested));
        units.extend(std::iter::repeat_n(UnitStatus::Conformant, conformant));
        Self {
            id: id.into(),
            label,
            not_started,
            implemented,
            tested,
            conformant,
            units,
            groups: Vec::new(),
        }
    }

    pub fn from_units(id: impl Into<String>, label: &'static str, units: Vec<UnitStatus>) -> Self {
        Self::from_groups(id, label, Vec::new(), units)
    }

    pub fn from_groups(
        id: impl Into<String>,
        label: &'static str,
        groups: Vec<FeatureGroup>,
        fallback_units: Vec<UnitStatus>,
    ) -> Self {
        let units = if groups.is_empty() {
            fallback_units
        } else {
            groups
                .iter()
                .flat_map(|g| g.units.iter().copied())
                .collect()
        };
        let not_started = units
            .iter()
            .filter(|s| **s == UnitStatus::NotStarted)
            .count();
        let implemented = units
            .iter()
            .filter(|s| **s == UnitStatus::Implemented)
            .count();
        let tested = units.iter().filter(|s| **s == UnitStatus::Tested).count();
        let conformant = units
            .iter()
            .filter(|s| **s == UnitStatus::Conformant)
            .count();
        Self {
            id: id.into(),
            label,
            not_started,
            implemented,
            tested,
            conformant,
            units,
            groups,
        }
    }

    pub fn total(&self) -> usize {
        if self.units.is_empty() {
            self.not_started + self.implemented + self.tested + self.conformant
        } else {
            self.units.len()
        }
    }

    pub fn unit_statuses(&self) -> Vec<UnitStatus> {
        if !self.units.is_empty() {
            return self.units.clone();
        }
        let mut units = Vec::with_capacity(self.total());
        units.extend(std::iter::repeat_n(
            UnitStatus::NotStarted,
            self.not_started,
        ));
        units.extend(std::iter::repeat_n(
            UnitStatus::Implemented,
            self.implemented,
        ));
        units.extend(std::iter::repeat_n(UnitStatus::Tested, self.tested));
        units.extend(std::iter::repeat_n(UnitStatus::Conformant, self.conformant));
        units
    }
}

#[derive(Clone, Debug)]
pub struct VendorPin {
    pub name: String,
    pub tag: String,
}

#[derive(Clone, Debug)]
pub struct Metrics {
    pub passing: Option<usize>,
    pub total: Option<usize>,
    pub coverage: Option<f64>,
    pub conformance: Option<f64>,
    pub components: Vec<ComponentBlock>,
    pub by_component: BTreeMap<String, f64>,
    pub vendor: BTreeMap<String, VendorPin>,
    pub source: String,
    pub stage: String,
    pub stage_short: String,
    pub human_interventions: usize,
    pub spend_label: String,
    pub next_milestone: String,
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
            vendor: BTreeMap::new(),
            source: "placeholder".into(),
            stage: FALLBACK_STAGE.into(),
            stage_short: FALLBACK_STAGE_SHORT.into(),
            human_interventions: 0,
            spend_label: "—".into(),
            next_milestone: "Level 1 · REST + Auth".into(),
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

    pub fn component(&self, id: &str) -> Option<&ComponentBlock> {
        self.components.iter().find(|c| c.id == id)
    }

    pub fn component_progress(&self, id: &str) -> String {
        match self.component(id) {
            Some(block) => format!("{} / {}", comma(block.conformant), comma(block.total())),
            None if self.has_data() => "0 / 0".into(),
            None => "—".into(),
        }
    }

    pub fn pin(&self, vendor_key: &str) -> String {
        self.vendor
            .get(vendor_key)
            .map(|v| v.tag.clone())
            .filter(|t| !t.is_empty())
            .unwrap_or_else(|| "—".into())
    }

    pub fn human_interventions_label(&self) -> String {
        comma(self.human_interventions)
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
                apply_vendor(&mut metrics, &value);
                if let Some(summary) = value.get("summary") {
                    if metrics.source == "placeholder" {
                        apply_summary(&mut metrics, summary);
                    }
                    merge_components(&mut metrics, summary);
                }
                if let Some(n) = value.get("total").and_then(Value::as_u64) {
                    if metrics.total.is_none() {
                        metrics.total = Some(n as usize);
                    }
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

fn apply_vendor(metrics: &mut Metrics, value: &Value) {
    let Some(list) = value.get("vendor").and_then(Value::as_array) else {
        return;
    };
    for item in list {
        let name = item
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        if name.is_empty() {
            continue;
        }
        let tag = item
            .get("tag")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        metrics.vendor.insert(name.clone(), VendorPin { name, tag });
    }
}

fn apply_summary(metrics: &mut Metrics, value: &Value) {
    if let Some(totals) = value.get("totals") {
        if let Some(n) = totals.get("units").and_then(Value::as_u64) {
            metrics.total = Some(n as usize);
        }
        if let Some(n) = totals.get("conformant").and_then(Value::as_u64) {
            metrics.passing = Some(n as usize);
        }
    }
    if let Some(n) = value.get("total_units").and_then(Value::as_u64) {
        metrics.total = Some(n as usize);
    }
    if let Some(n) = value.get("conformant").and_then(Value::as_u64) {
        metrics.passing = Some(n as usize);
    }
    if let Some(pct) = value.get("percent") {
        if let Some(n) = pct.get("coverage").and_then(Value::as_f64) {
            metrics.coverage = Some(n);
        }
        if let Some(n) = pct.get("conformance").and_then(Value::as_f64) {
            metrics.conformance = Some(n);
        }
    }
    if let Some(n) = value.get("coverage_percent").and_then(Value::as_f64) {
        metrics.coverage = Some(n);
    }
    if let Some(n) = value.get("conformance_percent").and_then(Value::as_f64) {
        metrics.conformance = Some(n);
    }
    merge_components(metrics, value);
    if let Some(map) = value.get("components").and_then(Value::as_object) {
        merge_phase0_components(metrics, map);
    }
    if metrics.components.is_empty() {
        blocks_from_summary(metrics, value);
    }
}

fn merge_phase0_components(metrics: &mut Metrics, map: &serde_json::Map<String, Value>) {
    if !metrics.components.is_empty() {
        for (name, stats) in map {
            if let Some(n) = stats
                .get("units")
                .or_else(|| stats.get("total"))
                .and_then(Value::as_u64)
            {
                if n > 0 {
                    metrics.by_component.insert(
                        name.clone(),
                        stats
                            .get("implemented")
                            .and_then(Value::as_u64)
                            .map(|implemented| 100.0 * implemented as f64 / n as f64)
                            .unwrap_or(0.0),
                    );
                }
            }
        }
        return;
    }
    let mut blocks = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for id in COMPONENT_ORDER {
        if let Some(stats) = map.get(*id) {
            seen.insert(*id);
            if let Some(block) = block_from_phase0(id, stats) {
                blocks.push(block);
            }
        }
    }
    for (id, stats) in map {
        if seen.contains(id.as_str()) {
            continue;
        }
        if let Some(block) = block_from_phase0(id, stats) {
            blocks.push(block);
        }
    }
    if !blocks.is_empty() {
        metrics.components = blocks;
    }
}

fn block_from_phase0(id: &str, stats: &Value) -> Option<ComponentBlock> {
    let total = stats
        .get("units")
        .or_else(|| stats.get("total"))
        .and_then(Value::as_u64)
        .unwrap_or(0) as usize;
    if total == 0 {
        return None;
    }
    let implemented = stats
        .get("implemented")
        .and_then(Value::as_u64)
        .unwrap_or(0) as usize;
    let tested = stats.get("tested").and_then(Value::as_u64).unwrap_or(0) as usize;
    let conformant = stats.get("conformant").and_then(Value::as_u64).unwrap_or(0) as usize;
    let mut groups = Vec::new();
    if let Some(map) = stats.get("groups").and_then(Value::as_object) {
        let mut keys: Vec<_> = map.keys().cloned().collect();
        keys.sort();
        for key in keys {
            let Some(gstats) = map.get(&key) else {
                continue;
            };
            let n = gstats.get("units").and_then(Value::as_u64).unwrap_or(0) as usize;
            if n == 0 {
                continue;
            }
            let g_impl = gstats
                .get("implemented")
                .and_then(Value::as_u64)
                .unwrap_or(0) as usize;
            let g_tested = gstats.get("tested").and_then(Value::as_u64).unwrap_or(0) as usize;
            let g_conf = gstats
                .get("conformant")
                .and_then(Value::as_u64)
                .unwrap_or(0) as usize;
            let impl_only = g_impl.saturating_sub(g_tested.max(g_conf));
            let tested_only = g_tested.saturating_sub(g_conf);
            let not_started = n.saturating_sub(g_impl.max(g_tested).max(g_conf));
            let mut units = Vec::with_capacity(n);
            units.extend(std::iter::repeat_n(UnitStatus::NotStarted, not_started));
            units.extend(std::iter::repeat_n(UnitStatus::Implemented, impl_only));
            units.extend(std::iter::repeat_n(UnitStatus::Tested, tested_only));
            units.extend(std::iter::repeat_n(UnitStatus::Conformant, g_conf));
            while units.len() < n {
                units.push(UnitStatus::NotStarted);
            }
            groups.push(FeatureGroup {
                id: key.clone(),
                label: group_label(&key),
                units,
            });
        }
    }
    Some(ComponentBlock::from_groups(
        id,
        component_label(id),
        groups,
        {
            let impl_only = implemented.saturating_sub(tested.max(conformant));
            let tested_only = tested.saturating_sub(conformant);
            let not_started = total.saturating_sub(implemented.max(tested).max(conformant));
            let mut units = Vec::with_capacity(total);
            units.extend(std::iter::repeat_n(UnitStatus::NotStarted, not_started));
            units.extend(std::iter::repeat_n(UnitStatus::Implemented, impl_only));
            units.extend(std::iter::repeat_n(UnitStatus::Tested, tested_only));
            units.extend(std::iter::repeat_n(UnitStatus::Conformant, conformant));
            units
        },
    ))
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
        blocks.push(ComponentBlock::from_counts(
            *id,
            component_label(id),
            not_started,
            implemented_only,
            tested_only,
            conformant,
        ));
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
        blocks.push(ComponentBlock::from_counts(
            id.clone(),
            component_label(id),
            total,
            0,
            0,
            0,
        ));
    }
    metrics.components = blocks;
}

fn apply_units(metrics: &mut Metrics, list: &[Value]) {
    let mut grouped: BTreeMap<String, BTreeMap<String, Vec<UnitStatus>>> = BTreeMap::new();
    let mut passing = 0usize;
    let mut implemented_n = 0usize;
    for item in list {
        let component = item
            .get("component")
            .and_then(Value::as_str)
            .unwrap_or("other")
            .to_string();
        let group = item
            .get("group")
            .and_then(Value::as_str)
            .unwrap_or("units")
            .to_string();
        let status = status_of(item);
        if status == UnitStatus::Conformant {
            passing += 1;
        }
        if status != UnitStatus::NotStarted {
            implemented_n += 1;
        }
        grouped
            .entry(component)
            .or_default()
            .entry(group)
            .or_default()
            .push(status);
    }

    let total = list.len();
    metrics.total = Some(total);
    metrics.passing = Some(passing);
    if metrics.coverage.is_none() && total > 0 {
        metrics.coverage = Some(100.0 * implemented_n as f64 / total as f64);
    }
    if metrics.conformance.is_none() && total > 0 {
        metrics.conformance = Some(100.0 * passing as f64 / total as f64);
    }

    let mut blocks = Vec::new();
    let mut remaining = grouped;
    for id in COMPONENT_ORDER {
        let Some(groups_map) = remaining.remove(*id) else {
            continue;
        };
        let block = block_from_grouped(id, groups_map);
        let t = block.total();
        if t > 0 {
            let implemented = t - block.not_started;
            metrics
                .by_component
                .insert((*id).into(), 100.0 * implemented as f64 / t as f64);
        }
        blocks.push(block);
    }
    for (id, groups_map) in remaining {
        blocks.push(block_from_grouped(&id, groups_map));
    }
    metrics.components = blocks;
}

fn block_from_grouped(id: &str, groups_map: BTreeMap<String, Vec<UnitStatus>>) -> ComponentBlock {
    let mut groups: Vec<FeatureGroup> = groups_map
        .into_iter()
        .map(|(gid, units)| FeatureGroup {
            label: group_label(&gid),
            id: gid,
            units,
        })
        .collect();
    groups.sort_by(|a, b| a.id.cmp(&b.id));
    let flat: Vec<UnitStatus> = groups
        .iter()
        .flat_map(|g| g.units.iter().copied())
        .collect();
    ComponentBlock::from_groups(id, component_label(id), groups, flat)
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

pub fn group_label(id: &str) -> String {
    id.replace('-', " ").replace('_', " ")
}

pub struct CatalogRow {
    pub id: &'static str,
    pub name: &'static str,
    pub upstream: &'static str,
    pub vendor_key: &'static str,
    pub path: &'static str,
    pub crate_name: &'static str,
    pub levels: &'static str,
}

pub const CATALOG: &[CatalogRow] = &[
    CatalogRow {
        id: "rest",
        name: "REST API",
        upstream: "PostgREST · Haskell · MIT",
        vendor_key: "postgrest",
        path: "/rest/v1",
        crate_name: "megabase-rest",
        levels: "L1",
    },
    CatalogRow {
        id: "auth",
        name: "Auth",
        upstream: "Supabase Auth · Go · MIT",
        vendor_key: "auth",
        path: "/auth/v1",
        crate_name: "megabase-auth",
        levels: "L1–L2",
    },
    CatalogRow {
        id: "storage",
        name: "Storage",
        upstream: "Storage API · TypeScript · Apache-2.0",
        vendor_key: "storage",
        path: "/storage/v1",
        crate_name: "megabase-storage",
        levels: "L2",
    },
    CatalogRow {
        id: "realtime",
        name: "Realtime",
        upstream: "Realtime · Elixir · Apache-2.0",
        vendor_key: "realtime",
        path: "/realtime/v1",
        crate_name: "megabase-realtime",
        levels: "L3",
    },
    CatalogRow {
        id: "functions",
        name: "Edge Functions",
        upstream: "Edge Runtime · Rust+Deno · MIT",
        vendor_key: "edge-runtime",
        path: "/functions/v1",
        crate_name: "megabase-functions",
        levels: "L4",
    },
    CatalogRow {
        id: "pooler",
        name: "Pooler",
        upstream: "Supavisor · Elixir · Apache-2.0",
        vendor_key: "supavisor",
        path: "pg wire",
        crate_name: "megabase-pooler",
        levels: "L4",
    },
    CatalogRow {
        id: "meta",
        name: "Postgres Meta",
        upstream: "postgres-meta · TypeScript · Apache-2.0",
        vendor_key: "postgres-meta",
        path: "/pg",
        crate_name: "megabase-meta",
        levels: "L4",
    },
    CatalogRow {
        id: "studio",
        name: "Studio",
        upstream: "supabase/studio · Next.js · Apache-2.0",
        vendor_key: "supabase",
        path: "/ (dashboard)",
        crate_name: "megabase-studio",
        levels: "L4 test · L5",
    },
];

pub fn status_tag(block: Option<&ComponentBlock>) -> &'static str {
    match block {
        None => "DAY 0",
        Some(b) if b.conformant == b.total() && b.total() > 0 => "CONFORMANT",
        Some(b) if b.tested > 0 || b.conformant > 0 => "IN PROGRESS",
        Some(b) if b.implemented > 0 => "IMPLEMENTED",
        Some(_) => "NOT STARTED",
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

    #[test]
    fn phase0_component_percent_falls_back_to_total() {
        let mut metrics = Metrics::placeholder();
        metrics.components = vec![ComponentBlock::from_counts("rest", "REST", 2, 0, 0, 0)];
        let map = json!({
            "rest": { "total": 10, "implemented": 2 }
        })
        .as_object()
        .unwrap()
        .clone();
        merge_phase0_components(&mut metrics, &map);
        assert_eq!(metrics.by_component.get("rest"), Some(&20.0));
    }
}
