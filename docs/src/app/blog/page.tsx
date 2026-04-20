import type { Metadata } from "next";
import { Page } from "../../components/ui";

export const metadata: Metadata = {
  title: "Blog",
  description: "Release notes, design decisions, and deep dives.",
};

export default function BlogIndexPage() {
  return (
    <Page
      eyebrow="Writing"
      title="Blog"
      description="Release notes, design decisions, and deep dives."
    >
      <p className="text-center text-neutral-500 dark:text-neutral-400">
        Coming soon — posts will live in <code>src/content/blog/*.mdx</code>.
      </p>
    </Page>
  );
}
