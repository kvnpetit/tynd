import { asError } from "./error-utils";
import {
  createWorkerTaskScript,
  MAX_PENDING_WORKER_RUNS,
  MAX_WORKERS,
  parseWorkerResponse,
} from "./worker-protocol";

interface PendingRun {
  resolve(value: unknown): void;
  reject(error: Error): void;
}

interface ManagedWorker {
  worker: Worker;
  pending: Map<number, PendingRun>;
  nextRunId: number;
}

/** Handle owned by trusted Bun backend code. Never exposed directly to a WebView. */
export interface BunWorkerHandle {
  readonly id: number;
  run<Out = unknown, In = unknown>(input: In): Promise<Out>;
  terminate(): Promise<void>;
}

const registry = new Map<number, ManagedWorker>();
let nextWorkerId = 0;

function allocateWorkerId(): number {
  for (;;) {
    nextWorkerId = nextWorkerId >= Number.MAX_SAFE_INTEGER ? 1 : nextWorkerId + 1;
    if (!registry.has(nextWorkerId)) return nextWorkerId;
  }
}

function rejectPending(managed: ManagedWorker, error: Error): void {
  for (const pending of managed.pending.values()) pending.reject(error);
  managed.pending.clear();
}

function disposeWorker(id: number, managed: ManagedWorker, error: Error): void {
  if (registry.get(id) === managed) registry.delete(id);
  managed.worker.terminate();
  rejectPending(managed, error);
}

function allocateRunId(managed: ManagedWorker): number {
  for (;;) {
    managed.nextRunId = managed.nextRunId >= Number.MAX_SAFE_INTEGER ? 1 : managed.nextRunId + 1;
    if (!managed.pending.has(managed.nextRunId)) return managed.nextRunId;
  }
}

function spawn(script: string): number {
  if (registry.size >= MAX_WORKERS) {
    throw new Error(`bunWorkers.spawn: worker limit reached (max ${MAX_WORKERS})`);
  }
  if (!script.trim()) throw new Error("bunWorkers.spawn: task source is empty");

  const id = allocateWorkerId();
  const url = `data:application/javascript,${encodeURIComponent(createWorkerTaskScript(script))}`;
  const worker = new Worker(url);
  const managed: ManagedWorker = { worker, pending: new Map(), nextRunId: 0 };

  worker.onmessage = (event: MessageEvent) => {
    const frame = parseWorkerResponse(event.data);
    if (!frame) {
      disposeWorker(id, managed, new Error("Bun Worker sent an invalid response"));
      return;
    }
    const pending = managed.pending.get(frame.runId);
    if (!pending) return;
    managed.pending.delete(frame.runId);
    if (frame.ok) pending.resolve(frame.value);
    else pending.reject(asError(frame.value, "Worker failed"));
  };
  worker.onerror = (event: ErrorEvent) => {
    event.preventDefault();
    disposeWorker(id, managed, new Error(event.message || "Bun Worker crashed"));
  };
  worker.onmessageerror = () => {
    disposeWorker(id, managed, new Error("Bun Worker response could not be decoded"));
  };
  registry.set(id, managed);
  return id;
}

function run(id: number, input: unknown): Promise<unknown> {
  const managed = registry.get(id);
  if (!managed) return Promise.reject(new Error(`bunWorkers.run: worker ${id} not found`));
  if (managed.pending.size >= MAX_PENDING_WORKER_RUNS) {
    return Promise.reject(new Error(`bunWorkers.run: worker ${id} has too many pending tasks`));
  }

  const runId = allocateRunId(managed);
  return new Promise((resolve, reject) => {
    managed.pending.set(runId, { resolve, reject });
    try {
      managed.worker.postMessage({ runId, input });
    } catch (error) {
      managed.pending.delete(runId);
      reject(asError(error));
    }
  });
}

function terminate(id: number): void {
  const managed = registry.get(id);
  if (!managed) return;
  disposeWorker(id, managed, new Error(`Worker ${id} terminated`));
}

/** Bun Worker API for trusted backend modules. */
export const bunWorkers = {
  async spawn<In = unknown, Out = unknown>(
    task: string | ((input: In) => Out | Promise<Out>),
  ): Promise<BunWorkerHandle> {
    if (typeof task !== "string" && typeof task !== "function") {
      throw new TypeError(
        `bunWorkers.spawn: expected a function or source string, got ${typeof task}`,
      );
    }
    const source = typeof task === "string" ? task : task.toString();
    const id = spawn(source);
    return {
      id,
      run: <Out, In>(input: In) => run(id, input) as Promise<Out>,
      terminate: async () => terminate(id),
    };
  },
  list(): number[] {
    return [...registry.keys()].sort((a, b) => a - b);
  },
};
