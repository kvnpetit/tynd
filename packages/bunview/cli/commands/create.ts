import path from "path";
import { existsSync } from "fs";
import { intro, outro, text, select, isCancel, cancel, spinner } from "@clack/prompts";
import { log, c } from "../utils/colors";

/**
 * `bunview create <name>` — scaffolds a fresh project interactively.
 * Supports: vanilla, react-ts (via `bun create vite`), vue-ts, svelte-ts.
 */
const VALID_TEMPLATES = ["vanilla", "react", "vue", "svelte", "solid"] as const;
type Template = typeof VALID_TEMPLATES[number];

function flagValue(args: string[], flag: string): string | undefined {
  const idx = args.indexOf(flag);
  if (idx >= 0 && args[idx + 1]) return args[idx + 1];
  const eq = args.find((a) => a.startsWith(`${flag}=`));
  return eq?.slice(flag.length + 1);
}

export async function runCreate(args: string[]) {
  const nonInteractive = args.includes("--yes") || args.includes("-y");
  const flagTemplate = flagValue(args, "--template") as Template | undefined;

  intro(c.bold("bunview create"));

  // Resolve name: positional arg or interactive prompt
  let name = args.find((a) => !a.startsWith("-"));
  if (!name && !nonInteractive) {
    const answer = await text({
      message: "Project name:",
      placeholder: "my-app",
      validate: (v) => (/^[\w.-]+$/.test(v) ? undefined : "Only letters, digits, _ - ."),
    });
    if (isCancel(answer)) { cancel("Cancelled."); return; }
    name = answer;
  }
  if (!name) {
    log.error("Missing project name. Usage: bunview create <name> [--template react] [--yes]");
    process.exit(1);
  }

  const dir = path.resolve(process.cwd(), name);
  if (existsSync(dir)) {
    log.error(`Directory already exists: ${dir}`);
    process.exit(1);
  }

  // Resolve template
  let template: Template;
  if (flagTemplate) {
    if (!VALID_TEMPLATES.includes(flagTemplate)) {
      log.error(`Unknown template: "${flagTemplate}". Use: ${VALID_TEMPLATES.join(", ")}`);
      process.exit(1);
    }
    template = flagTemplate;
  } else if (nonInteractive) {
    template = "vanilla";
  } else {
    const picked = await select({
      message: "Template:",
      options: [
        { value: "vanilla",  label: "Vanilla TypeScript", hint: "no framework, HTML + TS" },
        { value: "react",    label: "React + Vite",       hint: "TypeScript, HMR" },
        { value: "vue",      label: "Vue + Vite",         hint: "TypeScript, HMR" },
        { value: "svelte",   label: "Svelte + Vite",      hint: "TypeScript, HMR" },
        { value: "solid",    label: "Solid + Vite",       hint: "TypeScript, HMR" },
      ],
    });
    if (isCancel(picked)) { cancel("Cancelled."); return; }
    template = picked as Template;
  }

  if (template === "vanilla") {
    const s = spinner();
    s.start("Scaffolding vanilla project…");
    // Simply use `bunview init` flow on an empty directory
    const { mkdirSync } = await import("fs");
    mkdirSync(dir);
    await Bun.write(path.join(dir, "package.json"), JSON.stringify({
      name, private: true, type: "module",
    }, null, 2));
    const { runInit } = await import("./init");
    process.chdir(dir);
    await runInit([]);
    s.stop("Scaffolded");
  } else {
    // Delegate to `bun create vite` for framework templates
    const viteTemplate = `${template}-ts`;
    const s = spinner();
    s.start(`Running \`bun create vite ${name} --template ${viteTemplate}\`…`);
    const proc = Bun.spawn(["bun", "create", "vite", name, "--template", viteTemplate], {
      stdout: "ignore", stderr: "inherit",
    });
    const code = await proc.exited;
    if (code !== 0) {
      s.stop(c.red("Vite scaffold failed"));
      process.exit(1);
    }
    s.stop("Vite project created");

    process.chdir(dir);
    const { runInit } = await import("./init");
    await runInit([]);
  }

  outro(
    `${c.green("✓")} ${c.bold(name)} ready!\n` +
    `    ${c.dim("cd")} ${name}\n` +
    `    ${c.dim("bun run dev")}`,
  );
}
