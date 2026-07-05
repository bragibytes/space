# Creeps

Screeps for Rust — a sandbox colony MMO. Claim rooms, program your creeps in Rust (WASM), defend your empire. PvP only; no NPCs.

## Play (install in one command)

**macOS / Linux**

```bash
curl -fsSL https://raw.githubusercontent.com/bragibytes/creeps/main/scripts/install.sh | sh
realm
```

**Windows (PowerShell)**

```powershell
irm https://raw.githubusercontent.com/bragibytes/creeps/main/scripts/install.ps1 | iex
realm
```

That's it. The client auto-connects to the live server — no URLs, no config files.

## Program your creeps (local files)

Your colony AI lives in a normal Rust project on disk — edit in VS Code, Zed, or any editor:

```
~/.creeps/colony/
  Cargo.toml
  src/lib.rs      ← your code; save to auto-rebuild
```

First run of `cargo run -p realm-game` creates `~/.creeps/colony/` from the repo template. The game watches for saves, runs `cargo build --target wasm32-unknown-unknown`, and hot-reloads your WASM.

**Change the directory:** press **F2** in-game, or use the **Change…** button in the top bar. Your choice is saved for next launch.

Override with env: `CREEPS_COLONY_DIR=/any/path/you/want`

```bash
rustup target add wasm32-unknown-unknown
code ~/.creeps/colony   # or any path you set in F2
cargo run -p realm-game
```

1. Run `realm`
2. Type `register` to create a character (**warrior**, **mage**, or **rogue**), or `login` to return
3. Type `help` in-game for commands

**Tips**

| Situation | Command |
|-----------|---------|
| Simple scrollback terminal | `realm --plain` |
| Check version | `realm --version` |
| Local dev server | `REALM_SERVER=ws://localhost:4242/ws realm` |

Movement hotkeys: `n` `s` `e` `w` `l` `i` `h`

---

## Develop

```bash
cp .env.example .env   # DATABASE_URL for local server

cargo run -p realm-server
cargo run -p realm-client
```

Set `REALM_SERVER=ws://localhost:4242/ws` in `.env` when testing against a local server.

## Client UI

The default client is a **full-screen terminal game interface**:

- Persistent **HP / MP / XP** status bar
- Scrollable **world log** with color-coded messages
- **Location sidebar** — room info, entities, zone map, online players
- **Movement hotkeys** — `n` `s` `e` `w` `l` `i` `h` (no need to type full commands)
- **Combat flash** — status bar pulses on hits
- **Auto-reconnect** — up to 5 retries if connection drops

## Commands

| Category | Commands |
|----------|----------|
| Movement | `north/south/east/west` or `n/s/e/w` |
| Look | `look` / `l` |
| Combat | `attack <target>`, `ability`, `special` (Lv.5+) |
| PvP | Open PvP outside town; `duel <player>`, `duel accept <player>` |
| Items | `get`, `drop`, `inventory`, `equip`, `use`, `buy`, `craft` |
| Social | `say`, `yell`, `whisper`, `party invite/join/leave/say`, `p <msg>` |
| Trade | `trade <player>`, `trade accept`, `trade offer`, `trade confirm` |
| Quests | `talk <npc>`, `accept <id>`, `complete <id>`, `quest` |
| Info | `stats`, `who`, `help`, `rest`, `quit` |

## World

| Zone | Content |
|------|---------|
| **Eldermoor** | Safe town — shops, tavern, smith, quests (PvP disabled) |
| **Whispering Woods** | Goblins, wolves, goblin chief, shrine vault |
| **Ironspine Mountains** | Bandits, cave troll, crystal golem (east from North Gate) |

## Multiplayer Features

- **Parties** — `party invite <player>`, shared XP in the same room
- **Trading** — secure two-player item/gold exchange
- **Duels** — consented PvP (works even in town once accepted)
- **Online list** — sidebar shows who's playing and where

## Crafting

Visit **Greta the Smith** at the North Gate:

```
craft                    # list recipes
craft craft_leather      # wolf pelts → leather armor
craft craft_iron_sword   # goblin ears → iron sword
```

## Release client binaries

Tag a release to build installable binaries for macOS, Linux, and Windows:

```bash
git tag v0.2.0 && git push origin v0.2.0
```

GitHub Actions uploads assets to [Releases](https://github.com/bragibytes/creeps/releases). The install scripts download from there automatically.

## Deploy (Railway)

1. Add **Postgres** plugin to the Railway project (`DATABASE_URL` is injected automatically).
2. Deploy the server service (`railway.toml` builds `realm-server`).
3. Set `ADMIN_USERS` in Railway variables.
4. Redeploy so `/config` is live (client auto-discovers the public `wss://` URL).

5. Run `cargo run -p realm-client` — it connects automatically.

Player and guild data live in **Postgres**, not the filesystem. No volume required for saves.

## Admin

Set `ADMIN_USERS=yourname` in `.env`, then:

```
admin teleport <room_id>
admin spawn <mob_id>
admin setlevel <n>
admin reload
```

## Configuration

| Variable | Default | Description |
|----------|---------|-------------|
| `PORT` | `4242` | Server WebSocket port |
| `REALM_SERVER` | `ws://localhost:4242/ws` | Client WebSocket URL (set in `.env`) |
| `DATABASE_URL` | — | **Required** — Postgres connection string |
| `ADMIN_USERS` | — | Comma-separated admin usernames |
| `REALM_PLAIN` | — | Force plain client mode |
| `WORLD_PATH` | `data/world.json` | World definition file |

## Development

```bash
cargo build
cargo run -p realm-server
cargo run -p realm-client
cargo test
```

### Workspace layout

```
crates/
  realm-protocol/   # shared WebSocket message types
  realm-core/       # game logic (combat, quests, world, etc.)
  realm-server/     # axum WebSocket server
  realm-client/     # ratatui TUI + plain CLI client
```

Player and guild data persist in **Postgres**. Online sessions auto-save every 30 seconds while connected.