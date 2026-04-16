import {
  confirm as clackConfirm,
  select as clackSelect,
  spinner as clackSpinner,
  text as clackText,
  isCancel,
} from "@clack/prompts"
import { getLogLevel } from "./logger.ts"

function onCancel(): never {
  process.stdout.write("\n")
  process.exit(0)
}

export async function confirm(question: string, initialValue = false): Promise<boolean> {
  const result = await clackConfirm({ message: question, initialValue })
  if (isCancel(result)) onCancel()
  return result as boolean
}

export async function text(message: string, defaultValue = ""): Promise<string> {
  const result = await clackText({
    message,
    ...(defaultValue && { placeholder: defaultValue, defaultValue }),
  })
  if (isCancel(result)) onCancel()
  return result as string
}

export type SelectOption<T extends string = string> = Parameters<
  typeof clackSelect<T>
>[0]["options"][number]

export async function select<T extends string>(
  message: string,
  options: SelectOption<T>[],
): Promise<T> {
  const result = await clackSelect<T>({ message, options })
  if (isCancel(result)) onCancel()
  return result as T
}

/**
 * Run `task` while showing an animated spinner with `label`. Disables itself in
 * verbose mode (debug output would fight the re-drawn spinner line) and in
 * non-TTY contexts where ANSI would bloat CI logs.
 */
export async function withSpinner<T>(label: string, task: () => Promise<T>): Promise<T> {
  const canAnimate = Boolean(process.stdout.isTTY) && getLogLevel() !== "verbose"
  if (!canAnimate) return task()

  const s = clackSpinner()
  s.start(label)
  try {
    const result = await task()
    s.stop(`${label} ✓`)
    return result
  } catch (err) {
    s.stop(`${label} ✗`)
    throw err
  }
}
