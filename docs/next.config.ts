import { fileURLToPath } from "node:url";
import type { NextConfig } from "next";
import nextra from "nextra";

const withNextra = nextra({
  defaultShowCopyCode: true,
  search: { codeblocks: true },
  staticImage: true,
  contentDirBasePath: "/docs",
});

const nextConfig: NextConfig = {
  output: "export",
  images: { unoptimized: true },
  reactCompiler: true,
  basePath: process.env.NEXT_PUBLIC_BASE_PATH ?? "",
  trailingSlash: true,
  turbopack: {
    root: fileURLToPath(new URL("..", import.meta.url)),
  },
};

export default withNextra(nextConfig);
