import { SITE } from "./site";
import { LATEST_SLUG } from "./versions";

export type NavItem = {
  label: string;
  href: string;
  external?: boolean;
};

export const TOP_NAV: readonly NavItem[] = [
  { label: "Docs", href: `/docs/${LATEST_SLUG}` },
  { label: "Showcase", href: "/showcase" },
  { label: "Blog", href: "/blog" },
  { label: "Changelog", href: "/changelog" },
  { label: "GitHub", href: SITE.links.github, external: true },
];

export const FOOTER_LINKS: ReadonlyArray<{
  title: string;
  items: readonly NavItem[];
}> = [
  {
    title: "Product",
    items: [
      { label: "Docs", href: `/docs/${LATEST_SLUG}` },
      { label: "Showcase", href: "/showcase" },
      { label: "Download", href: SITE.links.releases, external: true },
      { label: "Changelog", href: "/changelog" },
    ],
  },
  {
    title: "Resources",
    items: [
      { label: "API reference", href: `/docs/${LATEST_SLUG}` },
      { label: "Compare", href: "/compare" },
      { label: "Blog", href: "/blog" },
    ],
  },
  {
    title: "Community",
    items: [
      { label: "GitHub", href: SITE.links.github, external: true },
      { label: "Issues", href: SITE.links.issues, external: true },
      { label: "Discussions", href: SITE.links.discussions, external: true },
    ],
  },
];
