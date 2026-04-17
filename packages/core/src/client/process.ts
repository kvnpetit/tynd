import { osCall } from "./_internal.js"

export interface ExecOptions {
  args?: string[]
  cwd?: string
  env?: Record<string, string>
  input?: string
}

export interface ExecResult {
  code: number | null
  stdout: string
  stderr: string
}

export const process = {
  exec(cmd: string, opts?: ExecOptions): Promise<ExecResult> {
    return osCall("process", "exec", { cmd, ...opts })
  },
  execShell(cmd: string, opts?: Omit<ExecOptions, "args">): Promise<ExecResult> {
    return osCall("process", "execShell", { cmd, ...opts })
  },
}
