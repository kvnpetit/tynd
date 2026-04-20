import { getPageMap } from "nextra/page-map";
import { Layout, Navbar } from "nextra-theme-docs";
import { LlmsLinksFix } from "../../components/LlmsLinksFix";
import { SiteFooter } from "../../components/ui";
import { VersionPicker } from "../../components/VersionPicker";
import { TOP_NAV } from "../../lib/nav";
import { SITE } from "../../lib/site";
import { LATEST_SLUG } from "../../lib/versions";

const navbar = (
  <Navbar logo={<span className="font-bold text-lg">{SITE.name}</span>}>
    {TOP_NAV.filter((item) => item.href !== `/docs/${LATEST_SLUG}`).map(
      (item) => (
        <a
          key={item.href}
          href={item.href}
          {...(item.external
            ? { target: "_blank", rel: "noreferrer" }
            : null)}
          className="x:text-sm x:font-medium x:transition-colors x:hover:text-gray-900 x:dark:hover:text-gray-50"
        >
          {item.label}
        </a>
      ),
    )}
    <VersionPicker />
  </Navbar>
);

const footer = <SiteFooter />;

export default async function DocsLayout({
  children,
}: Readonly<{ children: React.ReactNode }>) {
  // Populate the sidebar with the latest version's content map — the version
  // root ("/docs/v0.1/...") acts as the top-level sidebar root so users don't
  // see the version folder itself as a nav entry.
  const pageMap = await getPageMap(`/docs/${LATEST_SLUG}`);

  return (
    <Layout
      navbar={navbar}
      footer={footer}
      pageMap={pageMap}
      docsRepositoryBase={`${SITE.links.github}/tree/main/docs`}
      sidebar={{ defaultMenuCollapseLevel: 1, autoCollapse: true }}
    >
      <LlmsLinksFix />
      {children}
    </Layout>
  );
}
