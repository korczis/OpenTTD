# 07 — Resource Catalog

Resources implemented in this build. All are read-only, wrapped in the common envelope
(`schema_version`, `source`, `source_revision`, `protocol_version`, `game_tick`, `game_date`,
`game_mode`, `availability`, `redactions`, `truncated`, `next_cursor`, `data`) built by
`MakeEnvelope()` in `prismatic_mcp_server.cpp`. Order in `resources/list` is fixed (deterministic).

| URI | `data` payload | Source |
|---|---|---|
| `openttd://mcp/status` | running, endpoint, bind, port, read_only, capabilities {reads, approval_gated_commands, script_api, ai_tools, pause, toolbar_menu, interface_input}, input_armed, requests_total, requests_denied, active_connections | server state; the capability fields derived from `tools/list`, `resources/list` and the process (below) |
| `openttd://meta/build` | openttd_version, revision_hash, newgrf_version, research_instrumentation, mcp_server, dedicated | `rev.h` globals |
| `openttd://game/state` | paused, map_width, map_height, num_companies, num_vehicles (pool items: wagons, parts, effects), fleet (primary vehicles: train, road, ship, aircraft, total), num_towns | `_pause_mode`, `Map`, pool counts, `IsPrimaryVehicle()` |
| `openttd://companies` | array of {id, name, custom_name, is_ai, money, loan, max_loan, loan_step, company_value, money_unit, inaugurated_year, fleet {train, road, ship, aircraft, total}} | `Company::Iterate()`, primary vehicles by owner |
| `openttd://map/terrain` | width, height, encoding; per row (y) strings, a character per tile (x): `slopes` (2 hex: Slope bits, steep 0x10), `ground` (g grass, b bare, r rough, k rocks, f fields, s snow, d desert; trees G R D C; water w n v c; `-` built), `gradients` (largest height step to a neighbour), `buildable` (`.` `t` `w` `x`), `vehicles` (`.` 1-9 `+`); `row_digests` (FNV-1a per row) | `GetTileSlope`, `GetClearGround`, `GetTreeGround`, `GetWaterClass`, `TileHeight`, `Vehicle::Iterate()` (I0073) |
| `openttd://ai` | `companies` [{company, running, ai, version, info, settings [{name, description, min, max, step, default, flags, labels, value, editable, control {kind, options?, picker?, table \| group}}]}], `available`, `can_start_new`, `ai_in_multiplayer`, `network_client` | `AIConfig`, `AIInfo`; `control` derived by `prismatic_mcp_ai_control.h` (README "AI settings and the control that edits each") |
| `openttd://vehicles` | array of primary vehicles {id, unit, type, owner, engine, engine_id, tile, x, y, z, speed, stopped, in_depot, crashed, lost (the pathfinder found no way: `VehicleFlag::PathfinderLost`), broken_down (stopped by a breakdown now: `breakdown_ctr == 1`), cargo_capacity, cargo_count, parts, consist_capacity, consist_count, profit_this_year, profit_last_year, age, max_age, reliability, breakdowns, value (the front vehicle's), consist_value (the whole consist's), current_order, orders [{type, destination}]} | `Vehicle::Iterate()`, `IsPrimaryVehicle()`, the whole consist summed (`prismatic_mcp_entities.cpp`) |
| `openttd://ai/status` | `companies` [{company, running, ai, source, fields, line, lines_since}], `network_client`: per AI company its newest script log line of the form `<source> heartbeat: key=value ...` (PrismaticAI logs one every heartbeat, `main.nut`), every token split into `fields` (an integer when the value is one, else its text, so `routes=5/8` is `"5/8"`); no field name is known to the server. `lines_since` counts the log lines after it; `fields`, `line`, `source` are null when the log holds none (and on a network client, where no AI runs). The line carries no tick of its own: the envelope's `game_tick` is when it was read, a `date` field is the AI's own | `ScriptInstance::GetLogData()` (the 400-line buffer the AI debug window shows), `ai_status::ParseHeartbeat` (`prismatic_mcp_ai_status.cpp`) |
| `openttd://engines` | array of enabled engines {id, type, name, intro_year, max_speed_kmh, effective_speed_kmh, capacity, cargo, price, running_cost_per_year, reliability_pct, buildable_by}; trains add wagon, power_hp and railtypes (labels); aircraft add plane_type | `Engine::Iterate()`, `IsEngineBuildable`, `RailVehicleInfo` (`prismatic_mcp_world.cpp`) |
| `openttd://economy/prices` | `prices` {name: money now}, `order` (enum order), `base` {name: start price}, `inflation`, `inflation_factor`, `construction_cost`, `unit` | `_price` (inflated, setting and NewGRF multipliers applied), `_price_base_specs`; names generated from `enum class Price` with a `static_assert` (I0073) |
| `openttd://railtypes` | `types` [{railtype, label, name, build_cost (RailBuildCost now), cost_multiplier, maintenance_multiplier, max_speed, curve_speed, introduction_date, powered, compatible, available_to}] | `GetRailTypeInfo`, `RailBuildCost`, `Company::avail_railtypes`; types with an empty label skipped (I0074) |
| `openttd://bridges` | `types` [{type, name, available, avail_year, min_length, max_length, spec_max_length, price, speed}], `max_bridge_length`, `max_tunnel_length`, the cost formula | `GetBridgeSpec`, `CheckBridgeAvailability`'s rules (I0073) |

