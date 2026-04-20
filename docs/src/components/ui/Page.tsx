import { Container } from "./Container";
import { SectionTitle } from "./SectionTitle";
import { SiteFooter } from "./SiteFooter";
import { SiteHeader } from "./SiteHeader";

type Props = {
  eyebrow?: string;
  title: string;
  description?: string;
  children?: React.ReactNode;
};

// Standard site page shell: header + hero + content + footer. Use for
// every non-doc route (landing uses a custom shell). Keeps visual parity
// across /showcase, /changelog, /compare, etc.
export function Page({ eyebrow, title, description, children }: Props) {
  return (
    <div className="min-h-screen flex flex-col bg-white dark:bg-neutral-950 text-neutral-900 dark:text-neutral-100">
      <SiteHeader />
      <main id="main-content" className="flex-1">
        <section className="py-16 border-b border-black/5 dark:border-white/10">
          <Container>
            <SectionTitle
              eyebrow={eyebrow}
              title={title}
              description={description}
            />
          </Container>
        </section>
        <section className="py-12">
          <Container>{children}</Container>
        </section>
      </main>
      <SiteFooter />
    </div>
  );
}
