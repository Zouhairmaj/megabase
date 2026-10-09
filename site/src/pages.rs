//! Page bodies matching the Kite Website frames.

use crate::chrome::Paths;
use crate::devlog;
use crate::html::esc;
use crate::markdown;
use crate::metrics::{self, CatalogRow, Metrics, CATALOG};
use crate::treemap;
use crate::GITHUB;

pub fn home(paths: &Paths, metrics: &Metrics) -> String {
    let treemap = treemap_block(metrics);
    let status_panel = status_panel(metrics);
    let components = component_cards(paths, metrics);
    let faq = home_faq(paths, metrics);
    format!(
        r#"<main id="main" class="home-main">
<section class="hero">
  <div class="hero-copy">
    <p class="kicker hide-mobile">{kicker_desktop}</p>
    <p class="kicker hide-desktop">{kicker_mobile}</p>
    <h1 class="display">Supabase,<br />rewritten in Rust.<br /><span class="headline-accent">By agents. In public.</span></h1>
    <p class="lede hide-mobile">Autonomous AI agents are porting every service Supabase has written into one Rust binary that sits next to PostgreSQL. The goal: any supabase-js app runs on it without changing a line of code. An external judge compares every response with the real Supabase stack.</p>
    <p class="lede hide-desktop">AI agents are porting every Supabase service into one Rust binary next to PostgreSQL, judged response by response against the real stack.</p>
    <div class="actions">
      <a class="btn btn-primary" href="{manifesto}">READ THE MANIFESTO</a>
      <a class="btn btn-ghost" href="{status}">SEE LIVE STATUS</a>
      <a class="btn btn-ghost hide-mobile" href="{GITHUB}" rel="noopener noreferrer">GITHUB ↗</a>
    </div>
    <p class="hero-disclaimer">Independent experiment. Not affiliated with or endorsed by Supabase, Inc.</p>
  </div>
  {status_panel}
</section>

<section class="band home-what">
  <p class="kicker">01 · WHAT AND WHY</p>
  <h2 class="section-title">One binary. The same API.<br /><span class="hide-mobile">A precise, verifiable target.</span></h2>
  <p class="lede hide-mobile">Self-hosting Supabase today means running about a dozen containers written in six languages. Megabase aims for one Rust binary next to standard PostgreSQL, serving /rest/v1, /auth/v1, /storage/v1, /realtime/v1 and /functions/v1 exactly as Supabase does.</p>
  <div class="triple">
    <article class="stat-card">
      <p class="card-kicker">SUPABASE SELF-HOSTED</p>
      <p class="stat-xl">~12 containers</p>
      <p>Haskell, Go, Elixir, TypeScript, Rust and Lua.</p>
    </article>
    <article class="stat-card is-accent">
      <p class="card-kicker accent">MEGABASE TARGET</p>
      <p class="stat-xl">1 binary</p>
      <p>Rust only, next to the PostgreSQL you already run. Memory target: under 256 MB.</p>
    </article>
    <article class="stat-card">
      <p class="card-kicker">YOUR APP</p>
      <p class="stat-xl">0 code changes (goal)</p>
      <p>Point supabase-js at Megabase. Same endpoints, bodies, status codes and errors.</p>
    </article>
  </div>
  <div class="triple hide-mobile">
    <article>
      <h3>Measure what agents can build</h3>
      <p>Not a toy or a demo: a large, multi-language, production-grade system with a precise target.</p>
    </article>
    <article>
      <h3>A target with no guessing</h3>
      <p>Supabase is open source, documented and runnable locally. Correct means identical to the real thing.</p>
    </article>
    <article>
      <h3>A useful result</h3>
      <p>One binary, a fraction of the memory, and the same API for everyone who self-hosts.</p>
    </article>
  </div>
</section>

<section class="band home-how">
  <p class="kicker">02 · HOW IT WORKS</p>
  <h2 class="section-title">Agents write the code.<br />An external judge grades it.</h2>
  <ol class="step-row">
    <li><span class="step-num">01</span><h3>Read pinned upstream source</h3><p>Upstream lives in vendor/, frozen. That is the specification.</p></li>
    <li><span class="step-num">02</span><h3>Spec one unit</h3><p>Spec first, then Rust. Failures return a structured 501.</p></li>
    <li><span class="step-num">03</span><h3>Diff against real Supabase</h3><p>Official Supabase runs next to Megabase from the same pins.</p></li>
    <li><span class="step-num">04</span><h3>Keep or revert, never regress</h3><p>A commit is kept only if total conformance does not fall.</p></li>
    <li><span class="step-num">05</span><h3>Record in public</h3><p>Coverage, the treemap, the human log.</p></li>
  </ol>
  <p class="hide-mobile"><a class="text-link" href="{how}">How the loop works →</a></p>
</section>

<section class="band home-live" id="status">
  <div class="band-head hide-mobile">
    <div>
      <p class="kicker">03 · LIVE STATUS</p>
      <h2 class="section-title">Each cell is one unit.<br />Grey until the judge says green.</h2>
    </div>
    <a class="text-link" href="{status}">Full status →</a>
  </div>
  <p class="lede hide-mobile">Nested squarified treemap: component, then feature group, then one whole square per unit. Regenerated at build time from coverage/units.json. Grey is not started. Green is conformant.</p>
  {treemap}
</section>

<section class="band home-levels">
  <div class="band-head">
    <div>
      <p class="kicker">04 · ROADMAP</p>
      <h2 class="section-title hide-mobile">Five public levels.<br />Each one gated by the judge.</h2>
    </div>
    <a class="text-link" href="{roadmap}">Full roadmap →</a>
  </div>
  {levels}
  <p class="note hide-mobile">Each percentage is the conformance required on that level’s own scope before the next level may start, and earlier levels must hold their score. Thresholds come from PROGRESS.md when it exists. Level 5 has no gate yet: it is deferred until a feasibility study.</p>
</section>

<section class="band home-components hide-mobile">
  <p class="kicker">05 · COMPONENTS</p>
  <h2 class="section-title">Every service Supabase wrote.<br />Ported to Rust.</h2>
  <div class="component-grid">{components}</div>
  <p class="note">Plus the API gateway that replaces Kong (Lua / Nginx). PostgreSQL stays external. Licenses: NOTICE.</p>
  <p><a class="text-link" href="{components_href}">Component catalog →</a></p>
</section>

<section class="band faq-teaser hide-mobile">
  <div class="faq-aside">
    <p class="kicker">06 · FAQ</p>
    <h2 class="section-title">Fair questions.</h2>
    <p><a class="text-link" href="{faq_href}">All questions →</a></p>
  </div>
  <div class="faq-list">{faq}</div>
</section>

<section class="cta-band home-cta">
  <h2 class="section-title">Watch the map turn green.</h2>
  <div class="actions">
    <a class="btn btn-primary" href="{GITHUB}" rel="noopener noreferrer">FOLLOW ON GITHUB ↗</a>
    <a class="btn btn-ghost hide-mobile" href="{status}">SEE STATUS</a>
  </div>
</section>
</main>"#,
        manifesto = paths.page("manifesto"),
        status = paths.page("status"),
        how = paths.page("how-it-works"),
        roadmap = paths.page("roadmap"),
        components_href = paths.page("components"),
        faq_href = paths.page("faq"),
        levels = level_cards(metrics, false),
        kicker_desktop = esc(&hero_kicker_desktop(metrics)),
        kicker_mobile = esc(&hero_kicker_mobile(metrics)),
    )
}

