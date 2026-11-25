# Language Guide

## 1. File Directives

### Syntax
```pstdr
@addon(target = "<bedrock_version>", api = "<script_api_version>")
bind_target("<relative_bp_json_path>")
```

- `@addon(...)` sets the target Bedrock version and `@minecraft/server` API version (optional; defaults to `26.60` / `2.11.0`; can also be set workspace-wide in `pstdr.toml`). Only one `@addon` directive is allowed per file.
- `bind_target(...)` links a `BP/items/*.json` or `BP/blocks/*.json` file so `pstdr-c` wires custom components and validates the JSON `"identifier"`.

### Multi-Target `ApiProfile` (`Modern 2.x` vs `Legacy 1.x LTS`)

Versions below `1.15.0` are rejected (`PSTDR-E016`).

| Feature | Modern 2.x (`2.0.0`–`2.11.x`, default) | Legacy 1.x LTS (`1.15.0`–`1.19.x`) |
| :--- | :--- | :--- |
| Default Target / Manifest | `26.60` (`[1, 26, 60]`), JSON `1.21.90` | `1.21.50` (`[1, 21, 50]`), JSON `1.21.50` |
| Component Registration | `system.beforeEvents.startup` | `world.beforeEvents.worldInitialize` |
| `command` / `run_fn` / `.run_command()` | `.runCommand(...)` | `.runCommandAsync(...)` |
| `on cmd` (`customCommandRegistry`) | **Supported (Stable)** | **Rejected (`PSTDR-E016`)** — use `on chat` |
| `on chat` (`chatSend`) | **Beta-only (`2.11.0-beta`, `PSTDR-E018`)** | **Stable** (no `-beta` required) |
| `@minecraft/server-ui` (conditional) | `"2.0.0"` | `"1.3.0"` |

### Example
```pstdr
@addon(target = "26.60", api = "2.11.0")
bind_target("BP/items/wand.json")

on join of player {
    send "Ready" to player
}
```

---

## 2. Variables and Persistent State

### Syntax
- `const <name> = <expr>` — Immutable binding.
- `mut <name> = <expr>` — Mutable binding (supports `=`, `+=`, `-=`, `*=`, `/=`).
- `state <name>[<owner>] = <default_expr>` — Per-entity Dynamic Property (`pstdr:<name>`).

### Example
```pstdr
const MAX_HITS = 5
mut hits = 0
state counter[player] = 0

on join of player {
    counter[player] += 1
}
```

### Compiled JS
```javascript
const MAX_HITS = 5;

let hits = 0;

function __get_counter(owner) { const v = owner.getDynamicProperty("pstdr:counter"); return v !== undefined ? v : (0); }
function __set_counter(owner, value) { owner.setDynamicProperty("pstdr:counter", value); }

world.afterEvents.playerSpawn.subscribe((ev) => {
    if (!ev.initialSpawn) return;
    const player = ev.player;
    __set_counter(player, __get_counter(player) + (1));
});
```

---

## 3. Primitives, Units, Collections, and Strings

| Feature | Syntax | Notes |
| :--- | :--- | :--- |
| Numbers & Booleans | `42`, `1.5`, `true`, `false`, `null` | `==` / `!=` with `null` emit JS `==` / `!=`; otherwise `===` / `!==`. |
| Durations | `50ms`, `3s`, `2m` | Converted to ticks (`50ms` = `1` tick, `1s` = `20` ticks, `1m` = `1200` ticks). **No `t` suffix** — use `50ms` for 1 tick. |
| Distances | `5b`, `5blocks` | Emitted as numeric block count. |
| Objects | `{ key = val }`, `{ "k": val }` | Key-value pairs separated by `,` (`=` or `:`). |
| Arrays | `[1, "a", true]` | Zero-indexed lists. |
| String Interpolation | `"Value: {expr}"` | Compiled to JS template literals `` `Value: ${expr}` ``. |

---

## 4. Control Flow

### Syntax
```pstdr
if <cond> { ... } else if <cond> { ... } else { ... }
for <var> in <iterable> { ... }
while <cond> { ... }
```

### Example
```pstdr
on join of player {
    mut score = 10
    if score >= 20 {
        send "Rank S" to player
    } else if score >= 10 {
        send "Rank A" to player
    } else {
        send "Rank B" to player
    }

    for p in world.players() {
        p.add_tag("checked")
    }

    mut i = 0
    while i < 3 {
        i += 1
    }
}
```

### Compiled JS
```javascript
world.afterEvents.playerSpawn.subscribe((ev) => {
    if (!ev.initialSpawn) return;
    const player = ev.player;
    let score = 10;
    if (score >= 20) {
        player.sendMessage("Rank S");
    } else if (score >= 10) {
        player.sendMessage("Rank A");
    } else {
        player.sendMessage("Rank B");
    }
    for (const p of world.getAllPlayers()) {
        p.addTag("checked");
    }
    let i = 0;
    while (i < 3) {
        i += 1;
    }
});
```

---

## 5. Operators and Predicates

