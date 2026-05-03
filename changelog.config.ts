/**
 * `changelogen` config — see CLI/SDK changelog.config.ts for usage.
 * Triggered manually (`bunx changelogen --release`) or from CI on tag push.
 */
export default {
  repo: "tynd-apps/tynd",
  types: {
    feat: { title: "Features" },
    perf: { title: "Performance" },
    fix: { title: "Bug Fixes" },
    refactor: { title: "Refactors" },
    docs: { title: "Documentation" },
    build: { title: "Build" },
    types: { title: "Types" },
    chore: { title: "Chore", semver: "patch" as const },
  },
  excludeAuthors: [] as string[],
};
