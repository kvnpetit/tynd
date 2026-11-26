# @tynd/tooling

Native developer-tool distribution shared by `@tynd/cli` and `@tynd/mcp`.

`@tynd/tooling` resolves one package for the current OS and CPU. Every platform
package contains:

- `tynd` — project CLI;
- `tynd-mcp` — Model Context Protocol server;
- `tynd-bundler` — native application and installer bundler.

| Platform | Architectures |
| --- | --- |
| macOS | x64, ARM64 |
| Linux | x64, ARM64 |
| Windows | x64, ARM64 |

The native files are supplied by npm `optionalDependencies`:

```text
@tynd/tooling
  └─ @tynd/tooling-<darwin|linux|win32>-<x64|arm64>
       └─ bin/
            ├─ tynd[.exe]
            ├─ tynd-mcp[.exe]
            └─ tynd-bundler[.exe]
```

`@tynd/cli` and `@tynd/mcp` contain small launchers. They resolve the local
native executable, forward the original arguments and stdio, then preserve its
exit status.

## Resolver API

```js
import { resolveToolingBinary, runToolingBinary } from "@tynd/tooling";

const bundlerPath = resolveToolingBinary("tynd-bundler");
const exitCode = await runToolingBinary("tynd", ["--help"]);
```

Supported resolver names are `tynd`, `tynd-mcp`, and `tynd-bundler`.

## Bun's Role

The CLI and MCP implementations are Rust. Bun remains the runtime for Tynd
application backend code, frontend tooling, package scripts, and packaged user
applications.
