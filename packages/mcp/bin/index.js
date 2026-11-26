#!/usr/bin/env bun

import { runToolingBinary } from "@tynd/tooling";

process.exitCode = await runToolingBinary("tynd-mcp");
