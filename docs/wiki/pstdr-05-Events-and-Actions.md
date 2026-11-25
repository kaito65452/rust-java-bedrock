# Events and Actions

## Supported Events (Wave 1–3C)

| Event | Syntax | Scope Variables | Bedrock Bus / Component |
| :--- | :--- | :--- | :--- |
| **Custom Command** *(Wave 3B)* | `on cmd "<ns>:<name>" description "<desc>" [permission any\|gamemakers\|admin\|host] [(<p>: <type>, <opt>?: <type>)] [by <actor>] { ... }` | params, `[<actor>]` | `system.beforeEvents.startup` -> `customCommandRegistry` (**Stable `2.x`**; rejected on `1.x`) |
| **Chat Send** | `on chat <msg> by <actor> { ... }` | `<msg>`, `<actor>` | `world.beforeEvents.chatSend` (**`2.x` Beta-only `E018`**, **`1.x` Stable**) |
| **Player Join** | `on join of <actor> { ... }` | `<actor>` | `world.afterEvents.playerSpawn` (`ev.initialSpawn`) |
| **Player Leave** | `on leave of <actor> { ... }` | `<actor>` | `world.beforeEvents.playerLeave` (auto `system.run` for mutations) |
| **Dimension Change** *(Wave 3B)* | `on dimension_change of <player> { ... }` | `<player>`, `from_dimension`, `to_dimension`, `from_location`, `to_location` | `world.afterEvents.playerDimensionChange` |
| **Right Click** | `on right click with <item> by <actor> { ... }` | `<actor>` | `afterEvents.itemUse` / `beforeEvents.itemUse` (or item component `onUse`) |
| **Hit Entity** | `on hit <target> with <item> by <actor> { ... }` | `<target>`, `<actor>` | `world.afterEvents.entityHitEntity` (or item component `onHitEntity`) |
| **Consume Item** | `on consume <item> by <actor> { ... }` | `<actor>` | `world.afterEvents.itemCompleteUse` (or item component `onConsume`) |
| **Item Release** *(Wave 3B)* | `on item_release with <item> by <actor> { ... }` | `<actor>`, `item`, `use_duration` | `world.afterEvents.itemReleaseUse` |
| **Break Block** | `on break of <block_id> by <actor> { ... }` | `<actor>`, `block` | `afterEvents.playerBreakBlock` / `beforeEvents.playerBreakBlock` (or block component `onPlayerBreak`) |
| **Place Block** *(Wave 3B)* | `on place of <block> by <actor> { ... }` | `<block>`, `<actor>` | `afterEvents.playerPlaceBlock` / `beforeEvents.playerPlaceBlock` (Cancellable) |
| **Block Interact** | `on interact of <block_id> by <actor> { ... }` | `<actor>`, `block` | Block component `onPlayerInteract` (or `afterEvents`/`beforeEvents.playerInteractWithBlock`) |
| **Block Tick** | `on tick of <block_id> { ... }` | `block`, `dimension` | Custom block component `onTick` (requires matching `BP/blocks/*.json`) |
| **Interact Entity** *(Wave 3B)* | `on interact_entity of <target> by <actor> { ... }` | `<target>`, `<actor>` | `afterEvents.playerInteractWithEntity` / `beforeEvents.playerInteractWithEntity` (Cancellable) |
| **Entity Hurt** | `on hurt <target> [by <attacker>] { ... }` | `<target>`, `[<attacker>]` | `world.afterEvents.entityHurt` |
| **Entity Die** | `on die of <victim> [by <killer>] { ... }` | `<victim>`, `[<killer>]` | `world.afterEvents.entityDie` |
| **Health Change** *(Wave 3B)* | `on health_change of <entity> { ... }` | `<entity>`, `old_value`, `new_value` | `world.afterEvents.entityHealthChanged` |
| **Projectile Hit** | `on projectile hit <victim> [with <proj_id>] [by <shooter>] { ... }` | `<victim>`, `[<shooter>]`, `projectile` | `world.afterEvents.projectileHitEntity` |
| **Projectile Hit Block** *(Wave 3B)* | `on projectile_hit_block of <block> [by <shooter>] { ... }` | `<block>`, `projectile`, `<shooter>` (default `shooter`) | `world.afterEvents.projectileHitBlock` |
| **Entity Hit Block** *(Wave 3B)* | `on entity_hit_block of <block> by <attacker> { ... }` | `<block>`, `<attacker>` | `world.afterEvents.entityHitBlock` |
| **Explosion** *(Wave 3B)* | `on explosion { ... }` | `source`, `impacted_blocks` | `afterEvents.explosion` / `beforeEvents.explosion` (Cancellable) |
| **Entity Spawn** | `on spawn of <entity> [type <type_id>] { ... }` | `<entity>` | `world.afterEvents.entitySpawn` |
| **Entity Load** *(Wave 3B)* | `on entity_load of <entity> { ... }` | `<entity>` | `world.afterEvents.entityLoad` |
| **Entity Remove** *(Wave 3B)* | `on entity_remove of <entity> { ... }` | `<entity>` | `world.afterEvents.entityRemove` |
| **Script Event** *(Wave 3A)* | `on script_event "<namespace>:<id>" { ... }` | `event`, `source`, `source_block` | `system.afterEvents.scriptEventReceive` (namespace + ID filter) |
| **Entity Trigger** *(Wave 3A)* | `on entity_trigger "<event_id>" [for "<entity_type>"] { ... }` | `entity`, `event_id` | `world.afterEvents.dataDrivenEntityTrigger` |

