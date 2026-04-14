// Public types — barrel re-export. Keeping a single `./types` entry point
// preserves stable imports across the codebase even as sub-files evolve.

export * from "./rpc";
export * from "./config";
export * from "./dialogs";
export * from "./menus";
export * from "./hardware";
export * from "./ipc-protocol";