fn treemap_block(metrics: &Metrics) -> String {
    let desktop = treemap::render(metrics, treemap::HOME_DESKTOP);
    let mobile = treemap::render(metrics, treemap::HOME_MOBILE);
    let total = metrics
        .total
        .map(metrics::comma)
        .unwrap_or_else(|| "—".into());
    let passing = metrics
        .passing
        .map(metrics::comma)
        .unwrap_or_else(|| "—".into());
    let pct = metrics.conformance_label();
    let head = if metrics.has_data() {
        format!("SUPABASE COMPONENTS: {pct} CONFORMANT ({passing}/{total})")
    } else {
        "SUPABASE COMPONENTS".into()
    };
    let foot_left = if metrics.has_data() {
        format!("{total} units extracted from pinned upstream source · {passing} conformant")
    } else {
        "Units extracted from pinned upstream source. Counts arrive with coverage/.".into()
    };
    format!(
        r#"<div class="treemap-block">
  <div class="treemap-block-head">
    <span>{head}</span>
    <span class="legend-inline hide-mobile">
      <span class="swatch-row"><i class="swatch not-started"></i> not started</span>
      <span class="swatch-row"><i class="swatch implemented"></i> implemented</span>
      <span class="swatch-row"><i class="swatch tested"></i> tested</span>
      <span class="swatch-row"><i class="swatch conformant"></i> conformant (matches real Supabase)</span>
    </span>
  </div>
  <div class="treemap treemap-wide">
    <div class="hide-mobile">{desktop}</div>
    <div class="hide-desktop">{mobile}</div>
  </div>
  <div class="treemap-block-foot">
    <span>{foot_left}</span>
    <span class="hide-mobile">Generated at build from coverage/summary.json · <a href="{github}" rel="noopener noreferrer">commit ↗</a></span>
  </div>
</div>"#,
        head = esc(&head),
        foot_left = esc(&foot_left),
        github = GITHUB,
        desktop = desktop,
        mobile = mobile,
    )
}

fn status_panel(metrics: &Metrics) -> String {
    let desktop = treemap::render(metrics, treemap::HERO_DESKTOP);
    let mobile = treemap::render(metrics, treemap::HERO_MOBILE);
    format!(
        r#"<aside class="status-panel" aria-labelledby="status-title">
  <div class="status-head">
    <span id="status-title">EXPERIMENT STATUS</span>
    <span class="status-updated">updated on every commit</span>
  </div>
  <div class="status-treemap">
    <p class="visually-hidden">{alt}</p>
    <div class="hide-mobile">{desktop}</div>
    <div class="hide-desktop">{mobile}</div>
  </div>
  <dl>
    <div><dt>Units passing the judge</dt><dd>{passing}</dd></div>
    <div><dt>Coverage · Conformance</dt><dd>{coverage}</dd></div>
    <div><dt>Current stage</dt><dd class="accent">{stage}</dd></div>
  </dl>
</aside>"#,
        alt = esc(&treemap::panel_alt(metrics)),
        passing = esc(&metrics.passing_total_label()),
        coverage = esc(&metrics.coverage_conformance_label()),
        stage = esc(&metrics.stage),
    )
}

struct LevelSpec {
    n: u8,
    kicker: &'static str,
    gate: &'static str,
    gate_pct: Option<f64>,
    title: &'static str,
    body: &'static str,
    stretch: bool,
}

