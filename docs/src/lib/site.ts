// Single source of truth for site-wide identity, copy, and external links.
// Changing values here cascades to <meta>, OG image, landing, navbar, footer,
// sitemap, robots, and structured data — no other file needs editing for a rename.

const REPO_OWNER = "kvnpetit";
const REPO_NAME = "tynd";
const REPO = `https://github.com/${REPO_OWNER}/${REPO_NAME}`;
const URL = process.env.NEXT_PUBLIC_BASE_URL ?? "https://tynd.dev";

export const SITE = {
  name: "Tynd",
  title: "Tynd — Desktop apps in TypeScript",
  titleTemplate: "%s — Tynd",
  description:
    "Desktop apps in TypeScript. Small native binaries, zero-codegen typed RPC.",
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
    "quickjs",
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
  contactEmail: "contact@kvnpetit.com",
  license: "Apache-2.0",
  install: "bun add -g @tynd/cli",
  repo: {
    owner: REPO_OWNER,
    name: REPO_NAME,
  },
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
  verification: {
    google: "",
    bing: "",
    yandex: "",
  },
  robots: {
    index: true,
    follow: true,
  },
  themeColor: {
    light: "#ffffff",
    dark: "#0a0a0a",
  },
  icons: {
    icon: "/favicon.ico",
    shortcut: "/favicon.ico",
  },
  screenshot: "/opengraph-image",
} as const;

export type SiteConfig = typeof SITE;

/**
 * Social profile URLs for JSON-LD `sameAs`. Filters out empty handles so only
 * configured socials end up in structured data.
 */
export function sameAs(): string[] {
  const out: string[] = [SITE.links.github];
  const s: Record<keyof typeof SITE.social, string> = SITE.social;
  if (s.twitter) out.push(`https://twitter.com/${s.twitter.replace(/^@/, "")}`);
  if (s.bluesky) out.push(`https://bsky.app/profile/${s.bluesky}`);
  if (s.mastodon) out.push(s.mastodon);
  if (s.discord) out.push(s.discord);
  return out;
}
