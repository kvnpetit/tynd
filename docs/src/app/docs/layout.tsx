import { getPageMap } from "nextra/page-map";
import { Footer, Layout, Navbar } from "nextra-theme-docs";
import { VersionPicker } from "../../components/VersionPicker";
import { SITE } from "../../lib/site";

const navbar = (
  <Navbar
    logo={<span className="font-bold text-lg">{SITE.name}</span>}
    projectLink={SITE.links.github}
  >
    <VersionPicker />
  </Navbar>
);

const footer = (
  <Footer>
    {SITE.license} {new Date().getFullYear()} © {SITE.name}.
  </Footer>
);

export default async function DocsLayout({
  children,
}: Readonly<{ children: React.ReactNode }>) {
  return (
    <Layout
      navbar={navbar}
      footer={footer}
      pageMap={await getPageMap("/docs")}
      docsRepositoryBase={`${SITE.links.github}/tree/main/docs`}
      sidebar={{ defaultMenuCollapseLevel: 2 }}
    >
      {children}
    </Layout>
  );
}
