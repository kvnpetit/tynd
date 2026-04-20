"use client";

import { usePathname, useRouter } from "next/navigation";
import { LATEST_SLUG, VERSIONS, versionFromPath } from "../lib/versions";

export function VersionPicker() {
  const pathname = usePathname();
  const router = useRouter();

  const current = versionFromPath(pathname)?.slug ?? LATEST_SLUG;

  return (
    <select
      aria-label="Documentation version"
      className="x:bg-transparent x:px-2 x:py-1 x:text-sm x:rounded x:border x:border-black/10 dark:x:border-white/15 x:cursor-pointer"
      value={current}
      onChange={(e) => {
        const next = e.target.value;
        const rest = pathname.replace(/^\/docs\/v[^/]+/, "") || "/";
        router.push(`/docs/${next}${rest}`);
      }}
    >
      {VERSIONS.map((v) => (
        <option key={v.slug} value={v.slug}>
          {v.status === "latest" ? `${v.label} (latest)` : v.label}
        </option>
      ))}
    </select>
  );
}
