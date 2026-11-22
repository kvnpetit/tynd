import { defineCollection } from "astro:content";
import { docsLoader } from "@astrojs/starlight/loaders";
import { docsSchema } from "@astrojs/starlight/schema";

export const collections = {
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
