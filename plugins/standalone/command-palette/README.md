# @tynd/plugin-command-palette

Headless command-palette registry for Tynd apps — command store, fuzzy
search, open/close signal, default Cmd/Ctrl+K binding. You render the UI
with whatever framework you're already using; the plugin stays out of
the view layer.

## Install

```sh
tynd add command-palette
```

## Use

```ts
import {
  registerMany,
  search,
  run,
  bindShortcut,
  onOpenChange,
  isOpen,
  close,
} from "@tynd/plugin-command-palette"

registerMany([
  {
    id: "file.new",
    title: "New File",
    keywords: ["create", "empty"],
    hint: "Ctrl+N",
    action: () => createNewFile(),
  },
  {
    id: "view.toggle-sidebar",
    title: "Toggle Sidebar",
    group: "View",
    action: () => toggleSidebar(),
    when: () => !isFullscreen(),
  },
])

// Default Cmd+K / Ctrl+K
const off = bindShortcut()
onOpenChange((open) => {
  if (open) document.querySelector("#palette-root")!.classList.add("open")
  else document.querySelector("#palette-root")!.classList.remove("open")
})

// In your palette UI:
const results = search(query)      // ranked SearchResult[]
await run(results[0]!.id)          // fires action, closes palette
```

## API

| Export | Purpose |
|---|---|
| `register(cmd)` / `registerMany(cmds)` | Add or replace commands. |
| `unregister(id)` / `clear()` | Remove. |
| `list()` | Snapshot of registered commands. |
| `search(q, limit?)` | Ranked results. Empty query returns all visible. |
| `run(id)` | Execute + close. Returns `false` when the ID is unknown. |
| `open()` / `close()` / `toggle()` / `isOpen()` | Visibility control. |
| `onOpenChange(fn)` | Subscribe to open/close transitions. |
| `bindShortcut(opts?)` | Keyboard handler. Returns removal fn. |

## Scoring

`search()` ranks from tight to loose:

| Score | Match |
|---|---|
| 0 | Title equals query |
| 1 | Title starts with query |
| 2 | Title contains query |
| 3 | Subtitle or keyword contains query |
| 4 | Title matches query as a subsequence (type `gno` for "Go To Symbol") |

Non-matches are filtered out. Ties preserve registration order.
Good enough for under ~1000 commands; swap for something like `fuzzysort`
if you need extreme-scale ranking.

## Why headless

The palette is 80% UI concerns — animation, focus ring, list virtualization,
theming — that every app already solves differently. This plugin only owns
what stays the same: the registry, search, and keyboard trigger.

---

See also: [Standalone plugin index](../README.md).
