use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

pub const COMPONENTS: &[(&str, &str)] = &[
    ("rest", "REST"),
    ("auth", "Auth"),
    ("realtime", "Realtime"),
    ("storage", "Storage"),
    ("functions", "Functions"),
    ("pooler", "Pooler"),
    ("meta", "Meta"),
    ("studio", "Studio"),
];

pub fn component_label(component: &str) -> &str {
    COMPONENTS
        .iter()
        .find(|(id, _)| *id == component)
        .map(|(_, label)| *label)
        .unwrap_or(component)
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Source {
    pub repo: String,
    pub file: String,
    pub line: usize,
}

/// One unit of upstream behavior: the atom of the coverage denominator.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Unit {
    pub id: String,
    pub component: String,
    pub group: String,
    pub kind: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub method: Option<String>,
    /// Path as a client sees it through the gateway, or on the component's
    /// own listener when it is not behind the gateway (pooler).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    pub level: u8,
    pub source: Source,
}

/// Upstream behavior that was found but deliberately left out of the
/// denominator, with the reason.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Excluded {
    pub component: String,
    pub name: String,
    pub reason: String,
    pub source: Source,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Pin {
    pub name: String,
    pub path: String,
    pub repo: String,
    pub tag: String,
    pub commit: String,
    pub license: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct PinsFile {
    pub pin: Vec<Pin>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UnitsFile {
    pub schema: u32,
    pub generated_by: String,
    pub vendor: Vec<Pin>,
    pub total: usize,
    pub by_component: BTreeMap<String, usize>,
    pub units: Vec<Unit>,
    pub excluded: Vec<Excluded>,
}

/// Accumulates units during extraction, deduplicating by id.
#[derive(Default)]
pub struct Collector {
    units: BTreeMap<String, Unit>,
    excluded: Vec<Excluded>,
}

pub struct UnitSpec<'a> {
    pub component: &'a str,
    pub group: &'a str,
    pub kind: &'a str,
    pub name: String,
    pub level: u8,
    pub source: Source,
}

impl Collector {
    pub fn item(&mut self, spec: UnitSpec<'_>) {
        let id = format!("{}:{}:{}", spec.component, spec.kind, spec.name);
        self.units.entry(id.clone()).or_insert(Unit {
            id,
            component: spec.component.into(),
            group: spec.group.into(),
            kind: spec.kind.into(),
            name: spec.name,
            method: None,
            path: None,
            level: spec.level,
            source: spec.source,
        });
    }

    #[allow(clippy::too_many_arguments)]
    pub fn route(
        &mut self,
        component: &str,
        group: &str,
        method: &str,
        path: &str,
        level: u8,
        source: Source,
    ) {
        let name = format!("{method} {path}");
        let id = format!("{component}:route:{name}");
        self.units.entry(id.clone()).or_insert(Unit {
            id,
            component: component.into(),
            group: group.into(),
            kind: "route".into(),
            name,
            method: Some(method.into()),
            path: Some(path.into()),
            level,
            source,
        });
    }

    /// Folds route-only groups of `component` with fewer than `min` units into
    /// one `endpoints` group, so the treemap is not a mosaic of tiny blocks.
    pub fn merge_small_route_groups(&mut self, component: &str, min: usize) {
        let mut sizes: BTreeMap<String, (usize, bool)> = BTreeMap::new();
        for unit in self.units.values().filter(|u| u.component == component) {
            let entry = sizes.entry(unit.group.clone()).or_insert((0, true));
            entry.0 += 1;
            entry.1 &= unit.kind == "route";
        }
        for unit in self.units.values_mut().filter(|u| u.component == component) {
            let (size, routes_only) = sizes[&unit.group];
            if routes_only && size < min {
                unit.group = "endpoints".into();
            }
        }
    }

    pub fn exclude(&mut self, component: &str, name: String, reason: &str, source: Source) {
        self.excluded.push(Excluded {
            component: component.into(),
            name,
            reason: reason.into(),
            source,
        });
    }

    pub fn finish(self, vendor: Vec<Pin>) -> UnitsFile {
        let order = |c: &str| COMPONENTS.iter().position(|(id, _)| *id == c).unwrap_or(99);
        let mut units: Vec<Unit> = self.units.into_values().collect();
        units.sort_by(|a, b| {
            (order(&a.component), &a.group, &a.id).cmp(&(order(&b.component), &b.group, &b.id))
        });
        let mut excluded = self.excluded;
        excluded.sort_by(|a, b| {
            (order(&a.component), &a.name, &a.source.file).cmp(&(
                order(&b.component),
                &b.name,
                &b.source.file,
            ))
        });
        excluded.dedup_by(|a, b| a.component == b.component && a.name == b.name);
        let mut by_component = BTreeMap::new();
        for unit in &units {
            *by_component.entry(unit.component.clone()).or_insert(0) += 1;
        }
        UnitsFile {
            schema: 1,
            generated_by: "cargo run -p megabase-coverage -- update".into(),
            vendor,
            total: units.len(),
            by_component,
            units,
            excluded,
        }
    }
}
