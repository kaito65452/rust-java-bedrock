# Quick Start

## Workspace Structure

A `.pstdr` workspace contains:

```text
example-pack/
├── pstdr.toml               # Pack and compiler configuration
├── src/                     # .pstdr source files
├── BP/
│   ├── manifest.json        # Behavior Pack manifest (data + script modules)
│   ├── items/               # Item JSON definitions
│   ├── blocks/              # Block JSON definitions
│   ├── functions/           # .mcfunction files (hand-written or emitted from mcfunction blocks)
│   └── scripts/
│       ├── main.js          # Auto-generated entry point
│       └── generated/       # Compiled .js modules
└── build/ts/                # Emitted .ts files (when emit_ts = true)
```

---

## Step 1: Initialize the Workspace

```sh
mkdir example-pack && cd example-pack
pstdr-c init "Example Pack"
```

This generates `pstdr.toml`, `BP/manifest.json` (with `min_engine_version: [1, 26, 60]`, `data` module, `script` module, and `@minecraft/server 2.11.0`), and a starter `src/welcome.pstdr`.

---

## Step 2: Add a Player Join Script

Create `src/hello.pstdr`:

```pstdr
@addon(target = "26.60", api = "2.11.0")

on join of player {
    send "Welcome, {player.name_tag}!" to player
}
```

---

## Step 3: Add a Bound Item

Create `BP/items/wand.json` using a valid vanilla icon alias (`"blaze_rod"`):

```json
{
  "format_version": "1.21.90",
  "minecraft:item": {
    "description": {
      "identifier": "example:wand"
    },
    "components": {
      "minecraft:icon": "blaze_rod"
    }
  }
}
```

Create `src/wand.pstdr`:

```pstdr
@addon(target = "26.60", api = "2.11.0")
bind_target("BP/items/wand.json")

const WAND_ID = "example:wand"

on right click with WAND_ID by player {
    apply speed 2 to player for 5s
    player.actionbar("Boost active")
}
```

---

## Step 4: Build (Check, Convert, and Audit)

Run the full pipeline in one command:

```sh
pstdr-c build --workspace
```

Or run each stage individually:

```sh
pstdr-c check --workspace
pstdr-c convert --workspace
pstdr-c audit --workspace
```

### Expected Output

- `pstdr-c build --workspace` → `Build succeeded (check -> convert -> audit): 0 errors.`
- `pstdr-c check --workspace` → `Check passed: 0 errors.`
- `pstdr-c convert --workspace` → `Workspace conversion succeeded -> BP/scripts/generated/ & build/ts/`
- `pstdr-c audit --workspace` → `Audit Gate passed: all scripts and manifests are valid.`

### Generated Files

`BP/items/wand.json` is wired automatically for Custom Components V2 (`format_version >= 1.21.90`):

```json
{
  "format_version": "1.21.90",
  "minecraft:item": {
    "components": {
      "minecraft:icon": "blaze_rod",
      "pstdr:wand": {}
    },
    "description": {
      "identifier": "example:wand"
    }
  }
}
```

`BP/scripts/main.js` imports all compiled modules:

```javascript
// Auto-generated entry point by pstdr-c
import "./generated/hello.js";
import "./generated/wand.js";
import "./generated/welcome.js";
```

---

## Multi-Target Partitioning (Single File → Multiple Modules)

You can define logic for multiple custom items, custom blocks, and world events inside a **single** `.pstdr` file (for example, `src/pack_logic.pstdr`). During `pstdr-c convert --workspace`, `pstdr-c` scans `BP/items/**/*.json` and `BP/blocks/**/*.json`, partitions each target into isolated modules, wires every matching JSON file, and cleans up stale orphan `.js` files automatically:

```pstdr
@addon(target = "26.60", api = "2.11.0")

const WAND_ID = "example:wand"
const ALTAR_ID = "example:altar"
mut uses = 0

on right click with WAND_ID by player {
    uses += 1
    player.actionbar("Wand uses: {uses}")
}

on interact of ALTAR_ID by player {
    send "Altar activated" to player
}

on join of player {
    send "Welcome!" to player
}
```

When `BP/items/wand.json` (`example:wand`) and `BP/blocks/altar.json` (`example:altar`) exist in the workspace, `pstdr-c convert --workspace` emits:
- `BP/scripts/generated/_shared/pack_logic.shared.js` — shared `const`, `state`, `func`, and `export const __pstdr_mut = { uses: 0 };`
- `BP/scripts/generated/items/wand.js` — merged item custom component (`onUse`, `onHitEntity`, `onConsume`)
- `BP/scripts/generated/blocks/altar.js` — merged block custom component (`onPlayerInteract`, `onTick`, `onPlayerBreak`)
- `BP/scripts/generated/commands/<stem>.js` — custom slash commands (`on cmd`) registered via `customCommandRegistry`
- `BP/scripts/generated/world/pack_logic.js` — global events (`on join`, timers, unbound or cancellable `beforeEvents`)
- `BP/scripts/generated/_registry.js` — single centralized `system.beforeEvents.startup` (`Modern 2.x`) or `world.beforeEvents.worldInitialize` (`Legacy 1.x LTS`) registration callback imported by `BP/scripts/main.js`

For a full playable single-file example tested in-game on Minecraft Bedrock 26.60, see **[Zombie Rush](https://github.com/kaito65452/pstdr-dsl/blob/main/docs/zombie-rush-example/README.md)**.

---

> [← Previous: Installation](./pstdr-02-Installation.md) | [Next: Language Guide →](./pstdr-04-Language-Guide.md)
