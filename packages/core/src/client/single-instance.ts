import { osCall } from "./_internal.ts"

export interface SingleInstanceResult {
  acquired: boolean
  already: boolean
}

export const singleInstance = {
  acquire(id: string): Promise<SingleInstanceResult> {
    return osCall("singleInstance", "acquire", { id })
  },
  isAcquired(): Promise<boolean> {
    return osCall("singleInstance", "isAcquired")
  },
}