> **Automatic `beforeEvents` Read-Only Safety:** `on leave` and `on chat` always subscribe to `world.beforeEvents` (as do `on right click`, `on break`, `on place`, `on interact`, `on interact_entity`, and `on explosion` when they contain `cancel event`). World-mutating actions inside `beforeEvents` are automatically wrapped in `system.run(() => { ... })`.
>
> **Stable vs. Beta APIs (`PSTDR-E018`):** On Modern `2.x`, use **`on cmd`** for Stable slash commands; `on chat` requires `api = "2.11.0-beta"` (+ Beta APIs toggle). On Legacy `1.x` LTS (`api = "1.16.0"`), `on chat` is Stable and `on cmd` is rejected (`PSTDR-E016`).

### Core & Combat Events Example

```pstdr
@addon(target = "26.60", api = "2.11.0")

const WAND_ID = "example:wand"
const POTION_ID = "example:potion"
const SPEAR_ID = "example:spear"
const ORE_ID = "example:ore"

on join of player {
    send "Welcome, {player.name_tag}" to player
    player.give_item("minecraft:blaze_rod", 1, "Starter Wand")
    player.set_score("kills", 0)
}

on leave of player {
    const leaving_name = player.name
    send "{leaving_name} left the arena" to world
}

on right click with WAND_ID by player {
    player.actionbar("Activated")
}

on hit target with WAND_ID by player {
    damage target by 4
    apply slowness 2 to target for 3s
}

on consume POTION_ID by player {
    apply strength 2 to player for 5s
    spawn particle "minecraft:heart_particle" at player
}

on break of ORE_ID by player {
    cancel event
    player.actionbar("Protected block")
}

on interact of "minecraft:tnt" by player {
    cancel event
    send "TNT is disabled!" to player
}

on hurt target by attacker {
    if attacker != null {
        spawn particle "minecraft:totem_particle" at target
    }
}

on die of victim by killer {
    if killer != null {
        killer.play_sound("random.orb")
    }
}

on projectile hit victim with SPEAR_ID by shooter {
    damage victim by 6
    if shooter != null {
        send "Direct hit with {projectile.typeId}!" to shooter
    }
}

on spawn of mob type "minecraft:zombie" {
    apply strength 1 to mob for 10s
}
```

### Chat Event (`on chat` — Beta API Only)

`on chat` requires a `-beta` `@minecraft/server` API version and the **Beta APIs** toggle enabled in world settings:

```pstdr
@addon(target = "26.60", api = "2.11.0-beta")

on chat msg by player {
    if msg == "!kit" {
        cancel event
        player.give_item("minecraft:golden_apple", 2)
        player.add_score("kits", 1)
        const total = player.get_score("kits")
        send "Kit granted (total: {total})" to player
    }
}
```

### Custom Block Events (`on interact` & `on tick`)

When bound via `bind_target("BP/blocks/altar.json")` (or auto-discovered in `BP/blocks/` during `convert --workspace`), non-cancellable `on interact` and `on tick` compile into custom block component callbacks:

```pstdr
@addon(target = "26.60", api = "2.11.0")
bind_target("BP/blocks/altar.json")

const ALTAR_ID = "example:altar"

on interact of ALTAR_ID by player {
    send "Altar activated!" to player
}

on tick of ALTAR_ID {
    spawn particle "minecraft:villager_happy" at block.location in dimension
}
```

### Wave 3A: `mcfunction`, `command`, `run_fn`, `on script_event` & `on entity_trigger`

