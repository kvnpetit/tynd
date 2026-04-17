export function pipeWithPrefix(
  src: ReadableStream<Uint8Array>,
  dest: NodeJS.WriteStream,
  prefix: string,
): void {
  const decoder = new TextDecoder()
  let buf = ""
  ;(async () => {
    const reader = src.getReader()
    try {
      while (true) {
        const { done, value } = await reader.read()
        if (done) break
        buf += decoder.decode(value, { stream: true })
        let idx = buf.indexOf("\n")
        while (idx !== -1) {
          dest.write(`${prefix} ${buf.slice(0, idx)}\n`)
          buf = buf.slice(idx + 1)
          idx = buf.indexOf("\n")
        }
      }
      if (buf) dest.write(`${prefix} ${buf}\n`)
    } catch {
      /* stream closed mid-read — child exited */
    }
  })()
}

export async function waitForServer(url: string, timeout = 30_000): Promise<boolean> {
  const start = Date.now()
  while (Date.now() - start < timeout) {
    try {
      const res = await fetch(url, { signal: AbortSignal.timeout(1000) })
      if (res.ok || res.status < 500) return true
    } catch {
      /* server not up yet */
    }
    await Bun.sleep(500)
  }
  return false
}
