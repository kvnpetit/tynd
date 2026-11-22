/** Pure helpers for updater channel names. */

/** Channel used when neither the config nor `setChannel()` named one. */
export const DEFAULT_UPDATER_CHANNEL = "stable";

const MAX_CHANNEL_LENGTH = 64;
const CHANNEL_PATTERN = /^[A-Za-z0-9_-][A-Za-z0-9._-]*$/;

/** Throws when `name` is not a valid channel name (mirrors the host check). */
export function assertValidChannel(name: string): void {
  if (typeof name !== "string" || name.length === 0) {
    throw new Error("updater: channel must be a non-empty string");
  }
  if (name.length > MAX_CHANNEL_LENGTH) {
    throw new Error(`updater: channel exceeds ${MAX_CHANNEL_LENGTH} characters`);
  }
  if (!CHANNEL_PATTERN.test(name)) {
    throw new Error(
      `updater: invalid channel '${name}'; use letters, digits, '.', '_' or '-' (not starting with '.')`,
    );
  }
}
