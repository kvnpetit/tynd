import { osCall } from "./_internal";

/**
 * `biometric` — Touch ID / Face ID / Windows Hello / polkit prompt.
 *
 * Backed by `robius-authentication`. `isAvailable()` reports whether the
 * platform support was compiled in; the canonical "is biometrics actually
 * usable?" signal is the result of {@link biometric.authenticate} —
 * platforms differ on whether enrolment can be probed without prompting.
 */
export const biometric = {
  /**
   * Whether biometric authentication is wired into this build. Static
   * `true` on supported platforms — the actual enrolment check happens
   * inside `authenticate`.
   */
  isAvailable(): Promise<boolean> {
    return osCall("biometric", "isAvailable", {});
  },
  /**
   * Prompt the user to authenticate. Resolves to `true` on success and
   * `false` when the user cancels or auth fails — never rejects on a
   * normal cancel flow.
   *
   * @param reason  Shown to the user — explain *why* the app is asking
   *                ("Unlock saved password", "Confirm purchase", …).
   * @param fallbackTitle  Title for the system PIN/password fallback UI
   *                       (Windows Hello, polkit). Defaults to
   *                       `"Authenticate"`.
   */
  authenticate(reason: string, fallbackTitle?: string): Promise<boolean> {
    return osCall("biometric", "authenticate", { reason, fallbackTitle });
  },
};
