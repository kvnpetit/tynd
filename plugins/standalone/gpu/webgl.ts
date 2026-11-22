/**
 * Free a WebGL context right away instead of waiting for garbage collection.
 *
 * Browsers cap live contexts per page (about 16) and evict the oldest one when
 * the cap is hit, which can silently kill the app's own canvas. Every probe
 * context created by this plugin goes through here once it is done with it.
 */
export function releaseWebGLContext(gl: WebGLRenderingContext | WebGL2RenderingContext): void {
  try {
    gl.getExtension("WEBGL_lose_context")?.loseContext();
  } catch {
    // Context already lost or extension unavailable: nothing left to release.
  }
}
