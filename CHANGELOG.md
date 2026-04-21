# Changelog

## [0.3.0](https://github.com/kvnpetit/tynd/compare/v0.2.0...v0.3.0) (2026-04-21)


### Features

* **app:** getFrameworkVersion + getBundleIdentifier ([344b711](https://github.com/kvnpetit/tynd/commit/344b7119b0f6d9ca0647061ef7ac62e12bb5039a))
* **app:** resourceDir + quitOnLastWindowClosed flag ([a991b65](https://github.com/kvnpetit/tynd/commit/a991b6553f712f3726f808e3287242cc91f58764))
* **backend:** evalInFrontend(script, label?) ([7b4d228](https://github.com/kvnpetit/tynd/commit/7b4d228892e1479678e4fc4cdffdcda9d83aab83))
* **clipboard:** onChange polling monitor ([8cf7e54](https://github.com/kvnpetit/tynd/commit/8cf7e542b980414cba2d842fd9f2f9156b09b451))
* **core:** add log API with JSON-lines file + size rotation ([1199fdd](https://github.com/kvnpetit/tynd/commit/1199fddd69b6bbeb0701e1fe477befc365c4c8e6))
* **core:** frontend -&gt; backend events via createFrontendEmitter ([c91e101](https://github.com/kvnpetit/tynd/commit/c91e1011e92945e304277407f3c084f0e02cec5a))
* **dialog:** canCreateDirectories option (macOS) ([135cd42](https://github.com/kvnpetit/tynd/commit/135cd420b4609a82d97d6b64e3bfa9d0683322e5))
* **events:** emitter.emitTo(label) targets a single window ([5f97fd9](https://github.com/kvnpetit/tynd/commit/5f97fd9b823534e0ae77be6773c44f1d91364f33))
* **fs:** file handle, symlink, hardlink, copyDir ([5004f62](https://github.com/kvnpetit/tynd/commit/5004f6261f4e898b96889fa5681f63efe82397af))
* **host:** add shell.revealInFolder and fs.trash ([eccbea9](https://github.com/kvnpetit/tynd/commit/eccbea9a306fb3e3aef6457be6402c60857cb3d9))
* **host:** emit native drag-drop events (window:drop, window:drag-*) ([ec59183](https://github.com/kvnpetit/tynd/commit/ec59183863f1cffb84b4b2e9be97c79936fa2076))
* **log:** captureConsole bridges console.* into log.write ([dc0b21e](https://github.com/kvnpetit/tynd/commit/dc0b21ea8cae7843a8383ba9a6bfd4bc2f9e7648))
* **menu:** add programmatic context menu (Windows) ([065e5c0](https://github.com/kvnpetit/tynd/commit/065e5c04a75534c31aac6ce412e4ce8c172b2dee))
* **monitor:** fromPoint + cursorPosition ([f9f70e0](https://github.com/kvnpetit/tynd/commit/f9f70e057a03d411fe001ca2152106c61cb271c1))
* **notification:** add icon, sound, action buttons + onAction ([7aa76cd](https://github.com/kvnpetit/tynd/commit/7aa76cdd4b855dd65d8f62b2ae898d662eb00a79))
* **notification:** checkPermission + schedule + cancel ([a636b99](https://github.com/kvnpetit/tynd/commit/a636b99ccdd35e40b98ef4560f5fc40d1c156559))
* **power:** add power.getIdleTime (cross-platform) ([fa36385](https://github.com/kvnpetit/tynd/commit/fa363850a8287c7fd51214839d0f05f844c03052))
* **rpc:** listenN + once helpers for ad-hoc events ([96b821f](https://github.com/kvnpetit/tynd/commit/96b821ff050055294cd3ea433ef5cb217d0efc3e))
* **rpc:** withTimeout + abortable helpers ([ed7dab0](https://github.com/kvnpetit/tynd/commit/ed7dab00f58d17df8e43937e3656dd647d436c33))
* **security:** add opt-in capability ACL for fs + http ([640e66c](https://github.com/kvnpetit/tynd/commit/640e66ccf7395426bc98749a27a6417bd6e0d408))
* **tray:** onEnter/onMove/onLeave mouse events ([dcb55b3](https://github.com/kvnpetit/tynd/commit/dcb55b3b33111966014c53c90b56266326fb24be))
* **tray:** runtime setIcon/setTooltip/setTitle/setMenu/setVisible ([5d4983e](https://github.com/kvnpetit/tynd/commit/5d4983e73671803e8eac905b88f233f544352470))
* **tray:** setIconAsTemplate (macOS) ([b10e062](https://github.com/kvnpetit/tynd/commit/b10e0625a6f1b023a3fbacadc789e2407a2ac2f3))
* **updater:** periodic check + proxy + custom headers + allowDowngrade ([c91ed86](https://github.com/kvnpetit/tynd/commit/c91ed860df5392df06d5a9445b7022e9cce79510))
* **webview:** userAgent + onNavigation + onPageLoad ([118a780](https://github.com/kvnpetit/tynd/commit/118a780c510ccbffcc34eaa8b3d7516015802e66))
* **window:** add findInPage + stopFindInPage ([cf9cad4](https://github.com/kvnpetit/tynd/commit/cf9cad4ef665d31c3f34090f2e5c9455ebaf004c))
* **window:** add print, navigate, loadHtml, getUrl ([2a9e93a](https://github.com/kvnpetit/tynd/commit/2a9e93a0bef5f91c3740b5f78e314206b9d3c757))
* **window:** appearance polish (theme, bg, shadow, progress, badge, skip taskbar) ([2e7f1cf](https://github.com/kvnpetit/tynd/commit/2e7f1cf4846c2c90d69eb19ff50a5cde7e041558))
* **window:** closeSelf helper ([c31baf3](https://github.com/kvnpetit/tynd/commit/c31baf3d9810c04106d1d36632337549a3c85c9b))
* **window:** cursor & mouse API (whole §6 category) ([bcd5ea2](https://github.com/kvnpetit/tynd/commit/bcd5ea2cde506fedf5962bc57934c6087daab2d3))
* **window:** getTitle getter ([9a485f3](https://github.com/kvnpetit/tynd/commit/9a485f3e515ea9f6c3faebe2edbcd25437d14904))
* **window:** getZoom cached getter ([6c662c1](https://github.com/kvnpetit/tynd/commit/6c662c1a5a5699ab9b7f0a902212f1c704848b37))
* **window:** hideOnFocusLost + hideOnEscape config flags ([ac1de16](https://github.com/kvnpetit/tynd/commit/ac1de16b887fde708a08fe1a5052ffa9dfeddf9e))
* **window:** macOS titlebar transparent + fullsize content + traffic light inset ([01eb849](https://github.com/kvnpetit/tynd/commit/01eb849ecb08565b2c8f827b2a400cdc6f2dcab8))
* **window:** modalTo parent handle on create (Win+macOS) ([88af830](https://github.com/kvnpetit/tynd/commit/88af83006a5eb2ee1aa1dacc956f3df5582f0068))
* **window:** positionAt(preset) — 13-preset positioner ([bb26ce7](https://github.com/kvnpetit/tynd/commit/bb26ce7cc8cedb54140453850da3341beb0106ee))
* **window:** setFocusable + setEnabled (Windows) ([30b2a04](https://github.com/kvnpetit/tynd/commit/30b2a04a9607ab9ca05a849f8d00fc826716b350))
* **window:** setShadow (macOS) + setWindowIcon runtime ([207373a](https://github.com/kvnpetit/tynd/commit/207373a85fc9b38858c8f01d92f7ae319e2e344e))
* **window:** setVisibleOnAllWorkspaces (macOS) ([fc64b1c](https://github.com/kvnpetit/tynd/commit/fc64b1c0cdb87cadc151a4726b802cfccbd9ca88))
* **window:** Windows 11 Mica/Acrylic/Tabbed system backdrop ([0ba282f](https://github.com/kvnpetit/tynd/commit/0ba282f6fc9f6fbce95f07a589665eb9ba245027))


### Bug Fixes

* audit-surfaced bugs + document RPC helpers ([6661cd1](https://github.com/kvnpetit/tynd/commit/6661cd104d13a83b9dcec2b921721468950e94b9))
* **menu:** cross-OS context menu (macOS + Linux) ([1a35713](https://github.com/kvnpetit/tynd/commit/1a35713da647954044be8dbdfd45a471aee02460))
* **notification:** action callbacks on all OS ([0290319](https://github.com/kvnpetit/tynd/commit/0290319d86fb55f50950f912dc7d3edf57784c1e))


### Documentation

* **comparison:** mark window-scoped listeners supported via emitTo ([5672ce7](https://github.com/kvnpetit/tynd/commit/5672ce707d23991b648666808cb2022a653ca458))
* point homepage fields to tynd.kvnpetit.com ([5483a59](https://github.com/kvnpetit/tynd/commit/5483a5936955b3afb5532b5c69f99bc715f96bd1))
* **readme:** strip emoji from headings + tables ([09cbd49](https://github.com/kvnpetit/tynd/commit/09cbd49fee0568ca414a078e938d983618663f98))
* scaffold v0.2, fix version picker dropdown ([46dbf5b](https://github.com/kvnpetit/tynd/commit/46dbf5b1879592fba9a509a40fdf6c0a85d8d4c1))
* sync README/RUNTIMES/API with log/power/security + all v3 window APIs ([54b7a9b](https://github.com/kvnpetit/tynd/commit/54b7a9bc2b3fb9100d7a2ef964b69f5b6d5d8851))
* **v0.3:** cut new latest version with all v3 features ([c818ac1](https://github.com/kvnpetit/tynd/commit/c818ac191fd9062f60fc39e1fdf18e286ad3d7d2))


### Continuous Integration

* add Next.js + node_modules caching to workflows ([8d5b842](https://github.com/kvnpetit/tynd/commit/8d5b842da399f149d2791e499c7b5fdfa90bc299))
* flip separate-pull-requests true so title pattern resolves ([3efa3b6](https://github.com/kvnpetit/tynd/commit/3efa3b6fc9a0e550feebb418e17b3b2eacaba9aa))
* move release-please title pattern into the package block ([69c3423](https://github.com/kvnpetit/tynd/commit/69c342371da0e139e149eda7b26a02f22e1285d4))
* pin release-please PR title to `chore: release <version>` ([c7c6abe](https://github.com/kvnpetit/tynd/commit/c7c6abec0a12a2c9de3cc3e8d31a2c7ef3532795))

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