### What `openttd://mcp/status` says the server can do

`capabilities` is computed on every read from what the running server registers, never written by
hand (`ServerCapabilities()` in `prismatic_mcp_server.cpp`, the rules in
`detail::MUTATING_CAPABILITIES`, `prismatic_mcp_detail.h`). A tool counts only when its
`readOnlyHint` is not true, so a reader under the same prefix (`openttd.ai.list`,
`openttd.ui.windows`) makes nothing:

| Field | True when the server registers |
|---|---|
| `reads` | a resource or a read-only tool |
| `approval_gated_commands` | an `openttd.cmd.<Name>` tool (one per exposable game command) |
| `script_api` | an `openttd.api.*` tool (it executes with `execute: true`, approval-gated) |
| `ai_tools` | an `openttd.ai.*` action (start, stop, reload, set_setting, set_settings) |
| `pause` | `openttd.game.set_pause` |
| `toolbar_menu` | `openttd.ui.menu.run` or `openttd.ui.requests.answer` (a GUI process; local to it) |
| `interface_input` | an acting `openttd.input.*` or `openttd.ui.*` tool **and** the process is armed |

`input_armed` is whether the process was started with `PRISMATIC_MCP_INPUT=1`; a dedicated
server can be armed and still have no `interface_input`, since it lists no interface tool.
`read_only` is derived: true only when every field but `reads` is false, so a client can tell a
read-only build from a mutating one by reading status alone. `openttd.view.*` moves only the
local viewport and `openttd.decision.submit` runs only action types the tools above carry, so
neither is a capability of its own. `prismatic_mcp status` in the console prints the same values
(`read_only=… capabilities=… input_armed=…`, the present fields comma-separated or `none`),
running or stopped.

A real run (2026-11-22, lundra, a dedicated server built with `OPTION_RESEARCH_INSTRUMENTATION=ON`
`OPTION_PRISMATIC_MCP_SERVER=ON` `OPTION_DEDICATED=ON`, revision `20261122--m9aaa7beed8` with this
change, a new game, `prismatic_mcp start 28741`). `resources/read openttd://mcp/status` answered:

```json
{"running":true,"read_only":false,"capabilities":{"ai_tools":true,"approval_gated_commands":true,"interface_input":false,"pause":true,"reads":true,"script_api":true,"toolbar_menu":false},"input_armed":false}
```

(these four `data` fields as the server wrote them, keys sorted; endpoint, counters and the
envelope left out). The same process printed,
running and after `prismatic_mcp stop`:

```
prismatic_mcp running endpoint=http://127.0.0.1:28741/mcp requests=1 denied=0 conns=0 read_only=false capabilities=reads,approval_gated_commands,script_api,ai_tools,pause input_armed=false
prismatic_mcp stopped read_only=false capabilities=reads,approval_gated_commands,script_api,ai_tools,pause input_armed=false
```

Started again with `PRISMATIC_MCP_INPUT=1`, the dedicated server answered `input_armed: true` and
still `interface_input: false`, since it has no interface tool to arm.

The table names some resources; the live list, with every description, is `resources/list` on a
running server (and the hub's catalogue across every instance).

Each `resources/read` is served in its own poll, so a client reading several while the game runs
gets several ticks. `openttd.resources.read_many {uris: [...]}` (`08-tool-catalog.md`) builds them
all in one call on the main thread instead: one `game_tick` for every answer, without a pause.
`mcp-plan --capture` uses it when the server lists it (`research/battlebrain/README.md`).

Rules honored: unknown → `null`/explicit availability (never coerced to 0); invalid uri → JSON-RPC
`-32602`; lists sorted by stable id (pool ascending index); money states its unit; date states
calendar Y-M-D; no memory addresses; no secrets. See `exposure-coverage-matrix.md` for the domains
catalogued but not yet adapted (network-redacted, debug, game settings).

Status update (2026-11-17): this paragraph used to list vehicles, towns, map regions and UI as not
yet adapted. They are resources now (`openttd://vehicles`, `openttd://towns`, `openttd://stations`,
`openttd://map`, `openttd://map/ways`, `openttd://ui/menu`, ...: the rows of `RESOURCES` in
`src/prismatic_mcp/prismatic_mcp_server.cpp`).
