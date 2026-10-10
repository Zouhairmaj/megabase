# Changelog

All notable changes to Megabase are documented in this file.
The format is produced by [release-please](https://github.com/googleapis/release-please).
Sections: Features, Bug Fixes, Performance, Conformance/judge, Documentation.

## [0.1.5](https://github.com/Zouhairmaj/megabase/compare/v0.1.4...v0.1.5) (2026-10-10)


### Features

* **auth:** serve user routes ([#183](https://github.com/Zouhairmaj/megabase/issues/183)) ([11e49e9](https://github.com/Zouhairmaj/megabase/commit/11e49e95594445dd54c13ede27eb654ed53623e0))
* **auth:** verify signup, invite, recovery, and email change ([#193](https://github.com/Zouhairmaj/megabase/issues/193)) ([543d587](https://github.com/Zouhairmaj/megabase/commit/543d587c9aa9edc6bdc0c1e72bd27ec285cf01b9))
* **judge:** add a hidden held-out judge suite ([#205](https://github.com/Zouhairmaj/megabase/issues/205)) ([846b842](https://github.com/Zouhairmaj/megabase/commit/846b842b8a8a4ec30bdfa0b44e5b47261e0315f9))
* **judge:** add adversarial auth and rls cases ([#208](https://github.com/Zouhairmaj/megabase/issues/208)) ([06cd945](https://github.com/Zouhairmaj/megabase/commit/06cd94504431e51d2d52e89706630b5968378f79))
* **rest:** add imatch, in, is, like, lt, and not filters ([#207](https://github.com/Zouhairmaj/megabase/issues/207)) ([d7ae903](https://github.com/Zouhairmaj/megabase/commit/d7ae903aac3d05578ac54cc86332efc7aadc8129))


### Bug Fixes

* **coverage:** quote live judge conformance instead of the baseline ([#206](https://github.com/Zouhairmaj/megabase/issues/206)) ([2c58ef8](https://github.com/Zouhairmaj/megabase/commit/2c58ef85a978857f0d6cce3327707ee0adbed1fd))


### Documentation

* record owner phase 0 review ([#204](https://github.com/Zouhairmaj/megabase/issues/204)) ([acb4990](https://github.com/Zouhairmaj/megabase/commit/acb49904340cb21b5631be39c2884e652bd1be46))
* remove cost rule from manifesto (owner decision) ([#197](https://github.com/Zouhairmaj/megabase/issues/197)) ([2d9df9a](https://github.com/Zouhairmaj/megabase/commit/2d9df9a6e4febef10518a1424582b0657af3bee4))

## [0.1.4](https://github.com/Zouhairmaj/megabase/compare/v0.1.3...v0.1.4) (2026-10-10)


### Features

* **rest:** add horizontal filter operators ([#192](https://github.com/Zouhairmaj/megabase/issues/192)) ([0e1f7e6](https://github.com/Zouhairmaj/megabase/commit/0e1f7e668b22d4583b61625f3c636fa48e322ecb))

## [0.1.3](https://github.com/Zouhairmaj/megabase/compare/v0.1.2...v0.1.3) (2026-10-10)


### Features

* **auth:** serve admin user, sso, and oauth routes ([#188](https://github.com/Zouhairmaj/megabase/issues/188)) ([e0b735b](https://github.com/Zouhairmaj/megabase/commit/e0b735b9062202b1e408da33e59e4d4a189736c7))

## [0.1.2](https://github.com/Zouhairmaj/megabase/compare/v0.1.1...v0.1.2) (2026-10-10)


### Features

* **auth:** serve password and refresh_token grants ([#178](https://github.com/Zouhairmaj/megabase/issues/178)) ([951df46](https://github.com/Zouhairmaj/megabase/commit/951df46078c97e9713f2f96e7ae9ee3cf7cf3319))
* harden the gateway http stack and auth queries ([#179](https://github.com/Zouhairmaj/megabase/issues/179)) ([9e441ad](https://github.com/Zouhairmaj/megabase/commit/9e441ada58bb32f0c860d6d321790929f6db1b1b))


### Bug Fixes

* **coverage:** publish main judge results to status ([#177](https://github.com/Zouhairmaj/megabase/issues/177)) ([4a40114](https://github.com/Zouhairmaj/megabase/commit/4a4011444776d417c82f69dbeffcbcdc70112390))

## [0.1.1](https://github.com/Zouhairmaj/megabase/compare/v0.1.0...v0.1.1) (2026-10-09)


### Features

* **auth:** install auth sql functions and tables ([9dbe7f6](https://github.com/Zouhairmaj/megabase/commit/9dbe7f6289a932e6391ef2877615f92b82621e81))
* **auth:** install saml sso scim users and sessions tables ([c6aa059](https://github.com/Zouhairmaj/megabase/commit/c6aa0590b3ebf727685cafb640c80ad1aaf457e7))
* **auth:** install webauthn challenges and credentials tables ([#148](https://github.com/Zouhairmaj/megabase/issues/148)) ([10c4857](https://github.com/Zouhairmaj/megabase/commit/10c485777fdd8b710fb03d735af20cfe4e4df126))
* **judge:** database side-effect checks ([#149](https://github.com/Zouhairmaj/megabase/issues/149)) ([9eca1b4](https://github.com/Zouhairmaj/megabase/commit/9eca1b431b306ee1807e31c3015ff62dcd65afd4))


### Bug Fixes

* **ci:** dispatch required checks on lockfile commits ([#161](https://github.com/Zouhairmaj/megabase/issues/161)) ([0d09a48](https://github.com/Zouhairmaj/megabase/commit/0d09a4800ec5f9c06745e2c5896ac11320e98509))
* **coverage:** traffic-light badge colors ([#165](https://github.com/Zouhairmaj/megabase/issues/165)) ([30f82d3](https://github.com/Zouhairmaj/megabase/commit/30f82d32dac9c0f512e9d89b6059100ebb8c7fa7))
* **site:** align roadmap card progress bars ([#145](https://github.com/Zouhairmaj/megabase/issues/145)) ([bd0aa72](https://github.com/Zouhairmaj/megabase/commit/bd0aa729e20c844cb14724e22583764d2a6255ab))
* **site:** patch rustsec-2026-0206 and 0192 ([#156](https://github.com/Zouhairmaj/megabase/issues/156)) ([0165042](https://github.com/Zouhairmaj/megabase/commit/016504259a8cf4c39d46a5e21e79c10eaad50990))
* **site:** show implemented units in treemaps ([#154](https://github.com/Zouhairmaj/megabase/issues/154)) ([5f2e9a2](https://github.com/Zouhairmaj/megabase/commit/5f2e9a29c7ea0da80fa42e9ea4cbc9f0263e5b69))


### Conformance/judge

* 0% (unchanged) ([c6aa059](https://github.com/Zouhairmaj/megabase/commit/c6aa0590b3ebf727685cafb640c80ad1aaf457e7))
* not measured ([30f82d3](https://github.com/Zouhairmaj/megabase/commit/30f82d32dac9c0f512e9d89b6059100ebb8c7fa7))
* not measured (README only) ([02324aa](https://github.com/Zouhairmaj/megabase/commit/02324aaa2a41683185c7a071f2a84f071ab642fc))


### Documentation

* add per-unit template and level 1 stubs ([#146](https://github.com/Zouhairmaj/megabase/issues/146)) ([e0bdddf](https://github.com/Zouhairmaj/megabase/commit/e0bdddfd30ddf894097c77aefb85145345a74e92))
* color-code coverage badges ([#159](https://github.com/Zouhairmaj/megabase/issues/159)) ([f8a8d0c](https://github.com/Zouhairmaj/megabase/commit/f8a8d0ce464530e35b939b5bd39ee1ed8e8a1b65))
* drop internal note from readme status ([#167](https://github.com/Zouhairmaj/megabase/issues/167)) ([70d1f79](https://github.com/Zouhairmaj/megabase/commit/70d1f7995e4d8e8422d2a18cc5543e7136409edb))
* readme badges ([02324aa](https://github.com/Zouhairmaj/megabase/commit/02324aaa2a41683185c7a071f2a84f071ab642fc))

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
