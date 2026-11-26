import { SITE, withBase } from "./site";
import { LATEST_SLUG } from "./versions";

export type NavItem = {
  label: string;
  /** Root-relative path (base is added at render time) or an absolute URL. */
  href: string;
  external?: boolean;
};

/** `<a>` attributes for a nav item: external links open in a new tab, internal ones get the base. */
export function navLinkAttrs(item: NavItem) {
  return item.external
    ? { href: item.href, target: "_blank", rel: "noreferrer" }
    : { href: withBase(item.href) };
}

export const TOP_NAV: readonly NavItem[] = [
  { label: "Docs", href: `/docs/${LATEST_SLUG}/` },
  { label: "Changelog", href: "/changelog/" },
  { label: "GitHub", href: SITE.links.github, external: true },
];

export const FOOTER_LINKS: ReadonlyArray<{
  title: string;
  items: readonly NavItem[];
}> = [
  {
    title: "Product",
    items: [
      { label: "Docs", href: `/docs/${LATEST_SLUG}/` },
      { label: "Download", href: SITE.links.releases, external: true },
      { label: "Changelog", href: "/changelog/" },
    ],
  },
  {
    title: "Resources",
    items: [
      { label: "API reference", href: `/docs/${LATEST_SLUG}/api/` },
      { label: "Compare", href: `/docs/${LATEST_SLUG}/compare/` },
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
