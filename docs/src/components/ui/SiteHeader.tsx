import Link from "next/link";
import { TOP_NAV } from "../../lib/nav";
import { SITE } from "../../lib/site";
import { Container } from "./Container";
import { ThemeToggle } from "./ThemeToggle";

export function SiteHeader() {
  return (
    <header className="border-b border-black/5 dark:border-white/10 bg-white/80 dark:bg-neutral-950/80 backdrop-blur sticky top-0 z-50">
      <Container className="py-4 flex items-center justify-between">
        <Link href="/" className="font-bold text-lg">
          {SITE.name}
        </Link>
        <nav className="flex items-center gap-6 text-sm">
          {TOP_NAV.map((item) =>
            item.external ? (
              <a
                key={item.href}
                href={item.href}
                target="_blank"
                rel="noreferrer"
                className="hover:text-cyan-600 dark:hover:text-cyan-400"
              >
                {item.label}
              </a>
            ) : (
              <Link
                key={item.href}
                href={item.href}
                className="hover:text-cyan-600 dark:hover:text-cyan-400"
              >
                {item.label}
              </Link>
            ),
          )}
          <ThemeToggle />
        </nav>
      </Container>
    </header>
  );
}
