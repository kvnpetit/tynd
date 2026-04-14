
export type Target =
  | "windows-x64" | "windows-arm64"
  | "linux-x64"   | "linux-arm64"
  | "macos-x64"   | "macos-arm64";

export type BuildTool  = "vite" | "cra" | "angular" | "parcel" | "rsbuild" | "webpack" | "none";
export type UILibrary  = "react" | "vue" | "svelte" | "solid" | "preact" | "angular" | "lit" | "qwik" | "vanilla";

export interface DetectedProject {
  buildTool: BuildTool;
  ui: UILibrary;
  outDir: string;
  name: string;
  hasPackageJson: boolean;
  hasBunviewConfig: boolean;
  serverFramework: string | null;
}