const LEVELS: &[LevelSpec] = &[
    LevelSpec {
        n: 1,
        kicker: "LEVEL 1",
        gate: "≥ 95%",
        gate_pct: Some(95.0),
        title: "REST API + email/password auth",
        body: "Goal: most apps run.",
        stretch: false,
    },
    LevelSpec {
        n: 2,
        kicker: "LEVEL 2",
        gate: "≥ 90%",
        gate_pct: Some(90.0),
        title: "OAuth, magic links, Storage",
        body: "Files and social login.",
        stretch: false,
    },
    LevelSpec {
        n: 3,
        kicker: "LEVEL 3",
        gate: "≥ 85%",
        gate_pct: Some(85.0),
        title: "Realtime",
        body: "Database changes, broadcast, presence.",
        stretch: false,
    },
    LevelSpec {
        n: 4,
        kicker: "LEVEL 4",
        gate: "≥ 80%",
        gate_pct: Some(80.0),
        title: "Functions, pooler, Meta + the Studio test",
        body: "Official Studio can’t tell the difference.",
        stretch: false,
    },
    LevelSpec {
        n: 5,
        kicker: "LEVEL 5",
        gate: "stretch",
        gate_pct: None,
        title: "Studio, served from the binary",
        body: "Deferred pending a feasibility study.",
        stretch: true,
    },
];

fn hero_kicker_desktop(metrics: &Metrics) -> String {
    match metrics.passing {
        Some(n) if n > 0 => format!(
            "{} · {} {} PASS · BUILT BY AGENTS, IN PUBLIC",
            metrics.stage_short.to_ascii_uppercase(),
            metrics::comma(n),
            if n == 1 { "UNIT" } else { "UNITS" }
        ),
        _ => "DAY 0 · NOTHING PASSES YET · BUILT BY AGENTS, IN PUBLIC".into(),
    }
}

fn hero_kicker_mobile(metrics: &Metrics) -> String {
    match metrics.passing {
        Some(n) if n > 0 => format!(
            "{} · {} {} PASS",
            metrics.stage_short.to_ascii_uppercase(),
            metrics::comma(n),
            if n == 1 { "UNIT" } else { "UNITS" }
        ),
        _ => "DAY 0 · PHASE 0".into(),
    }
}

fn level_scope_counts(metrics: &Metrics, level: u8) -> Option<(usize, usize)> {
    if !metrics.has_data() {
        return None;
    }
    let needle = format!("L{level}");
    let mut total = 0;
    let mut conformant = 0;
    for row in CATALOG {
        if !row.levels.contains(&needle) {
            continue;
        }
        if let Some(block) = metrics.component(row.id) {
            total += block.total();
            conformant += block.conformant;
        }
    }
    Some((conformant, total))
}

fn level_complete(metrics: &Metrics, spec: &LevelSpec) -> bool {
    let Some(gate) = spec.gate_pct else {
        return false;
    };
    match level_scope_counts(metrics, spec.n) {
        Some((conformant, total)) if total > 0 => 100.0 * conformant as f64 / total as f64 >= gate,
        _ => false,
    }
}

fn level_progress_label(metrics: &Metrics, spec: &LevelSpec) -> String {
    if spec.stretch {
        return "deferred".into();
    }
    match level_scope_counts(metrics, spec.n) {
        None => "— · not started".into(),
        Some((conformant, total)) => {
            let value = if total == 0 {
                0.0
            } else {
                100.0 * conformant as f64 / total as f64
            };
            let state = if conformant == 0 {
                "not started"
            } else if spec.gate_pct.is_some_and(|gate| value >= gate) {
                "complete"
            } else {
                "in progress"
            };
            format!("{} · {state}", metrics::compact_pct(value))
        }
    }
}

