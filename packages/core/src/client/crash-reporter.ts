import { osCall } from "./_internal.ts"

export interface CrashReporterEnableResult {
  enabled: boolean
  already: boolean
  dir?: string
}

export const crashReporter = {
  enable(appId: string): Promise<CrashReporterEnableResult> {
    return osCall("crashReporter", "enable", { appId })
  },
  logDir(): Promise<string> {
    return osCall("crashReporter", "logDir") as Promise<string>
  },
  listCrashes(): Promise<string[]> {
    return osCall("crashReporter", "listCrashes")
  },
}
