/**
 * Shell utilities — open URLs/files in default app, execute commands.
 */

/** Open a URL or file path with the OS default application. */
export async function open(target: string): Promise<void> {
  const cmd = process.platform === "win32" ? ["cmd", "/c", "start", "", target]
            : process.platform === "darwin" ? ["open", target]
            : ["xdg-open", target];
  const proc = Bun.spawn(cmd, { stdout: "ignore", stderr: "ignore" });
  await proc.exited;
}

/** Execute a shell command and return stdout. */
export async function exec(command: string, args: string[] = []): Promise<{ stdout: string; stderr: string; exitCode: number }> {
  const proc = Bun.spawn([command, ...args], { stdout: "pipe", stderr: "pipe" });
  const [stdout, stderr] = await Promise.all([
    new Response(proc.stdout).text(),
    new Response(proc.stderr).text(),
  ]);
  return { stdout, stderr, exitCode: await proc.exited };
}
