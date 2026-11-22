import { unified } from "@astrojs/markdown-remark";
import sitemap from "@astrojs/sitemap";
import starlight from "@astrojs/starlight";
import { defineConfig } from "astro/config";
import { buildRedirects } from "./src/lib/redirects";
import rehypeBaseLinks from "./src/lib/rehype-base-links";
import rehypeDropH1 from "./src/lib/rehype-drop-h1";
import { buildSidebar } from "./src/lib/sidebar";
import { BASE, SITE, SITE_ORIGIN } from "./src/lib/site";

// https://astro.build/config
export default defineConfig({
  site: SITE_ORIGIN,
  base: BASE,
  trailingSlash: "always",
  redirects: buildRedirects(),
  markdown: {
    processor: unified({ rehypePlugins: [[rehypeBaseLinks, { base: BASE }], rehypeDropH1] }),
  },
  vite: {
    // Starlight's components pull in Sätteri (native addon); keep it out of the bundle so
    // its platform binding resolves from its own package (isolated bun installs).
    ssr: { external: ["satteri"] },
  },
  integrations: [
    sitemap(),
    starlight({
      title: SITE.name,
      description: SITE.description,
      favicon: "/favicon.ico",
      social: [{ icon: "github", label: "GitHub", href: SITE.links.github }],
      editLink: { baseUrl: `${SITE.links.github}/edit/main/apps/docs/src/content/docs/` },
      customCss: ["./src/styles/starlight.css"],
      sidebar: buildSidebar(),
      tableOfContents: { minHeadingLevel: 2, maxHeadingLevel: 3 },
      disable404Route: true,
      titleDelimiter: "-",
      pagefind: true,
      head: [
        { tag: "meta", attrs: { name: "author", content: SITE.author.name } },
        { tag: "meta", attrs: { name: "keywords", content: SITE.keywords.join(", ") } },
        { tag: "meta", attrs: { name: "application-name", content: SITE.name } },
        { tag: "meta", attrs: { property: "og:site_name", content: SITE.name } },
        { tag: "meta", attrs: { property: "og:image", content: `${SITE.url}${SITE.screenshot}` } },
        { tag: "meta", attrs: { name: "twitter:image", content: `${SITE.url}${SITE.screenshot}` } },
      ],
    }),
  ],
});
