import Link from "next/link";
import { FOOTER_LINKS } from "../../lib/nav";
import { SITE } from "../../lib/site";
import { Container } from "./Container";

export function SiteFooter() {
  return (
    <footer className="border-t border-black/5 dark:border-white/10 bg-neutral-50 dark:bg-neutral-900/50">
      <Container className="py-12 grid grid-cols-2 md:grid-cols-4 gap-8">
        <div className="col-span-2 md:col-span-1">
          <div className="font-bold text-lg mb-2">{SITE.name}</div>
          <p className="text-sm text-neutral-600 dark:text-neutral-400">
            {SITE.description}
          </p>
        </div>
        {FOOTER_LINKS.map((col) => (
          <div key={col.title}>
            <h3 className="font-semibold text-sm mb-3">{col.title}</h3>
            <ul className="space-y-2 text-sm">
              {col.items.map((item) =>
                item.external ? (
                  <li key={item.href}>
                    <a
                      href={item.href}
                      target="_blank"
                      rel="noreferrer"
                      className="text-neutral-600 dark:text-neutral-400 hover:text-cyan-600 dark:hover:text-cyan-400"
                    >
                      {item.label}
                    </a>
                  </li>
                ) : (
                  <li key={item.href}>
                    <Link
                      href={item.href}
                      className="text-neutral-600 dark:text-neutral-400 hover:text-cyan-600 dark:hover:text-cyan-400"
                    >
                      {item.label}
                    </Link>
                  </li>
                ),
              )}
            </ul>
          </div>
        ))}
      </Container>
      <div className="border-t border-black/5 dark:border-white/10 py-6 text-center text-xs text-neutral-500">
        {SITE.license} {new Date().getFullYear()} © {SITE.name}
      </div>
    </footer>
  );
}
