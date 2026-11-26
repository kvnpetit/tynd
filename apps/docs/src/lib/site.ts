const REPO_OWNER = "kvnpetit";
const REPO_NAME = "tynd";
const REPO = `https://github.com/${REPO_OWNER}/${REPO_NAME}`;

/** GitHub Pages project site: https://<owner>.github.io/<repo>. */
export const SITE_ORIGIN = `https://${REPO_OWNER}.github.io`;
export const BASE = `/${REPO_NAME}`;
const URL = `${SITE_ORIGIN}${BASE}`;

export const SITE = {
  name: "Tynd",
  title: "Tynd — Desktop apps in TypeScript",
  description: "Desktop apps in TypeScript. Small native binaries, zero-codegen typed RPC.",
  tagline: "Desktop apps, in TypeScript",
  taglineHighlight: "in TypeScript",
  subtagline:
    "Small native binaries, zero-codegen typed RPC, no bridge language to learn. Write your whole app in TypeScript.",
  url: URL,
  locale: "en",
  ogLocale: "en_US",
  keywords: [
    "desktop app",
    "typescript",
    "bun",
    "rust",
    "webview",
    "electron alternative",
    "tauri alternative",
    "wails alternative",
    "wry",
    "tao",
    "typed rpc",
    "native binary",
    "cross-platform",
  ],
  category: "developer tools",
  applicationCategory: "DeveloperApplication",
  operatingSystem: "Windows, macOS, Linux",
  price: { amount: "0", currency: "USD" },
  author: {
    name: "Kevin Petit",
    url: "https://github.com/kvnpetit",
  },
  license: "Apache-2.0",
  install: "bunx @tynd/cli create my-app",
  links: {
    github: REPO,
    releases: `${REPO}/releases`,
    issues: `${REPO}/issues`,
    discussions: `${REPO}/discussions`,
    npm: "https://www.npmjs.com/package/@tynd/cli",
  },
  social: {
    twitter: "",
    bluesky: "",
    discord: "",
    mastodon: "",
  },
  icons: {
    icon: "/favicon.ico",
  },
  screenshot: "/opengraph-image.svg",
} as const;

/** Social profile URLs for JSON-LD `sameAs`. Filters out empty handles. */
export function sameAs(): string[] {
  const out: string[] = [SITE.links.github];
  const s: Record<keyof typeof SITE.social, string> = SITE.social;
  if (s.twitter) out.push(`https://twitter.com/${s.twitter.replace(/^@/, "")}`);
  if (s.bluesky) out.push(`https://bsky.app/profile/${s.bluesky}`);
  if (s.mastodon) out.push(s.mastodon);
  if (s.discord) out.push(s.discord);
  return out;
}

/** Prefix a root-relative path with the site base (`/docs/` -> `/tynd/docs/`). */
export function withBase(path: string): string {
  if (!path.startsWith("/")) return path;
  return `${BASE}${path}`;
}
