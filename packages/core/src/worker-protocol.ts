export const MAX_WORKERS = 64;
export const MAX_PENDING_WORKER_RUNS = 4096;

export interface WorkerResponse {
  runId: number;
  ok: boolean;
  value: unknown;
}

export function createWorkerTaskScript(taskSource: string): string {
  return `
const __task = (${taskSource});
self.onmessage = async (event) => {
  const { runId, input } = event.data;
  try {
    postMessage({ runId, ok: true, value: await __task(input) });
  } catch (error) {
    postMessage({
      runId,
      ok: false,
      value: error instanceof Error ? error.message : String(error),
    });
  }
};`;
}

export function parseWorkerResponse(value: unknown): WorkerResponse | null {
  if (typeof value !== "object" || value === null) return null;
  const frame = value as Partial<WorkerResponse>;
  if (
    typeof frame.runId !== "number" ||
    !Number.isSafeInteger(frame.runId) ||
    frame.runId <= 0 ||
    typeof frame.ok !== "boolean"
  ) {
    return null;
  }
  return { runId: frame.runId, ok: frame.ok, value: frame.value };
}
