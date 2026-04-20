export const VERSIONS = [
  { slug: "v0.2", label: "v0.2", status: "latest" as const },
  { slug: "v0.1", label: "v0.1", status: "archived" as const },
] as const;

export type Version = (typeof VERSIONS)[number];

export const LATEST =
  VERSIONS.find((v) => v.status === "latest") ?? VERSIONS[0];
export const LATEST_SLUG = LATEST.slug;

export function versionFromPath(pathname: string): Version | undefined {
  const match = pathname.match(/^\/docs\/(v[^/]+)/);
  return match ? VERSIONS.find((v) => v.slug === match[1]) : undefined;
}
