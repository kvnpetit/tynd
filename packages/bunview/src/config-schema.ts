import { z } from "zod";

// Shared Zod schemas used by both the CLI (build time) and the runtime (createApp).

const SigningWindowsSchema = z.object({
  certificate:  z.string(),
  password:     z.string().optional(),
  timestampUrl: z.string().optional(),
  description:  z.string().optional(),
});

const SigningMacosSchema = z.object({
  identity:        z.string(),
  entitlements:    z.string().optional(),
  hardenedRuntime: z.boolean().optional(),
  notarize: z.object({
    appleId:  z.string(),
    teamId:   z.string(),
    password: z.string(),
  }).optional(),
});

const FileAssociationSchema = z.object({
  ext:       z.string().regex(/^[a-z0-9]+$/i, "extension: letters/digits only, no dot"),
  name:      z.string(),
  handler:   z.string(),
  mimeType:  z.string().optional(),
  role:      z.enum(["Editor", "Viewer"]).optional(),
});

const UrlSchemeSchema = z.object({
  name:    z.string().regex(/^[a-z][a-z0-9+\-.]*$/i, "URL scheme must match RFC 3986"),
  handler: z.string().optional(),
});

const SingleInstanceSchema = z.union([
  z.boolean(),
  z.object({ enabled: z.literal(true), handler: z.string().optional() }),
]);

const WindowConfigSchema = z.object({
  title:       z.string().optional(),
  width:       z.number().positive().optional(),
  height:      z.number().positive().optional(),
  resizable:   z.boolean().optional(),
  debug:       z.boolean().optional(),
  frameless:   z.boolean().optional(),
  transparent: z.boolean().optional(),
  icon:        z.string().optional(),
  vibrancy:    z.enum(["mica", "acrylic", "tabbed", "dark", "light", "none"]).optional(),
  showMinimizeButton: z.boolean().optional(),
  showMaximizeButton: z.boolean().optional(),
  showCloseButton:    z.boolean().optional(),
  decorations: z.boolean().optional(),
  shadow:      z.boolean().optional(),
  backgroundColor: z.object({
    r: z.number(), g: z.number(), b: z.number(), a: z.number(),
  }).optional(),
  titleBarStyle: z.enum(["visible", "transparent", "overlay", "hidden"]).optional(),
  hardwareAcceleration: z.boolean().optional(),
}).passthrough();

export const BunviewConfigSchema = z.object({
  entry:    z.string(),
  frontend: z.string().optional(),
  icon:     z.string().optional(),
  name:     z.string().optional(),
  outDir:   z.string().optional(),
  window:   WindowConfigSchema.optional(),
  windowState:     z.boolean().optional(),
  urlScheme:       UrlSchemeSchema.optional(),
  fileAssociations: z.array(FileAssociationSchema).optional(),
  singleInstance:  SingleInstanceSchema.optional(),
  dev: z.object({ url: z.string(), command: z.string() }).optional(),
  codeSigning: z.object({
    windows: SigningWindowsSchema.optional(),
    macos:   SigningMacosSchema.optional(),
  }).optional(),
}).passthrough();