- **Logical:** `and` (`&&`), `or` (`||`), `not` (`!`)
- **Null-safety & Ternary:** `?.` (optional chaining), `??` (nullish coalescing), `cond ? a : b`
- **Membership & Proximity:** `x in container`, `a within b` (checks Euclidean distance `<= 5` blocks)
- **Entity Predicates (`is` / `is not`):** `sneaking`, `sprinting`, `swimming`, `flying`, `sleeping`, `on_ground` (`on ground`), `in_water` (`in water`), `holding <item_id>`

### Example
```pstdr
on hit target with "example:wand" by player {
    const label = target.name_tag ?? "Mob"
    const held_id = player.mainhand?.id
    const tier = target.hp > 10 ? "boss" : "normal"
    if player is sneaking and not (target is in_water) {
        if target within player and held_id in ["example:wand", "minecraft:stick"] {
            send "{label} ({tier})" to player
        }
    }
}
```

### Compiled JS
```javascript
world.afterEvents.entityHitEntity.subscribe((ev) => {
    if (ev.damagingEntity.typeId !== "minecraft:player") return;
    const player = ev.damagingEntity;
    const target = ev.hitEntity;
    const __held = player.getComponent("minecraft:equippable")?.getEquipment(EquipmentSlot.Mainhand);
    if (!__held || __held.typeId !== ("example:wand")) return;
    const label = target.nameTag ?? "Mob";
    const held_id = player.getComponent("minecraft:equippable")?.getEquipment(EquipmentSlot.Mainhand)?.typeId;
    const tier = target.getComponent("minecraft:health")?.currentValue > 10 ? "boss" : "normal";
    if (player.isSneaking && !(target.isInWater)) {
        if (__pstdr_within(target, player) && __pstdr_in(held_id, ["example:wand", "minecraft:stick"])) {
            player.sendMessage(`${label} (${tier})`);
        }
    }
});
```

---

## 6. Functions (`func`) and Raw JS Escape Hatch (`unsafe`)

- `func name(p1, p2) { ... }` declares a validated `.pstdr` function.
- `unsafe name(p1, p2) { ... }` or `unsafe const/mut/var/let x = ...;` isolates raw JavaScript/TypeScript at the top level. `unsafe` cannot be used inside blocks or as an expression (`PSTDR-E006`).

> **What `unsafe` bypasses:** Code inside an `unsafe` declaration is emitted verbatim as raw JavaScript/TypeScript, intentionally bypassing `.pstdr` static checks such as vanilla effect/sound ID validation (`PSTDR-E013`) and automatic `beforeEvents` `system.run(...)` wrapping. Keep `unsafe` helpers small, single-purpose, and declared at the top level so the rest of your add-on stays protected by safe-mode guarantees.

### Example
```pstdr
func boost(p) {
    apply speed 1 to p for 3s
}

unsafe get_tps_note() {
    return "raw_js_helper";
}

on join of player {
    boost(player)
    const note = get_tps_note()
    send "{note}" to player
}
```

### Compiled JS
```javascript
function boost(p) {
    p.addEffect("speed", 60, { amplifier: 0 });
}

// [unsafe:get_tps_note @ func_unsafe.pstdr:5]
function get_tps_note() {
    return "raw_js_helper";
}

world.afterEvents.playerSpawn.subscribe((ev) => {
    if (!ev.initialSpawn) return;
    const player = ev.player;
    boost(player);
    const note = get_tps_note();
    player.sendMessage(`${note}`);
});
```

---

## 7. Inline `mcfunction` Blocks (Wave 3A)

Top-level `mcfunction "<relative/path>" { ... }` blocks let you author Bedrock `.mcfunction` files directly inside `.pstdr` sources. During `pstdr-c convert --workspace`, each block is emitted to `BP/functions/<relative/path>.mcfunction` (with `//` comments stripped) and emits no JavaScript.

```pstdr
@addon(target = "26.60", api = "2.11.0")

mcfunction "arena/reset" {
    // Line comments are stripped in the generated .mcfunction
    effect @s clear
    tp @s 0 64 0
}

on join of player {
    run_fn on player "arena/reset"
}
```

- Paths must be non-empty relative paths without `..` or leading slashes (`PSTDR-E009`).
- Inline `mcfunction` declarations are automatically recognized when validating `run_fn` targets (`PSTDR-E017`).

---

## 8. Forbidden Keywords in Safe Mode

Using the following keywords or operators outside `unsafe` triggers **`PSTDR-E001`**:

`class`, `extends`, `super`, `new`, `this`, `constructor`, `var`, `let`, `function`, `nil`, `then`, `end`, `import`, `export`, `require`, `eval`, `~=`, `..`

Single-quoted (`'...'`) and backtick (`` `...` ``) strings are also restricted to `unsafe` blocks (`PSTDR-E016`).

---

> [← Previous: Quick Start](./pstdr-03-Quick-Start.md) | [Next: Events and Actions →](./pstdr-05-Events-and-Actions.md)
