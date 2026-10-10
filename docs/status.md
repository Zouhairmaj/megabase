---
title: API status per service
description: Units and conformance for REST, Auth, Storage, Realtime, Functions, Pooler, Meta and Studio.
section: use
order: 2
card: Units and conformance for REST, Auth, Storage, Realtime, Functions, Pooler, Meta and Studio. Counts come from coverage/summary.json.
tag: status
---

# API status per service

Numbers on this page come from `coverage/summary.json` (and `coverage/units.json`) at build time. If those files are absent the cells are an em dash, never a made-up total.

The table below is generated, not typed. Open [Status](/status/) for the treemap and the same counts.

## How to read a row

- **Units** is the denominator for that component, from the units file.
- **Conformant** is how many of those units currently match the judge.
- **Status** is Day 0 until a component has coverage data.

The denominator is `coverage/units.json`. Implemented units are those with a
`// megabase:unit` marker. Auth SQL objects, the first admin GET/DELETE
routes, health, settings, autoconfirm email signup, logout, and the
password and refresh-token grants, the REST resource routes, and the RPC
routes carry markers. Embeds, `Prefer`, and other HTTP routes are still 501.

> [!NOTE]
> Units passing and coverage are on [megabase.sh/status](https://megabase.sh/status/). The standalone conformance percentage is not shown on that page for now; the README conformance badge is the live Judge publication ([decision 0032](decisions/0032-hide-standalone-conformance-percent.md)). The committed `coverage/judge-results.json` is the regression baseline, not that score ([decision 0030](decisions/0030-conformance-is-the-live-judge-score.md)).
