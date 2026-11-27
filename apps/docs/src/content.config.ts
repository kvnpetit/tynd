import { defineCollection } from "astro:content";
import { docsLoader, i18nLoader } from "@astrojs/starlight/loaders";
import { docsSchema, i18nSchema } from "@astrojs/starlight/schema";

export const collections = {
  // Register English with no overrides so Starlight uses its built-in text.
  i18n: defineCollection({ loader: i18nLoader(), schema: i18nSchema() }),
  docs: defineCollection({
    loader: docsLoader({
      // The default id generator slugifies path segments (`v1.0` -> `v10`); keep the
      // version directories verbatim so URLs stay `/docs/v1.0/...`.
      generateId: ({ entry, data }) =>
        typeof data.slug === "string"
          ? data.slug
          : entry.replace(/\.mdx?$/, "").replace(/(^|\/)index$/, ""),
    }),
    schema: docsSchema(),
  }),
};
