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
  const result = await importPage((await props.params).mdxPath);
  const { default: MDXContent, toc, metadata, sourceCode } = result;
  const Wrapper = useMDXComponents().wrapper;

  return Wrapper ? (
    <Wrapper toc={toc} metadata={metadata} sourceCode={sourceCode}>
      <MDXContent {...props} params={props.params} />
    </Wrapper>
  ) : (
    <MDXContent {...props} params={props.params} />
  );
}
