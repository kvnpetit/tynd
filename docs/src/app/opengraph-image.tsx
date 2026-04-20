import { ImageResponse } from "next/og";
import { SITE } from "../lib/site";

export const alt = SITE.title;
export const size = { width: 1200, height: 630 };
export const contentType = "image/png";
export const dynamic = "force-static";

export default function OpengraphImage() {
  return new ImageResponse(
    <div
      style={{
        width: "100%",
        height: "100%",
        display: "flex",
        flexDirection: "column",
        alignItems: "center",
        justifyContent: "center",
        background: "linear-gradient(135deg, #0a0a0a 0%, #164e63 100%)",
        color: "white",
        fontFamily: "system-ui, sans-serif",
        padding: 80,
      }}
    >
      <div
        style={{
          fontSize: 120,
          fontWeight: 800,
          letterSpacing: "-0.04em",
          lineHeight: 1,
        }}
      >
        {SITE.name}
      </div>
      <div
        style={{
          fontSize: 44,
          marginTop: 30,
          color: "#67e8f9",
          fontWeight: 600,
        }}
      >
        {SITE.tagline}
      </div>
      <div
        style={{
          fontSize: 28,
          marginTop: 20,
          color: "#a3a3a3",
          maxWidth: 900,
          textAlign: "center",
        }}
      >
        {SITE.description}
      </div>
    </div>,
    size,
  );
}
