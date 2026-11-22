import { describe, expect, test } from "bun:test";
import * as facade from "./config";
import {
  type ApiName,
  type BundleConfig,
  defineConfig,
  type PluginEntry,
  type TyndConfig,
  type UpdaterConfig,
} from "./config";

const KEY = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";

describe("facade", () => {
  test("exports defineConfig and types only", () => {
    expect(Object.keys(facade)).toEqual(["defineConfig"]);
  });

  test("exports the generated config types", () => {
    const api: ApiName = "tray";
    const plugin: PluginEntry = { pkg: "@tynd/plugin-tray", apis: [api] };
    const bundle: BundleConfig = { identifier: "com.example.app" };
    const updater: UpdaterConfig = { endpoints: ["https://e"], publicKeys: [KEY] };
    // @ts-expect-error a bundle needs an identifier
    const noIdentifier: BundleConfig = {};
    // @ts-expect-error signature checks cannot be turned off
    const unsigned: UpdaterConfig = { ...updater, requireSignature: false };
    const config = defineConfig({ $schema: "./schema.json", plugins: [plugin], bundle, updater });
    expect(config.bundle.identifier).toBe("com.example.app");
    expect([noIdentifier, unsigned]).toHaveLength(2);
  });
});

describe("defineConfig types", () => {
  test("accepts list-based updater configs", () => {
    const single = defineConfig({ updater: { endpoints: ["https://e"], publicKeys: [KEY] } });
    const lists = defineConfig({
      updater: {
        endpoints: ["https://a.example.com/{{channel}}.json", "https://b.example.com/m.json"],
        publicKeys: [KEY, KEY],
        channel: "beta",
      },
    });
    expect(single.updater.publicKeys).toEqual([KEY]);
    expect(lists.updater.channel).toBe("beta");
  });

  test("accepts backend-only proxy and headers", () => {
    const config = defineConfig({
      updater: {
        endpoints: ["https://updates.example.com/m.json"],
        publicKeys: [KEY],
        proxy: "http://proxy.corp:3128",
        // biome-ignore lint/suspicious/noTemplateCurlyInString: literal ${env:NAME} placeholder, expanded by the host
        headers: { Authorization: "Bearer ${env:UPDATE_TOKEN}" },
      },
    });
    expect(config.updater.proxy).toBe("http://proxy.corp:3128");
    // biome-ignore lint/suspicious/noTemplateCurlyInString: literal ${env:NAME} placeholder, expanded by the host
    expect(config.updater.headers?.Authorization).toContain("${env:UPDATE_TOKEN}");
  });

  test("requires an endpoint and a key", () => {
    // @ts-expect-error `endpoints` is required
    const noEndpoint: TyndConfig = { updater: { publicKeys: [KEY] } };
    // @ts-expect-error `publicKeys` is required
    const noKey: TyndConfig = { updater: { endpoints: ["https://e.example.com"] } };
    // @ts-expect-error an empty list does not satisfy `endpoints`
    const emptyList: TyndConfig = { updater: { endpoints: [], publicKeys: [KEY] } };
    const singular: TyndConfig = {
      // @ts-expect-error singular `endpoint` / `publicKey` are not config keys
      updater: { endpoint: "https://e.example.com", publicKey: KEY },
    };
    expect([noEndpoint, noKey, emptyList, singular]).toHaveLength(4);
  });

  test("types observability and the new APIs", () => {
    const config = defineConfig({
      apis: ["menu", "tray", "multiwindow"],
      observability: {
        sentry: { dsn: "https://k@o.example.com/1", sampleRate: 0.5, environment: "prod" },
        otel: { endpoint: "http://localhost:4317", serviceName: "app" },
      },
    });
    expect(config.observability.otel.serviceName).toBe("app");
    // @ts-expect-error unknown observability pipeline
    const bad: TyndConfig = { observability: { datadog: {} } };
    // @ts-expect-error unknown API name
    const badApi: TyndConfig = { apis: ["debug-screenshot"] };
    expect([bad, badApi]).toHaveLength(2);
  });

  test("factory configs keep their literal types", async () => {
    const factory = defineConfig(({ envName }) => ({ backend: `backend/${envName}.ts` }));
    expect((await factory({ envName: "production" })).backend).toBe("backend/production.ts");
  });
});
