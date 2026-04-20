# Changelog

## [0.2.0](https://github.com/kvnpetit/tynd/compare/v0.1.0...v0.2.0) (2026-04-20)


### Features

* **docs:** scaffold docs site on Next.js + Nextra with full SEO wiring ([8b20e08](https://github.com/kvnpetit/tynd/commit/8b20e08592e1207dfeea2f85708d9022a69e0e40))


### Bug Fixes

* **cli:** auto-download @tynd/host binary when postinstall skipped ([3195c2b](https://github.com/kvnpetit/tynd/commit/3195c2b28851913fd66864211c1a9c72e0e63bed))


### Documentation

* rework site — custom Cards, theme sync, nav parity, LLMs sidebar ([b7c3294](https://github.com/kvnpetit/tynd/commit/b7c3294be6b517f34b5836752913ae925990a7ea))


### Continuous Integration

* **docs:** deploy docs to Cloudflare Workers after release ([db0f796](https://github.com/kvnpetit/tynd/commit/db0f79650a435712c04221e6ddb53a6cb2bd8fd6))
* drop release-as pin from release-please config ([9092b7d](https://github.com/kvnpetit/tynd/commit/9092b7d141a7d114ae7d7f72d7e7a12a7ac1a1d5))

## 0.1.0 (2026-04-19)


### Features

* 2nd-launch focus + argv forwarding ([f1c15de](https://github.com/kvnpetit/tynd/commit/f1c15de1b10f82b3f15ae19d851396f628d19071))
* app APIs, OS info, dialogs, clipboard, menu accelerators ([a6a7322](https://github.com/kvnpetit/tynd/commit/a6a73224f7ae96dd23772ab50c88a19b3864e8fd))
* **cli:** --cwd/--json on info, --dry-run on clean, global --verbose/--quiet, typo suggestions ([d587dde](https://github.com/kvnpetit/tynd/commit/d587dde1f4af25c7139916099341dc67c6e2a184))
* **cli:** @vorn/cli — scaffolding, dev mode, build pipeline ([69f57a5](https://github.com/kvnpetit/tynd/commit/69f57a5ed06896caf2e9f3167322f3a963a36668))
* **cli:** add `tynd start` to run app from prebuilt artifacts ([e8ac05c](https://github.com/kvnpetit/tynd/commit/e8ac05c49ec7b4051120a8a29487953549d9a0b4))
* **cli:** bundle sidecar binaries into the built .exe ([56ee3d3](https://github.com/kvnpetit/tynd/commit/56ee3d3c6fbc8d6a073ffbd667ff26a4302db13a))
* **cli:** code signing + macOS notarization ([a56b958](https://github.com/kvnpetit/tynd/commit/a56b958e1e94ca3096b8773c926b1b79077ba482))
* **cli:** DX — verbose mode propagates to host, smarter binary hint ([2c2c292](https://github.com/kvnpetit/tynd/commit/2c2c292e9bd0a028bb8557d15b6e42f122f80efd))
* **cli:** installer bundles via tynd build --bundle (app/dmg/deb/rpm/appimage/nsis/msi) ([87463e0](https://github.com/kvnpetit/tynd/commit/87463e06a86a13abddc369768f23c21746f7beb2))
* **cli:** invalidate cache on CLI version bump + watch package.json ([02facd2](https://github.com/kvnpetit/tynd/commit/02facd2338675f25c1a7bdb63f379b7bfcdfda16))
* **cli:** keygen + sign for the auto-updater ([aea90a6](https://github.com/kvnpetit/tynd/commit/aea90a65bb49f69027fa5a2bd840cbb471f03898))
* **cli:** multi-size icon generation per OS ([45fa915](https://github.com/kvnpetit/tynd/commit/45fa91535835dd67365746d995a7c333c372dc05))
* **cli:** prefix child process logs with [host] / [vite] for clarity ([e567bf4](https://github.com/kvnpetit/tynd/commit/e567bf41d1280b0dcb959f7e118cb0da991682c0))
* **cli:** prompt before installing deps + pin runtime versions ([3f33b51](https://github.com/kvnpetit/tynd/commit/3f33b518127dee0256fb314c2661dc8a5a4baa68))
* **cli:** rebuild in `tynd start`; square SVG icons; NSIS per-user by default ([982a757](https://github.com/kvnpetit/tynd/commit/982a757340c5494e12bb91675bae125ebc2f713a))
* **cli:** reorder runtime prompts, update outdated hints ([74933f9](https://github.com/kvnpetit/tynd/commit/74933f946a0ee7b802bca038705c50de7e17dd33))
* **cli:** runtime-validate vorn.config with valibot ([a32afbd](https://github.com/kvnpetit/tynd/commit/a32afbdbf63ea0cfb36d6d16458cf28d80ac2de8))
* **cli:** watch vorn.config.ts + log reload duration ([556e996](https://github.com/kvnpetit/tynd/commit/556e996bc2cbbe10d4d5f5d417c73a13aede6156))
* **cli:** wire up log.debug, TTY/NO_COLOR detection, spinners + unified error hints ([918d404](https://github.com/kvnpetit/tynd/commit/918d40425c5d41df7084dc2f7784affe35d0fb71))
* **core:** @vorn/core — typed RPC client and app lifecycle ([f944b9e](https://github.com/kvnpetit/tynd/commit/f944b9e0bc39431947aee74e69a08a5f8788fb80))
* **core:** re-export Web-platform globals from @tynd/core/client ([7fb4030](https://github.com/kvnpetit/tynd/commit/7fb4030c1ee02753bb50de6028c41fcfde50853f))
* **core:** route binary client APIs through tynd-bin:// ([d74cebc](https://github.com/kvnpetit/tynd/commit/d74cebc0b1b99b5fed9fa1f3aaad0efef18e1974))
* **core:** streaming IPC via AsyncGenerator ([fcb1170](https://github.com/kvnpetit/tynd/commit/fcb1170c9d85a72b1de3d4b5a3ef0066fa4db122))
* deep-link URL schemes (myapp://) ([1b1ac8a](https://github.com/kvnpetit/tynd/commit/1b1ac8a2365d5761a833e0f230f2410a81d2f8a6))
* **dev:** hot-reload backend without tearing down the WebView ([4ba0117](https://github.com/kvnpetit/tynd/commit/4ba01175dffb52e38ba47954146be8ae502e4593))
* fs.watch + global keyboard shortcuts ([dc0cb86](https://github.com/kvnpetit/tynd/commit/dc0cb86aef69f765f39e4fe6b42b452c5d8e679f))
* **full:** vorn-full runtime launcher — Bun subprocess backend ([250067f](https://github.com/kvnpetit/tynd/commit/250067f8d8c7543c497e6865e8132979416acaba))
* **host-rs:** native WebView host library ([30d1f76](https://github.com/kvnpetit/tynd/commit/30d1f76fb5c689aa17deabe14aa69208e4dc9ace))
* **host:** @vorn/host — npm wrapper for native binaries ([32bf955](https://github.com/kvnpetit/tynd/commit/32bf95528ff7e9db84d0de8022784746c9f2250f))
* **host:** `compute.randomBytes` — CSPRNG for lite + full parity ([9495149](https://github.com/kvnpetit/tynd/commit/9495149ff4805b99fb3fd085ddf995a5c0971671))
* **host:** add `singleInstance` API to prevent dual-launch ([6bade07](https://github.com/kvnpetit/tynd/commit/6bade074abc4e7b2d0c49e25b1e5f522d88f10f0))
* **host:** add process / fs / store / os OS APIs (lite + full parity) ([8713f57](https://github.com/kvnpetit/tynd/commit/8713f57e45f3ac2eb23c03df0a3f6f61b5c7f6e7))
* **host:** embedded SQL via rusqlite (lite parity with bun:sqlite) ([92cf162](https://github.com/kvnpetit/tynd/commit/92cf16206091bfc8136110d7f46155498285fe66))
* **host:** HTTP client, sidecar, PTY terminal, binary fs + shared event bus ([0584ffe](https://github.com/kvnpetit/tynd/commit/0584ffea74605c300e7002ebccc6ac4c0be901d5))
* **host:** native compute + workers for lite/full perf parity ([95799f2](https://github.com/kvnpetit/tynd/commit/95799f251de20c0c8ee61376c1064a4b6c4ada51))
* **host:** opt-in crash reporter (panic -&gt; file) ([a241b26](https://github.com/kvnpetit/tynd/commit/a241b2671e35ca6b432b6eec19ee669c9e6a5335))
* **host:** TYND_LOG env-gated logger (info/warn/error/debug) ([7623876](https://github.com/kvnpetit/tynd/commit/76238761e7e663d35be6f7cbd9e305aecb937606))
* **host:** WebSocket client via tungstenite (lite/full parity) ([6cdc26a](https://github.com/kvnpetit/tynd/commit/6cdc26a11caef8e0dfc29ca8f5aaa7f09b380ae5))
* **host:** zero-copy binary IPC via tynd-bin:// custom scheme ([e378893](https://github.com/kvnpetit/tynd/commit/e3788939772a893281357992df01ed6593f36e1d))
* keyring, CSP, webview reload / zoom / devtools ([15d54c2](https://github.com/kvnpetit/tynd/commit/15d54c20b4d5c8bad2961b8f29f5f6970dd9e04d))
* **lite:** hot-reload backend in QuickJS mode without tearing down the WebView ([7bd96d9](https://github.com/kvnpetit/tynd/commit/7bd96d94d0256beb8633cd6ff9b91a5d059b5afe))
* **lite:** vorn-lite runtime launcher — embedded QuickJS backend ([253557b](https://github.com/kvnpetit/tynd/commit/253557b4ea92a965fd3ff09f9ca7db8b06c347fc))
* **lite:** Web-standards polyfill layer ([1334f15](https://github.com/kvnpetit/tynd/commit/1334f1555871e85bcf3f37ccc734d1d85e089c78))
* multi-window support via tyndWindow.create ([91c5ca7](https://github.com/kvnpetit/tynd/commit/91c5ca7dbced21d834798a2c0eb2884833c2e605))
* **playground:** example demo + generic LLM chatbot ([9bf0a1b](https://github.com/kvnpetit/tynd/commit/9bf0a1b9e4aa719943029f27b5f22fd096c820f2))
* **updater:** install + relaunch on Windows and Linux ([bf73a96](https://github.com/kvnpetit/tynd/commit/bf73a969465cb7e08b27f3473309871b2353dd5b))
* **updater:** manifest check + signed download ([a672912](https://github.com/kvnpetit/tynd/commit/a6729127b369d423cb98acb57fec549cea6783ac))
* validate every external input with valibot + trace hot paths with log.debug ([0f955bb](https://github.com/kvnpetit/tynd/commit/0f955bb3be96d32c7ca241ef0534b6a9af65114f))
* window events via tyndWindow ([9d53794](https://github.com/kvnpetit/tynd/commit/9d5379472588b12942453ae51bc4eb5a640c0678))
* window geometry, monitors, autolaunch ([4f7cc57](https://github.com/kvnpetit/tynd/commit/4f7cc57ea904c8d632639cc31fda480a5d2131aa))


### Bug Fixes

* 17 bugs across host / core / cli ([172e685](https://github.com/kvnpetit/tynd/commit/172e6852468807bd1d06bfcb2de12462dd4f70ff))
* bulletproof streaming RPC at any yield rate ([5a6aaac](https://github.com/kvnpetit/tynd/commit/5a6aaac4051223a16612304f538d6f1c52aea114))
* **ci:** restore Rust builds on all runners ([a9df36e](https://github.com/kvnpetit/tynd/commit/a9df36e6597f6e8c2bd74bba6dc9fc54ed42b0e6))
* **cli:** create rolls back on scaffold failure + init bootstraps package.json ([d2d66bd](https://github.com/kvnpetit/tynd/commit/d2d66bdc2a7281b6ffca1f0de6bcfd5609b6538a))
* **cli:** default Angular outputPath to dist/&lt;project&gt;[/browser] ([6aec2b0](https://github.com/kvnpetit/tynd/commit/6aec2b0a73425d35c1b1d816af3653875635f024))
* **host:** add typecheck — previously skipped due to missing tsconfig ([89d2ff1](https://github.com/kvnpetit/tynd/commit/89d2ff18d5117e33e1700c158a12600e17d58a11))
* **host:** register tynd-bin in dev mode, offload IO to call_pool ([38437b7](https://github.com/kvnpetit/tynd/commit/38437b7fa75ffbe8ced1de7fa80305eda866d7ae))


### Performance

* **cli:** Bun-first file APIs in config + pkg loaders ([1fa5a4a](https://github.com/kvnpetit/tynd/commit/1fa5a4accdfd5783294418d955ad849ae0cbc693))
* **cli:** use Bun.hash / Bun.write / Bun.CryptoHasher on hot paths ([c3f660b](https://github.com/kvnpetit/tynd/commit/c3f660b9e02fcf23ce5413a30e1e0fe01de1a674))
* **core:** inline runtime check so Bun DCE drops the dead branch ([ed557e3](https://github.com/kvnpetit/tynd/commit/ed557e39c67f474fa107ae70e3faccb243f47e80))
* **host:** bounded worker pool for OS dispatch (no more spawn-per-call) ([368cc44](https://github.com/kvnpetit/tynd/commit/368cc44dfc138383ef383ca441b9f2a417ffb1ca))
* **host:** enable WAL + synchronous=NORMAL on on-disk SQL opens ([a08c146](https://github.com/kvnpetit/tynd/commit/a08c14642cfe75979679ebf91c03d058dbc20592))
* **host:** pre-serialize OS events to skip a Value clone ([74d93c0](https://github.com/kvnpetit/tynd/commit/74d93c00b3483e25b82abef8e76873926a1b399a))
* **host:** replace websocket busy-poll with blocking read_timeout ([3dbf5a4](https://github.com/kvnpetit/tynd/commit/3dbf5a4b66e365292504b1fe58e400c95548684c))
* **host:** swap Mutex&lt;HashMap&gt; registries to DashMap ([1225108](https://github.com/kvnpetit/tynd/commit/1225108d57916389cd75a6587884f26e51cc02d5))
* **host:** swap std::sync Mutex/Condvar to parking_lot ([623e0ea](https://github.com/kvnpetit/tynd/commit/623e0ea8fe8ed1f8850bd7a2497038d2f0f30a57))
* migrate compression from gzip to zstd + bump deps ([63f3798](https://github.com/kvnpetit/tynd/commit/63f3798751c2ecc01716de6b70d9f1b70fe0ca21))


### Refactors

* **cli:** swap all remaining file IO to Bun.file / Bun.write ([4dd54c0](https://github.com/kvnpetit/tynd/commit/4dd54c0713b8065158e1d52026bfff22cafd8400))
* **cli:** use Bun.semver.order instead of hand-rolled comparator ([5d60ce2](https://github.com/kvnpetit/tynd/commit/5d60ce2c1d64cf07e88537935e4fff9e09f7c203))
* **core,cli:** split client.ts + extract pack.ts and spawn-helpers ([aa446e3](https://github.com/kvnpetit/tynd/commit/aa446e3e2fd54a548e3ca4141f541bf4365d410a))
* **core:** trim non-essential public surface ([43f8536](https://github.com/kvnpetit/tynd/commit/43f85363894698d6392c8ec18548a542d6bdf8a2))
* **host:** drop dead JSON binary paths in fs + compute ([ce2342c](https://github.com/kvnpetit/tynd/commit/ce2342cbed145e466635388b0d4665aaef902857))
* **host:** rename custom scheme from bv:// to tynd:// ([1126be4](https://github.com/kvnpetit/tynd/commit/1126be4ffcaf335edb39d772452c45a288cc082b))
* share TYNDPKG reader + isolate dev reload machinery ([c51953e](https://github.com/kvnpetit/tynd/commit/c51953edd92ccf46c430222b80b59d19f9a8682e))
* strip decorative separators and extract menu/tray modules ([3efdb9b](https://github.com/kvnpetit/tynd/commit/3efdb9b2ac15925a9227cadd054658d0d9058c74))
* trim narrative comments, keep only the WHY ([c54a0ee](https://github.com/kvnpetit/tynd/commit/c54a0ee9a0d0a2b29075acbd7254e56fb62b8fbc))


### Documentation

* add FRAMEWORKS.md with per-framework support matrix ([39d25cf](https://github.com/kvnpetit/tynd/commit/39d25cfd29b2faf25a81d13960fd3dd4767c2633))
* add SIGNING.md covering Windows, macOS and Linux workflows ([2133e28](https://github.com/kvnpetit/tynd/commit/2133e2883274775d062bb446f9e7f774265f3b91))
* clarify Bun prerequisite across READMEs ([23892f0](https://github.com/kvnpetit/tynd/commit/23892f05fb02ba701398a02b2b7530101f895ffc))
* **comparison:** add 7 missing categories and refresh feature tables ([2b999e2](https://github.com/kvnpetit/tynd/commit/2b999e25be093334ec8ee5a37029b02543089680))
* document compute + workers; add Concurrency comparison section ([0e2b5c2](https://github.com/kvnpetit/tynd/commit/0e2b5c275a4dbcd6b4443a33d0858eee48e2441a))
* document randomBytes, websocket, sql APIs across docs ([ab985ef](https://github.com/kvnpetit/tynd/commit/ab985ef584c05d3e6c321616c5631922f2b1496d))
* document tynd:// rename and tynd-bin:// binary scheme ([406cdc5](https://github.com/kvnpetit/tynd/commit/406cdc5aa960ec68d2359525b9ec5bf47c1ae7d7))
* drop stack jargon from user-facing docs, surface phase 5-10 features ([6798302](https://github.com/kvnpetit/tynd/commit/67983024a90f891e3685d723ee3ded96b269ab1b))
* emoji section anchors + banned unicode arrows purged ([d21b3ea](https://github.com/kvnpetit/tynd/commit/d21b3ea8f311015d67965d38bb7c802c74943e2d))
* overhaul around lite/full philosophy + comprehensive catalog ([2985f9e](https://github.com/kvnpetit/tynd/commit/2985f9eac9418448596f86e85f3245abe3e3cd3b))
* README, runtime guide, framework comparison ([a0429c8](https://github.com/kvnpetit/tynd/commit/a0429c810101e7e6cb3a72510979173bb75d2ccd))
* refresh binary sizes, highlight Bun JIT, hide engine name ([ea093f8](https://github.com/kvnpetit/tynd/commit/ea093f89727068ff904fe486b5458e46a18746d4))
* **runtimes:** drop forward-looking mentions of a tynd bench command ([38d78a4](https://github.com/kvnpetit/tynd/commit/38d78a4ea2339b31b96ab917a431609d7481498d))
* **runtimes:** mark tynd-bin routes in the OS API table ([12b75a5](https://github.com/kvnpetit/tynd/commit/12b75a5d6b1b5e9cb1ab8b4d5c46a9064cb74f00))
* split API reference into API.md; document new OS APIs ([4555786](https://github.com/kvnpetit/tynd/commit/4555786a7d122b93abc4f7fa76ea3653c026ecee))
* sync COMPARISON + GETTING_STARTED with singleInstance, crashReporter, tests ([224e779](https://github.com/kvnpetit/tynd/commit/224e7794e40200d8fde01a5bdb7988fc83ab2fa9))
* sync NOTICE, getting-started, contributing with current state ([c276a4c](https://github.com/kvnpetit/tynd/commit/c276a4c2d00c76e88e999771500f831b310ba171))


### Build System

* add parking_lot + dashmap + ahash, drop ureq json feature ([87d6ec0](https://github.com/kvnpetit/tynd/commit/87d6ec0e3e25542b1a6614ee77c97f244396f0eb))


### Continuous Integration

* drop redundant cargo check after clippy ([c6000e1](https://github.com/kvnpetit/tynd/commit/c6000e16cbd790a60ba2e30d5c1b3fb15abfde8b))
* GitHub Actions pipeline — CI, release-please, host build ([5403ec1](https://github.com/kvnpetit/tynd/commit/5403ec12d25f05c8426069c57a2481e64930fc1a))
* overhaul release pipeline — unified changelog, atomic publish, wider matrix ([3b82ef1](https://github.com/kvnpetit/tynd/commit/3b82ef1dd0493143721a0755f19bc608e0beaf11))
* **release:** cross-compile macOS and drop macos-13 runner ([36151f4](https://github.com/kvnpetit/tynd/commit/36151f45d472ce732d991e3c34cc36398508f82a))
