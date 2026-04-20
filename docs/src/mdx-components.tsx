import {
  Bleed,
  Callout,
  Cards,
  FileTree,
  Steps,
  Tabs,
} from "nextra/components";
import { useMDXComponents as getDocsComponents } from "nextra-theme-docs";

const docsComponents = getDocsComponents();

// Nextra components available in any .mdx without explicit import.
// Add project-specific components via `src/components/mdx/` when needed.
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
