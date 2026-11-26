import type { APIRoute } from "astro";
import { SITE } from "../lib/site";

const SIZE = { width: 1200, height: 630 } as const;
const FONT = "system-ui, -apple-system, BlinkMacSystemFont, 'Segoe UI', sans-serif";

function escapeXml(value: string) {
  return value
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;");
}

export const GET: APIRoute = () =>
  new Response(
    `<svg xmlns="http://www.w3.org/2000/svg" width="${SIZE.width}" height="${SIZE.height}" viewBox="0 0 ${SIZE.width} ${SIZE.height}" role="img" aria-label="${escapeXml(SITE.title)}">
  <defs>
    <linearGradient id="bg" x1="0" x2="1" y1="0" y2="1">
      <stop offset="0" stop-color="#0a0a0a"/>
      <stop offset="1" stop-color="#164e63"/>
    </linearGradient>
  </defs>
  <rect width="${SIZE.width}" height="${SIZE.height}" fill="url(#bg)"/>
  <text x="600" y="270" fill="#ffffff" font-family="${FONT}" font-size="120" font-weight="800" text-anchor="middle">${escapeXml(SITE.name)}</text>
  <text x="600" y="345" fill="#67e8f9" font-family="${FONT}" font-size="44" font-weight="600" text-anchor="middle">${escapeXml(SITE.tagline)}</text>
  <text x="600" y="410" fill="#d4d4d4" font-family="${FONT}" font-size="28" text-anchor="middle">${escapeXml(SITE.description)}</text>
</svg>`,
    { headers: { "content-type": "image/svg+xml; charset=utf-8" } },
  );
