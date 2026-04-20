import {
  Button,
  Container,
  FeatureCard,
  SectionTitle,
  SiteFooter,
  SiteHeader,
} from "../components/ui";
import { SITE, sameAs } from "../lib/site";
import { LATEST_SLUG } from "../lib/versions";

const DOCS_HREF = `/docs/${LATEST_SLUG}`;

const JSON_LD = {
  "@context": "https://schema.org",
  "@type": "SoftwareApplication",
  name: SITE.name,
  description: SITE.description,
  url: SITE.url,
  applicationCategory: SITE.applicationCategory,
  operatingSystem: SITE.operatingSystem,
  offers: {
    "@type": "Offer",
    price: SITE.price.amount,
    priceCurrency: SITE.price.currency,
  },
  license: `https://spdx.org/licenses/${SITE.license}.html`,
  screenshot: `${SITE.url}${SITE.screenshot}`,
  author: {
    "@type": "Person",
    name: SITE.author.name,
    url: SITE.author.url,
  },
  sameAs: sameAs(),
};

export default function LandingPage() {
  return (
    <div className="min-h-screen bg-white dark:bg-neutral-950 text-neutral-900 dark:text-neutral-100">
      <SiteHeader />

      <main id="main-content">
        <section className="py-24">
          <Container className="text-center">
            <h1 className="text-5xl sm:text-6xl font-bold tracking-tight">
              {SITE.tagline.replace(SITE.taglineHighlight, "")}
              <span className="text-cyan-600 dark:text-cyan-400">
                {SITE.taglineHighlight}
              </span>
            </h1>
            <p className="mt-6 text-lg sm:text-xl text-neutral-600 dark:text-neutral-400 max-w-2xl mx-auto">
              {SITE.subtagline}
            </p>
            <div className="mt-10 flex items-center justify-center gap-4">
              <Button href={DOCS_HREF}>Get started</Button>
              <Button href={SITE.links.github} external variant="secondary">
                Star on GitHub
              </Button>
            </div>
            <div className="mt-6 text-sm text-neutral-500 font-mono">
              {SITE.install}
            </div>
          </Container>
        </section>

        <section className="py-16 border-t border-black/5 dark:border-white/10">
          <Container>
            <div className="grid grid-cols-1 md:grid-cols-3 gap-8">
              <FeatureCard
                title="One language"
                description="Backend, frontend, everything in TypeScript. Bun-first. No Go, no Rust, no bridge."
              />
              <FeatureCard
                title="Small binaries"
                description="~6 MB host on Windows x64. Fraction of the size of Electron apps."
              />
              <FeatureCard
                title="Typed RPC"
                description="Call backend functions from frontend with full type inference. Zero codegen."
              />
              <FeatureCard
                title="Two runtimes, one API"
                description="Pick full (Bun subprocess) or lite (embedded QuickJS). Same TypeScript API."
              />
              <FeatureCard
                title="Native OS APIs"
                description="Window, menu, tray, dialog, clipboard, shell, notifications, SQL, HTTP, PTY — all built-in."
              />
              <FeatureCard
                title="Single binary ship"
                description="tynd build produces one .exe/.app/.deb. No installers to manage."
              />
            </div>
          </Container>
        </section>

        <section className="py-16 border-t border-black/5 dark:border-white/10">
          <Container size="narrow">
            <SectionTitle
              title="Ready to build?"
              description="Scaffold your first app in under a minute."
            />
            <div className="mt-8 text-center">
              <Button href={DOCS_HREF}>Read the docs</Button>
            </div>
          </Container>
        </section>
      </main>

      <SiteFooter />

      <script
        type="application/ld+json"
        // biome-ignore lint/security/noDangerouslySetInnerHtml: safe, static JSON-LD
        dangerouslySetInnerHTML={{ __html: JSON.stringify(JSON_LD) }}
      />
    </div>
  );
}
