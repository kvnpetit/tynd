/**
 * Web-platform globals re-exported as named exports so consumers can
 * either import them individually or grab the whole surface via
 * `import * as tynd from "@tynd/core/client"`.
 *
 * These are references to `globalThis.*` captured at module load. In
 * lite they point at our polyfills; in full they point at Bun's native
 * implementations. Behavior is the spec — same code runs on both
 * runtimes.
 */

// biome-ignore-all lint/suspicious/noShadowRestrictedNames: intentional re-exports

const g = globalThis as unknown as Record<string, unknown>

// Fetch family
export const fetch = g["fetch"] as typeof globalThis.fetch
export const Request = g["Request"] as typeof globalThis.Request
export const Response = g["Response"] as typeof globalThis.Response
export const Headers = g["Headers"] as typeof globalThis.Headers
export const AbortController = g["AbortController"] as typeof globalThis.AbortController
export const AbortSignal = g["AbortSignal"] as typeof globalThis.AbortSignal
export const ReadableStream = g["ReadableStream"] as typeof globalThis.ReadableStream

// Socket
export const WebSocket = g["WebSocket"] as typeof globalThis.WebSocket
export const EventSource = g["EventSource"] as typeof globalThis.EventSource

// Crypto
export const crypto = g["crypto"] as typeof globalThis.crypto

// URL + encoding
export const URL = g["URL"] as typeof globalThis.URL
export const URLSearchParams = g["URLSearchParams"] as typeof globalThis.URLSearchParams
export const TextEncoder = g["TextEncoder"] as typeof globalThis.TextEncoder
export const TextDecoder = g["TextDecoder"] as typeof globalThis.TextDecoder
export const atob = g["atob"] as typeof globalThis.atob
export const btoa = g["btoa"] as typeof globalThis.btoa

// Binary data
export const Blob = g["Blob"] as typeof globalThis.Blob
export const File = g["File"] as typeof globalThis.File
export const FormData = g["FormData"] as typeof globalThis.FormData

// Misc
export const structuredClone = g["structuredClone"] as typeof globalThis.structuredClone
export const performance = g["performance"] as typeof globalThis.performance
