import { osCall } from "./_internal.js"

export const clipboard = {
  readText(): Promise<string> {
    return osCall("clipboard", "readText")
  },
  writeText(text: string): Promise<void> {
    return osCall("clipboard", "writeText", text)
  },
}
