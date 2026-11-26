#!/usr/bin/env bun
/**
 * API catalog and config schema generator.
 *
 * Sources of truth:
 *   - `[package.metadata.tynd.apis]` in crates/host/Cargo.toml, plus the `tynd add` names (ids
 *     and aliases) of plugins/catalog.json, for the API catalog;
 *   - the Rust config model (crates/cli/src/config/model.rs, `schemars::JsonSchema`) for the
 *     config schema; the schema's `ApiName` list comes from the catalog through it.
 * Emits every derived copy so nothing is maintained by hand:
 *   - crates/host/Cargo.toml                       `api-all` feature list
 *     (packages/host-src/native/Cargo.toml is produced from it by `sync:host-src`)
 *   - crates/cli/src/config/api_catalog.generated.rs   KNOWN_APIS / CORE_HOST_APIS /
 *                                                  PLUGIN_FIRST_APIS / API_PLUGIN_PACKAGES /
 *                                                  PLUGIN_ADD_NAMES
 *   - packages/cli/schemas/tynd.config.json and apps/docs/public/schemas/tynd.config.json,
 *     emitted by the model through `cargo test` (see crates/cli/src/config/schema.rs)
 *   - packages/cli/src/lib/config/types.generated.ts, the config types of `defineConfig`
 *
 * Usage:
 *   bun scripts/gen-api-catalog.ts           rewrite the generated files
 *   bun scripts/gen-api-catalog.ts --check   exit 1 when any file is stale
 */
