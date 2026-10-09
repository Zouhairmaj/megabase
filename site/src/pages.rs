//! Page bodies matching the Kite Website frames.

use crate::chrome::Paths;
use crate::devlog;
use crate::html::esc;
use crate::markdown;
use crate::metrics::{self, CatalogRow, Metrics, CATALOG};
use crate::treemap;
use crate::GITHUB;

pub fn home(paths: &Paths, metrics: &Metrics) -> String {
    let treemap = treemap::svg_size(metrics, 1248.0, 280.0);
    let status_panel = status_panel(metrics);
    let components = component_cards(paths, metrics);
    let faq = home_faq(paths, metrics);
    format!(
        r#"<main id="main" class="home-main">
<section class="hero">
  <div class="hero-copy">
    <p class="kicker hide-mobile">DAY 0 · NOTHING PASSES YET · BUILT BY AGENTS, IN PUBLIC</p>
    <p class="kicker hide-desktop">DAY 0 · PHASE 0</p>
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
    <li><span class="step-num">05</span><h3>Record in public</h3><p>Coverage, the treemap, the human log, the spend.</p></li>
  </ol>
  <p class="hide-mobile"><a class="text-link" href="{how}">How the loop works →</a></p>
</section>

<section class="band home-live" id="status">
  <p class="kicker hide-mobile">03 · LIVE STATUS</p>
  <h2 class="section-title hide-mobile">Each cell is one unit.<br />Grey until the judge says green.</h2>
  <p class="lede hide-mobile">Nested squarified treemap: component, then feature group, then one whole square per unit. Regenerated at build time from coverage/units.json. Grey is not started. Green is conformant.</p>
  <div class="treemap treemap-wide">{treemap}</div>
  <p class="legend">■ not started · <span class="swatch implemented"></span> implemented · <span class="swatch tested"></span> tested · <span class="swatch conformant"></span> conformant</p>
  <p class="hide-mobile"><a class="text-link" href="{status}">Full status →</a></p>
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
    )
}

fn coverage_map(paths: &Paths, generated: &str, coverage_svg: bool) -> String {
    if !coverage_svg {
        return generated.to_string();
    }
    let dark = format!("{}coverage/treemap.svg", paths.asset());
    let light = format!("{}coverage/treemap-light.svg", paths.asset());
    format!(
        r#"<picture><source srcset="{light}" media="(prefers-color-scheme: light)" /><img src="{dark}" alt="Component coverage treemap. Each cell is one unit." width="1248" height="360" /></picture>"#
    )
}

