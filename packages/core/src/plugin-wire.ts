/**
 * Plugin wire format shared by `definePlugin` and the runtime registry:
 * the `$plugin$<name>$<method>` prefix and the name/method patterns.
 * Internal: not part of any public subpath.
 */

/** Wire prefix for plugin-invoked methods. See dispatch in `runtime/*.ts`. */
export const PLUGIN_WIRE_PREFIX = "$plugin$";
export const PLUGIN_NAME_PATTERN = /^[a-z][a-z0-9-]{0,30}$/;
export const PLUGIN_METHOD_PATTERN = /^[A-Za-z_][A-Za-z0-9_-]{0,63}$/;

export function assertPluginName(name: string, owner: string): void {
  if (!name) throw new Error(`${owner}: \`name\` is required`);
  if (!PLUGIN_NAME_PATTERN.test(name)) {
    throw new Error(
      `${owner}: plugin name "${name}" must match ^[a-z][a-z0-9-]{0,30}$ (canonical lowercase kebab-case)`,
    );
  }
}
