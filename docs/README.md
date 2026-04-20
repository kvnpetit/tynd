# Tynd docs site

Next.js 16 + Nextra 4 + Tailwind v4 + Pagefind. Static export.

## Structure

```
docs/
├── src/
│   ├── app/
│   │   ├── layout.tsx              # root shell, Head + metadata from SITE
│   │   ├── page.tsx                # landing (Tailwind v4)
│   │   ├── opengraph-image.tsx     # /opengraph-image (PNG generated at build)
│   │   ├── globals.css             # Tailwind v4 layered (preflight + theme + utilities)
│   │   ├── sitemap.ts              # /sitemap.xml (auto-collects routes)
│   │   ├── robots.ts               # /robots.txt
│   │   ├── not-found.tsx           # 404 page
│   │   ├── docs/
│   │   │   ├── layout.tsx          # Nextra Layout + navbar with VersionPicker
│   │   │   └── [[...mdxPath]]/page.tsx  # catch-all MDX renderer + per-page
│   │   │                           #   canonical/noindex + TechArticle/Breadcrumb JSON-LD
│   │   ├── showcase/page.tsx       # /showcase (apps built with the framework)
│   │   ├── blog/page.tsx           # /blog (list)
│   │   ├── changelog/page.tsx      # /changelog (link to GH releases)
│   │   ├── community/page.tsx      # /community (GH discussions + issues cards)
│   │   └── compare/page.tsx        # /compare (vs Electron/Tauri/Wails)
│   ├── content/
│   │   ├── _meta.ts                # top-level: versions + future top pages
│   │   ├── v0.1/                   # docs content for v0.1
│   │   │   ├── _meta.ts            # sidebar order
│   │   │   └── *.mdx
│   │   ├── blog/                   # blog MDX entries (future)
│   │   └── showcase/               # showcase entries (future — JSON or MDX)
│   ├── components/
│   │   ├── ui/                     # marketing primitives: Button, Container, SiteHeader,
│   │   │                           #   SiteFooter, SectionTitle, FeatureCard, Page
│   │   ├── mdx/                    # custom MDX components (register via mdx-components.tsx)
│   │   └── VersionPicker.tsx       # navbar dropdown (client)
│   ├── lib/
│   │   ├── site.ts                 # SSOT — brand, copy, URL, links, icons, author, ...
│   │   ├── nav.ts                  # SSOT — TOP_NAV + FOOTER_LINKS (reads from site.ts)
│   │   └── versions.ts             # SSOT — version list + LATEST
│   └── mdx-components.tsx          # Nextra components (Callout, Steps, Tabs, Cards, ...)
│                                   # pre-registered so they work in any .mdx without import
├── public/
│   ├── _headers                    # security + cache headers (Workers Static Assets)
│   └── _redirects                  # 301s (/docs -> latest) — file-based, no Worker code
├── scripts/
│   └── new-version.ts              # bun run new-version vX.Y
├── next.config.ts                  # withNextra + output:'export' + search + basePath env
├── biome.json
└── package.json                    # scripts: dev, build (+ postbuild pagefind), new-version
```

## Routes

| URL | Source | Purpose |
|---|---|---|
| `/` | `src/app/page.tsx` | Landing |
| `/docs/latest` | `public/_redirects` (CDN 301 at edge) | Redirects to LATEST_SLUG — no Next.js page, so CF / Netlify `_redirects` fires |
| `/docs/v0.1` | `src/content/v0.1/index.mdx` | Versioned docs intro |
| `/docs/v0.1/<page>` | `src/content/v0.1/<page>.mdx` | Versioned doc page |
| `/showcase` | `src/app/showcase/page.tsx` | Apps built with Tynd |
| `/blog` | `src/app/blog/page.tsx` | Blog index |
| `/changelog` | `src/app/changelog/page.tsx` | Release history |
| `/community` | `src/app/community/page.tsx` | Community hub |
| `/compare` | `src/app/compare/page.tsx` | Compare vs alternatives |
| `/opengraph-image` | `src/app/opengraph-image.tsx` | OG + Twitter card PNG |
| `/sitemap.xml` | `src/app/sitemap.ts` | Full sitemap |
| `/robots.txt` | `src/app/robots.ts` | Crawl policy |
| `/_pagefind/*` | Pagefind postbuild | Search index |
| `/404` | `src/app/not-found.tsx` | Not found |

## Search

Nextra navbar ships a search input (keyboard: Cmd+K on macOS, Ctrl+K elsewhere). Index is built by Pagefind in a `postbuild` script — no extra action needed, `bun run build` handles it.

