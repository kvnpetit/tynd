"use client";

import { usePathname, useRouter } from "next/navigation";
import { useEffect, useRef, useState } from "react";
import { LATEST_SLUG, VERSIONS, versionFromPath } from "../lib/versions";

export function VersionPicker() {
  const pathname = usePathname();
  const router = useRouter();
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) return;
    const onDown = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node))
        setOpen(false);
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") setOpen(false);
    };
    document.addEventListener("mousedown", onDown);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onDown);
      document.removeEventListener("keydown", onKey);
    };
  }, [open]);

  const current = versionFromPath(pathname)?.slug ?? LATEST_SLUG;
  const currentVersion =
    VERSIONS.find((v) => v.slug === current) ?? VERSIONS[0];

  const select = (slug: string) => {
    setOpen(false);
    if (slug === current) return;
    const rest = pathname.replace(/^\/docs\/v[^/]+/, "") || "/";
    router.push(`/docs/${slug}${rest}`);
  };

  return (
    <div ref={ref} className="relative">
      <button
        type="button"
        aria-label="Documentation version"
        aria-haspopup="listbox"
        aria-expanded={open}
        onClick={() => setOpen((v) => !v)}
        className="inline-flex h-8 items-center gap-1.5 rounded-md border border-black/10 px-2.5 text-sm text-neutral-700 transition-colors hover:bg-black/5 hover:text-neutral-900 dark:border-white/15 dark:text-neutral-300 dark:hover:bg-white/10 dark:hover:text-neutral-50"
      >
        <span>
          {currentVersion.label}
          {currentVersion.status === "latest" ? (
            <span className="ml-1 text-xs opacity-60">(latest)</span>
          ) : null}
        </span>
        <svg
          xmlns="http://www.w3.org/2000/svg"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          strokeWidth="2"
          strokeLinecap="round"
          strokeLinejoin="round"
          className={`h-3 w-3 transition-transform ${open ? "rotate-180" : ""}`}
          aria-hidden="true"
        >
          <polyline points="6 9 12 15 18 9" />
        </svg>
      </button>
      {open && (
        <ul
          role="listbox"
          className="absolute right-0 top-10 z-50 min-w-[10rem] overflow-hidden rounded-md border border-black/10 bg-white text-sm shadow-lg dark:border-white/15 dark:bg-neutral-900"
        >
          {VERSIONS.map((v) => {
            const selected = v.slug === current;
            return (
              <li key={v.slug}>
                <button
                  type="button"
                  role="option"
                  aria-selected={selected}
                  onClick={() => select(v.slug)}
                  className={`flex w-full items-center justify-between gap-3 px-3 py-2 text-left transition-colors hover:bg-black/5 dark:hover:bg-white/5 ${
                    selected
                      ? "text-cyan-600 dark:text-cyan-400"
                      : "text-neutral-700 dark:text-neutral-300"
                  }`}
                >
                  <span>{v.label}</span>
                  <span className="text-xs opacity-60">
                    {v.status === "latest" ? "latest" : v.status}
                  </span>
                </button>
              </li>
            );
          })}
        </ul>
      )}
    </div>
  );
}