fn status_panel(metrics: &Metrics) -> String {
    let desktop = treemap::panel(metrics, 442.0, 220.0);
    let mobile = treemap::panel(metrics, 308.0, 290.0);
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

fn level_cards(metrics: &Metrics, detailed: bool) -> String {
    let items = [
        (
            "LEVEL 1 · NEXT",
            "≥ 95%",
            "REST API + email/password auth",
            "Goal: most apps run.",
            true,
            false,
        ),
        (
            "LEVEL 2",
            "≥ 90%",
            "OAuth, magic links, Storage",
            "Files and social login.",
            false,
            false,
        ),
        (
            "LEVEL 3",
            "≥ 85%",
            "Realtime",
            "Database changes, broadcast, presence.",
            false,
            false,
        ),
        (
            "LEVEL 4",
            "≥ 80%",
            "Functions, pooler, Meta + the Studio test",
            "Official Studio can’t tell the difference.",
            false,
            false,
        ),
        (
            "LEVEL 5",
            "stretch",
            "Studio, served from the binary",
            "Deferred pending a feasibility study.",
            false,
            true,
        ),
    ];
    let mut cards = String::new();
    for (kicker, gate, title, body, next, stretch) in items {
        let class = if stretch {
            "level-card is-stretch"
        } else if next {
            "level-card is-next"
        } else {
            "level-card"
        };
        let progress = if stretch {
            "deferred".into()
        } else if metrics.has_data() {
            format!("{} · not started", metrics.conformance_label())
        } else {
            "— · not started".into()
        };
        let extra = if detailed {
            String::new()
        } else {
            format!(r#"<div class="bar" aria-hidden="true"></div><p class="muted">{progress}</p>"#)
        };
        cards.push_str(&format!(
            r#"<article class="{class}"><div class="level-head"><span class="card-kicker {kicker_class}">{kicker}</span><span class="muted">{gate}</span></div><h3>{title}</h3><p>{body}</p>{extra}</article>"#,
            kicker_class = if next { "accent" } else { "" },
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

pub fn status(paths: &Paths, metrics: &Metrics, coverage_svg: bool) -> String {
    let generated = treemap::svg_size(metrics, 1248.0, 360.0);
    let treemap = coverage_map(paths, &generated, coverage_svg);
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
    <div class="metric-grid metric-grid-5">
      <div class="metric"><p class="muted">Units done</p><p class="stat-xl">{passing}</p></div>
      <div class="metric"><p class="muted">Coverage</p><p class="stat-xl">{coverage}</p></div>
      <div class="metric"><p class="muted">Conformance</p><p class="stat-xl">{conformance}</p></div>
      <div class="metric"><p class="muted">Human interventions</p><p class="stat-xl">{humans}</p></div>
      <div class="metric"><p class="muted">Spend</p><p class="stat-xl">{spend}</p></div>
    </div>
    <p class="note">Spend is not tracked yet. Token and dollar tracking starts after Phase 0.</p>
  </header>
  <section class="band">
    <h2>Component map</h2>
    <p class="note">Each block is a Supabase component. When coverage/units.json is present, cells are one unit each. Until then the tiles are the known crates, unfilled — never a made-up count.</p>
    <div class="treemap treemap-wide treemap-status">{treemap}</div>
    <p class="legend">■ not started · <span class="swatch implemented"></span> implemented · <span class="swatch tested"></span> tested · <span class="swatch conformant"></span> conformant</p>
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
        spend = esc(&metrics.spend_label),
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
            "How much does it cost?",
            "Tokens and money will be published continuously. Tracking starts after the human review of Phase 0, so there is no number yet.".into(),
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

pub fn docs(paths: &Paths) -> String {
    let clone = format!("git clone {GITHUB}\ncd megabase\ncargo build --release");
    let run = "./target/release/megabase\ncurl http://localhost:8000/health\n# today: HTTP 501 {\"code\":\"MEGABASE_NOT_IMPLEMENTED\"}\n# a 501 means the server is up; nothing is implemented yet";
    let js = "import { createClient } from '@supabase/supabase-js'\nconst supabase = createClient('http://localhost:8000', ANON_KEY)\nconst { data, error } = await supabase.from('todos').select()\n// today: error.code === 'MEGABASE_NOT_IMPLEMENTED'";
    let judge = "docker compose up -d\npython judge/compare.py";
    format!(
        r##"<main id="main" class="docs-layout">
  <nav class="docs-side" aria-label="Docs">
    <p class="toc-label">DOCS</p>
    <a class="is-active" href="./" aria-current="page">Getting started</a>
    <a href="{how}">How it works</a>
    <a href="{status}">Status</a>
    <a href="{roadmap}">Roadmap</a>
    <a href="{components}">Components</a>
    <a href="{faq}">FAQ</a>
    <a href="{GITHUB}" rel="noopener noreferrer">README ↗</a>
  </nav>
  <article class="docs-body article-prose">
    <aside class="callout"><strong>Day 0: nothing works yet</strong><p>The binary builds and starts, and every endpoint answers 501 MEGABASE_NOT_IMPLEMENTED. These docs exist so you can follow along, not to run production.</p></aside>
    <h1>Getting started</h1>
    <h2 id="build-from-source">Build from source</h2>
    <p>You need Rust 1.75 or newer and PostgreSQL 15 or newer. Running the judge also needs Docker Compose and Python 3.</p>
    {clone_block}
    <h2 id="configuration">Configuration</h2>
    <div class="data-table">
      <div class="data-row data-head"><span>VARIABLE</span><span>DESCRIPTION</span></div>
      <div class="data-row"><span>DATABASE_URL</span><span>PostgreSQL connection string</span></div>
      <div class="data-row"><span>JWT_SECRET</span><span>Secret used to sign and verify JWTs</span></div>
      <div class="data-row"><span>MEGABASE_PORT</span><span>HTTP port, default 8000</span></div>
    </div>
    <h2 id="check-it-runs">Check it runs</h2>
    {run_block}
    <h2 id="use-with-supabase-js">Use with supabase-js</h2>
    <p>Point supabase-js at your local base URL. ANON_KEY is a JWT signed with your JWT_SECRET. No API behaviour is implemented yet: until Level 1 (see Roadmap) every call returns 501.</p>
    {js_block}
    <h2 id="run-the-judge">Run the judge</h2>
    <p>The judge starts the official Supabase reference stack with Docker Compose and compares it with Megabase. Run it from the repository root.</p>
    {judge_block}
    <p class="docs-next"><a href="{GITHUB}" rel="noopener noreferrer">Source: README.md ↗</a><a class="text-link" href="{status}">Next: Status →</a></p>
  </article>
  <nav class="docs-toc" aria-label="On this page">
    <p class="toc-label">ON THIS PAGE</p>
    <a href="#build-from-source">Build from source</a>
    <a href="#configuration">Configuration</a>
    <a href="#check-it-runs">Check it runs</a>
    <a href="#use-with-supabase-js">supabase-js</a>
    <a href="#run-the-judge">Run the judge</a>
  </nav>
</main>"##,
        how = paths.page("how-it-works"),
        status = paths.page("status"),
        roadmap = paths.page("roadmap"),
        components = paths.page("components"),
        faq = paths.page("faq"),
        clone_block = code_block("SHELL", &clone),
        run_block = code_block("SHELL", run),
        js_block = code_block("JAVASCRIPT", js),
        judge_block = code_block("SHELL", judge),
    )
}

fn code_block(lang: &str, code: &str) -> String {
    format!(
        r#"<div class="code-block"><div class="code-head"><span>{}</span><button type="button" class="copy-btn" data-copy>⧉ COPY</button></div><pre><code>{}</code></pre></div>"#,
        esc(lang),
        esc(code)
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

pub fn cost(paths: &Paths, metrics: &Metrics) -> String {
    let _ = paths;
    format!(
        r#"<main id="main" class="doc-page">
  <header class="page-hero">
    <p class="kicker">COST · TOKENS AND MONEY</p>
    <h1 class="display-sm">Published continuously. Not started.</h1>
    <p class="lede">The manifesto requires tokens and money spent to be public in real time. Tracking starts after the human review of Phase 0, so this page has no number yet.</p>
  </header>
  <section class="metric-grid band">
    <div class="metric"><p class="muted">Spend</p><p class="stat-xl">{spend}</p></div>
    <div class="metric"><p class="muted">Stage</p><p class="stat-xl">{stage}</p></div>
  </section>
  <aside class="callout">Until the ledger exists, this page refuses to invent a dollar figure.</aside>
</main>"#,
        spend = esc(&metrics.spend_label),
        stage = esc(&metrics.stage),
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
        body = &entry.body_html,
    )
}
