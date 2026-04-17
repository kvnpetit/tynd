import { osCall } from "./_internal.js"

export interface WorkerHandle {
  id: number
  run<Out = unknown, In = unknown>(input: In): Promise<Out>
  terminate(): Promise<void>
}

let bunWorkerSeq = 0

function spawnBunWorker(script: string): WorkerHandle {
  const id = ++bunWorkerSeq
  const wrapped = `
    const __fn = (${script});
    self.onmessage = async (e) => {
      const { rid, input } = e.data
      try {
        const result = await __fn(input)
        postMessage({ rid, result })
      } catch (err) {
        postMessage({ rid, error: String((err && err.message) || err) })
      }
    }
  `
  const url = `data:application/javascript,${encodeURIComponent(wrapped)}`
  const w = new (globalThis as unknown as { Worker: new (u: string) => Worker }).Worker(url)
  const pending = new Map<number, { resolve: (v: unknown) => void; reject: (e: Error) => void }>()
  let rseq = 0

  w.onmessage = (e: MessageEvent) => {
    const { rid, result, error } = e.data as { rid: number; result?: unknown; error?: string }
    const p = pending.get(rid)
    if (!p) return
    pending.delete(rid)
    if (error) p.reject(new Error(error))
    else p.resolve(result)
  }

  return {
    id,
    run<Out, In>(input: In): Promise<Out> {
      const rid = ++rseq
      return new Promise<Out>((resolve, reject) => {
        pending.set(rid, { resolve: resolve as (v: unknown) => void, reject })
        w.postMessage({ rid, input })
      })
    },
    terminate() {
      w.terminate()
      return Promise.resolve()
    },
  }
}

function isBunWorkerAvailable(): boolean {
  return (
    typeof (globalThis as { Worker?: unknown }).Worker === "function" &&
    !(globalThis as { __tynd_lite__?: unknown }).__tynd_lite__
  )
}

export const workers = {
  async spawn(taskFn: string | ((input: unknown) => unknown)): Promise<WorkerHandle> {
    const script = typeof taskFn === "function" ? taskFn.toString() : taskFn
    if (isBunWorkerAvailable()) return spawnBunWorker(script)
    const { id } = await osCall<{ id: number }>("workers", "spawn", { script })
    return {
      id,
      run: <Out, In>(input: In) => osCall<Out>("workers", "run", { id, input }),
      terminate: () => osCall<void>("workers", "terminate", { id }),
    }
  },
  list(): Promise<number[]> {
    if (isBunWorkerAvailable()) return Promise.resolve([])
    return osCall("workers", "list")
  },
}

export const parallel = {
  async map<In, Out>(
    items: In[],
    task: string | ((input: In) => Out),
    opts?: { concurrency?: number },
  ): Promise<Out[]> {
    const concurrency = Math.max(1, Math.min(opts?.concurrency ?? 4, items.length))
    if (items.length === 0) return []

    const pool = await Promise.all(
      Array.from({ length: concurrency }, () => workers.spawn(task as never)),
    )
    const out = new Array<Out>(items.length)
    let next = 0
    try {
      await Promise.all(
        pool.map(async (w) => {
          while (true) {
            const idx = next++
            if (idx >= items.length) return
            out[idx] = await w.run<Out, In>(items[idx]!)
          }
        }),
      )
    } finally {
      await Promise.all(pool.map((w) => w.terminate().catch(() => undefined)))
    }
    return out
  },
}
