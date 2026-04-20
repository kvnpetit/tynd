"use client";

import { useEffect } from "react";
import { LATEST_SLUG } from "../lib/versions";

/**
 * Static-export-friendly redirect. Used by the content index at /docs so
 * visitors land on the latest documented version. Includes a <meta refresh>
 * fallback for no-JS clients.
 */
export function RedirectToLatest() {
  useEffect(() => {
    window.location.replace(`/docs/${LATEST_SLUG}/`);
  }, []);

  return (
    <meta
      httpEquiv="refresh"
      content={`0; url=/docs/${LATEST_SLUG}/`}
    />
  );
}
