/** Web-platform globals re-exported — polyfills in lite, Bun natives in full. */

// biome-ignore-all lint/suspicious/noShadowRestrictedNames: intentional re-exports

const g = globalThis as unknown as Record<string, unknown>;

export const fetch = g.fetch as typeof globalThis.fetch;
export const Request = g.Request as typeof globalThis.Request;
export const Response = g.Response as typeof globalThis.Response;
export const Headers = g.Headers as typeof globalThis.Headers;
export const AbortController = g.AbortController as typeof globalThis.AbortController;
export const AbortSignal = g.AbortSignal as typeof globalThis.AbortSignal;
export const ReadableStream = g.ReadableStream as typeof globalThis.ReadableStream;
export const WebSocket = g.WebSocket as typeof globalThis.WebSocket;
export const EventSource = g.EventSource as typeof globalThis.EventSource;
export const crypto = g.crypto as typeof globalThis.crypto;
export const URL = g.URL as typeof globalThis.URL;
export const URLSearchParams = g.URLSearchParams as typeof globalThis.URLSearchParams;
export const TextEncoder = g.TextEncoder as typeof globalThis.TextEncoder;
export const TextDecoder = g.TextDecoder as typeof globalThis.TextDecoder;
export const atob = g.atob as typeof globalThis.atob;
export const btoa = g.btoa as typeof globalThis.btoa;
export const Blob = g.Blob as typeof globalThis.Blob;
export const File = g.File as typeof globalThis.File;
export const FormData = g.FormData as typeof globalThis.FormData;
export const structuredClone = g.structuredClone as typeof globalThis.structuredClone;
export const performance = g.performance as typeof globalThis.performance;
