export interface ExecOptions {
  cwd?: string
  env?: Record<string, string>
  silent?: boolean
}

/** Run a command, stream output, reject on non-zero exit. */
export async function exec(cmd: string, args: string[], opts: ExecOptions = {}): Promise<void> {
  const proc = Bun.spawn([cmd, ...args], {
    ...(opts.cwd !== undefined && { cwd: opts.cwd }),
    env: opts.env ? { ...process.env, ...opts.env } : process.env,
    stdout: opts.silent ? "pipe" : "inherit",
    stderr: "pipe",
  })

  // Stream stderr to process.stderr in real time while also capturing it
  const stderrChunks: Uint8Array[] = []
  const stderrReader = proc.stderr.getReader()
  const stderrDrain = (async () => {
    while (true) {
      const { done, value } = await stderrReader.read()
      if (done) break
      stderrChunks.push(value)
      if (!opts.silent) process.stderr.write(value)
    }
  })()

  const code = await proc.exited
  await stderrDrain

  if (code !== 0) {
    const stderr = Buffer.concat(stderrChunks.map((c) => Buffer.from(c)))
      .toString("utf8")
      .trim()
    throw new Error(
      `${cmd} ${args.join(" ")} exited with code ${code}${stderr ? `\n${stderr}` : ""}`,
    )
  }
}
