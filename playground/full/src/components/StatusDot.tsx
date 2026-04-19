import type { ServerStatus } from "../types"

const LABEL: Record<ServerStatus, string> = {
  probing: "Probing server…",
  online: "Connected",
  offline: "Server unreachable",
}

export function StatusDot({ status, baseUrl }: { status: ServerStatus; baseUrl: string }) {
  return <span className={`dot dot-${status}`} title={`${LABEL[status]} — ${baseUrl}`} />
}
