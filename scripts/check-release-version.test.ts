import { describe, expect, test } from "bun:test";
import path from "node:path";
import {
  checkReleaseVersion,
  FRAMEWORK_PACKAGES,
  validateReleaseVersions,
} from "./check-release-version";

const repoRoot = path.resolve(import.meta.dir, "..");

describe("check-release-version", () => {
  test("framework scope covers tooling and every tooling platform package", () => {
    const expected = [
      "npm/tooling",
      ...["darwin-arm64", "darwin-x64", "linux-arm64", "linux-x64", "win32-arm64", "win32-x64"].map(
        (target) => `npm/tooling-${target}`,
      ),
    ];
    for (const dir of expected) expect(FRAMEWORK_PACKAGES).toContain(dir as never);
  });

  test("a tooling package left on an old version fails the framework preflight", () => {
    const packages = FRAMEWORK_PACKAGES.map((dir) => ({
      name: dir,
      path: dir,
      version: dir === "npm/tooling" ? "1.0.0" : "1.1.0",
    }));
    expect(() => validateReleaseVersions("framework", "v1.1.0", packages, "1.1.0")).toThrow(
      /npm\/tooling \(npm\/tooling\): 1\.0\.0/,
    );
  });

  test("framework scope releases @tynd/mcp with the framework version", () => {
    expect(FRAMEWORK_PACKAGES).toContain("packages/mcp" as never);
  });

  test("a @tynd/tooling pin left on an old version fails the framework preflight", () => {
    const stalePin: Record<string, string> = { "@tynd/tooling": "1.0.0" };
    const packages = FRAMEWORK_PACKAGES.map((dir) => ({
      name: dir,
      path: dir,
      version: "1.1.0",
      toolingPins: dir === "packages/mcp" ? stalePin : {},
    }));
    expect(() => validateReleaseVersions("framework", "v1.1.0", packages, "1.1.0")).toThrow(
      /packages\/mcp \(packages\/mcp\) -> @tynd\/tooling: 1\.0\.0/,
    );
  });

  test("the working tree passes its own framework preflight", async () => {
    const tooling = (await Bun.file(path.join(repoRoot, "npm/tooling/package.json")).json()) as {
      version: string;
    };
    expect(await checkReleaseVersion("framework", `v${tooling.version}`, repoRoot)).toBe(
      tooling.version,
    );
  });
});
