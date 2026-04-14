import { createApp } from "bunview";
import * as commands from "./commands";
import { cleanup } from "./internal";

export type AppCommands = typeof commands;

const app = createApp({
  entry: "./dist",
  commands,
  window: { title: "react-app", width: 1024, height: 768 },
});

app.onReady(() => console.log("[react-app] window ready"));
app.onClose(() => { cleanup(); console.log("[react-app] window closed"); });

await app.run();
