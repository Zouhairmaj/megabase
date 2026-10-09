# Changelog

All notable changes to Megabase are documented in this file.
The format is produced by [release-please](https://github.com/googleapis/release-please).
Sections: Features, Bug Fixes, Performance, Conformance/judge, Documentation.

## [0.1.1](https://github.com/Zouhairmaj/megabase/compare/v0.1.0...v0.1.1) (2026-10-09)


### Features

* **auth:** install mfa oauth one-time and refresh tables ([e708e9c](https://github.com/Zouhairmaj/megabase/commit/e708e9cfa4feb51859fef5a261eb31e6157001da))
* jwt validation and config ([590b369](https://github.com/Zouhairmaj/megabase/commit/590b3695468341694282007d92f0a62d4eed5104))


### Bug Fixes

* **ci:** board sync status update ([#135](https://github.com/Zouhairmaj/megabase/issues/135)) ([c5e6aa8](https://github.com/Zouhairmaj/megabase/commit/c5e6aa88e0e3d0f0cefdc42ad7dd0087e978a195))
* **site:** dark square treemaps sized for 1,024 units ([098b2d9](https://github.com/Zouhairmaj/megabase/commit/098b2d9f4ac16384a910bcd548b267543e871833))
* **site:** remove cost mentions ([#130](https://github.com/Zouhairmaj/megabase/issues/130)) ([37f4128](https://github.com/Zouhairmaj/megabase/commit/37f412811bd052a1afbaed09b501b4f5ee684c23))


### Conformance/judge

* 0% (0/30, no regression) ([8281d6b](https://github.com/Zouhairmaj/megabase/commit/8281d6b6a63e710c37313bf97216db681c566386))
* 0% → 0% (0/30, no regression) ([e708e9c](https://github.com/Zouhairmaj/megabase/commit/e708e9cfa4feb51859fef5a261eb31e6157001da))
* not measured (Judge 0/30, no regression) ([098b2d9](https://github.com/Zouhairmaj/megabase/commit/098b2d9f4ac16384a910bcd548b267543e871833))
* not measured (Judge 0/30, no regression) ([590b369](https://github.com/Zouhairmaj/megabase/commit/590b3695468341694282007d92f0a62d4eed5104))

## 0.1.0 (2026-10-09)


### Features

* **auth:** install mfa oauth one-time and refresh tables ([e708e9c](https://github.com/Zouhairmaj/megabase/commit/e708e9cfa4feb51859fef5a261eb31e6157001da))
* bootstrap 501 gateway, coverage extractor, and judge ([d0f0cd9](https://github.com/Zouhairmaj/megabase/commit/d0f0cd96aafabbc217fa4a8317b2b12d36508fd4))
* bootstrap phase 0 workspace, judge, coverage, and ci ([7aa41e8](https://github.com/Zouhairmaj/megabase/commit/7aa41e84ecdc9bc5399ef0a41e1f3aeba46ac441))
* **coverage:** nested squarified Status treemap ([e851f3d](https://github.com/Zouhairmaj/megabase/commit/e851f3dcf21c072332f0c43748b9603d52108499))
* jwt validation and config ([590b369](https://github.com/Zouhairmaj/megabase/commit/590b3695468341694282007d92f0a62d4eed5104))
* **site:** full website from Kite designs ([#126](https://github.com/Zouhairmaj/megabase/issues/126)) ([b381348](https://github.com/Zouhairmaj/megabase/commit/b381348a4ced53f812c709c2b7125fb9a7acbe6c))
* **site:** placeholder home and manifesto pages ([ec21783](https://github.com/Zouhairmaj/megabase/commit/ec217835e2e81c2a1dda9e392bc7c4b43c69cd4a))
* **site:** SEO, per-page Open Graph cards and structured data ([381a7b5](https://github.com/Zouhairmaj/megabase/commit/381a7b58cbf040aa914cb38e93de9f62e805e253))


### Bug Fixes

* address CodeRabbit findings on workflows and parse_megabase_id ([72b5c0f](https://github.com/Zouhairmaj/megabase/commit/72b5c0fe76b58c8238052cf0c1e2b57ccc283896))
* **ci:** board sync status update ([#135](https://github.com/Zouhairmaj/megabase/issues/135)) ([c5e6aa8](https://github.com/Zouhairmaj/megabase/commit/c5e6aa88e0e3d0f0cefdc42ad7dd0087e978a195))
* classify nested Auth admin/user routes as Level 2 ([3735641](https://github.com/Zouhairmaj/megabase/commit/3735641edf811e3f88b4142c42a25fa99baf9716))
* **coverage:** stop regenerating Kite README banners ([6533ae3](https://github.com/Zouhairmaj/megabase/commit/6533ae3fef83880be600711a918652b0176c1208))
* exclude site from the workspace and regenerate the backlog plan ([d08b611](https://github.com/Zouhairmaj/megabase/commit/d08b6116bcf8fe2050ff8be75f89a6471bc4ea93))
* **judge:** wait on the admin-only PostgREST root with the service_role key ([6615b5b](https://github.com/Zouhairmaj/megabase/commit/6615b5b21d7c62b2ef98b6e306b685042be0e4e7))
* make megabase-backlog an idempotent GraphQL sync ([a835bfd](https://github.com/Zouhairmaj/megabase/commit/a835bfd5ef5e1e514fefa81e5eb557597812c530))
* **site:** dark square treemaps sized for 1,024 units ([098b2d9](https://github.com/Zouhairmaj/megabase/commit/098b2d9f4ac16384a910bcd548b267543e871833))
* **site:** remove cost mentions ([#130](https://github.com/Zouhairmaj/megabase/issues/130)) ([37f4128](https://github.com/Zouhairmaj/megabase/commit/37f412811bd052a1afbaed09b501b4f5ee684c23))
* skip reclosing closed issues and document judge abort ([e287f08](https://github.com/Zouhairmaj/megabase/commit/e287f081b544726787b519f66efbbe71d7bd29bb))


### Conformance/judge

* 0% → 0% (0/30, no regression) ([e708e9c](https://github.com/Zouhairmaj/megabase/commit/e708e9cfa4feb51859fef5a261eb31e6157001da))
* not measured (Judge 0/30, no regression) ([098b2d9](https://github.com/Zouhairmaj/megabase/commit/098b2d9f4ac16384a910bcd548b267543e871833))
* not measured (Judge 0/30, no regression) ([590b369](https://github.com/Zouhairmaj/megabase/commit/590b3695468341694282007d92f0a62d4eed5104))


### Documentation

* add a single Built with table to the README ([5f7a0cd](https://github.com/Zouhairmaj/megabase/commit/5f7a0cda4288e3ca25d0a2e298d6b647189ddfe6))
* add AGENTS.md and nested agent guides ([f024132](https://github.com/Zouhairmaj/megabase/commit/f024132012e3c248bbf22a98f0d905ad961c8bbb))
* add README and brand assets ([c38f0d8](https://github.com/Zouhairmaj/megabase/commit/c38f0d84cf1f703fe060cd15ef785f745af8be34))
* agents design in Kite, reviewed by an LLM committee ([28eaa83](https://github.com/Zouhairmaj/megabase/commit/28eaa839ac19d442586608885f71b7bb8d2fb47a))
* **brand:** center README banner content vertically ([#125](https://github.com/Zouhairmaj/megabase/issues/125)) ([49c773f](https://github.com/Zouhairmaj/megabase/commit/49c773f5d990b81927c318d74d45805e3518b5d5))
* credit vendored LICENSE files in NOTICE ([c28421f](https://github.com/Zouhairmaj/megabase/commit/c28421fe0ab6ecfbbbd8ddd88986c39d7021fde9))
* **goal:** add design-first rule (Kite) ([5decfde](https://github.com/Zouhairmaj/megabase/commit/5decfde10fece0cf305d247ce861d9b8d0d1e14c))
* **goal:** add documentation-as-deliverable rule ([a576d0d](https://github.com/Zouhairmaj/megabase/commit/a576d0dc767c58ba4cd5b38396bbafe3d73553c4))
* **goal:** Kite holds templates, docs/ holds content ([e7839fc](https://github.com/Zouhairmaj/megabase/commit/e7839fcc8321cb7cbda57fe4045b6b18a37d0844))
* merge committee-reviewed AGENTS.md ([8b4100d](https://github.com/Zouhairmaj/megabase/commit/8b4100dde86860e7cdf73257d09d8573ea32e7a5))
* point GitHub links at Zouhairmaj/megabase ([6fcc9d3](https://github.com/Zouhairmaj/megabase/commit/6fcc9d35d0a47321b0280ceca28549b0208cbfb7))
* remove duplicate logo below README banner ([#124](https://github.com/Zouhairmaj/megabase/issues/124)) ([3783039](https://github.com/Zouhairmaj/megabase/commit/3783039aa6a85a698810e8623e2f3cf1b17c1111))
