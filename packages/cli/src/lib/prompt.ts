import {
  confirm as clackConfirm,
  select as clackSelect,
  text as clackText,
  isCancel,
} from "@clack/prompts"

function onCancel(): never {
  process.stdout.write("\n")
  process.exit(0)
}

// ── confirm ───────────────────────────────────────────────────────────────────

export async function confirm(question: string, initialValue = false): Promise<boolean> {
  const result = await clackConfirm({ message: question, initialValue })
  if (isCancel(result)) onCancel()
  return result as boolean
}

// ── text ──────────────────────────────────────────────────────────────────────

export async function text(message: string, defaultValue = ""): Promise<string> {
  const result = await clackText({
    message,
    ...(defaultValue && { placeholder: defaultValue, defaultValue }),
  })
  if (isCancel(result)) onCancel()
  return result as string
}

// ── select ────────────────────────────────────────────────────────────────────

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
