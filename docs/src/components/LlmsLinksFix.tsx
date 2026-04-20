"use client";

import { useEffect } from "react";

// The llms.txt / llms-full.txt sidebar entries point at an absolute URL so
// Nextra's Anchor renders them as external links (otherwise Next's <Link>
// tries to client-route and the `.txt` extension breaks). The absolute URL is
// hardcoded to the production domain at build time, which is wrong for local
// dev (localhost) and for any preview deploy on a different host. This
// client-only component rewrites those links to the *current* origin on
// mount, so the download works from wherever the site is served.
export function LlmsLinksFix() {
  useEffect(() => {
    const fix = () => {
      const links = document.querySelectorAll<HTMLAnchorElement>(
        'a[href*="/llms.txt"], a[href*="/llms-full.txt"]',
      );
      for (const a of links) {
        try {
          const parsed = new URL(a.href);
          if (parsed.origin !== window.location.origin) {
            a.href = `${window.location.origin}${parsed.pathname}`;
          }
          a.target = "_blank";
          a.rel = "noreferrer";
        } catch {
          // ignore malformed hrefs
        }
      }
    };
    fix();
    // Nextra re-renders sidebar on route change — run again on each nav.
    const obs = new MutationObserver(fix);
    const aside = document.querySelector("aside.nextra-sidebar-container");
    if (aside) obs.observe(aside, { childList: true, subtree: true });
    return () => obs.disconnect();
  }, []);
  return null;
}