import { existsSync, readdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import path from "node:path";

export interface ApiEntry {
  name: string;
  /** Compiled into every host unconditionally; a core API has no cargo feature. */
  core: boolean;
  /** Cargo feature `api-<name>` that gates a non-core API. */
  feature?: string;
  /** Host-backed plugin package that fronts a plugin-first API. */
  plugin?: string;
}

const LINE_WIDTH = 100;

function byName(left: string, right: string): number {
  return left < right ? -1 : left > right ? 1 : 0;
}

interface Manifest {
  package?: { metadata?: { tynd?: { apis?: Record<string, unknown> } } };
  features?: Record<string, unknown>;
}

function parseEntry(name: string, raw: unknown, features: Record<string, unknown>): ApiEntry {
  if (!/^[a-z][a-z0-9]*(-[a-z0-9]+)*$/.test(name)) {
    throw new Error(`catalog api "${name}" must be lowercase kebab-case`);
  }
  const value = (raw ?? {}) as Record<string, unknown>;
  for (const key of Object.keys(value)) {
    if (key !== "core" && key !== "plugin") {
      throw new Error(`catalog api "${name}" has unknown key "${key}"`);
    }
  }
  if (value.core !== undefined && typeof value.core !== "boolean") {
    throw new Error(`catalog api "${name}": core must be a boolean`);
  }
  const core = value.core === true;
  const plugin = typeof value.plugin === "string" ? value.plugin : undefined;
  if (value.plugin !== undefined && !plugin?.startsWith("@")) {
    throw new Error(`catalog api "${name}": plugin must be a scoped package name`);
  }
  if (core && plugin) {
    throw new Error(`catalog api "${name}" cannot be both core and plugin-first`);
  }
  const feature = gatingFeature(name, core, features);
  return { name, core, ...(feature ? { feature } : {}), ...(plugin ? { plugin } : {}) };
}

/** A core API is always compiled and must have no cargo feature; any other needs `api-<name>`. */
function gatingFeature(
  name: string,
  core: boolean,
  features: Record<string, unknown>,
): string | undefined {
  const feature = `api-${name}`;
  const exists = feature in features;
  if (core && exists) {
    throw new Error(`catalog api "${name}" is core and must not have cargo feature "${feature}"`);
  }
  if (!core && !exists) {
    throw new Error(`catalog api "${name}": cargo feature "${feature}" does not exist`);
  }
  return core ? undefined : feature;
}

/** Every `api-*` cargo feature (except the `api-all` meta list) must be catalogued. */
function checkFeatureCoverage(entries: ApiEntry[], features: Record<string, unknown>): void {
  const catalogued = new Set(entries.map((entry) => entry.feature));
  for (const feature of Object.keys(features)) {
    if (feature.startsWith("api-") && feature !== "api-all" && !catalogued.has(feature)) {
      throw new Error(`cargo feature "${feature}" has no entry in [package.metadata.tynd.apis]`);
    }
  }
}

/** Parse + validate the manifest table. Entries come back sorted by name. */
export function parseCatalog(cargoToml: string): ApiEntry[] {
  const manifest = Bun.TOML.parse(cargoToml) as Manifest;
  const table = manifest.package?.metadata?.tynd?.apis;
  if (!table || typeof table !== "object") {
    throw new Error("missing [package.metadata.tynd.apis] in crates/host/Cargo.toml");
  }
  const features = manifest.features ?? {};
  const entries = Object.entries(table).map(([name, raw]) => parseEntry(name, raw, features));
  entries.sort((left, right) => byName(left.name, right.name));
  checkFeatureCoverage(entries, features);
  return entries;
}

/** Cross-check plugin packages against plugins/host/*\/package.json. */
export function checkPlugins(
  entries: ApiEntry[],
  packages: { name: string; apis: string[] }[],
): string[] {
  const issues: string[] = [];
  for (const entry of entries) {
    const pkg = entry.plugin ? packages.find((item) => item.name === entry.plugin) : undefined;
    if (entry.plugin && !pkg) {
      issues.push(`api "${entry.name}": plugin ${entry.plugin} not found in plugins/host`);
    } else if (pkg && !pkg.apis.includes(entry.name)) {
      issues.push(`api "${entry.name}": ${pkg.name} does not declare tynd.apis "${entry.name}"`);
    }
  }
  for (const pkg of packages) {
    for (const api of pkg.apis) {
      const entry = entries.find((item) => item.name === api);
      if (entry?.plugin !== pkg.name) {
        issues.push(`${pkg.name} declares "${api}" but the catalog maps it to ${entry?.plugin}`);
      }
    }
  }
  return issues;
}

/** A name accepted by `tynd add` (catalog id or alias) and what it installs. */
export interface AddName {
  name: string;
  /** Plugin package to declare and install. */
  package: string;
  /** Host API to enable, for a host-backed plugin that fronts one. */
  api?: string;
  /** npm dist-tag the package is published under, when it is not `latest`. */
  distTag?: string;
}

interface CatalogPlugin {
  id?: unknown;
  kind?: unknown;
  pkg?: unknown;
  aliases?: unknown;
  apis?: unknown;
  distTag?: unknown;
}

const ADD_NAME = /^[a-z0-9]+(-[a-z0-9]+)*$/;
const DIST_TAG = /^[a-z][a-z0-9-]*$/;

/** Validate one catalog entry; returns its `tynd add` names and the api it fronts. */
function parseCatalogPlugin(
  plugin: CatalogPlugin,
  entries: ApiEntry[],
): { id: string; pkg: string; names: string[]; api?: string; distTag?: string } {
  const { id, pkg, aliases = [], distTag } = plugin;
  if (typeof id !== "string" || !ADD_NAME.test(id)) {
    throw new Error(`plugin catalog: invalid id ${JSON.stringify(id)}`);
  }
  if (typeof pkg !== "string" || !pkg.startsWith("@")) {
    throw new Error(`plugin catalog "${id}": pkg must be a scoped package name`);
  }
  if (!Array.isArray(aliases) || aliases.some((alias) => typeof alias !== "string")) {
    throw new Error(`plugin catalog "${id}": aliases must be an array of strings`);
  }
  const names = [id, ...(aliases as string[])];
  const invalid = names.find((name) => !ADD_NAME.test(name));
  if (invalid !== undefined) {
    throw new Error(`plugin catalog "${id}": alias "${invalid}" must be lowercase kebab-case`);
  }
  if (
    distTag !== undefined &&
    (typeof distTag !== "string" || !DIST_TAG.test(distTag) || distTag === "latest")
  ) {
    throw new Error(`plugin catalog "${id}": distTag must be an npm dist-tag other than "latest"`);
  }
  const tag = distTag === undefined ? {} : { distTag };
  if (plugin.kind !== "host") return { id, pkg, names, ...tag };
  const apis = plugin.apis;
  if (!Array.isArray(apis) || apis.length !== 1) {
    throw new Error(`plugin catalog "${id}": a host-backed plugin fronts exactly one api`);
  }
  const entry = entries.find((item) => item.name === apis[0]);
  if (entry?.plugin !== pkg) {
    throw new Error(`plugin catalog "${id}": api "${apis[0]}" is not fronted by ${pkg}`);
  }
  return { id, pkg, names, api: entry.name, ...tag };
}

/**
 * Parse the `tynd add` names of plugins/catalog.json. Fails on a malformed entry, on a name
 * claimed twice (id or alias), and on an alias that shadows a different host API.
 */
export function parseAddNames(catalogJson: string, entries: ApiEntry[]): AddName[] {
  const plugins = JSON.parse(catalogJson) as CatalogPlugin[];
  if (!Array.isArray(plugins)) throw new Error("plugins/catalog.json must be an array");
  const owners = new Map<string, string>();
  const output: AddName[] = [];
  for (const raw of plugins) {
    const { id, pkg, names, ...fields } = parseCatalogPlugin(raw, entries);
    for (const name of names) {
      const owner = owners.get(name);
      if (owner !== undefined) {
        throw new Error(`plugin catalog: "${name}" is claimed by both "${owner}" and "${id}"`);
      }
      if (name !== fields.api && entries.some((item) => item.name === name)) {
        throw new Error(`plugin catalog "${id}": "${name}" shadows the "${name}" host api`);
      }
      owners.set(name, id);
      output.push({ name, package: pkg, ...fields });
    }
  }
  return output.sort((left, right) => byName(left.name, right.name));
}

export function readAddNames(root: string, entries: ApiEntry[]): AddName[] {
  return parseAddNames(readFileSync(path.join(root, "plugins/catalog.json"), "utf8"), entries);
}

const names = (entries: ApiEntry[]) => entries.map((entry) => entry.name);

/** rustfmt keeps an array on one line up to this width (its default `array_width`). */
const RUST_ARRAY_WIDTH = 60;

/** Matches `cargo fmt`, so the generated block survives a format pass unchanged. */
function rustList(constName: string, items: string[]): string {
  const array = `[${items.map((item) => `"${item}"`).join(", ")}]`;
  const single = `pub(crate) const ${constName}: &[&str] = &${array};`;
  if (array.length <= RUST_ARRAY_WIDTH && single.length <= LINE_WIDTH) return single;
  return `pub(crate) const ${constName}: &[&str] = &[\n${items.map((item) => `    "${item}",`).join("\n")}\n];`;
}

export function renderRust(entries: ApiEntry[], addNames: AddName[] = []): string {
  const pluginFirst = entries.filter((entry) => entry.plugin);
  const addRows = addNames.map((item) => {
    const api = item.api ? `Some("${item.api}")` : "None";
    const fields = [`"${item.name}"`, `"${item.package}"`, api];
    // rustfmt wraps a tuple whose fields exceed its `fn_call_width` (60).
    if (fields.join(", ").length <= RUST_ARRAY_WIDTH) return `    (${fields.join(", ")}),`;
    return `    (\n${fields.map((field) => `        ${field},`).join("\n")}\n    ),`;
  });
  const distTags = [
    ...new Map(
      addNames.flatMap((item) => (item.distTag ? [[item.package, item.distTag] as const] : [])),
    ),
  ].sort(([left], [right]) => byName(left, right));
  return [
    "// @generated by scripts/gen-api-catalog.ts from [package.metadata.tynd.apis] in",
    "// crates/host/Cargo.toml and the ids and aliases of plugins/catalog.json. Do not edit by",
    "// hand; run `bun scripts/gen-api-catalog.ts`. Included by config/model.rs.",
    rustList("KNOWN_APIS", names(entries)),
    "",
    "/// APIs compiled into every host unconditionally; they map to no cargo feature.",
    rustList("CORE_HOST_APIS", names(entries.filter((entry) => entry.core))),
    "",
    "/// APIs fronted by a host-backed plugin package (`@tynd/plugin-*`). Only the",
    "/// tests use it, to check that `KNOWN_APIS` is exactly core plus plugin-first.",
    "#[cfg(test)]",
    rustList("PLUGIN_FIRST_APIS", names(pluginFirst)),
    "",
    "/// `(api, plugin package)` for every plugin-first API.",
    `pub(crate) const API_PLUGIN_PACKAGES: &[(&str, &str)] = &[\n${pluginFirst
      .map((entry) => `    ("${entry.name}", "${entry.plugin}"),`)
      .join("\n")}\n];`,
    "",
    "/// `(name, plugin package, host API)` for every id and alias of plugins/catalog.json, as",
    "/// accepted by `tynd add`. The API is set for host-backed plugins that front one.",
    addRows.length > 0
      ? `pub(crate) const PLUGIN_ADD_NAMES: &[(&str, &str, Option<&str>)] = &[\n${addRows.join("\n")}\n];`
      : "pub(crate) const PLUGIN_ADD_NAMES: &[(&str, &str, Option<&str>)] = &[];",
    "",
    "/// `(plugin package, npm dist-tag)` for the catalog plugins published under a tag other",
    "/// than `latest`; `tynd add` installs them as `<package>@<tag>`.",
    distTags.length > 0
      ? `pub(crate) const PLUGIN_DIST_TAGS: &[(&str, &str)] = &[\n${distTags
          .map(([pkg, tag]) => `    ("${pkg}", "${tag}"),`)
          .join("\n")}\n];`
      : "pub(crate) const PLUGIN_DIST_TAGS: &[(&str, &str)] = &[];",
    "",
  ].join("\n");
}

/** The subset of JSON Schema that the Rust config model emits. */
export interface SchemaNode {
  $ref?: string;
  const?: unknown;
  enum?: unknown[];
  anyOf?: SchemaNode[];
  oneOf?: SchemaNode[];
  type?: string;
  items?: SchemaNode;
  minItems?: number;
  properties?: Record<string, SchemaNode>;
  required?: string[];
  additionalProperties?: boolean | SchemaNode;
  description?: string;
  default?: unknown;
  $defs?: Record<string, SchemaNode>;
}

/** Keywords that only constrain values; they have no TypeScript counterpart. */
const VALIDATION_KEYWORDS = new Set([
  "$id",
  "$schema",
  "title",
  "description",
  "default",
  "format",
  "pattern",
  "minLength",
  "maxLength",
  "minimum",
  "maximum",
  "maxItems",
  "uniqueItems",
  "maxProperties",
  "propertyNames",
  "not",
]);
const SHAPE_KEYWORDS = new Set([
  "$ref",
  "const",
  "enum",
  "anyOf",
  "oneOf",
  "type",
  "items",
  "minItems",
  "properties",
  "required",
  "additionalProperties",
  "$defs",
]);

function checkKeywords(node: SchemaNode): void {
  for (const key of Object.keys(node)) {
    if (!VALIDATION_KEYWORDS.has(key) && !SHAPE_KEYWORDS.has(key)) {
      throw new Error(`config schema: unsupported keyword "${key}" in ${JSON.stringify(node)}`);
    }
  }
}

const refName = (ref: string) => {
  const match = /^#\/\$defs\/([A-Za-z_$][\w$]*)$/.exec(ref);
  if (!match?.[1]) throw new Error(`config schema: unsupported $ref "${ref}"`);
  return match[1];
};

const isUnion = (node: SchemaNode) =>
  (node.anyOf ?? node.oneOf ?? node.enum ?? []).length > 1 && node.$ref === undefined;

const propertyKey = (name: string) => (/^[A-Za-z_$][\w$]*$/.test(name) ? name : `"${name}"`);

/** Greedy word wrap of one paragraph. */
function wrap(paragraph: string, width: number): string[] {
  const lines: string[] = [];
  let line = "";
  for (const word of paragraph.split(/\s+/)) {
    if (line && line.length + 1 + word.length > width) {
      lines.push(line);
      line = word;
    } else {
      line = line ? `${line} ${word}` : word;
    }
  }
  return line ? [...lines, line] : lines;
}

/** JSDoc for a description (one paragraph per blank-line-separated block) and a default. */
function docComment(node: SchemaNode, indent: string): string[] {
  const width = LINE_WIDTH - indent.length - 3;
  const paragraphs = (node.description ?? "")
    .replaceAll("*/", "*\\/")
    .split("\n\n")
    .filter((paragraph) => paragraph.trim());
  const lines = paragraphs.flatMap((paragraph, index) => [
    ...(index > 0 ? [""] : []),
    ...wrap(paragraph, width),
  ]);
  if (["string", "number", "boolean"].includes(typeof node.default)) {
    lines.push(`@default ${JSON.stringify(node.default)}`);
  }
  if (lines.length === 0) return [];
  if (lines.length === 1) return [`${indent}/** ${lines[0]} */`];
  return [
    `${indent}/**`,
    ...lines.map((line) => (line ? `${indent} * ${line}` : `${indent} *`)),
    `${indent} */`,
  ];
}

function objectBody(node: SchemaNode, indent: string): string {
  const required = new Set(node.required ?? []);
  const members = Object.entries(node.properties ?? {}).flatMap(([name, property]) => [
    ...docComment(property, `${indent}  `),
    `${indent}  ${propertyKey(name)}${required.has(name) ? "" : "?"}: ${tsType(property, `${indent}  `)};`,
  ]);
  return members.length > 0 ? `{\n${members.join("\n")}\n${indent}}` : "{}";
}

function arrayType(node: SchemaNode, indent: string): string {
  if (!node.items) throw new Error("config schema: an array needs `items`");
  const item = tsType(node.items, indent);
  const element = isUnion(node.items) ? `(${item})` : item;
  return (node.minItems ?? 0) >= 1 ? `[${item}, ...${element}[]]` : `${element}[]`;
}

function objectType(node: SchemaNode, indent: string): string {
  if (node.properties) return objectBody(node, indent);
  if (typeof node.additionalProperties === "object") {
    return `Record<string, ${tsType(node.additionalProperties, indent)}>`;
  }
  return "Record<string, unknown>";
}

const PRIMITIVES: Record<string, string> = {
  string: "string",
  integer: "number",
  number: "number",
  boolean: "boolean",
};

/** TypeScript type of a schema node; validation-only keywords are ignored. */
export function tsType(node: SchemaNode, indent = ""): string {
  checkKeywords(node);
  if (node.$ref !== undefined) return refName(node.$ref);
  if ("const" in node) return JSON.stringify(node.const);
  if (node.enum) return node.enum.map((value) => JSON.stringify(value)).join(" | ");
  const union = node.anyOf ?? node.oneOf;
  if (union) return union.map((member) => tsType(member, indent)).join(" | ");
  if (node.type === "array") return arrayType(node, indent);
  if (node.type === "object") return objectType(node, indent);
  const primitive = node.type === undefined ? undefined : PRIMITIVES[node.type];
  if (primitive === undefined) {
    throw new Error(`config schema: unsupported node ${JSON.stringify(node)}`);
  }
  return primitive;
}

function declaration(name: string, node: SchemaNode): string {
  const body = node.properties
    ? `export interface ${name} ${tsType(node)}`
    : `export type ${name} = ${tsType(node)};`;
  return [...docComment(node, ""), body].join("\n");
}

/** `export` declarations for the root config (`TyndConfig`) and every `$defs` entry. */
export function renderConfigTypes(schema: SchemaNode): string {
  const { $defs: defs = {}, ...root } = schema;
  return [
    "// @generated by scripts/gen-api-catalog.ts from the Rust config model",
    "// (crates/cli/src/config/model.rs) through packages/cli/schemas/tynd.config.json.",
    "// Do not edit by hand; run `bun scripts/gen-api-catalog.ts`.",
    "",
    declaration("TyndConfig", root),
    ...Object.keys(defs)
      .sort(byName)
      .map((name) => `\n${declaration(name, defs[name] as SchemaNode)}`),
    "",
  ].join("\n");
}

/** The schema's `ApiName` list is the catalog, rendered through the Rust constants. */
export function checkSchemaApis(schema: SchemaNode, entries: ApiEntry[]): void {
  const listed = schema.$defs?.ApiName?.enum;
  if (JSON.stringify(listed) !== JSON.stringify(names(entries))) {
    throw new Error(
      "config schema ApiName list differs from the catalog; run `bun scripts/gen-api-catalog.ts`",
    );
  }
}

function cargoList(name: string, features: string[]): string {
  return `${name} = [\n${features.map((feature) => `    "${feature}",`).join("\n")}\n]`;
}

/** Rewrite the `api-all` array (every feature-gated API), leaving the rest of the file untouched. */
export function spliceCargo(source: string, entries: ApiEntry[]): string {
  const features = entries.flatMap((entry) => (entry.feature ? [entry.feature] : []));
  const pattern = /^api-all = \[[^\]]*\]/m;
  if (!pattern.test(source)) throw new Error("Cargo.toml has no `api-all = [...]` list");
  return source.replace(pattern, () => cargoList("api-all", features));
}

