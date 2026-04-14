// Backend-only helpers. NOT exposed to the frontend — this file is never
// passed to `createApp({ commands })`, so the IPC bridge has no way to reach it.

export async function cleanup() {
  console.log("[react-app] cleanup — flushing caches, closing DB, …");
}
