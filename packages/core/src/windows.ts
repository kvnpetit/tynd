/**
 * Backend window control: the same window handle as `tyndWindow` in the
 * frontend, driven through the trusted host bridge.
 */

import { createWindowHandle, type TyndWindow } from "./client/window";
import { hostCall } from "./host";
import { onHostEvent } from "./host-events";

export type { StartDragOptions, TyndWindow } from "./client/window";

/**
 * Handle on the window labelled `label` (default: the primary window), for
 * the Bun backend. Every method and `on*` event works as in the frontend
 * (except `onDragOver`, which the host does not forward to the backend);
 * `onCloseRequested` + `event.preventDefault()` keeps any window open.
 * Secondary windows need `@tynd/plugin-multiwindow`.
 *
 * @example
 *   import { getWindow } from "@tynd/core";
 *   app.onReady(() => getWindow().setTitle("Ready"));
 *   getWindow("editor").onCloseRequested((event) => {
 *     if (hasUnsavedChanges()) event.preventDefault();
 *   });
 */
export function getWindow(label = "main"): TyndWindow {
  return createWindowHandle(
    label,
    (method, args) => hostCall("window", method, args ?? {}),
    onHostEvent,
  );
}