export interface Target {
  file: string;
  render: (current: string) => string;
}

export function readHostPlugins(root: string): { name: string; apis: string[] }[] {
  const directory = path.join(root, "plugins", "host");
  if (!existsSync(directory)) return [];
  const packages: { name: string; apis: string[] }[] = [];
  for (const entry of readdirSync(directory, { withFileTypes: true })) {
    const manifest = path.join(directory, entry.name, "package.json");
    if (!entry.isDirectory() || !existsSync(manifest)) continue;
    const json = JSON.parse(readFileSync(manifest, "utf8")) as {
      name: string;
      tynd?: { apis?: string[] };
    };
    packages.push({ name: json.name, apis: json.tynd?.apis ?? [] });
  }
  return packages.sort((left, right) => byName(left.name, right.name));
}

export const SCHEMA_FILE = "packages/cli/schemas/tynd.config.json";
const SCHEMA_TEST = "config::schema::tests::published_schema_matches_the_model";

/** Format with the repository's Biome settings, so `biome ci` accepts the generated file. */
export function biomeFormat(root: string, file: string, source: string): string {
  const result = Bun.spawnSync(
    [process.execPath, "x", "biome", "format", `--stdin-file-path=${file}`],
    { cwd: root, stdin: Buffer.from(source), stdout: "pipe", stderr: "pipe" },
  );
  if (result.exitCode !== 0) {
    throw new Error(`biome format ${file} failed:\n${result.stderr.toString()}`);
  }
  return result.stdout.toString();
}

