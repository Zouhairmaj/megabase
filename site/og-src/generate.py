#!/usr/bin/env python3
"""Regenerate the per-page Open Graph cards (1200x630 PNG) in site/static/og/.

Source of truth for the design: Kite file `megabase-identity`, page
"Website / OG images" (frames "OG / Home", "OG / Manifesto", "OG / Page template").
To add a page: add an entry to CARDS (eyebrow = page name, two-line title,
one-line summary), run this script, and point the page's `og_image` in
site/src/main.rs at `og/<key>.png`.

Usage (needs Google Chrome or Chromium on PATH):
    python3 site/og-src/generate.py            # all cards
    python3 site/og-src/generate.py manifesto  # one card
"""
import os, shutil, subprocess, sys, tempfile
HERE = os.path.dirname(os.path.abspath(__file__))
SITE = os.path.dirname(HERE)
BG="#0B0E12"; ACC="#00D892"; MUTED="#303235"; TXT="#F7F7F7"; SUB="#BABABB"; DIM="#8E9094"
F="font-family:'JetBrains Mono', monospace;"
def grid(sq, gap, name):
    cells=""
    for i in range(9):
        c = ACC if i==4 else MUTED
        cells+=f'<div style="width:{sq}px;height:{sq}px;background:{c};flex-shrink:0"></div>'
    w=sq*3+gap*2
    return f'<div data-name="{name}" style="width:{w}px;height:{w}px;display:flex;flex-wrap:wrap;gap:{gap}px;flex-shrink:0">{cells}</div>'
CARDS={
 "home":dict(name="OG / Home",eyebrow="DAY 0 · BUILT BY AGENTS, IN PUBLIC",l1="The Supabase API.",l2="One Rust binary.",sub="AI agents rewrite every Supabase service in Rust, judged response by response against the real stack."),
 "manifesto":dict(name="OG / Manifesto",eyebrow="MANIFESTO",l1="Supabase, in Rust.",l2="By agents. In public.",sub="What this is, the rules of the experiment, and how progress is measured."),
}
def card(k):
    c=CARDS[k]
    return f'''<div data-name="{c['name']}" style="width:1200px;height:630px;background:{BG};display:flex;flex-direction:column;justify-content:space-between;padding:64px 72px 56px 72px;box-sizing:border-box;{F}">
<div style="display:flex;justify-content:space-between;align-items:center">
<div style="display:flex;align-items:center;gap:18px">{grid(10,3,"Logo")}<span style="color:{TXT};font-size:28px;font-weight:700;letter-spacing:5px;{F}">MEGABASE</span></div>
<span style="color:{DIM};font-size:22px;{F}">megabase.sh</span></div>
<div style="display:flex;justify-content:space-between;align-items:center;gap:48px">
<div style="display:flex;flex-direction:column;gap:22px;width:780px">
<span style="color:{ACC};font-size:20px;letter-spacing:4px;{F}">{c['eyebrow']}</span>
<div style="display:flex;flex-direction:column;gap:4px"><span style="color:{TXT};font-size:56px;font-weight:700;line-height:1.15;{F}">{c['l1']}</span><span style="color:{ACC};font-size:56px;font-weight:700;line-height:1.15;{F}">{c['l2']}</span></div>
<span style="color:{SUB};font-size:23px;line-height:1.45;{F}">{c['sub']}</span></div>
{grid(64,20,"Mark")}</div>
<div style="display:flex;justify-content:space-between;align-items:center;border-top:1px solid {MUTED};padding-top:22px">
<span style="color:{SUB};font-size:18px;{F}">Unofficial experiment. Not affiliated with Supabase, Inc.</span>
<span style="color:{DIM};font-size:18px;{F}">Apache-2.0 · open source</span></div></div>'''
def page_html(k):
    fonts = os.path.join(SITE, "static", "fonts")
    return f'''<!doctype html><html><head><meta charset="utf-8"><style>
@font-face{{font-family:'JetBrains Mono';src:url(file://{fonts}/JetBrainsMono-Regular.woff2);font-weight:400}}
@font-face{{font-family:'JetBrains Mono';src:url(file://{fonts}/JetBrainsMono-Bold.woff2);font-weight:700}}
html,body{{margin:0;padding:0;background:{BG}}}</style></head><body>{card(k)}</body></html>'''

def chrome():
    for name in ("google-chrome", "chromium", "chromium-browser", "chrome"):
        path = shutil.which(name)
        if path:
            return path
    sys.exit("Chrome/Chromium not found on PATH")

if __name__ == "__main__":
    keys = sys.argv[1:] or list(CARDS)
    out_dir = os.path.join(SITE, "static", "og")
    os.makedirs(out_dir, exist_ok=True)
    with tempfile.TemporaryDirectory() as tmp:
        for k in keys:
            html = os.path.join(tmp, k + ".html")
            open(html, "w").write(page_html(k))
            out = os.path.join(out_dir, k + ".png")
            subprocess.run([chrome(), "--headless=new", "--no-sandbox", "--hide-scrollbars",
                "--force-device-scale-factor=1", "--window-size=1200,630",
                "--virtual-time-budget=3000", "--screenshot=" + out, "file://" + html],
                check=True, stderr=subprocess.DEVNULL)
            print("wrote", out)
    # The default card (used by 404 and any page without its own) is the home card.
    shutil.copyfile(os.path.join(out_dir, "home.png"), os.path.join(SITE, "static", "og-card.png"))
