import type { Metadata } from "next";
import { Page } from "../../components/ui";
import { SITE } from "../../lib/site";

export const metadata: Metadata = {
  title: "Showcase",
  description: `Apps built with ${SITE.name}.`,
};

export default function ShowcasePage() {
  return (
    <Page
      eyebrow="Showcase"
      title={`Built with ${SITE.name}`}
      description={`Real apps shipping on ${SITE.name}. Browse, learn, get inspired.`}
    >
      <p className="text-center text-neutral-500 dark:text-neutral-400">
        Coming soon — grid populated from <code>src/content/showcase/</code>.
      </p>
    </Page>
  );
}
