import { beforeEach, describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

/**
 * Runs the real frameless-window script (`crates/host/src/webview/shim/frame.js`,
 * injected into every webview) against a minimal fake DOM.
 */
const FRAME_JS = readFileSync(
  fileURLToPath(new URL("../../../../crates/host/src/webview/shim/frame.js", import.meta.url)),
  "utf8",
);

class FakeStyle {
  private readonly props = new Map<string, { value: string; priority: string }>();
  setProperty(name: string, value: string, priority = "") {
    if (value === "") this.props.delete(name);
    else this.props.set(name, { value, priority });
  }
  getPropertyValue(name: string) {
    return this.props.get(name)?.value ?? "";
  }
  getPropertyPriority(name: string) {
    return this.props.get(name)?.priority ?? "";
  }
}

class FakeElement {
  readonly style = new FakeStyle();
  parent: FakeElement | null = null;
  readonly tag: string;
  readonly attrs: Record<string, string>;
  constructor(tag: string, attrs: Record<string, string> = {}) {
    this.tag = tag;
    this.attrs = attrs;
  }
  append(child: FakeElement): FakeElement {
    child.parent = this;
    return child;
  }
  hasAttribute(name: string) {
    return name in this.attrs;
  }
  getAttribute(name: string) {
    return this.attrs[name] ?? null;
  }
  matches(selector: string): boolean {
    return selector.split(",").some((part) => {
      const attr = /^\[([\w-]+)(?:=([\w-]+))?\]$/.exec(part.trim());
      if (attr?.[1]) {
        const value = this.attrs[attr[1]];
        return value !== undefined && (attr[2] === undefined || value === attr[2]);
      }
      return part.trim() === this.tag;
    });
  }
  closest(selector: string): FakeElement | null {
    for (let el: FakeElement | null = this; el; el = el.parent) {
      if (el.matches(selector)) return el;
    }
    return null;
  }
  contains(other: FakeElement) {
    for (let el: FakeElement | null = other; el; el = el.parent) if (el === this) return true;
    return false;
  }
}

type Handler = (e: Record<string, unknown>) => void;
type Call = { method: string; args: unknown };

let calls: Call[];
let handlers: Record<string, Handler>;
let osListeners: Record<string, () => void>;
let frameState: { edgeResizable: boolean };

function load() {
  calls = [];
  handlers = {};
  osListeners = {};
  const win = {
    innerWidth: 800,
    innerHeight: 600,
    __tynd__: {
      os_call(_api: string, method: string, args: unknown) {
        calls.push({ method, args });
        return Promise.resolve(method === "frameState" ? frameState : null);
      },
      os_on(name: string, handler: () => void) {
        osListeners[name] = handler;
        return () => undefined;
      },
    },
  };
  const doc = {
    addEventListener(type: string, handler: Handler) {
      handlers[type] = handler;
    },
  };
  new Function("window", "document", "Element", "HTMLElement", FRAME_JS)(
    win,
    doc,
    FakeElement,
    FakeElement,
  );
}

function mouse(type: string, target: FakeElement, init: Record<string, unknown> = {}) {
  let prevented = false;
  handlers[type]?.({
    button: 0,
    buttons: 0,
    detail: 1,
    clientX: 400,
    clientY: 300,
    target,
    preventDefault() {
      prevented = true;
    },
    ...init,
  });
  return prevented;
}

const tick = () => new Promise((resolve) => setTimeout(resolve, 0));

beforeEach(() => {
  frameState = { edgeResizable: false };
  load();
});

describe("frame.js drag regions", () => {
  test("data-tynd-drag starts a native drag, also from a child", () => {
    const bar = new FakeElement("div", { "data-tynd-drag": "" });
    const title = bar.append(new FakeElement("span"));
    expect(mouse("mousedown", title)).toBe(true);
    expect(calls).toEqual([{ method: "startDragging", args: {} }]);
  });

  test("double-click toggles maximize", () => {
    const bar = new FakeElement("div", { "data-tynd-drag": "" });
    mouse("mousedown", bar, { detail: 2 });
    expect(calls).toEqual([{ method: "toggleMaximize", args: {} }]);
  });

  test("controls and data-tynd-no-drag stay clickable", () => {
    const bar = new FakeElement("div", { "data-tynd-drag": "" });
    const button = bar.append(new FakeElement("button"));
    const zone = bar.append(new FakeElement("div", { "data-tynd-no-drag": "" }));
    const inside = zone.append(new FakeElement("span"));
    expect(mouse("mousedown", button)).toBe(false);
    expect(mouse("mousedown", inside)).toBe(false);
    expect(calls).toEqual([]);
  });

  test("ignores right clicks and plain elements", () => {
    const bar = new FakeElement("div", { "data-tynd-drag": "" });
    mouse("mousedown", bar, { button: 2 });
    mouse("mousedown", new FakeElement("div"));
    expect(calls).toEqual([]);
  });
});

describe("frame.js resize", () => {
  test("data-tynd-resize maps the edge to a resize direction", () => {
    const grip = new FakeElement("div", { "data-tynd-resize": "se" });
    expect(mouse("mousedown", grip)).toBe(true);
    expect(calls).toEqual([{ method: "startResizeDragging", args: { direction: "southEast" } }]);
  });

  test("unknown data-tynd-resize values are ignored", () => {
    mouse("mousedown", new FakeElement("div", { "data-tynd-resize": "up" }));
    expect(calls).toEqual([]);
  });

  test("the page border resizes a frameless window once the host says so", async () => {
    frameState = { edgeResizable: true };
    const body = new FakeElement("body");
    mouse("mousemove", body, { clientX: 2, clientY: 300 });
    expect(calls).toEqual([{ method: "frameState", args: {} }]);
    await tick();
    mouse("mousemove", body, { clientX: 2, clientY: 300 });
    expect(body.style.getPropertyValue("cursor")).toBe("ew-resize");
    mouse("mousedown", body, { clientX: 798, clientY: 1 });
    expect(calls.at(-1)).toEqual({
      method: "startResizeDragging",
      args: { direction: "northEast" },
    });
    mouse("mousemove", body, { clientX: 400, clientY: 300 });
    expect(body.style.getPropertyValue("cursor")).toBe("");
  });

  test("the border stays inert on decorated windows and after a maximize", async () => {
    const body = new FakeElement("body");
    mouse("mousemove", body, { clientX: 2, clientY: 2 });
    await tick();
    expect(mouse("mousedown", body, { clientX: 2, clientY: 2 })).toBe(false);

    frameState = { edgeResizable: true };
    osListeners["window:maximized"]?.();
    mouse("mousemove", body, { clientX: 2, clientY: 2 });
    await tick();
    expect(mouse("mousedown", body, { clientX: 2, clientY: 2 })).toBe(true);
    osListeners["window:maximized"]?.();
    expect(mouse("mousedown", body, { clientX: 2, clientY: 2 })).toBe(false);
  });
});

describe("frame.js file drag-out", () => {
  test("data-tynd-drag-file drags its files out on dragstart", () => {
    const card = new FakeElement("div", { "data-tynd-drag-file": "/tmp/a.txt\n /tmp/b.png \n" });
    const icon = card.append(new FakeElement("img"));
    expect(mouse("dragstart", icon)).toBe(true);
    expect(calls).toEqual([{ method: "startDrag", args: { files: ["/tmp/a.txt", "/tmp/b.png"] } }]);
  });

  test("plain and empty drag sources keep the browser drag", () => {
    expect(mouse("dragstart", new FakeElement("img"))).toBe(false);
    expect(mouse("dragstart", new FakeElement("div", { "data-tynd-drag-file": " " }))).toBe(false);
    expect(calls).toEqual([]);
  });
});