/** The schema the Rust config model emits (a `cargo test` with `TYND_CONFIG_SCHEMA_OUT`). */
export function modelSchema(root: string): SchemaNode {
  const out = path.join(root, "target", "tynd.config.schema.json");
  rmSync(out, { force: true });
  const result = Bun.spawnSync(
    [
      "cargo",
      "test",
      "--quiet",
      "--locked",
      "-p",
      "tynd-cli",
      "--lib",
      SCHEMA_TEST,
      "--",
      "--exact",
    ],
    {
      cwd: root,
      env: { ...process.env, TYND_CONFIG_SCHEMA_OUT: out },
      stdout: "pipe",
      stderr: "pipe",
    },
  );
  if (result.exitCode !== 0 || !existsSync(out)) {
    throw new Error(
      `cargo could not emit the config schema:\n${result.stderr.toString()}${result.stdout.toString()}`,
    );
  }
  const schema = JSON.parse(readFileSync(out, "utf8")) as SchemaNode;
  rmSync(out, { force: true });
  return schema;
}

export const readPublishedSchema = (root: string) =>
  JSON.parse(readFileSync(path.join(root, SCHEMA_FILE), "utf8")) as SchemaNode;

/** Files derived from the catalog alone. */
export function catalogTargets(entries: ApiEntry[], addNames: AddName[]): Target[] {
  return [
    {
      file: "crates/host/Cargo.toml",
      render: (current) => spliceCargo(current, entries),
    },
    {
      file: "crates/cli/src/config/api_catalog.generated.rs",
      render: () => renderRust(entries, addNames),
    },
  ];
}

