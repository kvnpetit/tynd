import { describe, expect, test } from "bun:test";
import { decidePublish, resolvePublishTag } from "./publish-npm-package";

describe("resolvePublishTag", () => {
  test("a stable version without publishConfig.tag goes to latest", () => {
    expect(resolvePublishTag({ version: "1.0.0" })).toBe("latest");
  });

  test("a prerelease version without publishConfig.tag goes to next", () => {
    expect(resolvePublishTag({ version: "1.0.0-rc.1" })).toBe("next");
  });

  test("publishConfig.tag wins over the version", () => {
    expect(resolvePublishTag({ version: "1.0.0-rc.1", publishConfig: { tag: "beta" } })).toBe(
      "beta",
    );
    expect(resolvePublishTag({ version: "1.0.0", publishConfig: { tag: "next" } })).toBe("next");
  });

  test("an invalid publishConfig.tag is rejected", () => {
    expect(() => resolvePublishTag({ publishConfig: { tag: "1.0.0" } })).toThrow();
    expect(() => resolvePublishTag({ publishConfig: { tag: 3 } })).toThrow();
  });
});

describe("decidePublish", () => {
  const local = { name: "@tynd/core", version: "1.0.0", integrity: "sha512-a" };

  test("publishes when the registry has no such version", () => {
    expect(decidePublish(local, null)).toBe("publish");
  });

  test("skips an identical published version and refuses a different one", () => {
    expect(decidePublish(local, "sha512-a")).toBe("already-published");
    expect(() => decidePublish(local, "sha512-b")).toThrow();
  });
});
