import { Bleed, Callout, FileTree, Steps, Tabs } from "nextra/components";
import { useMDXComponents as getDocsComponents } from "nextra-theme-docs";
import { Cards } from "./components/mdx/cards";

const docsComponents = getDocsComponents();

// Nextra components available in any .mdx without explicit import.
// Cards is a local drop-in replacement for Nextra's Cards — same API
// (`<Cards><Cards.Card title href icon arrow>desc</Cards.Card></Cards>`) but
// a grid that actually looks right for text-only content.
const nextraComponents = {
  Bleed,
  Callout,
  Cards,
  FileTree,
  Steps,
  Tabs,
};

export function useMDXComponents(
  components?: Readonly<Record<string, React.ComponentType<unknown>>>,
) {
  return { ...docsComponents, ...nextraComponents, ...components };
}