fn level_cards(metrics: &Metrics, detailed: bool) -> String {
    let next = LEVELS
        .iter()
        .find(|spec| !spec.stretch && !level_complete(metrics, spec))
        .map(|spec| spec.n);
    let mut cards = String::new();
    for spec in LEVELS {
        let is_next = next == Some(spec.n);
        let class = if spec.stretch {
            "level-card is-stretch"
        } else if is_next {
            "level-card is-next"
        } else {
            "level-card"
        };
        let kicker = if is_next {
            format!("{} · NEXT", spec.kicker)
        } else {
            spec.kicker.to_string()
        };
        let progress = level_progress_label(metrics, spec);
        let extra = if detailed {
            String::new()
        } else {
            format!(
                r#"<div class="level-foot"><div class="bar" aria-hidden="true"></div><p class="muted">{}</p></div>"#,
                esc(&progress)
            )
        };
        cards.push_str(&format!(
            r#"<article class="{class}"><div class="level-head"><span class="card-kicker {kicker_class}">{kicker}</span><span class="muted">{gate}</span></div><h3>{title}</h3><p>{body}</p>{extra}</article>"#,
            kicker_class = if is_next { "accent" } else { "" },
            gate = spec.gate,
            title = spec.title,
            body = spec.body,
        ));
    }
    format!(r#"<div class="level-row">{cards}</div>"#)
}

fn component_cards(paths: &Paths, metrics: &Metrics) -> String {
    let mut out = String::new();
    for row in CATALOG {
        let tag = metrics::status_tag(metrics.component(row.id));
        let progress = metrics.component_progress(row.id);
        out.push_str(&format!(
            r#"<a class="comp-card" href="{href}">
  <p class="card-kicker">{name}</p>
  <p class="comp-path">{path}</p>
  <p class="muted">{upstream}</p>
  <p class="comp-meta"><span class="tag">{tag}</span><span>{progress}</span></p>
</a>"#,
            href = paths.page("components"),
            name = esc(row.name),
            path = esc(row.path),
            upstream = esc(row.upstream),
            progress = esc(&progress),
        ));
    }
    out
}

fn home_faq(paths: &Paths, metrics: &Metrics) -> String {
    let items = [
        (
            "Can I use it in production?",
            "No. Today every endpoint returns 501 MEGABASE_NOT_IMPLEMENTED. Watch the levels: Level 1 is the first point where real apps should run.".into(),
        ),
        (
            "Is this affiliated with Supabase?",
            "No. It is an independent experiment. “Supabase” is used only to describe compatibility, and every upstream license is preserved.".into(),
        ),
        (
            "Who decides what “correct” means?",
            "The real Supabase stack, run side by side from pinned versions. Agents can’t change what the judge compares against.".into(),
        ),
        (
            "Do humans write any code?",
            format!(
                "Humans wrote the manifesto, GOAL.md and the initial setup. Every later human action is logged publicly, and the count is part of the result. Interventions so far: {}.",
                metrics.human_interventions_label()
            ),
        ),
    ];
    let mut out = String::new();
    for (q, a) in items {
        out.push_str(&format!(
            r#"<div class="faq-row"><h3>{q}</h3><p>{a}</p></div>"#,
            q = esc(q),
            a = esc(&a),
        ));
    }
    let _ = paths;
    out
}

fn unit_count_phrase(metrics: &Metrics) -> String {
    match metrics.total {
        Some(n) => format!(
            "{} units are identified in coverage/units.json; {} are passing the judge.",
            metrics::comma(n),
            metrics
                .passing
                .map(metrics::comma)
                .unwrap_or_else(|| "—".into())
        ),
        None => {
            "The denominator is extracted from the pinned upstream source into coverage/units.json. Until that file exists this page does not invent a count.".into()
        }
    }
}

pub fn how_it_works(paths: &Paths, metrics: &Metrics) -> String {
    let _ = paths;
    format!(
        r#"<main id="main" class="doc-page">
  <header class="page-hero">
    <p class="kicker">HOW IT WORKS · FROM GOAL.md</p>
    <h1 class="display-sm">Agents read the source.<br />A judge they cannot touch keeps score.</h1>
    <p class="lede">The upstream Supabase repositories are pinned in vendor/ and treated as a frozen specification. Agents port one unit at a time. Nothing is “done” until the external judge says Megabase’s response matches the real stack.</p>
    {strip}
  </header>
  <section class="band">
    <p class="kicker">01 · WHO DOES WHAT</p>
    <div class="quad">
      <article class="panel"><h3>Orchestrator</h3><p>Reads coverage, assigns targets, merges branches, enforces levels.</p></article>
      <article class="panel"><h3>Builders</h3><p>One per component, each on its own branch, following the loop.</p></article>
      <article class="panel"><h3>Reviewer</h3><p>Has not seen the code being reviewed. Checks the hard rules. Any change to the judge needs reviewer approval.</p></article>
      <article class="panel"><h3>Humans</h3><p>Wrote the mission and the operating manual. Every other action is an intervention, logged in public.</p></article>
    </div>
  </section>
  <section class="band">
    <p class="kicker">02 · THE LOOP, EVERY ITERATION</p>
    <ol class="loop-list">
      <li><strong>01 Read the state</strong> GOAL.md, PROGRESS.md, coverage/, the board.</li>
      <li><strong>02 Pick one target</strong> Top Ready issue for this role and component.</li>
      <li><strong>03 Write the spec first</strong> Inputs, outputs, errors, upstream files.</li>
      <li><strong>04 Implement in Rust</strong> Fail loudly with MEGABASE_NOT_IMPLEMENTED.</li>
      <li><strong>05 Judge</strong> Side-by-side against the pinned official stack.</li>
      <li><strong>06 Keep or revert</strong> No regressions. Conformance must not fall.</li>
      <li><strong>07 Record</strong> PR, coverage, treemap, comments.</li>
    </ol>
  </section>
  <section class="split-band">
    <article class="panel">
      <p class="kicker">03 · THE JUDGE</p>
      <h2>Official Supabase runs next to Megabase.</h2>
      <p>Same pinned versions. Same requests. Status, headers that matter, and bodies are compared, with documented normalization for timestamps and IDs. Agents may build the harness. They may never change what it compares against.</p>
    </article>
    <article class="panel">
      <p class="kicker">04 · FAIL LOUDLY</p>
      <h2>Nothing silently wrong.</h2>
      <div class="code-block">
        <div class="code-head"><span>JSON</span></div>
        <pre><code>{{
  "code": "MEGABASE_NOT_IMPLEMENTED",
  "component": "rest",
  "unit": "…",
  "message": "…"
}}</code></pre>
      </div>
    </article>
  </section>
  <section class="band">
    <p class="kicker">06 · HARD RULES FOR AGENTS</p>
    <ol class="rules">
      <li>Rust only for in-scope code.</li>
      <li>Never modify vendor/ or judge/ on a feature branch.</li>
      <li>Never disable, skip or weaken a test to raise a score.</li>
      <li>No regressions.</li>
      <li>Fail loudly: HTTP 501 with a structured body.</li>
      <li>Credit the source in NOTICE and LICENSES/.</li>
      <li>No secrets, no production data.</li>
      <li>One iteration, one commit.</li>
    </ol>
    <p class="note">Total conformance = share of all differential tests where Megabase’s response is identical to Supabase’s.</p>
  </section>
  <section class="band">
    <p class="kicker">07 · RUN IN PUBLIC ON GITHUB</p>
    <p class="lede">Milestones are levels. Epics group units. Every change is a pull request. The board is the work queue.</p>
    <p class="note">Issue counts per column are filled at build time from the GitHub Project when that data is present. Today: —</p>
    <p><a class="text-link" href="{GITHUB}" rel="noopener noreferrer">Open the repository ↗</a></p>
  </section>
</main>"#,
        strip = day0_strip(metrics),
    )
}

fn day0_strip(metrics: &Metrics) -> String {
    format!(
        r#"<div class="day0-strip">
  <div><span class="muted">Units</span><strong>{passing}</strong></div>
  <div><span class="muted">Coverage</span><strong>{coverage}</strong></div>
  <div><span class="muted">Conformance</span><strong>{conformance}</strong></div>
  <div><span class="muted">Stage</span><strong class="accent">{stage}</strong></div>
</div>"#,
        passing = esc(&metrics.passing_total_label()),
        coverage = esc(&metrics.coverage_label()),
        conformance = esc(&metrics.conformance_label()),
        stage = esc(&metrics.stage_short),
    )
}

pub fn status(paths: &Paths, metrics: &Metrics) -> String {
    let desktop = treemap::render(metrics, treemap::STATUS_DESKTOP);
    let mobile = treemap::render(metrics, treemap::STATUS_MOBILE);
    let units_headline = metrics.passing_total_label();
    let units_sub = match metrics.total {
        Some(_) => format!(
            "units conformant · {} components · {} groups",
            metrics.live_component_count(),
            metrics.group_count()
        ),
        None => format!(
            "units conformant · {} components · — groups",
            metrics.live_component_count()
        ),
    };
    let mut rows = String::from(
        r#"<div class="data-table status-table" role="table" aria-label="By component">
<div class="data-row head" role="row"><span>Component</span><span>Path</span><span>Level</span><span>Units</span><span>Status</span></div>"#,
    );
    for row in CATALOG {
        let block = metrics.component(row.id);
        let units = match block {
            Some(b) if b.total() > 0 => format!(
                "{} / {}",
                metrics::comma(b.conformant),
                metrics::comma(b.total())
            ),
            _ => "—".into(),
        };
        rows.push_str(&format!(
            r#"<div class="data-row" role="row"><span class="scope-name">{name}</span><span>{path}</span><span>{levels}</span><span>{units}</span><span class="tag">{tag}</span></div>"#,
            name = esc(row.name),
            path = esc(row.path),
            levels = esc(row.levels),
            tag = metrics::status_tag(block),
        ));
    }
    rows.push_str("</div>");
    format!(
        r#"<main id="main" class="doc-page">
  <header class="page-hero">
    <p class="kicker">LIVE STATUS · DAY 0 · {stage}</p>
    <h1 class="display-sm">Where the experiment stands.</h1>
    <p class="lede">Every number on this page is generated at build time from files in the repository. If a file is missing the cell is an em dash, never a made-up total.</p>
    <div class="metric-grid">
      <div class="metric"><p class="muted">Units done</p><p class="stat-xl">{passing}</p></div>
      <div class="metric"><p class="muted">Coverage</p><p class="stat-xl">{coverage}</p></div>
      <div class="metric"><p class="muted">Conformance</p><p class="stat-xl">{conformance}</p></div>
      <div class="metric"><p class="muted">Human interventions</p><p class="stat-xl">{humans}</p></div>
    </div>
  </header>
  <section class="band status-map">
    <div class="status-map-copy">
      <p class="kicker">COMPONENT MAP</p>
      <h2>Component map</h2>
      <p>Each block is one Supabase component, split into its feature groups. Each square is one unit extracted from the pinned upstream source. Grey until the judge says green.</p>
      <p class="stat-xl status-map-count">{units_headline}</p>
      <p class="status-map-sub">{units_sub}</p>
      <ul class="status-legend">
        <li><i class="swatch not-started"></i> not started</li>
        <li><i class="swatch implemented"></i> implemented</li>
        <li><i class="swatch tested"></i> tested</li>
        <li><i class="swatch conformant"></i> conformant (matches real Supabase)</li>
      </ul>
      <p class="status-map-source">Generated at build time from coverage/units.json and coverage/summary.json. One whole square per unit, never cut.</p>
    </div>
    <div class="status-map-svg treemap">
      <div class="hide-mobile">{desktop}</div>
      <div class="hide-desktop">{mobile}</div>
    </div>
  </section>
  <section class="band">
    <h2>By component</h2>
    {rows}
  </section>
  <section class="split-band">
    <div>
      <h2>Level gates</h2>
      {levels}
    </div>
    <aside class="panel">
      <h2>Record</h2>
      <p>Verified apps —</p>
      <p>Regressions —</p>
      <p>Blocked —</p>
      <p>Judge disputes —</p>
      <p><a class="text-link" href="{roadmap}">Roadmap →</a></p>
    </aside>
  </section>
</main>"#,
        stage = esc(&metrics.stage_short),
        passing = esc(&metrics.passing_total_label()),
        coverage = esc(&metrics.coverage_label()),
        conformance = esc(&metrics.conformance_label()),
        humans = esc(&metrics.human_interventions_label()),
        units_headline = esc(&units_headline),
        units_sub = esc(&units_sub),
        desktop = desktop,
        mobile = mobile,
        levels = level_cards(metrics, true),
        roadmap = paths.page("roadmap"),
    )
}

pub fn roadmap(paths: &Paths, metrics: &Metrics, extra_md: Option<&str>) -> String {
    let extra = extra_md
        .map(|md| {
            format!(
                r#"<section class="band article-prose"><h2>From docs/ROADMAP.md</h2>{}</section>"#,
                markdown::render(md)
            )
        })
        .unwrap_or_default();
    format!(
        r#"<main id="main" class="doc-page">
  <header class="page-hero">
    <p class="kicker">ROADMAP · DAY 0 · PHASE 0 IN PROGRESS</p>
    <h1 class="display-sm">Five levels, gated in order. No skipping.</h1>
    <p class="lede">A level starts only when the previous one holds its conformance threshold. The experiment succeeds when real, unmodified open-source Supabase apps run on Megabase and the official Studio cannot tell the difference.</p>
    {strip}
  </header>
  <section class="band">
    <p class="kicker">Critical path</p>
    {levels}
    <p class="note">Gates are conformance thresholds from PROGRESS.md: the share of differential tests on that level’s scope that must match Supabase. Until PROGRESS.md exists the gates shown are the published defaults from GOAL.md.</p>
  </section>
  <section class="band">
    <h2>What each level means</h2>
    <article class="level-detail"><h3>Phase 0 · bootstrap</h3><p>Workspace, empty crates, judge, coverage denominator, CI, NOTICE. Awaits human review.</p></article>
    <article class="level-detail"><h3>Level 1</h3><p>/rest/v1 (PostgREST behaviour) + /auth/v1 email/password, JWT, auth.users, auth.uid(), auth.jwt().</p></article>
    <article class="level-detail"><h3>Level 2</h3><p>OAuth providers, magic links, OTP, /storage/v1 with storage.objects and its RLS.</p></article>
    <article class="level-detail"><h3>Level 3</h3><p>/realtime/v1: database changes via logical replication, broadcast, presence.</p></article>
    <article class="level-detail"><h3>Level 4</h3><p>/functions/v1, pooler, Postgres Meta, then the Studio test.</p></article>
    <article class="level-detail is-stretch"><h3>Level 5 · stretch</h3><p>Studio served from the megabase binary. Deferred pending a feasibility study. Never embed a Node runtime.</p></article>
  </section>
  <aside class="callout">Do not start a level until the previous one reaches the conformance threshold. Prefer finishing started work over starting new work.</aside>
  <p class="band"><a class="text-link" href="{status}">SEE LIVE STATUS →</a></p>
  {extra}
</main>"#,
        strip = day0_strip(metrics),
        levels = level_cards(metrics, false),
        status = paths.page("status"),
    )
}

pub fn components(paths: &Paths, metrics: &Metrics) -> String {
    let total = metrics
        .total
        .map(|n| format!("{} UNITS", metrics::comma(n)))
        .unwrap_or_else(|| "— UNITS".into());
    let mut head = String::from(
        r#"<div class="data-table" role="table" aria-label="Supabase components">
<div class="data-row data-head" role="row"><span>COMPONENT</span><span>UPSTREAM</span><span>VENDOR PIN</span><span>PATH</span><span>CRATE</span><span>LEVELS</span><span>PROGRESS</span><span>STATUS</span></div>"#,
    );
    for row in CATALOG {
        head.push_str(&catalog_row(metrics, row));
    }
    head.push_str(
        r#"<div class="data-row" role="row"><span class="scope-name">API gateway</span><span>Kong · Lua / Nginx · Apache-2.0</span><span>—</span><span>/</span><span>megabase-server</span><span>L1</span><span>—</span><span class="tag">DAY 0</span></div></div>"#,
    );
    format!(
        r#"<main id="main" class="doc-page">
  <header class="page-hero">
    <p class="kicker">SUPABASE COMPONENTS · 8 IN SCOPE · {total}</p>
    <h1 class="display-sm">Every service Supabase wrote. One crate each.</h1>
    <p class="lede">Supabase self-hosted runs about 12 containers in 6 languages. Megabase reimplements the Supabase-authored services as one Rust binary. PostgreSQL stays the database you already run.</p>
  </header>
  <section class="band table-wrap">{head}</section>
  <section class="triple band">
    <article class="panel"><h3>Gateway</h3><p>Kong is replaced by megabase-server. Same URL layout: /rest/v1, /auth/v1, /storage/v1, /realtime/v1, /functions/v1, /pg.</p></article>
    <article class="panel"><h3>Licenses</h3><p>Each upstream license is preserved in NOTICE and LICENSES/. Megabase itself is Apache-2.0.</p></article>
    <article class="panel"><h3>Studio</h3><p>Official Studio is the Level 4 judge. A Rust rewrite is Level 5, deferred.</p></article>
  </section>
  <p class="band"><a class="text-link" href="{status}">Live map →</a></p>
</main>"#,
        status = paths.page("status"),
    )
}

fn catalog_row(metrics: &Metrics, row: &CatalogRow) -> String {
    let pin = metrics.pin(row.vendor_key);
    let progress = metrics.component_progress(row.id);
    let tag = metrics::status_tag(metrics.component(row.id));
    format!(
        r#"<div class="data-row" role="row"><span class="scope-name">{name}</span><span>{upstream}</span><span>{pin}</span><span>{path}</span><span>{crate_name}</span><span>{levels}</span><span>{progress}</span><span class="tag">{tag}</span></div>"#,
        name = esc(row.name),
        upstream = esc(row.upstream),
        pin = esc(&pin),
        path = esc(row.path),
        crate_name = esc(row.crate_name),
        levels = esc(row.levels),
        progress = esc(&progress),
    )
}

pub fn faq(paths: &Paths, metrics: &Metrics) -> String {
    let units = unit_count_phrase(metrics);
    let humans = metrics.human_interventions_label();
    let items = [
        (
            "Can I use Megabase today?",
            "No. It is Day 0: every endpoint answers 501 MEGABASE_NOT_IMPLEMENTED. Level 1 (REST and email/password auth) is the first point where real apps could run. Follow the Status page.".to_string(),
        ),
        (
            "Is this made by Supabase?",
            "No. Megabase is an independent experiment. It is not affiliated with or endorsed by Supabase, Inc. It ports their open-source code under its licenses and credits every source.".into(),
        ),
        (
            "Do humans write any of the code?",
            format!("No. Humans wrote the instructions: the mission (MANIFESTO.md) and the operating manual (GOAL.md). Agents write all implementation code. Every other human action is logged in the Human log. Interventions so far: {humans}."),
        ),
        (
            "How do you know it really matches Supabase?",
            "An external judge runs official Supabase from the same pinned versions next to Megabase and compares status codes, headers and bodies. Agents cannot change the judge. At Level 4 the unmodified Supabase Studio is replayed against both.".into(),
        ),
        ("What is a unit?", units),
        (
            "Coverage or conformance: what's the difference?",
            "Coverage is the share of units implemented. Conformance is the share of differential tests where Megabase answers exactly like Supabase. A unit is done only when it is both.".into(),
        ),
        (
            "Why Rust? Why one binary?",
            "Self-hosting Supabase means about 12 containers in 6 languages. One Rust binary next to a standard PostgreSQL is simpler to run. The target is under 256 MB of RAM; that is a goal, not a measurement yet.".into(),
        ),
        (
            "Can I contribute?",
            "Code comes from the agents only. You can help by reporting a real Supabase app to verify (an issue labeled verified-app, from Level 1), by flagging judge mistakes, or by following and sharing.".into(),
        ),
    ];
    let mut body = String::new();
    for (q, a) in items {
        body.push_str(&format!(
            r#"<div class="faq-item"><h2>{q}</h2><p>{a}</p></div>"#,
            q = esc(q),
            a = esc(&a),
        ));
    }
    let _ = paths;
    format!(
        r#"<main id="main" class="doc-page">
  <header class="page-hero">
    <p class="kicker">FAQ</p>
    <h1 class="display-sm">Questions people ask.</h1>
  </header>
  <section class="faq-page">{body}</section>
</main>"#
    )
}

pub fn human_log(paths: &Paths, metrics: &Metrics, log_html: &str) -> String {
    let _ = paths;
    format!(
        r#"<main id="main" class="doc-page">
  <header class="page-hero split-hero">
    <div>
      <p class="kicker">HUMAN LOG</p>
      <h1 class="display-sm">Every time a human touched the experiment.</h1>
      <p class="lede">An intervention is any action a human takes on the repository or the running agents: a fix, a nudge, a reverted commit, a changed prompt. The count is part of the result.</p>
    </div>
    <div class="metric">
      <p class="muted">INTERVENTIONS</p>
      <p class="stat-xl">{n}</p>
    </div>
  </header>
  <section class="band">{log_html}</section>
  <section class="triple band">
    <article class="panel"><h3>What counts</h3><p>A merge, a prompt change, a reverted commit, turning Pages on, anything a human did that the agents did not.</p></article>
    <article class="panel"><h3>What does not</h3><p>The manifesto, GOAL.md, and the initial environment. Those are the mission, written before the experiment started.</p></article>
    <article class="panel"><h3>Pending</h3><p>Items still waiting for a human do not increment the public count.</p></article>
  </section>
</main>"#,
        n = esc(&metrics.human_interventions_label()),
    )
}

pub fn not_found(paths: &Paths) -> String {
    format!(
        r#"<main id="main" class="not-found">
  <p class="kicker">HTTP 404</p>
  <h1 class="display">Page not found.</h1>
  <pre class="code-panel" tabindex="0"><code>{{
  "status": 404,
  "code": "MEGABASE_PAGE_NOT_FOUND",
  "message": "Nothing lives at this URL."
}}</code></pre>
  <div class="actions">
    <a class="btn btn-primary" href="{home}">GO HOME</a>
    <a class="btn btn-ghost" href="{components}">BROWSE COMPONENTS</a>
  </div>
</main>"#,
        home = paths.home_href(),
        components = paths.page("components"),
    )
}

pub fn devlog_index(paths: &Paths, entries: &[devlog::Entry]) -> String {
    let list = devlog::index_html(entries, &paths.root);
    format!(
        r#"<main id="main" class="doc-page">
  <header class="page-hero">
    <p class="kicker">DEVLOG · ONE SHORT ENTRY PER DAY</p>
    <h1 class="display-sm">What the agents did today, written for humans.</h1>
    <p class="lede">Each entry is a file in devlog/YYYY-MM-DD.md, written by the agents, published as-is. Until the first file lands this page stays empty rather than inventing a post.</p>
  </header>
  <section class="band">{list}</section>
</main>"#
    )
}

pub fn devlog_article(paths: &Paths, entry: &devlog::Entry) -> String {
    let day = entry.day.map(|n| format!(" · DAY {n}")).unwrap_or_default();
    format!(
        r#"<main id="main" class="doc-page">
  <article class="article-prose devlog-article">
    <p><a class="text-link" href="{index}">← All entries</a></p>
    <p class="kicker">{date}{day} · WRITTEN BY THE AGENTS</p>
    <h1>{title}</h1>
    {body}
  </article>
</main>"#,
        index = paths.page("devlog"),
        date = esc(&entry.date),
        title = esc(&entry.title),
        body = entry.body_html,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metrics::ComponentBlock;

    fn live_progress() -> Metrics {
        let mut metrics = Metrics::placeholder();
        metrics.total = Some(5);
        metrics.passing = Some(2);
        metrics.conformance = Some(40.0);
        metrics.components = vec![
            ComponentBlock::from_counts("rest", "REST", 0, 0, 0, 2),
            ComponentBlock::from_counts("auth", "Auth", 1, 0, 0, 0),
            ComponentBlock::from_counts("storage", "Storage", 1, 0, 0, 0),
            ComponentBlock::from_counts("realtime", "Realtime", 1, 0, 0, 0),
        ];
        metrics
    }

    #[test]
    fn hero_claim_follows_live_passing_count() {
        let live = live_progress();
        assert!(hero_kicker_desktop(&live).contains("2 UNITS PASS"));
        assert!(!hero_kicker_desktop(&live).contains("NOTHING PASSES YET"));
        assert!(hero_kicker_desktop(&Metrics::placeholder()).contains("NOTHING PASSES YET"));
        assert_eq!(
            hero_kicker_mobile(&Metrics::placeholder()),
            "DAY 0 · PHASE 0"
        );
    }

    #[test]
    fn roadmap_progress_is_per_level_scope() {
        let live = live_progress();
        assert_eq!(
            level_progress_label(&live, &LEVELS[0]),
            "66.7% · in progress"
        );
        assert_eq!(level_progress_label(&live, &LEVELS[1]), "0% · not started");
        assert_eq!(level_progress_label(&live, &LEVELS[2]), "0% · not started");
        assert_eq!(level_progress_label(&live, &LEVELS[4]), "deferred");
        assert_ne!(
            level_progress_label(&live, &LEVELS[0]),
            format!("{} · not started", live.conformance_label())
        );
        let html = level_cards(&live, false);
        assert!(html.contains("LEVEL 1 · NEXT"));
        assert!(!html.contains("40.0% · not started"));
        assert!(!html.contains("40% · not started"));
        assert!(
            html.contains(r#"class="level-foot""#),
            "progress bar and status must share a footer so they pin together"
        );
        assert!(html.contains(r#"class="bar""#));
        let foot_at = html.find("level-foot").expect("footer");
        let bar_at = html.find(r#"class="bar""#).expect("bar");
        assert!(
            bar_at > foot_at,
            "bar must live inside level-foot, not as a loose sibling of the description"
        );
        let detailed = level_cards(&live, true);
        assert!(
            !detailed.contains("level-foot"),
            "status-page cards have no progress footer"
        );
    }

    #[test]
    fn level_card_css_pins_progress_to_the_bottom() {
        let css = include_str!("../static/styles.css");
        assert!(
            css.contains(".level-card {\n  display: flex;\n  flex-direction: column;"),
            "level cards are a flex column so leftover height can sit under the copy"
        );
        let foot = css.split(".level-foot {").nth(1).expect(".level-foot rule");
        let foot_block = foot.split('}').next().expect("level-foot body");
        assert!(
            foot_block.contains("margin-top: auto"),
            "level-foot must pin to the bottom of the card: {foot_block}"
        );
        assert!(
            css.contains("min-height: 260px"),
            "desktop cards share a floor height so the five-up grid lines up"
        );
        let mobile = css
            .split("@media (max-width: 900px)")
            .nth(1)
            .expect("stacked breakpoint");
        assert!(
            mobile.contains(".level-card {\n    min-height: 0;\n    height: auto;\n  }"),
            "stacked .level-card must drop the desktop min-height and keep natural flow"
        );
    }

    #[test]
    fn status_and_faq_omit_spend_rows() {
        let metrics = Metrics::placeholder();
        let paths = Paths::nested("status", false);
        let status = status(&paths, &metrics);
        assert!(status.contains("Human interventions"));
        assert!(!status.to_ascii_lowercase().contains("spend"));
        assert!(!status.to_ascii_lowercase().contains("dollar"));
        let faq = faq(&Paths::nested("faq", false), &metrics);
        assert!(!faq.to_ascii_lowercase().contains("cost"));
        assert!(!faq.to_ascii_lowercase().contains("tokens and money"));
        let home = home(&Paths::home(false), &metrics);
        assert!(home.contains("the human log."));
        assert!(!home.to_ascii_lowercase().contains("spend"));
        assert!(!home.contains("<picture"));
        assert!(!home.contains("coverage/treemap.svg"));
        assert!(!status.contains("coverage/treemap.svg"));
        assert!(!status.contains("coverage/treemap-light.svg"));
    }
}