Use `command` and `run_fn` to execute Bedrock slash commands and `.mcfunction` files cleanly (automatically wrapped in `system.run(...)` inside `beforeEvents`), and bridge back from command blocks / behavior-pack entity events via `on script_event` and `on entity_trigger`:

```pstdr
@addon(target = "26.60", api = "2.11.0")

mcfunction "arena/reset" {
    effect @s clear
    clear @s
}

on script_event "my_pack:reset" {
    if source != null {
        run_fn on source "arena/reset"
        command on source "gamemode adventure @s"
        send "Arena reset ({event.message})" to source
    }
}

on entity_trigger "my_pack:enrage" for "minecraft:zombie" {
    apply strength 2 to entity for 10s
}
```

- `on script_event` requires a static string literal or `const` in `"<namespace>:<id>"` format (`PSTDR-E016`).
- `run_fn` validates that the target exists either as `BP/functions/<path>.mcfunction` or as an inline `mcfunction "<path>" { ... }` block (`PSTDR-E017`).

---

## Timers (`after` and `every`)

Timers work both at the top level and nested inside event blocks:
- `after <duration> { ... }` → compiles to `system.runTimeout(..., ticks)`.
- `every <duration> { ... }` → compiles to `system.runInterval(..., ticks)`. Minimum interval is **`50ms`** (`1` tick); smaller intervals fail with `PSTDR-E010`.

```pstdr
on join of player {
    after 2s {
        send "Delayed message" to player
    }
}

every 5s {
    for p in world.players() {
        p.add_tag("ticked")
    }
}
```

---

## Built-in Action Statements

| Action | Syntax | Notes |
| :--- | :--- | :--- |
| **Cancel Event** | `cancel event` | Valid only in `on right click`, `on break`, `on place`, `on interact`, `on interact_entity`, `on explosion`, and `on chat` (`PSTDR-E004` otherwise). |
| **Damage** | `damage <target> by <amount>` | Emits `<target>.applyDamage(<amount>)`. |
| **Ignite** | `ignite <target> for <duration>` | Emits `<target>.setOnFire(<seconds>, true)`. |
| **Apply Effect** | `apply <effect> <level> to <target> for <duration>` | `<level>` is 1-based (`PSTDR-E011`); `<effect>` is checked against vanilla spec (`PSTDR-E013`). |
| **Send Message** | `send <message> to <target>` | Emits `<target>.sendMessage(<message>)`. |
| **Spawn Particle** | `spawn particle <id> at <entity_or_loc> [in <dimension>]` | Emits `<dim>.spawnParticle(<id>, <loc>)`. Accepts an entity (`target`), `.location`, or `{ x = 0, y = 64, z = 0 }`. |
| **Run Command** *(Wave 3A)* | `command [on <target>] <cmd_expr>` | Emits `.runCommand(<cmd>)` (`2.x`) or `.runCommandAsync(<cmd>)` (`1.x` LTS); defaults to overworld. |
| **Run Function** *(Wave 3A)* | `run_fn [on <target>] <path_expr>` | Runs `function <path>` on `<target>` (or overworld). Validates `BP/functions/<path>.mcfunction` or inline `mcfunction` (`PSTDR-E017`). |
| **Native UI Forms** *(Wave 3B)* | `ui action\|message\|modal "<title>" [body "<text>"] to <player> { ... }` | Emits `ActionFormData` / `MessageFormData` / `ModalFormData` and conditionally injects `@minecraft/server-ui` (`"2.0.0"` on `2.x`, `"1.3.0"` on `1.x`). |
| **Native Scoreboards** *(Wave 3B)* | `score["<obj>"][<target>] = \| += \| -= <n>`, `scoreboard show\|hide ...` | Null-safe scoreboard indexing and `sidebar` / `list` / `belowname` display slots. |

---

## `snake_case` Mapping Reference

### Entity & Player Properties

| `.pstdr` | Bedrock API |
| :--- | :--- |
| `name_tag` | `nameTag` |
| `type_id` | `typeId` |
| `is_sneaking` | `isSneaking` |
| `is_sprinting` | `isSprinting` |
| `is_swimming` | `isSwimming` |
| `is_flying` | `isFlying` |
| `is_sleeping` | `isSleeping` |
| `is_on_ground` | `isOnGround` |
| `is_in_water` | `isInWater` |
| `is_climbing` | `isClimbing` |
| `is_falling` | `isFalling` |
| `is_valid` | `isValid` |
| `selected_slot` | `selectedSlotIndex` |
| `head_location` | `getHeadLocation()` |
| `view_direction` | `getViewDirection()` |
| `hp` | `getComponent("minecraft:health")?.currentValue` |
| `max_hp` | `getComponent("minecraft:health")?.effectiveMax` |
| `holding` / `mainhand` | `getComponent("minecraft:equippable")?.getEquipment(EquipmentSlot.Mainhand)` |

