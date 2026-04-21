import { osCall } from "./_internal.js"

export const power = {
  /**
   * Seconds since the user's last keyboard / mouse input, as reported by
   * the OS (GetLastInputInfo, CGEventSource, XScreenSaver). Useful to pause
   * sync or mute notifications when the user is away.
   */
  async getIdleTime(): Promise<number> {
    const { seconds } = await osCall<{ seconds: number }>("power", "getIdleTime")
    return seconds
  },
}
