import {
  text     as clackText,
  select   as clackSelect,
  confirm  as clackConfirm,
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
    placeholder: defaultValue || undefined,
    defaultValue: defaultValue || undefined,
  })
  if (isCancel(result)) onCancel()
  return result as string
}

// ── select ────────────────────────────────────────────────────────────────────

export interface SelectOption<T extends string = string> {
  label:  string
  value:  T
  hint?:  string
}

export async function select<T extends string>(
  message: string,
  options: SelectOption<T>[],
): Promise<T> {
  const result = await clackSelect({ message, options: options as { value: T; label: string; hint?: string }[] })
  if (isCancel(result)) onCancel()
  return result as T
}
