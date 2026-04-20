import type { Metadata } from "next";
import { Page } from "../../components/ui";
import { SITE } from "../../lib/site";

export const metadata: Metadata = {
  title: "Compare",
  description: `${SITE.name} vs Electron, Tauri, Wails — side by side.`,
};

export default function ComparePage() {
  return (
    <Page
      eyebrow="Compare"
      title={`${SITE.name} vs alternatives`}
      description={`How ${SITE.name} stacks up against Electron, Tauri, Wails, and Neutralino.`}
    >
      <p className="text-center text-neutral-500 dark:text-neutral-400">
        Coming soon — sourced from <code>COMPARISON.md</code>.
      </p>
    </Page>
  );
}
