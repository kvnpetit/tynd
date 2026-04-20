import { SITE } from "../../lib/site";

// llms.txt + llms-full.txt are static files in public/docs/v0.1/, not MDX
// pages. Nextra's sidebar uses Next's <Link> for relative hrefs, which tries
// client-side routing and drops the .txt extension. Using an absolute URL
// forces Anchor to render a plain <a target="_blank">, so the browser does a
// hard navigation and serves the file as-is. `LlmsLinksFix` overrides the
// origin at runtime so links still work on localhost and preview deploys.

export default {
  index: "Introduction",
  "getting-started": "Getting Started",
  concepts: "Core Concepts",
  guides: "Guides",
  tutorials: "Tutorials",
  recipes: "Recipes",
  api: "API Reference",
  cli: "CLI Reference",
  runtimes: "Runtimes",
  compare: "Comparisons",
  "production-checklist": "Production Checklist",
  faq: "FAQ",
  glossary: "Glossary",
  troubleshooting: "Troubleshooting",
  "-llms-sep": { type: "separator", title: "For AI Assistants" },
  "llms-txt": {
    title: "llms.txt",
    href: `${SITE.url}/docs/v0.1/llms.txt`,
  },
  "llms-full-txt": {
    title: "llms-full.txt",
    href: `${SITE.url}/docs/v0.1/llms-full.txt`,
  },
};
