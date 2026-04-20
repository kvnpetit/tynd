import type { Metadata } from "next";
import { generateStaticParamsFor, importPage } from "nextra/pages";
import { SITE } from "../../../lib/site";
import { LATEST_SLUG, VERSIONS } from "../../../lib/versions";
import { useMDXComponents } from "../../../mdx-components";

export const generateStaticParams = generateStaticParamsFor("mdxPath");

type PageProps = Readonly<{
  params: Promise<{ mdxPath?: string[] }>;
}>;

function splitVersion(segments: readonly string[]): {
  version: string | undefined;
  rest: string[];
} {
  const first = segments[0];
  const hit = first ? VERSIONS.find((v) => v.slug === first) : undefined;
  return hit
    ? { version: hit.slug, rest: segments.slice(1) }
    : { version: undefined, rest: [...segments] };
}

function titleCase(slug: string): string {
  return slug
    .split(/[-_]/)
    .map((w) => (w ? w[0].toUpperCase() + w.slice(1) : w))
    .join(" ");
}

function docsUrl(parts: readonly string[]): string {
  return `/docs/${parts.join("/")}/`;
}

function pickString(value: unknown, fallback: string): string {
  return typeof value === "string" && value ? value : fallback;
}

export async function generateMetadata({
  params,
}: PageProps): Promise<Metadata> {
  const { mdxPath } = await params;
  const segments = mdxPath ?? [];
  const { metadata } = await importPage(mdxPath);

  const { version, rest } = splitVersion(segments);
  const isOutdated = version !== undefined && version !== LATEST_SLUG;
  const canonicalSegments = version
    ? [LATEST_SLUG, ...rest]
    : segments.length
      ? segments
      : [LATEST_SLUG];
  const canonical = docsUrl(canonicalSegments);

  const title = pickString(metadata?.title, SITE.name);
  const description = pickString(
    metadata?.description,
    `${title} — ${SITE.description}`,
  );

  return {
    ...metadata,
    title,
    description,
    alternates: { canonical },
    robots: isOutdated
      ? { index: false, follow: true }
      : { index: SITE.robots.index, follow: SITE.robots.follow },
    openGraph: {
      title,
      description,
      url: `${SITE.url}${canonical}`,
      siteName: SITE.name,
      type: "article",
      locale: SITE.ogLocale,
    },
    twitter: {
      card: "summary_large_image",
      title,
      description,
    },
  };
}

export default async function Page(props: PageProps) {
  const { mdxPath } = await props.params;
  const segments = mdxPath ?? [];
  const result = await importPage(mdxPath);
  const { default: MDXContent, toc, metadata, sourceCode } = result;
  const Wrapper = useMDXComponents().wrapper;

  const title = pickString(metadata?.title, SITE.name);
  const description = pickString(
    metadata?.description,
    `${title} — ${SITE.description}`,
  );
  const { version, rest } = splitVersion(segments);
  const canonicalSegments = version
    ? [LATEST_SLUG, ...rest]
    : segments.length
      ? segments
      : [LATEST_SLUG];
  const pageUrl = `${SITE.url}${docsUrl(canonicalSegments)}`;

  const breadcrumbParts: Array<{ name: string; url: string }> = [
    { name: "Home", url: `${SITE.url}/` },
    { name: "Docs", url: `${SITE.url}${docsUrl([LATEST_SLUG])}` },
  ];
  rest.forEach((seg, i) => {
    breadcrumbParts.push({
      name: titleCase(seg),
      url: `${SITE.url}${docsUrl([LATEST_SLUG, ...rest.slice(0, i + 1)])}`,
    });
  });

  const techArticle = {
    "@context": "https://schema.org",
    "@type": "TechArticle",
    headline: title,
    description,
    url: pageUrl,
    inLanguage: SITE.locale,
    isPartOf: { "@type": "WebSite", name: SITE.name, url: SITE.url },
    author: { "@type": "Person", name: SITE.author.name, url: SITE.author.url },
    publisher: {
      "@type": "Person",
      name: SITE.author.name,
      url: SITE.author.url,
    },
    license: `https://spdx.org/licenses/${SITE.license}.html`,
  };

  const breadcrumbList = {
    "@context": "https://schema.org",
    "@type": "BreadcrumbList",
    itemListElement: breadcrumbParts.map((b, i) => ({
      "@type": "ListItem",
      position: i + 1,
      name: b.name,
      item: b.url,
    })),
  };

  const body = Wrapper ? (
    <Wrapper toc={toc} metadata={metadata} sourceCode={sourceCode}>
      <MDXContent {...props} params={props.params} />
    </Wrapper>
  ) : (
    <MDXContent {...props} params={props.params} />
  );

  return (
    <>
      {body}
      <script
        type="application/ld+json"
        // biome-ignore lint/security/noDangerouslySetInnerHtml: static JSON-LD
        dangerouslySetInnerHTML={{ __html: JSON.stringify(techArticle) }}
      />
      <script
        type="application/ld+json"
        // biome-ignore lint/security/noDangerouslySetInnerHtml: static JSON-LD
        dangerouslySetInnerHTML={{ __html: JSON.stringify(breadcrumbList) }}
      />
    </>
  );
}