### ItemStack Properties (after `.mainhand` / `.holding`)

| `.pstdr` | Bedrock API |
| :--- | :--- |
| `id` / `type_id` | `typeId` |
| `name_tag` | `nameTag` |

### Entity, Player & Dimension Methods

| `.pstdr` | Bedrock API |
| :--- | :--- |
| `actionbar(msg)` | `onScreenDisplay.setActionBar(msg)` |
| `title(msg)` | `onScreenDisplay.setTitle(msg)` |
| `subtitle(msg)` | `onScreenDisplay.updateSubtitle(msg)` |
| `send_message(msg)` | `sendMessage(msg)` |
| `play_sound(id)` | `playSound(id)` *(sound ID checked against spec)* |
| `add_tag(tag)` / `remove_tag(tag)` / `has_tag(tag)` / `get_tags()` | `addTag` / `removeTag` / `hasTag` / `getTags` |
| `apply_damage(amount)` | `applyDamage(amount)` |
| `apply_impulse(vec)` | `applyImpulse(vec)` |
| `apply_knockback(x, z, h, v)` | `applyKnockback(x, z, h, v)` |
| `run_command(cmd)` | `runCommand(cmd)` |
| `add_effect(id, dur, opts)` / `remove_effect(id)` | `addEffect` / `removeEffect` |
| `set_on_fire(secs, ext)` / `extinguish_fire()` | `setOnFire` / `extinguishFire` |
| `spawn_particle(id, loc)` | `spawnParticle(id, loc)` |
| `spawn_entity(id, loc)` | `spawnEntity(id, loc)` |
| `spawn_item(item_id, loc_or_entity, amount?)` | `__pstdr_spawn_item(dim, itemId, loc, amount = 1)` *(auto-imports `ItemStack`)* |
| `give_item(item_id, amount?, name_tag?)` | `__pstdr_give_item(entity, itemId, amount = 1, nameTag = null)` *(auto-imports `ItemStack`)* |
| `get_score(objective)` | `__pstdr_get_score(target, objective)` *(null-safe, defaults to `0`)* |
| `set_score(objective, value)` | `__pstdr_set_score(target, objective, value)` *(auto-creates objective)* |
| `add_score(objective, delta)` | `__pstdr_add_score(target, objective, delta)` *(auto-creates objective)* |
| `get_entities(opts)` / `get_players(opts)` | `getEntities(opts)` / `getPlayers(opts)` |
| `teleport(loc)` | `teleport(loc)` |
| `kill()` | `kill()` |

### `snake_case` String & Array Helpers

| `.pstdr` | JavaScript |
| :--- | :--- |
| `.lower()` / `.upper()` / `.trim()` | `.toLowerCase()` / `.toUpperCase()` / `.trim()` |
| `.starts_with(s)` / `.ends_with(s)` / `.contains(x)` | `.startsWith(s)` / `.endsWith(s)` / `.includes(x)` |
| `.replace(a, b)` / `.split(sep)` / `.join(sep)` / `.push(x)` | `.replace(a, b)` / `.split(sep)` / `.join(sep)` / `.push(x)` |

### Global `world` & `system` Helpers

| `.pstdr` | Bedrock API |
| :--- | :--- |
| `world.players()` / `world.get_players(opts)` | `world.getAllPlayers()` / `world.getPlayers(opts)` |
| `world.send_message(msg)` | `world.sendMessage(msg)` |
| `world.dimension(id)` | `world.getDimension(id)` |
| `world.play_sound(id, loc)` | `world.playSound(id, loc)` |
| `world.get_absolute_time()` / `world.get_time_of_day()` | `world.getAbsoluteTime()` / `world.getTimeOfDay()` |
| `system.current_tick` | `system.currentTick` |
| `system.run_timeout(fn, t)` / `system.run_interval(fn, t)` | `system.runTimeout(fn, t)` / `system.runInterval(fn, t)` |

---

> [← Previous: Language Guide](./pstdr-04-Language-Guide.md) | [Next: CLI and Diagnostics →](./pstdr-06-CLI-and-Diagnostics.md)
