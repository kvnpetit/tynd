# Tynd Docs

[![License](https://img.shields.io/badge/license-Apache%202.0-blue.svg)](../../LICENSE)
[![Deploy](https://github.com/kvnpetit/tynd/actions/workflows/docs-deploy.yml/badge.svg)](https://github.com/kvnpetit/tynd/actions/workflows/docs-deploy.yml)
[![Site](https://img.shields.io/website?url=https%3A%2F%2Fkvnpetit.github.io%2Ftynd%2F&label=kvnpetit.github.io%2Ftynd)](https://kvnpetit.github.io/tynd/)

Astro + Starlight + Pagefind, published to GitHub Pages at <https://kvnpetit.github.io/tynd/>.

## Structure

```text
apps/docs/
|-- astro.config.ts              # site/base, Starlight, sidebar, redirects, sitemap
|-- src/
|   |-- content.config.ts        # docs collection (keeps `v1.0` ids verbatim)
|   |-- content/docs/docs/       # MDX docs, under the `v1.0` URL segment  
|   |   `-- v1.0/**/meta.json    # sidebar order and folder titles
|   |-- pages/                   # landing, changelog, 404, robots.txt, og image
|   |-- layouts/Base.astro       # shell of the non-docs pages
|   |-- components/              # Steps, DocCard                        
|   |   `-- site/                # landing page UI (header, footer, buttons...)
|   |-- lib/                     # site constants, sidebar, redirects
|   `-- styles/                  # starlight.css (docs theme), site.css (landing)
|-- public/                      # favicon, schemas, llms*.txt (generated)
|-- scripts/
|   |-- build-llms-txt.ts        # generate the LLM-friendly docs export
|   `-- check-links.ts           # post-build internal link checker
`-- package.json
```

## Routes

The site is served under the base path `/tynd` (GitHub Pages project site).

| URL                                     | Source                                | Purpose                                 |
| --------------------------------------- | ------------------------------------- | --------------------------------------- |
| `/`                                     | `src/pages/index.astro`               | Landing page                            |
| `/docs`, `/docs/latest`, `/docs/<page>` | `src/lib/redirects.ts`                | Static redirects to the latest version  |
| `/docs/v1.0/<page>`                     | `src/content/docs/docs/v1.0/**/*.mdx` | Docs page                               |
| `/changelog`                            | `src/pages/changelog.astro`           | Release history                         |
| `/compare`                              | `src/lib/redirects.ts`                | Redirects to the comparisons            |
| `/404.html`                             | `src/pages/404.astro`                 | Not-found page (served by GitHub Pages) |
| `/opengraph-image.svg`                  | `src/pages/opengraph-image.svg.ts`    | SVG social preview                      |
| `/sitemap-index.xml`                    | `@astrojs/sitemap`                    | Sitemap                                 |
| `/pagefind/`                            | Starlight                             | Search index                            |

## Development

```bash
bun install
bun run dev        # http://localhost:4321/tynd/
```

## Build

```bash
bun run build        # llms.txt, static site in ./dist, Pagefind index
bun run preview      # serve ./dist
bun run typecheck    # astro check + scripts
bun run check:links  # after a build: verify every internal link in ./dist
```

`build` runs `prebuild` (regenerates the `llms*.txt` files in `public/`) and then `astro build`, which also builds the Pagefind search index.

## Content

Docs live in `src/content/docs/docs/<version>`. Each folder can define a `meta.json` that sets the sidebar order and title:

```json
{
  "title": "API Reference",
  "pages": ["index", "backend", "os", "plugins"]
}
```

`---Label---` entries start a sub-group and `[text](url)` entries add a link. Pages not listed in `meta.json` are not shown in the sidebar.

Create pages as `.mdx` files with frontmatter:

```mdx
---
title: Page title
description: One-sentence page summary.
---

# Page title
```

Write internal links as root-relative paths (`/docs/v1.0/guides/...`); the base path is added at build time. Components are imported explicitly in each page:

```mdx
import { Aside, CardGrid, Tabs, TabItem, FileTree } from '@astrojs/starlight/components';
import DocCard from '@/components/DocCard.astro';
import Steps from '@/components/Steps.astro';
```

- `Aside` (`note`, `tip`, `caution`, `danger`), `Tabs` / `TabItem`, `FileTree`: Starlight built-ins.
- `CardGrid` + `DocCard title href`: grid of linked cards with a markdown body.
- `Steps`: numbers every `###` heading inside it.
- Code blocks use Expressive Code: ` ```ts title="src/main.ts" `.

## Versions

Only `v1.0` is published. The `v1.0` segment stays in the URLs (links and `$schema` URLs depend on it); `src/lib/versions.ts` holds it.

## SEO

Site-level metadata lives in `src/lib/site.ts`.

- The sitemap lists the landing page, the changelog and the docs pages.
- The landing page emits `SoftwareApplication` JSON-LD.

## GitHub Pages

`.github/workflows/docs-deploy.yml` builds the site and publishes `apps/docs/dist` with the official Pages actions on every push to `main` that touches `apps/docs/**`.

One-time setup: **Settings -> Pages -> Build and deployment -> Source: GitHub Actions**.

The URL and base path come from `SITE_ORIGIN` / `BASE` in `src/lib/site.ts`. GitHub Pages has no custom headers or server redirects, so redirects are generated as static meta-refresh pages (`src/lib/redirects.ts`). `robots.txt` is only honored by crawlers at a domain root, so it has no effect on a project site.

## License

Apache-2.0. See [LICENSE](../../LICENSE).
