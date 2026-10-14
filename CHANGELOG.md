# Changelog

All notable changes to Megabase are documented in this file.
The format is produced by [release-please](https://github.com/googleapis/release-please).
Sections: Features, Bug Fixes, Performance, Conformance/judge, Documentation.

## [0.1.27](https://github.com/Zouhairmaj/megabase/compare/v0.1.26...v0.1.27) (2026-10-14)


### Features

* **auth:** add custom access token hook for pg-functions ([#345](https://github.com/Zouhairmaj/megabase/issues/345)) ([d35e1d3](https://github.com/Zouhairmaj/megabase/commit/d35e1d359f0284896bb41822e44fd82da3a00434))

## [0.1.26](https://github.com/Zouhairmaj/megabase/compare/v0.1.25...v0.1.26) (2026-10-14)


### Bug Fixes

* **auth:** match go bad_json messages and add token rate limit ([#332](https://github.com/Zouhairmaj/megabase/issues/332)) ([97fb097](https://github.com/Zouhairmaj/megabase/commit/97fb0979a658c7c1abefe3e6fc6b8d6e1f298f0e))

## [0.1.25](https://github.com/Zouhairmaj/megabase/compare/v0.1.24...v0.1.25) (2026-10-14)


### Features

* **rest:** serve application/geo+json and application/octet-stream media types ([#328](https://github.com/Zouhairmaj/megabase/issues/328)) ([e9ffa29](https://github.com/Zouhairmaj/megabase/commit/e9ffa29c7e8b331161cf3873ea8107d84391dd0f))

## [0.1.24](https://github.com/Zouhairmaj/megabase/compare/v0.1.23...v0.1.24) (2026-10-14)


### Features

* create standard api roles at startup ([#324](https://github.com/Zouhairmaj/megabase/issues/324)) ([dbfc90d](https://github.com/Zouhairmaj/megabase/commit/dbfc90d10bc8ba1fab1a308de2dc010dfa6ba1a4))
* **rest:** plan media type is 406 while plans are disabled ([#314](https://github.com/Zouhairmaj/megabase/issues/314)) ([c11e203](https://github.com/Zouhairmaj/megabase/commit/c11e2039e46fecb425ed77290e2a155d06086e09))


### Bug Fixes

* **auth:** answer kong cors preflights at the gateway ([#323](https://github.com/Zouhairmaj/megabase/issues/323)) ([e0677a8](https://github.com/Zouhairmaj/megabase/commit/e0677a86f694f2ab64292c22e957e241e093968f))
* **auth:** read null request fields as absent, like go ([#322](https://github.com/Zouhairmaj/megabase/issues/322)) ([5657b64](https://github.com/Zouhairmaj/megabase/commit/5657b64419faf048d8e2c555fe52b9f3ae8836b9))
* **rest:** complete cors headers on preflight and simple requests ([#325](https://github.com/Zouhairmaj/megabase/issues/325)) ([13d857a](https://github.com/Zouhairmaj/megabase/commit/13d857a52984516af8503919685224ef5abc4dd3))
* **rest:** serve vnd.pgrst.object+json on writes and rpc ([#326](https://github.com/Zouhairmaj/megabase/issues/326)) ([b1dc2e9](https://github.com/Zouhairmaj/megabase/commit/b1dc2e967f13239e147a0d78e7e5b3ca35058006))

## [0.1.23](https://github.com/Zouhairmaj/megabase/compare/v0.1.22...v0.1.23) (2026-10-14)


### Features

* **rest:** embed-join inner/left for [#24](https://github.com/Zouhairmaj/megabase/issues/24) ([#294](https://github.com/Zouhairmaj/megabase/issues/294)) ([1717399](https://github.com/Zouhairmaj/megabase/commit/1717399e6aaad56933c43eb19ed586af92888edb))

## [0.1.22](https://github.com/Zouhairmaj/megabase/compare/v0.1.21...v0.1.22) (2026-10-14)


### Bug Fixes

* **rest:** honour aggregates-enabled so aggregate Judge cases pass ([#22](https://github.com/Zouhairmaj/megabase/issues/22)) ([#292](https://github.com/Zouhairmaj/megabase/issues/292)) ([45f3546](https://github.com/Zouhairmaj/megabase/commit/45f354657de733119ce326ee9ab9cc3eaf545332))


### Documentation

* add 13 October devlog entry ([#296](https://github.com/Zouhairmaj/megabase/issues/296)) ([f18d91c](https://github.com/Zouhairmaj/megabase/commit/f18d91ce0d1d78fc9e7948357f2d6e4c1ea52be9))

## [0.1.21](https://github.com/Zouhairmaj/megabase/compare/v0.1.20...v0.1.21) (2026-10-13)


### Features

* **auth:** validate resend requests before mailer 501 ([#285](https://github.com/Zouhairmaj/megabase/issues/285)) ([387c651](https://github.com/Zouhairmaj/megabase/commit/387c651397eac61fd93397912c2001f32eb4f24f))
* **rest:** serve x-www-form-urlencoded request bodies ([#283](https://github.com/Zouhairmaj/megabase/issues/283)) ([e91e2cf](https://github.com/Zouhairmaj/megabase/commit/e91e2cf54da13b490d81b084aeb8c56eba8e7ff2))

## [0.1.20](https://github.com/Zouhairmaj/megabase/compare/v0.1.19...v0.1.20) (2026-10-13)


### Features

* add megabase-model-share line attribution by model ([#275](https://github.com/Zouhairmaj/megabase/issues/275)) ([b7a694d](https://github.com/Zouhairmaj/megabase/commit/b7a694dea9ccfc41603a5cf369a30ab59c3d50cb))
* **auth:** validate recover requests and hide unknown emails ([#279](https://github.com/Zouhairmaj/megabase/issues/279)) ([9eda693](https://github.com/Zouhairmaj/megabase/commit/9eda693a6b3ac0b581a5b56285821fae51d9a7e2))

## [0.1.19](https://github.com/Zouhairmaj/megabase/compare/v0.1.18...v0.1.19) (2026-10-11)


### Features

* **site:** add dither database illustration to README banners ([#276](https://github.com/Zouhairmaj/megabase/issues/276)) ([7253521](https://github.com/Zouhairmaj/megabase/commit/7253521f5f8f7e8949d3dbd233815adc6a9018f4))

## [0.1.18](https://github.com/Zouhairmaj/megabase/compare/v0.1.17...v0.1.18) (2026-10-10)


### Features

* **rest:** serve prefer return, timezone and tx ([#269](https://github.com/Zouhairmaj/megabase/issues/269)) ([79ee905](https://github.com/Zouhairmaj/megabase/commit/79ee9057c7767ffc80a3b365f6b72371ba5db8d9))
* **rest:** serve text/csv and reject text/plain, text/xml on relations ([#266](https://github.com/Zouhairmaj/megabase/issues/266)) ([56f7a40](https://github.com/Zouhairmaj/megabase/commit/56f7a40ed0d9db3b7df777429a6c636d5691faf9))

## [0.1.17](https://github.com/Zouhairmaj/megabase/compare/v0.1.16...v0.1.17) (2026-10-10)


### Features

* **rest:** add !left and !inner resource embedding ([#256](https://github.com/Zouhairmaj/megabase/issues/256)) ([680ea6e](https://github.com/Zouhairmaj/megabase/commit/680ea6e9de039d2819104f1330ffae628af90670))
* **rest:** add prefer count, handling, max-affected, missing, resolution ([#267](https://github.com/Zouhairmaj/megabase/issues/267)) ([3fb0b37](https://github.com/Zouhairmaj/megabase/commit/3fb0b3794392c85dc21d667a3243994af6f20fb8))
* **rest:** serve object+json and nulls=stripped media types ([#265](https://github.com/Zouhairmaj/megabase/issues/265)) ([8730496](https://github.com/Zouhairmaj/megabase/commit/8730496ebae74b3f5ea492c953b1b134623df9be))

## [0.1.16](https://github.com/Zouhairmaj/megabase/compare/v0.1.15...v0.1.16) (2026-10-10)


### Features

* **rest:** mark and/or/not logic operators and spec ([#257](https://github.com/Zouhairmaj/megabase/issues/257)) ([94ebf84](https://github.com/Zouhairmaj/megabase/commit/94ebf841717308fd60c48eb5f1f21f1f4328c70b))

## [0.1.15](https://github.com/Zouhairmaj/megabase/compare/v0.1.14...v0.1.15) (2026-10-10)


### Features

* **auth:** serve admin user, SSO, OAuth client and custom-provider writes ([#250](https://github.com/Zouhairmaj/megabase/issues/250)) ([34f2b89](https://github.com/Zouhairmaj/megabase/commit/34f2b89fc1f922001532dc8a35313da2e4d1b993))

## [0.1.14](https://github.com/Zouhairmaj/megabase/compare/v0.1.13...v0.1.14) (2026-10-10)


### Features

* **rest:** serve select aggregate functions ([#254](https://github.com/Zouhairmaj/megabase/issues/254)) ([f62a8bd](https://github.com/Zouhairmaj/megabase/commit/f62a8bd126bbab3f5edcb98c7dd8f7ea0589c38f))

## [0.1.13](https://github.com/Zouhairmaj/megabase/compare/v0.1.12...v0.1.13) (2026-10-10)


### Bug Fixes

* **coverage:** hide standalone conformance % in README treemap ([#258](https://github.com/Zouhairmaj/megabase/issues/258)) ([83f4506](https://github.com/Zouhairmaj/megabase/commit/83f4506db7203387f9b5776f48b75b66537cfdb0))


### Documentation

* **judge:** add Level 1 minimum cases and hidden suite gate ([#255](https://github.com/Zouhairmaj/megabase/issues/255)) ([a5cf89f](https://github.com/Zouhairmaj/megabase/commit/a5cf89f03c15f71b218310cbcca86ea7060eea22))

## [0.1.12](https://github.com/Zouhairmaj/megabase/compare/v0.1.11...v0.1.12) (2026-10-10)


### Features

* **judge:** compare acls, column defaults and rls policies ([#246](https://github.com/Zouhairmaj/megabase/issues/246)) ([cbeab52](https://github.com/Zouhairmaj/megabase/commit/cbeab525cf59e6f4445773ee1437969f5b2a914c))


### Bug Fixes

* **judge:** compare decoded jwt claims in normalisation ([#245](https://github.com/Zouhairmaj/megabase/issues/245)) ([e746bcd](https://github.com/Zouhairmaj/megabase/commit/e746bcdf0f0856977577b389bcaf3e594016991f))
* **site:** show commit and PR totals on home stats ([#252](https://github.com/Zouhairmaj/megabase/issues/252)) ([5e91e44](https://github.com/Zouhairmaj/megabase/commit/5e91e44d2f47b85a1aaf7ca953d601c1eb40b675))

## [0.1.11](https://github.com/Zouhairmaj/megabase/compare/v0.1.10...v0.1.11) (2026-10-10)


### Features

* **site:** align home page with kite mockups ([#236](https://github.com/Zouhairmaj/megabase/issues/236)) ([de42810](https://github.com/Zouhairmaj/megabase/commit/de42810cd1aaf119255764e1e5e99e690c82ed3d))


### Bug Fixes

* **site:** cfg-gate counts.rs Write/Stdio imports ([#244](https://github.com/Zouhairmaj/megabase/issues/244)) ([b8851ff](https://github.com/Zouhairmaj/megabase/commit/b8851ff769bf0e61b46202953f2cdfa8e5a883f4))
* **site:** mobile docs layout matches Kite ([#239](https://github.com/Zouhairmaj/megabase/issues/239)) ([642f18a](https://github.com/Zouhairmaj/megabase/commit/642f18ab35751985725dc2f15a90f7e0f2cc0cf5))

## [0.1.10](https://github.com/Zouhairmaj/megabase/compare/v0.1.9...v0.1.10) (2026-10-10)


### Features

* **rest:** serve rpc routes ([91564b3](https://github.com/Zouhairmaj/megabase/commit/91564b36552d650491e89a817d6edf53f5eadaad))

## [0.1.9](https://github.com/Zouhairmaj/megabase/compare/v0.1.8...v0.1.9) (2026-10-10)


### Features

* **rest:** serve resource routes ([7c162a7](https://github.com/Zouhairmaj/megabase/commit/7c162a7dddb1425939a4b4db7facb694bb255cfb))
* **site:** animated dither database in the home hero ([#229](https://github.com/Zouhairmaj/megabase/issues/229)) ([c0015d3](https://github.com/Zouhairmaj/megabase/commit/c0015d3cc01e5baad9275d1a2c5fdee820310afe))


### Bug Fixes

* drop rsa from the lockfile ([#230](https://github.com/Zouhairmaj/megabase/issues/230)) ([11bca54](https://github.com/Zouhairmaj/megabase/commit/11bca547aa79dc8283e1b43b397261c3406ecbbe))

## [0.1.8](https://github.com/Zouhairmaj/megabase/compare/v0.1.7...v0.1.8) (2026-10-10)


### Features

* **rest:** serve query parameters on get ([#223](https://github.com/Zouhairmaj/megabase/issues/223)) ([34e4538](https://github.com/Zouhairmaj/megabase/commit/34e4538ede0a078b98e905b45b982f4e11d5fe6e))
* **site:** show release version next to logo ([#221](https://github.com/Zouhairmaj/megabase/issues/221)) ([cca58fb](https://github.com/Zouhairmaj/megabase/commit/cca58fb83235fccaa5f9c65190f4bf9b0fd4d75d))
* **site:** simplify hero eyebrow and hide conformance % ([#226](https://github.com/Zouhairmaj/megabase/issues/226)) ([f5ebf2c](https://github.com/Zouhairmaj/megabase/commit/f5ebf2ceb13f8b4e0a9dfe4b6b772db8e28345d8))

## [0.1.7](https://github.com/Zouhairmaj/megabase/compare/v0.1.6...v0.1.7) (2026-10-10)


### Features

* **rest:** serve range and full-text filters ([#217](https://github.com/Zouhairmaj/megabase/issues/217)) ([bcfe3a8](https://github.com/Zouhairmaj/megabase/commit/bcfe3a8f8029ad9f963fcc4227d5a648c07d9527))
* **site:** make live status the primary hero CTA ([#218](https://github.com/Zouhairmaj/megabase/issues/218)) ([05ab7be](https://github.com/Zouhairmaj/megabase/commit/05ab7be433918e1414460766df03679a1c85b5fa))


### Documentation

* **site:** add the 9 and 10 october devlog ([#219](https://github.com/Zouhairmaj/megabase/issues/219)) ([83e8a32](https://github.com/Zouhairmaj/megabase/commit/83e8a32c6a3034e308999f94b6160c80057c7efb))

## [0.1.6](https://github.com/Zouhairmaj/megabase/compare/v0.1.5...v0.1.6) (2026-10-10)


### Bug Fixes

* **ci:** tolerate non-issue PRs in board-sync ([#212](https://github.com/Zouhairmaj/megabase/issues/212)) ([3c9bad5](https://github.com/Zouhairmaj/megabase/commit/3c9bad5bd6024b8854bbe858a811718402075ee5))

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