/** Files derived from the config schema the Rust model emits. */
export function schemaTargets(root: string, schema: SchemaNode, entries: ApiEntry[]): Target[] {
  checkSchemaApis(schema, entries);
  const json = () => biomeFormat(root, SCHEMA_FILE, `${JSON.stringify(schema, null, 2)}\n`);
  const types = "packages/cli/src/lib/config/types.generated.ts";
  return [
    { file: SCHEMA_FILE, render: json },
    // The docs copy is a byte-identical publication of the CLI schema.
    { file: "apps/docs/public/schemas/tynd.config.json", render: json },
    { file: types, render: () => biomeFormat(root, types, renderConfigTypes(schema)) },
  ];
}

/**
 * Every generated file. Without an explicit schema, the published one stands in for the
 * model's (`cargo test -p tynd-cli` checks that they agree).
 */
export function targets(
  root: string,
  entries: ApiEntry[],
  addNames: AddName[] = readAddNames(root, entries),
  schema: SchemaNode = readPublishedSchema(root),
): Target[] {
  return [...catalogTargets(entries, addNames), ...schemaTargets(root, schema, entries)];
}

const out = (message: string) => process.stdout.write(`${message}\n`);
const fail = (message: string): never => {
  process.stderr.write(`${message}\n`);
  return process.exit(1);
};

