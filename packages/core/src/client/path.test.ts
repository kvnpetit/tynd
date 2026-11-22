import { describe, expect, test } from "bun:test";
import { path } from "./path";

describe("client path", () => {
  test("join always outputs forward slashes, even from backslash input", () => {
    expect(path.join("C:\\", "Users", "me")).toBe("C:/Users/me");
    expect(path.join("/tmp", "notes", "hello.md")).toBe("/tmp/notes/hello.md");
    expect(path.join("", "")).toBe("");
  });

  test("exposes no native separator that would contradict join", () => {
    expect("sep" in path).toBe(false);
  });

  test("dirname / basename / extname accept either separator", () => {
    expect(path.dirname("/tmp/notes/hello.md")).toBe("/tmp/notes");
    expect(path.dirname("hello.md")).toBe("");
    expect(path.basename("C:\\notes\\hello.md", ".md")).toBe("hello");
    expect(path.extname("hello.md")).toBe(".md");
  });
});