Search is scoped to `/docs/*`. Other pages (landing, blog, showcase) are not indexed.

## Dev

```bash
bun --cwd docs run dev   # http://localhost:3000
```

Search won't return results in dev (Pagefind only runs postbuild). Everything else works normally.

## Build

```bash
bun --cwd docs run build
# produces docs/out/ (deployable as static site)
```

`build` chains into `postbuild` which runs Pagefind against `out/` and writes the index to `out/_pagefind/`.

## Rebrand / rename / relink

Everything brand-related lives in three files:

- `src/lib/site.ts` — name, title, description, URL, author, links, icons, locale, license, **keywords**, **category**, **themeColor**, **verification** (google/bing/yandex), **social** (twitter/bluesky/discord/mastodon — wire through `sameAs()` JSON-LD helper), **robots** (index/follow), **price** / **applicationCategory** / **operatingSystem** / **screenshot** (schema.org SoftwareApplication), **contactEmail**
- `src/lib/nav.ts` — TOP_NAV (navbar) + FOOTER_LINKS (footer columns)
- `src/lib/versions.ts` — docs version list

Change a value in one of these, every consumer updates (landing, navbar, footer, metadata, OG image, sitemap, structured data, docs layout, ...). No grep-and-replace needed.

## SEO

Wired centrally; edit `src/lib/site.ts` to change, no per-page work required.

- **Canonical URL** — `src/app/docs/[[...mdxPath]]/page.tsx` `generateMetadata` emits `alternates.canonical` that points to the **latest-version** URL of the same doc slug. Outdated versions get `robots: { index: false, follow: true }` so duplicate content is not indexed.
- **MDX frontmatter** — every `.mdx` should declare `description: "..."` in frontmatter. Absent, a fallback `"${title} — ${SITE.description}"` is generated; unique per-page descriptions still rank better.
- **JSON-LD** — two scripts injected per doc page: `TechArticle` (headline/description/author/publisher/license) and `BreadcrumbList` (built from mdxPath segments, version stripped, slugs title-cased). Landing injects `SoftwareApplication` with `offers`, `operatingSystem`, `applicationCategory`, `sameAs`, `screenshot`, `author`.
- **Sitemap** — `src/app/sitemap.ts` walks the Nextra pageMap, dedupes entries via `Set`, and populates `lastModified` from each MDX file's mtime.
- **Robots / verification** — `layout.tsx` emits Google / Bing (`msvalidate.01`) / Yandex verification tags from `SITE.verification`, plus per-bot `max-image-preview`, `max-snippet`, `max-video-preview`.
- **Theme color + color scheme** — `viewport` export sets `themeColor` per `prefers-color-scheme` media query.
- **OG image** — single root OG (`/opengraph-image`) is used for all routes. Per-doc OG is **not** wired: Next.js rejects `opengraph-image.tsx` inside an optional catch-all (`[[...mdxPath]]`). Rich-result coverage is instead handled via JSON-LD.
- **Legacy `/docs/latest`** — no Next.js page; `public/_redirects` emits HTTP 301 at the edge (served by the CF Static Assets engine, same syntax as CF Pages). Bump the target rule whenever `LATEST_SLUG` changes.

## Adding a new docs version

```bash
bun run new-version v0.2
```

Then:
1. Edit `src/lib/versions.ts` — add the entry, flip `status: "latest"`.
2. Update `public/_redirects` — swap `/docs/v0.1` for the new slug on the 4 rules.
3. Review the copied MDX files for breaking changes.
4. Commit.

Old URLs (`/docs/v0.1/...`) keep working forever — only `/docs/latest/*` follows `LATEST_SLUG`.

## Adding a new docs page (MDX)

1. Create `src/content/<version>/<slug>.mdx` with frontmatter:

   ```mdx
   ---
   title: Page title
   description: One-sentence page summary (used for <meta description>, OG, Twitter, and JSON-LD).
   ---

   # Page title

   ...
   ```

2. Add `<slug>` to the version's `_meta.ts` to place it in the sidebar.
3. Sitemap picks it up automatically; JSON-LD + canonical are emitted by the catch-all.

Skipping `description` falls back to a generic `${title} — ${SITE.description}` — ship unique descriptions for best SEO.

## Adding a top-level page (e.g. pricing)

1. Create `src/app/pricing/page.tsx` using `<Page>` from `src/components/ui`.
2. Add `{ label: "Pricing", href: "/pricing" }` to `TOP_NAV` in `src/lib/nav.ts`.
3. Add the route to `STATIC_ROUTES` in `src/app/sitemap.ts` (use trailing slash to match `trailingSlash: true`).