/** Render each target; returns the stale ones, rewritten unless `check`. */
function update(root: string, list: Target[], check: boolean): string[] {
  const stale: string[] = [];
  for (const target of list) {
    const file = path.join(root, target.file);
    const current = existsSync(file) ? readFileSync(file, "utf8") : "";
    const next = target.render(current);
    if (next === current) continue;
    stale.push(target.file);
    if (!check) writeFileSync(file, next);
  }
  return stale;
}

function main(): void {
  const root = path.resolve(import.meta.dir, "..");
  const check = Bun.argv.includes("--check");
  const entries = parseCatalog(readFileSync(path.join(root, "crates/host/Cargo.toml"), "utf8"));
  const pluginIssues = checkPlugins(entries, readHostPlugins(root));
  if (pluginIssues.length > 0) {
    fail(pluginIssues.map((issue) => `api catalog: ${issue}`).join("\n"));
  }

  let addNames: AddName[] = [];
  try {
    addNames = readAddNames(root, entries);
  } catch (error) {
    fail(`api catalog: ${(error as Error).message}`);
  }

  const stale = update(root, catalogTargets(entries, addNames), check);
  // The schema is emitted by the Rust model, which includes the catalog constants: they
  // must be current first.
  if (!check || stale.length === 0) {
    try {
      stale.push(...update(root, schemaTargets(root, modelSchema(root), entries), check));
    } catch (error) {
      fail(`config schema: ${(error as Error).message}`);
    }
  }

  if (check && stale.length > 0) {
    fail(
      `api catalog is stale; run \`bun scripts/gen-api-catalog.ts\`:\n${stale.map((file) => `  ${file}`).join("\n")}`,
    );
  }
  out(
    stale.length > 0
      ? `api catalog: updated ${stale.join(", ")}`
      : `api catalog and config schema are up to date (${entries.length} APIs)`,
  );
}

if (import.meta.main) main();
