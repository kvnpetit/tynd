import { osCall } from "@tynd/core/client/bridge";

export type AppleScriptLanguage = "AppleScript" | "JavaScript" | "JXA";

export interface AppleScriptRunResult<T = unknown> {
  value: T;
}

/** macOS-only AppleScript/JXA runner. Requires the `apple-script` host API. */
export const appleScript = {
  run<T = unknown>(
    script: string,
    language: AppleScriptLanguage = "AppleScript",
  ): Promise<AppleScriptRunResult<T>> {
    return osCall("appleScript", "run", { script, language });
  },
};