## Deploy — Cloudflare Workers with Static Assets (recommended)

This project targets **Workers with Static Assets**, not Pages. Pages is in maintenance mode; Workers is Cloudflare's recommended static-hosting path (see [Cloudflare docs: Static Assets](https://developers.cloudflare.com/workers/static-assets/)). Same underlying network, but programmable — `/docs/latest/*` redirects and any future edge logic live in `worker/index.ts` and compile together with the static bundle.

### Config files

- `wrangler.jsonc` — `assets` binding pointing at `out/` (no Worker code; this is "Workers Static Assets" mode — pure static hosting on the Workers runtime)
- `public/_headers` — security + cache headers, copied into `out/`, applied by the Static Assets engine (same syntax as CF Pages)
- `public/_redirects` — 301 redirects (`/docs/latest/*` → `/docs/<latest>/*`), same syntax as CF Pages

### First-time setup

```bash
bun install
bun --cwd docs run build
bunx wrangler login                      # one-time OAuth
bun --cwd docs run deploy
```

First `deploy` prompts for a workers.dev subdomain; after that the site is live at `https://tynd-docs.<subdomain>.workers.dev`.

### Daily loop

```bash
bun --cwd docs run build        # Next static export -> docs/out/
bun --cwd docs run preview      # wrangler dev — local Worker + assets at http://localhost:8787
bun --cwd docs run deploy       # wrangler deploy — push to production
```

### Custom domain

Dashboard → **Workers & Pages** → `tynd-docs` → **Settings** → **Domains & Routes** → **Add** → enter `tynd.dev` (or `docs.tynd.dev`). CF provisions the cert automatically.

After adding the domain, update `NEXT_PUBLIC_BASE_URL` (build-time env) to the new origin so sitemap / robots / OG / JSON-LD match, then redeploy.

### Auto-deploy (GitHub)

Two options, pick one:

- **Workers Builds** (Cloudflare-native, recommended) — Dashboard → Worker → **Settings** → **Build** → connect GitHub → build command `bun install && bun --cwd docs run build`, root directory `docs`. Every push deploys; PRs get preview URLs.
- **GitHub Action** — add a workflow that runs `wrangler deploy` with a `CLOUDFLARE_API_TOKEN` secret. Not set up by default in this repo.

### What Workers + Static Assets gives you

- Programmable edge routing (Worker code has first shot at every request)
- `_headers` / `_redirects` file syntax still supported by the assets engine (but we moved redirects into `worker/index.ts` for SSOT with `versions.ts`)
- Preview versions (`wrangler versions upload`) — gated URLs, no prod impact
- Edge CDN, global POPs, auto HTTPS, Web Analytics, zero cold starts for static requests
- Free tier covers 100k requests/day on workers.dev; custom domain lifts this when pointed to a paid plan

## Deploy — other hosts

### GitHub Pages

Works but slower TTFB, no custom headers.

```bash
NEXT_PUBLIC_BASE_PATH=/tynd NEXT_PUBLIC_BASE_URL=https://kvnpetit.github.io/tynd bun --cwd docs run build
# push docs/out to gh-pages branch via GH Action
```

### Vercel

`vercel` CLI or connect repo. Zero config.

## Custom MDX components

1. Create in `src/components/mdx/` (e.g. `VersionBadge.tsx`).
2. Export from `src/components/mdx/index.tsx`.
3. Register in `src/mdx-components.tsx`:
   ```tsx
   import * as customs from "./components/mdx";
   return { ...docsComponents, ...nextraComponents, ...customs, ...components };
   ```

Usable in any `.mdx` without per-file import.

Pre-registered Nextra MDX components (no import needed):

- `Callout` - info/warning/tip boxes
- `Steps` - numbered step container
- `Tabs` + `Tabs.Tab` - tabbed content
- `Cards` + `Cards.Card` - navigation card grid
- `FileTree` - directory tree
- `Bleed` - full-width content

## Brand / theme

- Accent color = cyan (`cyan-600` / `cyan-400`). Set in `src/lib/site.ts` indirectly via Tailwind classes.
- Override Nextra palette in `globals.css` with `@layer nextra { :root { --nextra-primary-hue: ... } }`.
- Logo: drop `public/logo.svg` and reference in `SiteHeader` + Nextra `Navbar.logo`.
- Favicon: put `favicon.ico` / `icon.svg` at `src/app/` (Next auto-detects).
- OG image copy and colors: `src/app/opengraph-image.tsx` (reads `SITE.name` / `SITE.tagline`).
