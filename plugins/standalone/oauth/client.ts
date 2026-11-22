/**
 * `@tynd/plugin-oauth` — single-import entry
 * (`import { oauth } from "@tynd/plugin-oauth"`). The loopback listener's
 * lifecycle runs in the plugin's Bun backend.
 */

import { createPluginClient } from "@tynd/core/client";
import { composePluginClient, onHostEvent } from "@tynd/plugin-sdk";
import type oauthBackend from "./backend";
import { createOAuthFlow, type OAuthFlow } from "./flow";

export type { OAuthFlow };

const _api = createPluginClient<typeof oauthBackend>("oauth");

/** Options for `oauth.start`; omitted fields take the backend defaults. */
export interface StartOptions {
  port?: number;
  path?: string;
  successHtml?: string;
  expectedState?: string;
}

/** `oauth:callback` event payload — parsed query params from the redirect. */
export type { OAuthCallbackEvent } from "./types";

/** Frontend bindings for the `@tynd/plugin-oauth` plugin (PKCE-friendly localhost loopback). */
export const oauth = composePluginClient(_api, {
  /** Bind a localhost listener and return a flow handle. The listener
   *  self-tears-down after the redirect arrives. */
  async start(opts: StartOptions = {}): Promise<OAuthFlow> {
    const handle = await _api.start(opts);
    return createOAuthFlow(handle, {
      async cancel(id) {
        await _api.cancel({ id });
      },
      onCallback(handler) {
        return onHostEvent("oauth:callback", handler);
      },
      onError(handler) {
        return onHostEvent("oauth:error", handler);
      },
    });
  },
  /** List ids of every still-running OAuth listener for this app. */
  list(): Promise<number[]> {
    return _api.list();
  },
});

export default oauth;
