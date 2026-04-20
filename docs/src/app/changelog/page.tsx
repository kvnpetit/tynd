import type { Metadata } from "next";
import { Button, Page } from "../../components/ui";
import { SITE } from "../../lib/site";

export const metadata: Metadata = {
  title: "Changelog",
  description: "Release history.",
};

export default function ChangelogPage() {
  return (
    <Page eyebrow="Releases" title="Changelog" description="Release history.">
      <div className="text-center space-y-6">
        <p className="text-neutral-500 dark:text-neutral-400">
          Full release notes are on GitHub. A curated inline changelog will land
          here soon.
        </p>
        <Button href={SITE.links.releases} external>
          View releases on GitHub
        </Button>
      </div>
    </Page>
  );
}
