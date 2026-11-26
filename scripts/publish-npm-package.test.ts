import { describe, expect, test } from "bun:test";
import {
  decidePublish,
  parsePackInfo,
  parseRegistryIntegrity,
  resolvePublishTag,
} from "./publish-npm-package";

describe("parsePackInfo", () => {
  const info = { name: "@tynd/core", version: "1.0.0", integrity: "sha512-a" };

  test("accepts the legacy array and the npm 12 package-name map", () => {
    expect(parsePackInfo(JSON.stringify([info]))).toEqual(info);
    expect(parsePackInfo(JSON.stringify({ [info.name]: info }))).toEqual(info);
  });

  test("rejects ambiguous output and missing integrity", () => {
    expect(() => parsePackInfo(JSON.stringify([info, info]))).toThrow();
    expect(() =>
      parsePackInfo(JSON.stringify({ [info.name]: { ...info, integrity: 0 } })),
    ).toThrow();
  });
});

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

describe("parseRegistryIntegrity", () => {
  test("accepts the legacy string and the npm 12 array", () => {
    expect(parseRegistryIntegrity('"sha512-a"', "@tynd/core@1.0.0")).toBe("sha512-a");
    expect(parseRegistryIntegrity('["sha512-a"]', "@tynd/core@1.0.0")).toBe("sha512-a");
  });

  test("rejects multiple integrity values", () => {
    expect(() => parseRegistryIntegrity('["sha512-a","sha512-b"]', "@tynd/core@1.0.0")).toThrow();
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
