/*
 * This file is part of korczis's PRIVATE RESEARCH FORK of OpenTTD and is NOT
 * part of upstream OpenTTD. RESEARCH-ONLY / UNSTABLE / DEBUG-ONLY.
 *
 * Embedded, loopback-only MCP server (stable spec 2025-11-25, Streamable HTTP).
 * Compiled only when OTTD_PRISMATIC_MCP_SERVER is defined.
 *
 * Design (see research/prismatic-mcp/):
 *  - The listener binds 127.0.0.1 and ::1 only, never a public interface.
 *  - Every byte is serviced on the OpenTTD main thread from GameLoop()->Poll(),
 *    so request handlers touch pools/globals with no locking, exactly like the
 *    admin network. No worker thread ever dereferences game state.
 *  - Auth is a 256-bit random bearer token generated at Start(), compared in
 *    constant time. The token is never logged and never written to openttd.cfg.
 *  - It implements initialize, notifications/initialized, ping, prompts/list,
 *    resources/list and resources/read (live game state, read-only), and
 *    tools/list and tools/call. Reading tools change nothing; every tool that
 *    changes the game (openttd.cmd.<Name>, openttd.game.set_pause, the openttd.ai.*
 *    actions, openttd.api.* with execute, openttd.decision.submit) is approval-gated: an
 *    operator approves the exact arguments in the console, and the command is
 *    posted through the normal, network-safe command path. In client mode (this
 *    process is a network client) it acts only as its own company and refuses a
 *    server-only command. Interface input (openttd.input.* and the acting
 *    openttd.ui.* tools) runs only in a process armed with PRISMATIC_MCP_INPUT=1.
 *    openttd://mcp/status reports what this build and process can do, derived
 *    from the registry (ServerCapabilities()).
 *  - The tool catalogue is research/prismatic-mcp/08-tool-catalog.md; the
 *    resources are 07-resource-catalog.md.
 */

/** @file prismatic_mcp_server.cpp Loopback MCP transport, resources, approval-gated tools (research-only). */

#include "../stdafx.h"

#ifdef OTTD_PRISMATIC_MCP_SERVER

#include "prismatic_mcp.h"
#include "prismatic_mcp_detail.h"
#include "prismatic_mcp_console.h"
#include "prismatic_mcp_coverage.h"
#include "prismatic_mcp_entities.h"
#include "prismatic_mcp_graph.h"
#include "prismatic_mcp_menu.h"
#include "prismatic_mcp_registry.h"
#include "prismatic_mcp_ui.h"
#include "prismatic_mcp_view.h"
#include "prismatic_mcp_world.h"
#include "prismatic_mcp_commands.h"
#include "prismatic_mcp_ai.h"
#include "prismatic_mcp_ai_status.h"
#include "prismatic_mcp_api.h"

#include "../3rdparty/nlohmann/json.hpp"
#include "../core/random_func.hpp"
#include "../core/string_consumer.hpp"
#include "../fileio_func.h"
#include "../settings_type.h"
#include "../network/core/config.h"
#include "../network/network_base.h"
#include "../console_func.h"
#include "../debug.h"
#include "../rev.h"
#include "../openttd.h"
#include "../company_base.h"
#include "../strings_func.h"
#include "../table/strings.h"
#include "../vehicle_base.h"
#include "../town.h"
#include "../station_base.h"
#include "../map_func.h"
#include "../timer/timer_game_calendar.h"
#include "../timer/timer_game_tick.h"
#include "../network/core/os_abstraction.h"
#include "../network/core/address.h"
#include "../command_func.h"
#include "../command_type.h"
#include "../network/network.h"
#include "../misc_cmd.h"

#include <cstdlib>
#include <array>
#include <filesystem>
#if !defined(_WIN32)
#include <fcntl.h>
#include <sys/stat.h>
#include <unistd.h>
#endif
#include <deque>
#include <functional>
#include <charconv>
#include <ctime>
#include <map>
#include <optional>
#include <string>
#include <string_view>
#include <vector>

#include "../safeguards.h"

namespace PrismaticMCP {

using json = nlohmann::json;

namespace {

/* ---- HTTP/transport limits (mission section 11.6, conservative defaults) ---------- */
constexpr size_t MAX_REQUEST_BODY = 1u << 20;   ///< 1 MiB request body cap.
constexpr size_t MAX_HEADER_BYTES = 32u << 10;  ///< 32 KiB header section cap.
constexpr size_t MAX_CONNECTIONS = 16;          ///< Simultaneous TCP connections.

/** One in-flight loopback HTTP connection. Raw byte buffer; no OpenTTD Packet framing. */
struct Connection {
	SOCKET sock = INVALID_SOCKET;
	std::string inbuf;             ///< Accumulated request bytes until headers+body complete.
	bool headers_done = false;
	size_t content_length = 0;
	size_t header_end = 0;         ///< Offset of body start (end of "\r\n\r\n").
};

/** Process-local server state. Never serialized into a savegame; ephemeral. */
struct ServerState {
	bool running = false;
	uint16_t port = 0;
	SocketList listeners;          ///< Loopback listening sockets (v4 + v6).
	std::vector<Connection> conns;
	std::array<uint8_t, 32> token{}; ///< 256-bit bearer token (raw bytes).
	bool token_valid = false;
	uint64_t requests_total = 0;
	uint64_t requests_denied = 0;
	std::string last_error;
};

ServerState _mcp;

/* ---- small helpers -------------------------------------------------------- */

std::string ToHex(std::span<const uint8_t> bytes)
{
	static const char *digits = "0123456789abcdef";
	std::string out;
	out.reserve(bytes.size() * 2);
	for (uint8_t b : bytes) {
		out.push_back(digits[b >> 4]);
		out.push_back(digits[b & 0xF]);
	}
	return out;
}

/** Constant-time compare of the presented token (hex) against the live token. */
bool TokenMatches(std::string_view presented_hex)
{
	if (!_mcp.token_valid) return false;
	return detail::ConstantTimeEquals(presented_hex, ToHex(_mcp.token));
}

std::string_view TrimOWS(std::string_view s)
{
	while (!s.empty() && (s.front() == ' ' || s.front() == '\t')) s.remove_prefix(1);
	while (!s.empty() && (s.back() == ' ' || s.back() == '\t' || s.back() == '\r')) s.remove_suffix(1);
	return s;
}

/* ---- live game-state serializers (main thread only) ----------------------- */

/** The common response envelope (mission section 15), stamped with the live snapshot. */
json MakeEnvelope()
{
	json env;
	env["schema_version"] = "1.0";
	env["source"] = "OpenTTD";
	env["source_revision"] = _openttd_revision;
	env["protocol_version"] = std::string(PROTOCOL_VERSION);
	env["game_tick"] = (uint64_t)TimerGameTick::counter;

	TimerGameCalendar::YearMonthDay ymd = TimerGameCalendar::ConvertDateToYMD(TimerGameCalendar::date);
	env["game_date"] = fmt::format("{:04}-{:02}-{:02}", (int)ymd.year.base(), (int)ymd.month + 1, (int)ymd.day);

	const char *mode = "menu";
	switch (_game_mode) {
		case GameMode::Normal: mode = "normal"; break;
		case GameMode::Editor: mode = "editor"; break;
		case GameMode::Bootstrap: mode = "bootstrap"; break;
		default: mode = "menu"; break;
	}
	env["game_mode"] = mode;
	env["availability"] = "available";
	env["redactions"] = json::array();
	env["truncated"] = false;
	env["next_cursor"] = nullptr;
	return env;
}

detail::Capabilities ServerCapabilities();

json ResourceMcpStatus()
{
	json env = MakeEnvelope();
	json d;
	d["running"] = _mcp.running;
	d["endpoint"] = GetEndpoint();
	d["bind"] = "127.0.0.1,::1";
	d["port"] = _mcp.port;
	/* Derived from the registry and this process, never written: read_only is the negation of
	 * every mutating capability (detail::MUTATING_CAPABILITIES). */
	const detail::Capabilities caps = ServerCapabilities();
	d["read_only"] = caps.ReadOnly();
	d["capabilities"] = detail::CapabilitiesJson(caps);
	d["input_armed"] = caps.input_armed;
	d["requests_total"] = _mcp.requests_total;
	d["requests_denied"] = _mcp.requests_denied;
	d["active_connections"] = (uint64_t)_mcp.conns.size();
	env["data"] = d;
	return env;
}

json ResourceMetaBuild()
{
	json env = MakeEnvelope();
	json d;
	d["openttd_version"] = _openttd_revision;
	d["revision_hash"] = std::string(_openttd_revision_hash);
	d["newgrf_version"] = _openttd_newgrf_version;
	d["research_instrumentation"] = true;
	d["mcp_server"] = true;
#ifdef DEDICATED
	d["dedicated"] = true;
#else
	d["dedicated"] = false;
#endif
	env["data"] = d;
	return env;
}

/**
 * Primary vehicles per owner and type: what a player counts as a vehicle (a train, a road vehicle,
 * a ship, an aircraft), not the pool's items. Vehicle::GetNumItems() also counts every wagon,
 * articulated part, shadow, rotor, effect (smoke, sparks) and disaster vehicle, so it swings by
 * dozens from one day to the next while the fleet stays the same (the panel's fleet chart, 2026-11-17).
 * Index [owner][type]; an owner that is no company (OWNER_NONE ...) lands in the last row.
 */
using FleetCounts = std::array<std::array<uint32_t, 4>, MAX_COMPANIES + 1>;
static FleetCounts PrimaryFleet()
{
	FleetCounts fleet{};
	for (const Vehicle *v : Vehicle::Iterate()) {
		if (v->type >= VehicleType::CompanyEnd || !v->IsPrimaryVehicle()) continue;
		size_t row = v->owner.base() < MAX_COMPANIES ? v->owner.base() : MAX_COMPANIES;
		fleet[row][to_underlying(v->type)]++;
	}
	return fleet;
}

static json FleetJson(const std::array<uint32_t, 4> &n)
{
	return json{{"train", n[0]}, {"road", n[1]}, {"ship", n[2]}, {"aircraft", n[3]}, {"total", n[0] + n[1] + n[2] + n[3]}};
}

json ResourceGameState()
{
	json env = MakeEnvelope();
	json d;
	d["paused"] = _pause_mode.Any();
	d["map_width"] = Map::SizeX();
	d["map_height"] = Map::SizeY();
	d["num_companies"] = (uint64_t)Company::GetNumItems();
	d["num_vehicles"] = (uint64_t)Vehicle::GetNumItems(); // pool items: wagons, parts and effects included
	std::array<uint32_t, 4> all{};
	for (const auto &row : PrimaryFleet()) for (size_t t = 0; t < all.size(); t++) all[t] += row[t];
	d["fleet"] = FleetJson(all); // primary vehicles by type: what the player calls the fleet
	d["num_towns"] = (uint64_t)Town::GetNumItems();
	env["data"] = d;
	return env;
}

json ResourceCompanies()
{
	json env = MakeEnvelope();
	json list = json::array();
	const FleetCounts fleet = PrimaryFleet();
	/* Deterministic order: pools iterate by ascending index. */
	for (const Company *c : Company::Iterate()) {
		json jc;
		jc["id"] = c->index.base();
		jc["name"] = GetString(STR_COMPANY_NAME, c->index); // the displayed name, generated or renamed
		jc["custom_name"] = c->name;       // empty if the player never renamed it
		jc["is_ai"] = c->is_ai;
		jc["money"] = (int64_t)c->money;
		jc["loan"] = (int64_t)c->current_loan;
		jc["max_loan"] = (int64_t)c->GetMaxLoan();
		jc["loan_step"] = LOAN_INTERVAL; // one press of Borrow or Repay (misc_cmd.cpp); Repay takes min(loan, step)
		jc["company_value"] = (int64_t)CalculateCompanyValue(c);
		jc["money_unit"] = "internal_currency";
		jc["inaugurated_year"] = (int)c->inaugurated_year_calendar.base();
		jc["fleet"] = FleetJson(fleet[c->index.base()]);
		list.push_back(std::move(jc));
	}
	env["data"] = json{{"companies", std::move(list)}};
	return env;
}

/** openttd://commands -- the command catalogue, derived from the command table. */
json ResourceCommands()
{
	json env = MakeEnvelope();
	json list = json::array();
	size_t exposable = 0;
	for (size_t i = 0; i < cmds::Count(); i++) {
		Commands c = static_cast<Commands>(i);
		if (cmds::IsExposable(c)) exposable++;
		list.push_back(cmds::Describe(c));
	}
	env["data"] = json{{"count", cmds::Count()}, {"exposable", exposable}, {"commands", std::move(list)},
		{"usage", "tools/call openttd.cmd.<name> {company, params[, location]}; every call is approval-gated"}};
	return env;
}

json ResourceDecisions(); // defined below, in the decision/approval section
json ResourceApprovals(); // the same section: the one queue every approval channel reads

/** Static resource catalogue. Deterministic order; each entry is read-only here. */
struct ResourceEntry {
	const char *uri;
	const char *name;
	const char *description;
	json (*build)();
};
json ResourceTowns(); // defined below, next to the view tools that scroll to a town
json ResourceMap();   // the whole map (prismatic_mcp_world.cpp)
json ResourceStations();
json ResourceDepots();
json ResourceTerrain();
json ResourcePrices();
json ResourceBridges();
json ResourceRailTypes();
json ResourceMenu();
json ResourceRequests();
json ResourceEngines();  // every engine on offer now (prismatic_mcp_world.cpp)
json ResourceVehicles(); // every primary vehicle with its orders (prismatic_mcp_entities.cpp)
json ResourceCoverage(); // what clients exercised, against the published catalogue (prismatic_mcp_coverage.cpp)

json ResourceWays();  // road ways and rail tracks per tile (prismatic_mcp_graph.cpp)
json ResourceWorldGraph(); // the world as one typed graph (prismatic_mcp_graph.cpp)

json ResourceAi() { json env = MakeEnvelope(); env["data"] = ai::Resource(); return env; }
json ResourceAiStatus() { json env = MakeEnvelope(); env["data"] = ai_status::Resource(); return env; }

/** Every typed action type this build can execute: the fixed table, then one "command.<Name>" per
 * game command the catalogue exposes. openttd.decision.list_action_types and the decision tools'
 * input schemas read this, so a type is offered exactly when the validator accepts it. */
json ActionTypes()
{
	json types = json::array();
	for (std::string_view t : detail::ACTION_TYPES) types.push_back(t);
	for (size_t i = 0; i < cmds::Count(); i++) {
		Commands c = static_cast<Commands>(i);
		if (cmds::IsExposable(c)) types.push_back("command." + std::string(cmds::ShortName(c)));
	}
	return types;
}

/* The size is counted by the compiler, so adding a resource is one row. */
const auto RESOURCES = std::to_array<ResourceEntry>({
	{"openttd://mcp/status", "MCP server status", "Embedded MCP server runtime status", &ResourceMcpStatus},
	{"openttd://meta/build", "Build metadata", "OpenTTD version, revision and feature gates", &ResourceMetaBuild},
	{"openttd://game/state", "Game state", "Mode, pause, map size and object counts", &ResourceGameState},
	{"openttd://companies", "Companies", "All companies with basic finances (server-visible)", &ResourceCompanies},
	{"openttd://commands", "Commands", "Every game command, derived from the command table: category, flags, parameter schemas", &ResourceCommands},
	{"openttd://ai", "AIs", "Every company's AI with its settings as declared by its info.nut, their live values and the control a form edits each with; the AIs that can be started", &ResourceAi},
	{"openttd://ai/status", "AI status", "Every AI company's latest heartbeat line from its own script log (`<Name> heartbeat: key=value ...`), split into fields: an integer when the value is one, else its text; no field is named by the server; lines_since counts the lines logged after it", &ResourceAiStatus},
	{"openttd://decisions", "Decisions", "Provenance trace: decision -> approval -> command -> observed verdict", &ResourceDecisions},
	{"openttd://approvals", "Approvals", "The one approval queue (ADR 0038): every approval this server holds, pending first, with its tool, arguments, state and expiry; approving one executes it here", &ResourceApprovals},
	{"openttd://towns", "Towns", "Every town: id, name, tile, x/y, population, growth, exclusivity, noise, and per company its exact rating, statue, months unwanted and active/idle stations near the town (scroll_to a town by id; research/mechanisms/town-authority.md)", &ResourceTowns},
	{"openttd://map", "Map", "The whole map: tile types, heights and owners, one string per row, encoding included", &ResourceMap},
	{"openttd://stations", "Stations", "Every station: id, name, tile, x/y, owner, facilities, town", &ResourceStations},
	{"openttd://vehicles", "Vehicles", "Every primary vehicle: id, type, owner, engine, tile, state, cargo, profits, orders", &ResourceVehicles},
	{"openttd://map/terrain", "Map terrain", "Per tile, as rows: slope bits, ground (grass, rough, rocks, fields, snow, desert, trees, sea, canal, river, coast), the largest height step to a neighbour, what building there needs, the vehicles standing there, and a digest per row so a planner re-reads only what changed; encoding included", &ResourceTerrain},
	{"openttd://economy/prices", "Prices", "Every base price of the game by name (build_rail, build_road, build_bridge, build_tunnel, build_foundation, terraform, clear_*, build_station_*, build_depot_*, infrastructure_* ...) as it is now: inflated, scaled by the construction cost setting and NewGRF multipliers; with each start value", &ResourcePrices},
	{"openttd://railtypes", "Rail types", "Every rail type in use: label, name, build cost now (RailBuildCost), cost and maintenance multipliers, speeds, which types power and run on it, and the companies that may build it", &ResourceRailTypes},
	{"openttd://bridges", "Bridges", "Every bridge type: name, price multiplier, speed, min and max length, whether it can be built now; the game's bridge and tunnel length limits and the cost formula", &ResourceBridges},
	{"openttd://map/ways", "Map ways", "Per tile: where a road vehicle can leave it (road bits, stops, depots, crossings, bridges, tunnels) its rail tracks and foundation; every bridge and tunnel with both ends; vehicles per tile; encoding included", &ResourceWays},
	{"openttd://world/graph", "World graph", "The world as one typed graph (world-graph/v1): towns, industries, stations, vehicles; near_town, serves_town, order and cargo_link edges", &ResourceWorldGraph},
	{"openttd://depots", "Depots", "Every depot: id, tile, x/y, owner, type, town, entrance tile; and the game's service reach in tiles", &ResourceDepots},
	{"openttd://ui/menu", "Toolbar menu", "The fork's own main-toolbar menu: every item with its action (console, browse, ask) and target, and the hub URL", &ResourceMenu},
	{"openttd://ui/requests", "Requests", "Questions the player asked in the game through the toolbar menu, each with its answer once an MCP client gave one", &ResourceRequests},
	{"openttd://mcp/coverage", "MCP coverage", "Every published tool and resource with its call count since start, and those never called", &ResourceCoverage},
	{"openttd://engines", "Engines", "Every engine some company can buy now: type, speed (also effective for aircraft), capacity, cargo, price, running cost, plane type, buyers", &ResourceEngines},
});

/** @return The catalogue entry of resource `uri`, or nullptr when there is none. */
const ResourceEntry *FindResource(std::string_view uri)
{
	for (const auto &r : RESOURCES) {
		if (uri == r.uri) return &r;
	}
	return nullptr;
}

/**
 * openttd.resources.read_many: several resources built by their own builders inside this one
 * main-thread call, so every answer carries the same game tick (a coherent snapshot, without
 * pausing the game). Each entry's `data` is exactly what resources/read answers as text for it.
 * Reads only; an unknown or refused uri is an error for its entry, not for the call.
 */
json ReadMany(const json &args)
{
	detail::ReadManyPlan plan = detail::PlanReadMany(args, [](const std::string &u) { return FindResource(u) != nullptr; });
	if (!plan.error.empty()) return json{{"ok", false}, {"error", plan.error}, {"max_uris", detail::READ_MANY_MAX_URIS}};
	json head = MakeEnvelope();
	json out = {{"ok", true}, {"game_tick", head["game_tick"]}, {"game_date", head["game_date"]}, {"resources", json::array()}};
	for (const auto &e : plan.entries) {
		json entry = {{"uri", e.uri}, {"ok", e.error.empty()}};
		if (e.error.empty()) {
			Coverage::Count("resource", e.uri);
			try {
				entry["data"] = FindResource(e.uri)->build();
			} catch (const json::exception &ex) {
				entry = {{"uri", e.uri}, {"ok", false}, {"error", ex.what()}};
			}
		} else {
			entry["error"] = e.error;
		}
		out["resources"].push_back(std::move(entry));
	}
	return out;
}

json ResourceMap()
{
	json env = MakeEnvelope();
	env["data"] = World::Map();
	return env;
}

json ResourceStations()
{
	json env = MakeEnvelope();
	env["data"] = World::Stations();
	return env;
}

json ResourceTerrain()
{
	json env = MakeEnvelope();
	env["data"] = World::Terrain();
	return env;
}

json ResourcePrices()
{
	json env = MakeEnvelope();
	env["data"] = World::Prices();
	return env;
}

json ResourceRailTypes()
{
	json env = MakeEnvelope();
	env["data"] = World::RailTypes();
	return env;
}

json ResourceBridges()
{
	json env = MakeEnvelope();
	env["data"] = World::Bridges();
	return env;
}

json ResourceDepots()
{
	json env = MakeEnvelope();
	env["data"] = World::Depots();
	return env;
}

json ResourceMenu()
{
	json env = MakeEnvelope();
	env["data"] = Menu::Describe();
	return env;
}

json ResourceRequests()
{
	json env = MakeEnvelope();
	env["data"] = Menu::Requests();
	return env;
}

json ResourceEngines()
{
	json env = MakeEnvelope();
	env["data"] = World::Engines();
	return env;
}

json ResourceVehicles()
{
	json env = MakeEnvelope();
	env["data"] = Entities::Vehicles();
	return env;
}

json ResourceWays()
{
	json env = MakeEnvelope();
	env["data"] = Graph::Ways();
	return env;
}

json ResourceWorldGraph()
{
	json env = MakeEnvelope();
	env["data"] = Graph::World();
	return env;
}

json ToolsList();
json ResourcesList();

json ResourceCoverage()
{
	json env = MakeEnvelope();
	env["data"] = Coverage::Report(ToolsList(), ResourcesList());
	return env;
}

/**
 * Every town, so a controller can find where a bot builds and scroll the view there, with what its
 * local authority thinks of each company (RESEARCH-ONLY; research/mechanisms/town-authority.md).
 * Per company: the exact rating (an AI sees only its band), months unwanted after a failed bribe,
 * and its stations near the town split into active and idle the way UpdateTownRating and
 * CountActiveStations (town_cmd.cpp) split them: within the town-edge zone radius, active when it
 * loaded or unloaded within 20 rating periods. That 20 mirrors town_cmd.cpp; the town model
 * (tools/research/town-model/town_model.json, link station_service_monthly) records its source.
 */
json ResourceTowns()
{
	json env = MakeEnvelope();
	json list = json::array();
	for (const Town *t : Town::Iterate()) {
		json companies = json::array();
		for (const Company *c : Company::Iterate()) {
			int active = 0, idle = 0;
			uint32_t r2 = t->cache.squared_town_zone_radius[to_underlying(HouseZone::TownEdge)];
			for (const Station *st : Station::Iterate()) {
				if (st->owner != c->index || DistanceSquare(st->xy, t->xy) > r2) continue;
				if (st->time_since_load <= 20 || st->time_since_unload <= 20) active++; else idle++;
			}
			json e = {{"company", c->index.base()}, {"rating", t->ratings[c->index]},
				{"has_rating", t->have_ratings.Test(c->index)}, {"unwanted_months", t->unwanted[c->index]},
				{"statue", t->statues.Test(c->index)}, {"active_stations", active}, {"idle_stations", idle}};
			companies.push_back(std::move(e));
		}
		json town = {{"id", t->index.base()}, {"name", t->GetCachedName()}, {"tile", t->xy.base()},
			{"x", TileX(t->xy)}, {"y", TileY(t->xy)}, {"population", t->cache.population},
			{"houses", t->cache.num_houses}, {"growth_rate", t->growth_rate}, {"grow_counter", t->grow_counter},
			{"fund_buildings_months", t->fund_buildings_months}, {"road_build_months", t->road_build_months},
			{"exclusive_months", t->exclusive_counter}, {"noise_reached", t->noise_reached},
			{"max_noise", t->MaxTownNoise()}, {"companies", std::move(companies)}};
		town["exclusivity"] = t->exclusivity == CompanyID::Invalid() ? json(nullptr) : json(t->exclusivity.base());
		list.push_back(std::move(town));
	}
	env["data"] = json{{"towns", std::move(list)}};
	return env;
}

/* ---- decision / approval / provenance (main thread) ----------------------- *
 * The mission's core continuation vertical: a typed DecisionEnvelope is validated
 * (non-mutating), then a mutation tool is *gated* behind a two-phase, digest-bound,
 * operator approval before it ever touches the game via Command<>::Post. Because
 * tool handlers run inside Poll() on the main thread, the command executes on the
 * correct thread through the normal deterministic command path -- never a raw pool
 * write, never a mutation off the main thread. Approval is one-time (replay of an
 * approved call requires a fresh approval), and it binds to the tool and its exact
 * canonical arguments: a post-approval argument change, or the same arguments sent to
 * another tool, is refused and needs a fresh approval. */

enum class ApprovalState : uint8_t { Pending, Approved, Denied, Consumed };

struct PendingApproval {
	uint64_t id = 0;
	std::string tool;
	std::string arg_digest;      ///< FNV-1a of tool + canonical args; shown/recorded, not the binding.
	std::string canonical_args;  ///< the exact serialized args this approval binds to, together with `tool` (collision-free, unlike the 64-bit digest).
	std::string args_summary;    ///< Human-readable, shown in the console approval prompt.
	std::string mcp_tool;        ///< the MCP tool as it was called (openttd.ai.set_setting), for openttd://approvals
	std::string mcp_args;        ///< its arguments as called, serialized
	uint64_t superseded_by = 0;  ///< the newer approval that withdrew this one (state Denied), else 0
	std::string settled_by;      ///< "deadline keep" / "deadline approve (approver in company N)" when the deadline settled it, else empty
	bool approver_at_creation = false; ///< a registered approver played the company it names when it was asked (detail::ApproverSeats)
	uint64_t created_tick = 0;
	uint64_t expiry_tick = 0;
	ApprovalState state = ApprovalState::Pending;
};

} // anonymous namespace
/* Defined below, with the chat channel and the approvers, outside the anonymous namespace. */
static int CompanyOf(const PendingApproval &a);
static std::vector<detail::ApproverSeat> CurrentApproverSeats();
static bool ChatDecidable(const PendingApproval &a);
namespace {

/** One change of who approves from where: an approver joined or left a company, or its key was connected twice. */
struct ApproverEvent {
	uint64_t tick = 0;
	std::string change; ///< "joined", "left" or "shared"
	detail::ApproverSeat seat;
};
static std::vector<ApproverEvent> _approver_events; ///< the last MAX_APPROVER_EVENTS, oldest first (openttd://approvals)
static constexpr size_t MAX_APPROVER_EVENTS = 32;

/** Bounded provenance record: decision -> approval -> command -> observed verdict. */
struct ProvenanceRecord {
	uint64_t decision_id = 0;
	uint64_t approval_id = 0;
	std::string action_type;
	std::string arg_digest;
	std::string command_result;   ///< "queued", then "succeeded"/"failed"; "rejected" when never queued
	std::string verdict;          ///< PENDING_OBSERVATION until the command has run, then VERIFIED_* / EXECUTED_UNVERIFIED
	std::string verification;     ///< what the verdict rests on: "observed_state" or "command_result"
	std::string approved_via;     ///< the channel that gave the approval: "console", "chat client <id> company <n>" (ADR 0038)
	std::string error;            ///< the command's error text, when it failed
	int64_t cost = 0;             ///< the executed command's cost (internal currency)
	std::string actor;            ///< "server", "deity" or the company id
	Commands cmd = Commands::End; ///< the command posted, matched against its executed result
	int callbacks_left = 0;       ///< executed results still to arrive
	bool failed = false;          ///< one of its commands failed
	std::function<bool()> observe; ///< asked after the last result: is the intended effect there?
	uint64_t requested_tick = 0;
	uint64_t observed_tick = 0;
};

std::vector<PendingApproval> _approvals;
std::vector<ProvenanceRecord> _provenance;
std::deque<uint64_t> _awaiting;   ///< decision ids posted and not yet executed, in posting order
uint64_t _next_approval_id = 1;
uint64_t _next_decision_id = 1;
constexpr size_t MAX_APPROVALS = 64;
constexpr size_t MAX_PROVENANCE = 128;
constexpr uint64_t APPROVAL_TTL_TICKS = 100000; ///< generous; the operator approves interactively
/** The operator's deadline policy (detail::DeadlinePolicy): one game month, verdict keep, until changed. */
detail::DeadlinePolicy _deadline_policy;

/** FNV-1a 64-bit digest of tool+args, via the shared pure helper. */
std::string ArgDigest(std::string_view tool, const json &args)
{
	return detail::ArgDigestHex(tool, args.dump());
}

ProvenanceRecord *FindDecision(uint64_t id)
{
	for (auto &p : _provenance) if (p.decision_id == id) return &p;
	return nullptr;
}

PendingApproval *FindApproval(uint64_t id)
{
	for (auto &a : _approvals) if (a.id == id) return &a;
	return nullptr;
}

/** The owner's standing rules, in id order (ADR 0041; their lifecycle is below, with ApplyRules). */
std::vector<Rules::Rule> _rules;
uint64_t _next_rule_id = 1;

/** The rules' verdict on one approval. */
Rules::Verdict RuleVerdict(const PendingApproval &a, const json &args)
{
	return Rules::Decide(_rules, Rules::Subject{a.mcp_tool, a.tool, args});
}

/**
 * Why an unattended channel (Rules::Unattended: the auto-approve loop, a rule from the rules file)
 * may not approve an approval, by the condition #264 put on the deadline's approve: only one
 * company's own action, for a company a registered approver played when it was asked and plays
 * now (approver_at_creation, detail::ApproverPlays).
 * @param a The approval.
 * @param via The channel that would approve it.
 * @return Why not; empty when it may.
 */
std::string UnattendedHold(const PendingApproval &a, std::string_view via)
{
	if (!Rules::Unattended(via)) return {};
	const int company = CompanyOf(a);
	const bool chat_scoped = ChatDecidable(a);
	const bool approver_company = a.approver_at_creation && detail::ApproverPlays(CurrentApproverSeats(), company);
	if (Rules::UnattendedMayApprove(via, chat_scoped, company, approver_company)) return {};
	if (!chat_scoped) return fmt::format("held: {} approves only one company's own action", via == "file" ? "a rule from the rules file" : "the auto channel");
	if (company < 0) return "held: it names no company";
	return fmt::format("held: no registered approver plays company {}", company);
}

/** Whether the verdict's rule decides the approval now: a deny rule always; an allow rule unless it expired or an unattended rule is held. */
bool RuleDecidesNow(const PendingApproval &a, const Rules::Verdict &v, uint64_t now)
{
	if (v.decides == nullptr) return false;
	if (v.decides->effect == Rules::Effect::Deny) return true;
	return now <= a.expiry_tick && UnattendedHold(a, v.decides->created_via).empty();
}

using detail::IsKnownActionType;

/* ---- JSON-RPC dispatch (main thread) -------------------------------------- */

json RpcError(const json &id, int code, std::string_view message)
{
	return json{{"jsonrpc", "2.0"}, {"id", id}, {"error", {{"code", code}, {"message", std::string(message)}}}};
}

json RpcResult(const json &id, json result)
{
	return json{{"jsonrpc", "2.0"}, {"id", id}, {"result", std::move(result)}};
}

/* ---- decision tools ------------------------------------------------------- */

/** openttd://decisions -- the provenance trace (execution vs verified-outcome). */
json ResourceDecisions()
{
	json env = MakeEnvelope();
	json arr = json::array();
	for (const auto &p : _provenance) {
		arr.push_back({
			{"decision_id", p.decision_id}, {"approval_id", p.approval_id},
			{"action_type", p.action_type}, {"arg_digest", p.arg_digest},
			{"command_result", p.command_result}, {"verdict", p.verdict},
			{"verification", p.verification}, {"approved_via", p.approved_via}, {"error", p.error}, {"cost", p.cost}, {"actor", p.actor},
			{"requested_tick", p.requested_tick}, {"observed_tick", p.observed_tick},
		});
	}
	env["data"] = json{{"decisions", std::move(arr)}};
	return env;
}

/** The one approval queue (ADR 0038): what every channel (the panel, the console, a client's chat)
 * reads instead of keeping a copy. Pending approvals first, oldest first; then the decided ones. */
json ResourceApprovals()
{
	json env = MakeEnvelope();
	uint64_t now = (uint64_t)TimerGameTick::counter;
	json arr = json::array();
	for (int pass = 0; pass < 2; pass++) {
		for (const auto &a : _approvals) {
			if ((a.state == ApprovalState::Pending) != (pass == 0)) continue;
			const char *st = a.state == ApprovalState::Pending ? "pending" : a.state == ApprovalState::Approved ? "approved"
				: a.state == ApprovalState::Denied ? "denied" : "consumed";
			arr.push_back({{"id", a.id}, {"action", a.tool}, {"tool", a.mcp_tool},
				{"args", json::parse(a.mcp_args.empty() ? a.canonical_args : a.mcp_args, nullptr, false)},
				{"state", a.superseded_by != 0 ? "superseded" : st}, {"superseded_by", a.superseded_by}, {"expired", a.state == ApprovalState::Pending && now > a.expiry_tick},
				{"created_tick", a.created_tick}, {"expires_tick", a.expiry_tick},
				{"deadline_tick", detail::DeadlineTick(a.created_tick, _deadline_policy.ticks).value_or(0)},
				{"approver_at_creation", a.approver_at_creation}, {"settled_by", a.settled_by}});
			if (a.state == ApprovalState::Pending) {
				Rules::Verdict v = RuleVerdict(a, json::parse(a.canonical_args, nullptr, false));
				if (const Rules::Rule *r = v.decides != nullptr ? v.decides : v.blocked; r != nullptr) {
					arr.back()["matched_rule"] = {{"id", r->id}, {"effect", Rules::EffectName(r->effect)}, {"decides", v.decides == r && RuleDecidesNow(a, v, now)}};
				}
			}
		}
	}
	json rules = json::array();
	for (const auto &r : _rules) rules.push_back(Rules::ToJson(r));
	/* Where registered approvers play now: the companies the deadline's approve may stand in for. */
	json seats = json::array();
	for (const auto &s : CurrentApproverSeats()) seats.push_back({{"company", s.company}, {"client_id", s.client_id}, {"key_fp", s.key_fp}});
	json events = json::array();
	for (const auto &e : _approver_events) {
		events.push_back({{"tick", e.tick}, {"change", e.change}, {"company", e.seat.company}, {"client_id", e.seat.client_id}, {"key_fp", e.seat.key_fp}});
	}
	env["data"] = json{{"approvals", std::move(arr)}, {"capacity", MAX_APPROVALS}, {"rules", std::move(rules)},
		{"approver_companies", std::move(seats)}, {"approver_events", std::move(events)},
		{"deadline", {{"ticks", _deadline_policy.ticks}, {"days", _deadline_policy.ticks / detail::DEADLINE_DAY_TICKS},
			{"verdict", detail::DeadlineVerdictName(_deadline_policy.verdict)}}}};
	return env;
}

bool ExecuteAction(std::string_view type, const json &args, ProvenanceRecord &pr); // below, with the action catalogue

/**
 * Carry out an approval the operator has given: the one execution path, whichever channel
 * approved it (the console, the panel, a client's chat; ADR 0038). Main thread. The approval is
 * consumed (a replay needs a fresh one) and its decision recorded; the command runs through the
 * normal command path, its verdict taken when it has run.
 * @param via the channel that gave it, recorded with its decision (approved_via).
 * @return the answer: {status executed|rejected, approval_id, decision_id, command_result, verdict[, error]}.
 */
json ExecuteApproval(PendingApproval &a, std::string_view via)
{
	uint64_t now = (uint64_t)TimerGameTick::counter;
	a.state = ApprovalState::Consumed;
	if (_provenance.size() >= MAX_PROVENANCE) _provenance.erase(_provenance.begin());
	ProvenanceRecord &pr = _provenance.emplace_back();
	pr.decision_id = _next_decision_id++;
	pr.approval_id = a.id;
	pr.approved_via = std::string(via);
	pr.action_type = a.tool;
	pr.arg_digest = a.arg_digest;
	pr.command_result = "queued";
	pr.verdict = "PENDING_OBSERVATION";
	pr.requested_tick = now;
	uint64_t decision_id = pr.decision_id;
	bool queued = ExecuteAction(a.tool, json::parse(a.canonical_args), pr);
	const ProvenanceRecord *rec = FindDecision(decision_id);
	json out = {{"status", queued ? "executed" : "rejected"}, {"approval_id", a.id}, {"decision_id", decision_id},
		{"command_result", rec->command_result}, {"verdict", rec->verdict},
		{"note", "the verdict is taken when the command has run; read it with openttd.decision.get"}};
	if (!rec->error.empty()) out["error"] = rec->error;
	return out;
}

/* ---- the owner's standing rules (ADR 0041) ------------------------------------------------ *
 * "always allow <tool-glob> [<json matcher>]" and "always deny ...", made only by the owner: at the
 * game's console, by the owner's own click in the control window, or through the panel's
 * owner-authenticated routes (created_via "panel", which never decides an owner-only tool). None
 * exists until the owner makes one; each lives as long as this process, or in the rules file with
 * --persist. Every approval is checked as it is created, and every waiting one when a rule is
 * added; what a rule decides is recorded like a manual decision, naming the rule (approved_via of
 * its decision, settled_by of a denied approval). The matching itself is the pure Rules::Decide. */
/**
 * Let the standing rules decide approval \p a, if one applies and may: a deny rule withdraws it, an
 * allow rule approves it and carries it out through the one execution path. Expired approvals are
 * only ever denied by a rule, never approved.
 * @return The answer for the requester when a rule decided it; nullopt when it still waits.
 */
std::optional<json> ApplyRules(PendingApproval &a)
{
	if (_rules.empty() || a.state != ApprovalState::Pending) return std::nullopt;
	json args = json::parse(a.canonical_args, nullptr, false);
	Rules::Verdict v = RuleVerdict(a, args);
	if (v.decides == nullptr) return std::nullopt;
	/* The verdict points into _rules, which this function may count a hit in. */
	Rules::Rule &rule = *std::ranges::find_if(_rules, [&v](const Rules::Rule &r) { return &r == v.decides; });
	const bool expired = (uint64_t)TimerGameTick::counter > a.expiry_tick;
	const uint64_t rule_id = rule.id;
	/* Recorded with the rule and the channel it was made through. */
	const std::string via = fmt::format("{} (made via {})", Rules::Describe(rule), rule.created_via);
	if (rule.effect == Rules::Effect::Deny) {
		rule.hits++;
		a.state = ApprovalState::Denied;
		a.settled_by = via;
		Debug(Facility::Net, Severity::Error, "[prismatic-mcp] approval #{} ({}) denied by {}", a.id, a.tool, via);
		return json{{"status", "denied"}, {"approval_id", a.id}, {"rule", rule_id},
			{"message", fmt::format("denied by the owner's standing {}; nothing was enacted", via)}};
	}
	if (expired) return std::nullopt;
	/* An unattended rule (the auto channel's, the rules file's) holds where #264's deadline would. */
	if (!UnattendedHold(a, rule.created_via).empty()) return std::nullopt;
	rule.hits++;
	a.state = ApprovalState::Approved;
	Debug(Facility::Net, Severity::Error, "[prismatic-mcp] approval #{} ({}) approved by {}", a.id, a.tool, via);
	json out = ExecuteApproval(a, via);
	out["rule"] = rule_id;
	return out;
}

/** The current wall-clock time, ISO 8601 UTC, for a rule's created_at. */
std::string NowUtc()
{
	std::time_t t = std::time(nullptr);
	std::tm tm{};
#if defined(_WIN32)
	gmtime_s(&tm, &t);
#else
	gmtime_r(&t, &tm);
#endif
	return fmt::format("{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z", tm.tm_year + 1900, tm.tm_mon + 1, tm.tm_mday, tm.tm_hour, tm.tm_min, tm.tm_sec);
}

/** The command a "command.<Name>" action type names, if any. */
std::optional<Commands> CommandOf(std::string_view type)
{
	if (!type.starts_with("command.")) return std::nullopt;
	return cmds::Find(type.substr(8));
}

/** Who a command action is posted as: "server" (default), "deity" or a company id. */
bool ParseActor(const json &args, cmds::Actor &actor, std::string &label, std::string &err)
{
	if (_networking && !_network_server) {
		/* Client mode: this process is a player's client; it can only act as its own company,
		 * and its commands go to the server like the player's clicks. */
		if (args.contains("company") && !(args["company"].is_string() && args["company"] == "server")
				&& !(args["company"].is_number_integer() && args["company"].get<int64_t>() == _local_company.base())) {
			err = fmt::format("this process is a network client; it acts only as its own company {}", _local_company.base());
			return false;
		}
		actor.keep_current = true;
		label = fmt::format("client:{}", _local_company.base());
		return true;
	}
	if (!args.contains("company") || (args["company"].is_string() && args["company"] == "server")) {
		actor.keep_current = true;
		label = "server";
		return true;
	}
	const json &c = args["company"];
	if (c.is_string() && c == "deity") {
		actor.keep_current = false;
		actor.company = OWNER_DEITY;
		label = "deity";
		return true;
	}
	if (c.is_number_integer()) {
		int64_t v = c.get<int64_t>();
		if (v < 0 || v >= MAX_COMPANIES || !Company::IsValidID(CompanyID(static_cast<uint8_t>(v)))) {
			err = fmt::format("no company {}", v);
			return false;
		}
		actor.keep_current = false;
		actor.company = CompanyID(static_cast<uint8_t>(v));
		label = fmt::format("{}", v);
		return true;
	}
	err = "company must be a company id, \"deity\" or \"server\"";
	return false;
}

/** The location of a command action, required when the command takes one implicitly. */
bool ParseLocation(Commands cmd, const json &args, TileIndex &tile, std::string &err)
{
	tile = TileIndex{};
	if (!args.contains("location")) {
		if (cmds::NeedsLocation(cmd)) { err = "this command requires 'location' (a tile index)"; return false; }
		return true;
	}
	if (!args["location"].is_number_unsigned() || args["location"].get<uint64_t>() >= Map::Size()) {
		err = "location must be a tile index below " + fmt::format("{}", Map::Size());
		return false;
	}
	tile = TileIndex{static_cast<uint32_t>(args["location"].get<uint64_t>())};
	return true;
}

/**
 * Whether this process may post the command at all. A network client that posts a
 * CommandFlag::Server command (Pause, CompanyControl, ...) is kicked by the server, so a
 * client refuses it here, before anything is sent.
 */
bool ClientMayPost(Commands cmd, std::string &err)
{
	if (_networking && !_network_server && GetCommandFlags(cmd).Test(CommandFlag::Server)) {
		err = "this is a server-only command and this process is a network client (the server would kick it)";
		return false;
	}
	return true;
}

/** Non-mutating validation of a typed action + args (mission section 10/section 11). */
json ValidateAction(std::string_view type, const json &args, json &errors)
{
	json result;
	bool valid = true;
	if (type.starts_with("api.")) {
		/* A Script API function (prismatic_mcp_api.cpp): valid when it exists and its estimate succeeds. */
		result = Api::Validate(std::string{type}, args, errors);
		result["valid"] = errors.empty();
		result["errors"] = errors;
		result["required_scope"] = "SCRIPT_API";
		return result;
	}
	if (!IsKnownActionType(type)) { errors.push_back("unknown action type"); valid = false; }
	if (type == "game.set_pause") {
		if (!args.contains("pause") || !args["pause"].is_boolean()) {
			errors.push_back("game.set_pause requires boolean 'pause'");
			valid = false;
		}
		if (std::string err; !ClientMayPost(Commands::Pause, err)) { errors.push_back(err); valid = false; }
		result["required_scope"] = "SERVER_ADMIN";
	} else if (ai::IsAiAction(type)) {
		json details = ai::Validate(type, args, errors);
		if (!errors.empty()) valid = false;
		result["details"] = details;
		result["required_scope"] = "AI_CONTROL";
	} else if (auto cmd = CommandOf(type)) {
		cmds::Actor actor;
		std::string label, err;
		TileIndex tile;
		if (!cmds::IsExposable(*cmd)) { errors.push_back("this command has parameters that cannot be expressed in JSON"); valid = false; }
		if (valid && !ClientMayPost(*cmd, err)) { errors.push_back(err); valid = false; }
		if (valid && !ParseActor(args, actor, label, err)) { errors.push_back(err); valid = false; }
		if (valid && !ParseLocation(*cmd, args, tile, err)) { errors.push_back(err); valid = false; }
		if (valid) {
			cmds::TestResult t = cmds::Test(*cmd, actor, args.value("params", json::array()));
			if (!t.params_ok) { errors.push_back(t.error); valid = false; }
			result["would_succeed"] = t.would_succeed;
			result["estimated_cost"] = t.cost;
			if (!t.would_succeed && !t.error.empty()) result["test_error"] = t.error;
			result["actor"] = label;
		}
		result["command"] = cmds::Describe(*cmd);
		result["required_scope"] = "GAME_MUTATION";
	} else if (valid) {
		errors.push_back("unknown command");
		valid = false;
	}
	result["valid"] = valid;
	result["errors"] = errors;
	result["current_snapshot_id"] = fmt::format("tick-{}", (uint64_t)TimerGameTick::counter);
	result["approval_required"] = true;   // no mutation is ever auto-approved in this build
	result["mutating"] = true;
	return result;
}

/** Take the verdict of a decision whose commands have all run (or that ran none). */
void Conclude(ProvenanceRecord &p)
{
	p.observed_tick = (uint64_t)TimerGameTick::counter;
	p.command_result = p.failed ? "failed" : "succeeded";
	if (p.observe) {
		/* The effect, not the report: asked of the game after the last command ran. */
		p.verification = "observed_state";
		p.verdict = p.failed ? "VERIFIED_FAILURE" : (p.observe() ? "VERIFIED_SUCCESS" : "EXECUTED_UNVERIFIED");
	} else {
		p.verification = "command_result";
		p.verdict = p.failed ? "VERIFIED_FAILURE" : "VERIFIED_SUCCESS";
	}
	Debug(Facility::Net, Severity::Error, "[prismatic-mcp] decision #{} {} {} verdict={}", p.decision_id, p.action_type, p.command_result, p.verdict);
}

/** Register one executed result to come for a decision; called before each post. */
void Expect(ProvenanceRecord &pr)
{
	pr.callbacks_left++;
	_awaiting.push_back(pr.decision_id);
}

/** Withdraw the expectations of a decision whose post was refused before it was queued. */
void Unexpect(uint64_t decision_id)
{
	ProvenanceRecord *pr = FindDecision(decision_id);
	for (auto it = _awaiting.begin(); it != _awaiting.end();) {
		if (*it == decision_id) {
			it = _awaiting.erase(it);
			if (pr != nullptr && pr->callbacks_left > 0) pr->callbacks_left--;
		} else {
			++it;
		}
	}
}

/** Record a decision refused before anything was posted. @return false */
bool ExecuteRefused(uint64_t decision_id, const std::string &err)
{
	if (ProvenanceRecord *r = FindDecision(decision_id)) {
		r->failed = true;
		r->error = err;
		Conclude(*r);
		r->command_result = "rejected";
	}
	return false;
}

/**
 * Carry out an approved, digest-bound action. Main thread.
 * Commands go through the normal command path; on a network server they run on a later tick,
 * so the verdict is taken by OnCommandResult when the last of them has actually executed.
 * @return false when the action was refused before anything was queued; the record says why.
 */
bool ExecuteAction(std::string_view type, const json &args, ProvenanceRecord &pr)
{
	std::string err;
	bool ok = false;
	uint64_t did = pr.decision_id;
	if (type == "game.set_pause") {
		bool want = args["pause"].get<bool>();
		pr.cmd = Commands::Pause;
		pr.actor = "server";
		pr.observe = [want]() { return _pause_mode.Any() == want; };
		if (!ClientMayPost(Commands::Pause, err)) return ExecuteRefused(did, err);
		Expect(pr);
		/* SERVER_ADMIN synchronized command -- the deterministic path, not a raw write. */
		ok = Command<Commands::Pause>::Post(StringID{}, &CcPrismaticMcp, PauseMode::Normal, want);
		if (!ok) err = "the pause command was refused";
	} else if (ai::IsAiAction(type)) {
		pr.cmd = Commands::CompanyControl;
		pr.actor = "server";
		ai::Posted posted = ai::Execute(type, args, [did]() { if (ProvenanceRecord *r = FindDecision(did)) Expect(*r); });
		ok = posted.ok;
		err = posted.error;
		if (ProvenanceRecord *r = FindDecision(did)) {
			r->observe = posted.observe;
			/* A partly posted action keeps the expectations of what was posted. */
			while (r->callbacks_left > posted.callbacks) {
				auto it = std::find(_awaiting.rbegin(), _awaiting.rend(), did);
				if (it == _awaiting.rend()) break;
				_awaiting.erase(std::next(it).base());
				r->callbacks_left--;
			}
			if (ok && posted.callbacks == 0 && r->verdict == "PENDING_OBSERVATION") Conclude(*r);
		}
	} else if (auto cmd = CommandOf(type)) {
		cmds::Actor actor;
		TileIndex tile;
		pr.cmd = *cmd;
		if (ClientMayPost(*cmd, err) && ParseActor(args, actor, pr.actor, err) && ParseLocation(*cmd, args, tile, err)) {
			Expect(pr);
			ok = cmds::Post(*cmd, actor, args.value("params", json::array()), tile, err);
			if (!ok && err.empty()) {
				cmds::TestResult t = cmds::Test(*cmd, actor, args.value("params", json::array()));
				err = t.error.empty() ? "the command was refused before it was queued" : t.error;
			}
		}
	} else if (type.starts_with("api.")) {
		/* Posted in GSAsyncMode without a callback: the answer is the function's own (its result and
		 * the script's last error), taken as the command result. */
		pr.actor = "script_api";
		json out = Api::Execute(std::string{type}, args);
		ok = out.value("ok", false) && out["last_error"].is_null();
		if (!ok) err = out.contains("error") ? out["error"].dump() : out["last_error"].dump();
		if (ok) {
			if (ProvenanceRecord *r = FindDecision(did)) {
				if (_networking) {
					/* Posted to the network queue without a callback: it runs on a later tick, so
					 * nothing is verified here; read the effect back with the API's own getters. */
					r->observed_tick = (uint64_t)TimerGameTick::counter;
					r->command_result = "posted";
					r->verification = "none";
					r->verdict = "EXECUTED_UNVERIFIED";
				} else {
					Conclude(*r);
				}
			}
		}
	} else {
		err = "unknown action type";
	}
	ProvenanceRecord *r = FindDecision(did);
	if (!ok && r != nullptr) {
		/* Refused before it was queued: no result will come. A single-player post that failed
		 * has already been answered by its result, which keeps that verdict. */
		Unexpect(did);
		if (r->verdict == "PENDING_OBSERVATION") {
			r->failed = true;
			r->error = err;
			Conclude(*r);
			r->command_result = "rejected";
		}
	}
	return ok;
}

/** tools/call for the typed decision + mutation tools. */
json HandleToolsCall(const json &id, const json &params)
{
	if (!params.contains("name") || !params["name"].is_string()) {
		return RpcError(id, -32602, "Invalid params: missing tool name");
	}
	std::string name = params["name"].get<std::string>();
	json args = params.contains("arguments") ? params["arguments"] : json::object();

	/* Several resources at one game tick; reads only, so no approval. */
	if (name == "openttd.resources.read_many") {
		json out = ReadMany(args);
		bool ok = out.value("ok", false);
		return RpcResult(id, json{{"content", json::array({{{"type", "text"}, {"text", out.dump()}}})}, {"isError", !ok}});
	}

	/* The fork's toolbar menu and the questions asked through it (prismatic_mcp_menu.cpp); before the
	 * UI tools, whose names share the openttd.ui. prefix. */
	if (auto menu = Menu::Call(name, args); menu.has_value()) {
		bool ok = menu->value("ok", false);
		return RpcResult(id, json{{"content", json::array({{{"type", "text"}, {"text", menu->dump()}}})}, {"isError", !ok}});
	}

	/* UI tools: the interface as data, and armed input through the driver's path (prismatic_mcp_ui.cpp). */
	if (auto ui = UI::Call(name, args); ui.has_value()) {
		bool ok = ui->value("ok", false);
		return RpcResult(id, json{{"content", json::array({{{"type", "text"}, {"text", ui->dump()}}})}, {"isError", !ok}});
	}

	/* View tools: local viewport only, no approval, no command (prismatic_mcp_view.cpp). */
	if (auto view = View::Call(name, args); view.has_value()) {
		bool ok = view->value("ok", false);
		return RpcResult(id, json{{"content", json::array({{{"type", "text"}, {"text", view->dump()}}})}, {"isError", !ok}});
	}

	/* Non-mutating validation tool. */
	if (name == "openttd.decision.validate") {
		std::string type = args.value("action_type", args.contains("action") ? args["action"].value("type", "") : "");
		json aargs = args.contains("action") ? args["action"].value("arguments", json::object()) : args.value("arguments", json::object());
		json errs = json::array();
		json v = ValidateAction(type, aargs, errs);
		return RpcResult(id, json{{"content", json::array({{{"type", "text"}, {"text", v.dump()}}})}, {"isError", false}});
	}

	if (name == "openttd.decision.list_action_types") {
		return RpcResult(id, json{{"content", json::array({{{"type", "text"}, {"text", json{{"action_types", ActionTypes()}}.dump()}}})}, {"isError", false}});
	}

	if (name == "openttd.ai.list") {
		return RpcResult(id, json{{"content", json::array({{{"type", "text"}, {"text", ai::Resource().dump()}}})}});
	}

	if (name == "openttd.decision.get") {
		if (!args.contains("decision_id") || !args["decision_id"].is_number_unsigned()) {
			return RpcError(id, -32602, "Invalid params: decision_id (integer) required");
		}
		const ProvenanceRecord *p = FindDecision(args["decision_id"].get<uint64_t>());
		if (p == nullptr) return RpcResult(id, json{{"content", json::array({{{"type", "text"}, {"text", "{\"error\":\"no such decision\"}"}}})}, {"isError", true}});
		json d = {{"decision_id", p->decision_id}, {"approval_id", p->approval_id}, {"action_type", p->action_type},
			{"command_result", p->command_result}, {"verdict", p->verdict}, {"verification", p->verification},
			{"error", p->error}, {"cost", p->cost}, {"actor", p->actor},
			{"requested_tick", p->requested_tick}, {"observed_tick", p->observed_tick}};
		return RpcResult(id, json{{"content", json::array({{{"type", "text"}, {"text", d.dump()}}})}});
	}

	/* The Script API (prismatic_mcp_api.cpp): without execute a call runs in GSTestMode and changes
	 * nothing, so it needs no approval; with execute it joins the gated tools below. */
	if (name.starts_with("openttd.api.") && !Api::WantsExecution(name, args)) {
		json out = Api::Call(name, args).value_or(json{{"ok", false}});
		return RpcResult(id, json{{"content", json::array({{{"type", "text"}, {"text", out.dump()}}})}, {"isError", !out.value("ok", false)}});
	}

	/* The gated mutation tools. Two-phase: pending_approval -> (operator approves) -> executed. */
	bool is_cmd_tool = name.starts_with("openttd.cmd.");
	bool is_api_tool = Api::WantsExecution(name, args);
	bool is_ai_tool = name.starts_with("openttd.ai.") && ai::IsAiAction(name.substr(8));
	if (name == "openttd.game.set_pause" || name == "openttd.decision.submit" || is_cmd_tool || is_ai_tool || is_api_tool) {
		std::string type = "game.set_pause";
		json aargs = args;
		if (is_api_tool) {
			type = name.substr(8); // api.Script<Class>.<Function>
		} else if (is_ai_tool) {
			type = name.substr(8);
		} else if (is_cmd_tool) {
			type = "command." + name.substr(12);
		} else if (name == "openttd.decision.submit") {
			type = args.value("action", json::object()).value("type", "");
			aargs = args.value("action", json::object()).value("arguments", json::object());
		}
		json errs = json::array();
		json v = ValidateAction(type, aargs, errs);
		if (!v["valid"].get<bool>()) {
			return RpcResult(id, json{{"content", json::array({{{"type", "text"}, {"text", v.dump()}}})}, {"isError", true}});
		}

		std::string digest = ArgDigest(type, aargs);
		std::string canonical = aargs.dump();
		uint64_t now = (uint64_t)TimerGameTick::counter;

		/* The approval binds to the action AND its EXACT canonical arguments, not the 64-bit digest
		 * (which could collide): an approval for one tool is never spent by another tool with the same
		 * arguments. The decision -- execute an approved one, reuse a pending one, or ask the operator
		 * -- is the pure, unit-tested detail::ClassifySubmit. */
		std::vector<detail::ApprovalView> views;
		views.reserve(_approvals.size());
		for (const auto &a : _approvals) views.push_back({(uint8_t)a.state, a.tool, a.canonical_args, a.expiry_tick});
		size_t matched = 0;
		detail::SubmitDecision decision = detail::ClassifySubmit(views, type, canonical, now, matched);

		if (decision == detail::SubmitDecision::Execute) {
			/* An approval now executes when it is given (ApproveById), so one is approved and unspent
			 * only for the instant between; the one execution path serves this case too. */
			json out = ExecuteApproval(_approvals[matched], "resubmit");
			return RpcResult(id, json{{"content", json::array({{{"type", "text"}, {"text", out.dump()}}})}, {"isError", out["status"] != "executed"}});
		}

		if (decision == detail::SubmitDecision::ReusePending) {
			PendingApproval &a = _approvals[matched];
			json out = {{"status", "pending_approval"}, {"approval_id", a.id}, {"argument_digest", digest},
				{"message", "Approval required in OpenTTD: prismatic_mcp approve " + fmt::format("{}", a.id)}};
			return RpcResult(id, json{{"content", json::array({{{"type", "text"}, {"text", out.dump()}}})}});
		}

		/* A newer change of an AI setting of the same company withdraws the older pending one, before
		 * the queue is measured: approving the older afterwards would undo the newer for that setting
		 * (detail::Supersedes). */
		uint64_t new_id = _next_approval_id;
		for (auto &old : _approvals) {
			if (old.state != ApprovalState::Pending || !detail::Supersedes(type, aargs, old.tool, json::parse(old.canonical_args, nullptr, false))) continue;
			old.state = ApprovalState::Denied;
			old.superseded_by = new_id;
			Debug(Facility::Net, Severity::Error, "[prismatic-mcp] approval #{} superseded by #{} ({})", old.id, new_id, type);
		}
		PendingApproval pa;
		pa.id = _next_approval_id++;
		pa.tool = type;
		pa.arg_digest = digest;
		pa.canonical_args = canonical;
		pa.args_summary = aargs.dump();
		pa.mcp_tool = std::string(name);
		pa.mcp_args = args.dump();
		pa.created_tick = now;
		/* The deadline's approve stands in only for an approver who played this company when it was asked. */
		pa.approver_at_creation = detail::ApproverPlays(CurrentApproverSeats(), CompanyOf(pa));
		/* An action that names its own stale date (ai.set_settings' expires_tick, bound with the args)
		 * expires then, if that is sooner than the TTL; its execution re-checks the date as well. */
		pa.expiry_tick = detail::ApprovalExpiry(now, APPROVAL_TTL_TICKS, detail::BoundExpiresTick(type, aargs));
		pa.state = ApprovalState::Pending;
		/* A full queue drops what is decided or expired, never a pending approval: a new request that
		 * still does not fit is refused, with the reason, rather than silently pushing out one the
		 * operator has not answered (ADR 0038). One a standing rule decides at once (ADR 0041) waits
		 * for nobody, so it is decided even when the queue is full. */
		const bool ruled = RuleDecidesNow(pa, RuleVerdict(pa, aargs), now);
		if (_approvals.size() >= MAX_APPROVALS) {
			std::erase_if(_approvals, [now](const PendingApproval &a) { return a.state != ApprovalState::Pending || now > a.expiry_tick; });
		}
		if (_approvals.size() >= MAX_APPROVALS && !ruled) {
			_next_approval_id--; // not queued: the id is free again
			Debug(Facility::Net, Severity::Error, "[prismatic-mcp] approval queue full ({} pending): {} refused", _approvals.size(), type);
			json out = {{"status", "refused"}, {"error", fmt::format("the approval queue is full: {} decisions wait for the operator; approve or deny some first (prismatic_mcp approvals)", _approvals.size())}};
			return RpcResult(id, json{{"content", json::array({{{"type", "text"}, {"text", out.dump()}}})}, {"isError", true}});
		}
		_approvals.push_back(pa);
		Debug(Facility::Net, Severity::Error, "[prismatic-mcp] pending approval #{} for {} args={} (approve in console: prismatic_mcp approve {})",
			pa.id, type, pa.args_summary, pa.id);
		/* The owner's standing rules decide it at once when one applies (ADR 0041). */
		if (auto decided = ApplyRules(_approvals.back()); decided.has_value()) {
			return RpcResult(id, json{{"content", json::array({{{"type", "text"}, {"text", decided->dump()}}})}, {"isError", (*decided)["status"] != "executed"}});
		}
		json out = {{"status", "pending_approval"}, {"approval_id", pa.id}, {"argument_digest", digest},
			{"expires_at_tick", pa.expiry_tick},
			{"message", "Approval required in OpenTTD: prismatic_mcp approve " + fmt::format("{}", pa.id)}};
		return RpcResult(id, json{{"content", json::array({{{"type", "text"}, {"text", out.dump()}}})}});
	}

	return RpcError(id, -32602, "Unknown tool");
}

/** Static tool catalogue for tools/list. */
json ToolsList()
{
	json arr = json::array();
	/* View tools first: they change only the local viewport and need no approval. */
	for (auto &t : View::Tools()) arr.push_back(t);
	for (auto &t : UI::Tools()) arr.push_back(t);
	for (auto &t : Menu::Tools()) arr.push_back(t);
	/* The decision tools. Their action types come from ActionTypes(); their annotations say
	 * what a client (and the coverage probe, which calls only readOnlyHint tools) may assume. */
	const json types = ActionTypes();
	const json action = {{"type", "object"}, {"required", json::array({"type"})},
		{"properties", {{"type", {{"type", "string"}, {"enum", types}}}, {"arguments", {{"type", "object"}}}}}};
	const json reads = {{"readOnlyHint", true}, {"idempotentHint", true}, {"openWorldHint", false}};
	const json gated = {{"readOnlyHint", false}, {"destructiveHint", false}, {"idempotentHint", false}, {"openWorldHint", false}};
	arr.push_back({{"name", "openttd.decision.validate"}, {"description", "Validate a typed action/DecisionEnvelope without mutating game state: {action: {type, arguments}} or {action_type, arguments}"},
		{"inputSchema", {{"type", "object"}, {"properties", {{"action", action}, {"action_type", {{"type", "string"}, {"enum", types}}}, {"arguments", {{"type", "object"}}}}}}},
		{"annotations", reads}});
	arr.push_back({{"name", "openttd.decision.list_action_types"}, {"description", "List typed action types this build can execute"},
		{"inputSchema", {{"type", "object"}}}, {"annotations", reads}});
	arr.push_back({{"name", "openttd.decision.submit"}, {"description", "Submit a DecisionEnvelope; returns a pending approval until an operator approves, then executes via the normal command path"},
		{"inputSchema", {{"type", "object"}, {"required", json::array({"action"})}, {"properties", {{"action", action}}}}},
		{"annotations", gated}});
	arr.push_back({{"name", "openttd.game.set_pause"}, {"description", "Approval-gated: pause/unpause via Commands::Pause. Requires operator approval (two-phase, digest-bound)"},
		{"inputSchema", {{"type", "object"}, {"properties", {{"pause", {{"type", "boolean"}}}}}, {"required", json::array({"pause"})}}},
		{"annotations", gated}});
	arr.push_back({{"name", "openttd.decision.get"}, {"description", "Read one decision's provenance: command result and the verdict taken after the command ran"},
		{"inputSchema", {{"type", "object"}, {"properties", {{"decision_id", {{"type", "integer"}, {"minimum", 1}}}}}, {"required", json::array({"decision_id"})}}}});
	arr.push_back(detail::ReadManyTool());
	for (auto &t : ai::Tools()) arr.push_back(t);
	/* Every static function of the Game Script API, derived from its registration (prismatic_mcp_api.cpp). */
	for (auto &t : Api::Tools()) arr.push_back(t);
	/* One tool per game command whose parameters convert from JSON, derived from the command table. */
	for (size_t i = 0; i < cmds::Count(); i++) {
		Commands c = static_cast<Commands>(i);
		if (!cmds::IsExposable(c)) continue;
		json d = cmds::Describe(c);
		std::string desc = fmt::format("Approval-gated game command {} ({}; flags: {}). Validates by a side-effect-free test run, "
			"executes through the network-safe command path; verdict via openttd.decision.get.",
			d["function"].get<std::string>(), d["category"].get<std::string>(), d["flags"].dump());
		arr.push_back({{"name", "openttd.cmd." + std::string(cmds::ShortName(c))}, {"description", desc},
			{"inputSchema", cmds::ToolInputSchema(c)}});
	}
	/* MCP annotations for the decision, AI and derived command tools, from how each is handled:
	 * a tool that goes through the approval path mutates the game; the rest only read. */
	for (auto &t : arr) {
		std::string n = t["name"].get<std::string>();
		bool mine = n.starts_with("openttd.decision.") || n == "openttd.game.set_pause" || n.starts_with("openttd.cmd.") || n.starts_with("openttd.ai.");
		if (!mine || t.contains("annotations")) continue;
		bool gated = n == "openttd.decision.submit" || n == "openttd.game.set_pause" || n.starts_with("openttd.cmd.")
			|| (n.starts_with("openttd.ai.") && ai::IsAiAction(n.substr(8)));
		t["annotations"] = gated
			? json{{"readOnlyHint", false}, {"destructiveHint", true}, {"idempotentHint", false}, {"openWorldHint", false}}
			: json{{"readOnlyHint", true}, {"openWorldHint", false}};
	}
	return json{{"tools", arr}};
}

/** The resources/list answer: every entry of RESOURCES. */
json ResourcesList()
{
	json arr = json::array();
	for (const auto &r : RESOURCES) {
		arr.push_back({{"uri", r.uri}, {"name", r.name}, {"description", r.description}, {"mimeType", "application/json"}});
	}
	return json{{"resources", arr}};
}

/** What this server can do: what it registers (tools/list, resources/list) and whether this
 * process is armed for interface input. */
detail::Capabilities ServerCapabilities()
{
	return detail::CapabilitiesOf(ToolsList()["tools"], ResourcesList()["resources"].size(), UI::InputArmed());
}

/** Dispatch a single JSON-RPC request object. Returns std::nullopt for notifications. */
std::optional<json> DispatchOne(const json &req)
{
	json id = req.contains("id") ? req["id"] : json(nullptr);
	bool is_notification = !req.contains("id");

	if (!req.contains("method") || !req["method"].is_string()) {
		return is_notification ? std::nullopt : std::optional<json>(RpcError(id, -32600, "Invalid Request: missing method"));
	}
	std::string method = req["method"].get<std::string>();

	if (method == "initialize") {
		json caps = {{"resources", json::object()}, {"tools", json::object()}, {"logging", json::object()}};
		json result = {
			{"protocolVersion", std::string(PROTOCOL_VERSION)},
			{"capabilities", caps},
			{"serverInfo", {{"name", "openttd-prismatic-mcp"}, {"version", _openttd_revision}}},
			{"instructions", "RESEARCH-ONLY embedded OpenTTD MCP server. Read-only resources are live. "
			 "Every game command whose parameters convert from JSON is a tool openttd.cmd.<name> (catalogue: "
			 "openttd://commands), plus openttd.game.set_pause. Each mutation is validated by a side-effect-free "
			 "test run (openttd.decision.validate), then requires operator approval ('prismatic_mcp approve <id>' "
			 "in the OpenTTD console, the panel's Approve, or an approver's '!approve <id>' in the game's chat), "
			 "which posts it through the normal command path at once: do not resubmit it (a resubmit asks for a "
			 "fresh approval). The queue is openttd://approvals. The verdict is taken after the command has run: read it with openttd.decision.get."},
		};
		return RpcResult(id, result);
	}
	if (method == "notifications/initialized") return std::nullopt; // notification, no reply
	if (method == "ping") return RpcResult(id, json::object());

	if (method == "resources/list") return RpcResult(id, ResourcesList());
	if (method == "resources/read") {
		if (!req.contains("params") || !req["params"].contains("uri")) {
			return RpcError(id, -32602, "Invalid params: missing uri");
		}
		std::string uri = req["params"]["uri"].get<std::string>();
		Coverage::Count("resource", uri);
		const ResourceEntry *r = FindResource(uri);
		if (r == nullptr) return RpcError(id, -32602, "Unknown resource uri");
		json contents = json::array();
		contents.push_back({{"uri", r->uri}, {"mimeType", "application/json"}, {"text", r->build().dump()}});
		return RpcResult(id, json{{"contents", contents}});
	}
	if (method == "tools/list") return RpcResult(id, ToolsList());
	if (method == "tools/call") {
		if (req.contains("params") && req["params"].contains("name") && req["params"]["name"].is_string()) Coverage::Count("tool", req["params"]["name"].get<std::string>());
		if (!req.contains("params")) return RpcError(id, -32602, "Invalid params");
		return HandleToolsCall(id, req["params"]);
	}
	if (method == "prompts/list") return RpcResult(id, json{{"prompts", json::array()}});

	return is_notification ? std::nullopt : std::optional<json>(RpcError(id, -32601, "Method not found"));
}

/* ---- HTTP request handling ------------------------------------------------ */

/** Build a minimal HTTP/1.1 response with a JSON body and Connection: close. */
std::string HttpResponse(int status, std::string_view reason, std::string_view body, std::string_view extra_headers = "")
{
	std::string out;
	out += "HTTP/1.1 " + fmt::format("{}", status) + " " + std::string(reason) + "\r\n";
	out += "Content-Type: application/json\r\n";
	out += "Content-Length: " + fmt::format("{}", body.size()) + "\r\n";
	if (!extra_headers.empty()) out += std::string(extra_headers);
	out += "Connection: close\r\n\r\n";
	out += std::string(body);
	return out;
}

/** Parse and answer one fully-received HTTP request. Returns the raw response bytes. */
std::string HandleHttpRequest(std::string_view raw, size_t header_end, size_t content_length)
{
	_mcp.requests_total++;

	/* Request line: METHOD SP path SP HTTP/1.1 */
	size_t line_end = raw.find("\r\n");
	std::string_view request_line = raw.substr(0, line_end);
	size_t sp1 = request_line.find(' ');
	size_t sp2 = request_line.find(' ', sp1 + 1);
	if (sp1 == std::string_view::npos || sp2 == std::string_view::npos) {
		_mcp.requests_denied++;
		return HttpResponse(400, "Bad Request", R"({"error":"malformed request line"})");
	}
	std::string_view http_method = request_line.substr(0, sp1);
	std::string_view path = request_line.substr(sp1 + 1, sp2 - sp1 - 1);

	/* Header scan for Authorization + Origin. Header section is [line_end+2, header_end). */
	std::string_view headers = raw.substr(line_end + 2, header_end - (line_end + 2));
	std::string_view auth, origin;
	size_t pos = 0;
	while (pos < headers.size()) {
		size_t nl = headers.find("\r\n", pos);
		if (nl == std::string_view::npos) nl = headers.size();
		std::string_view h = headers.substr(pos, nl - pos);
		size_t colon = h.find(':');
		if (colon != std::string_view::npos) {
			std::string_view name = TrimOWS(h.substr(0, colon));
			std::string_view value = TrimOWS(h.substr(colon + 1));
			/* case-insensitive header name compare */
			auto ieq = [](std::string_view a, std::string_view b) {
				if (a.size() != b.size()) return false;
				for (size_t i = 0; i < a.size(); i++) if (tolower(a[i]) != tolower(b[i])) return false;
				return true;
			};
			if (ieq(name, "authorization")) auth = value;
			else if (ieq(name, "origin")) origin = value;
		}
		pos = nl + 2;
	}

	/* Loopback-only + Origin policy: reject any browser Origin (mission section 12). */
	if (!origin.empty()) {
		_mcp.requests_denied++;
		return HttpResponse(403, "Forbidden", R"({"error":"Origin header not allowed for loopback MCP endpoint"})");
	}

	if (path != "/mcp") {
		return HttpResponse(404, "Not Found", R"({"error":"only /mcp is served"})");
	}

	/* Bearer auth, constant-time. */
	std::string_view bearer = detail::ExtractBearer(auth);
	if (!TokenMatches(bearer)) {
		_mcp.requests_denied++;
		return HttpResponse(401, "Unauthorized", R"({"error":"missing or invalid bearer token"})",
			"WWW-Authenticate: Bearer\r\n");
	}

	if (http_method == "DELETE") {
		/* Explicit session termination: this stateless build has no session table yet. */
		return HttpResponse(200, "OK", R"({"ok":true})");
	}
	if (http_method != "POST") {
		return HttpResponse(405, "Method Not Allowed", R"({"error":"POST required for JSON-RPC"})",
			"Allow: POST, DELETE\r\n");
	}

	std::string_view body = raw.substr(header_end, content_length);
	json parsed;
	try {
		parsed = json::parse(body);
	} catch (const json::exception &) {
		return HttpResponse(200, "OK", RpcError(nullptr, -32700, "Parse error").dump());
	}

	/* Reject JSON-RPC batches explicitly (mission: batch rejected). */
	if (parsed.is_array()) {
		return HttpResponse(200, "OK", RpcError(nullptr, -32600, "Batch requests are not supported").dump());
	}

	std::optional<json> reply = DispatchOne(parsed);
	if (!reply.has_value()) {
		/* Notification: 202 Accepted with empty body per Streamable HTTP. */
		return HttpResponse(202, "Accepted", "");
	}
	return HttpResponse(200, "OK", reply->dump());
}

/* ---- socket servicing (main thread, non-blocking) ------------------------- */

void CloseConnection(Connection &c)
{
	if (c.sock != INVALID_SOCKET) closesocket(c.sock);
	c.sock = INVALID_SOCKET;
}

/** Try to parse headers once enough bytes have arrived; sets content_length. */
bool TryCompleteHeaders(Connection &c)
{
	if (c.headers_done) return true;
	size_t end = c.inbuf.find("\r\n\r\n");
	if (end == std::string::npos) {
		if (c.inbuf.size() > MAX_HEADER_BYTES) { CloseConnection(c); }
		return false;
	}
	c.header_end = end + 4;
	c.headers_done = true;

	/* Content-Length via the shared pure parser (case-insensitive). */
	std::optional<size_t> cl = detail::ParseContentLength(std::string_view(c.inbuf).substr(0, c.header_end));
	if (cl.has_value()) {
		c.content_length = *cl;
		if (c.content_length > MAX_REQUEST_BODY) { CloseConnection(c); return false; }
	}
	return true;
}

void ServiceConnection(Connection &c)
{
	if (c.sock == INVALID_SOCKET) return;

	/* Read whatever is available (non-blocking). */
	char buf[4096];
	for (;;) {
		ssize_t n = recv(c.sock, buf, sizeof(buf), 0);
		if (n > 0) {
			c.inbuf.append(buf, (size_t)n);
			if (c.inbuf.size() > MAX_REQUEST_BODY + MAX_HEADER_BYTES) { CloseConnection(c); return; }
			continue;
		}
		if (n == 0) { CloseConnection(c); return; } // peer closed
		NetworkError err = NetworkError::GetLast();
		if (err.WouldBlock()) break;
		CloseConnection(c);
		return;
	}

	if (!TryCompleteHeaders(c)) return;
	if (c.inbuf.size() < c.header_end + c.content_length) return; // body still arriving

	std::string response = HandleHttpRequest(c.inbuf, c.header_end, c.content_length);
	/* Best-effort blocking-ish send of the whole response, then close. */
	size_t sent = 0;
	while (sent < response.size()) {
		ssize_t n = send(c.sock, response.data() + sent, response.size() - sent, 0);
		if (n > 0) { sent += (size_t)n; continue; }
		NetworkError err = NetworkError::GetLast();
		if (err.WouldBlock()) continue;
		break;
	}
	CloseConnection(c);
}

} // anonymous namespace

json Menu::CallTool(std::string_view name, const json &args)
{
	/* The menu calls a tool the way a client's tools/call does, and gets the call's result. */
	json resp = HandleToolsCall(nullptr, {{"name", name}, {"arguments", args}});
	if (resp.contains("result")) return resp["result"];
	return json{{"isError", true}, {"error", resp.value("error", json::object())}};
}

/* ---- public API ----------------------------------------------------------- */

void Init()
{
	/* Nothing heavy at startup; the listener is created only on explicit Start(). */
	_mcp.running = false;
	/* The approval deadline the operator chose when starting this process (mcp-e2e
	 * --approval-deadline, research/mechanisms/approval-gate.md "The deadline"): a dedicated server
	 * has no console of its own to type it at, and the panel refuses an 'approve' line. */
	if (const char *env = std::getenv("PRISMATIC_MCP_APPROVAL_DEADLINE"); env != nullptr && *env != '\0') {
		if (auto policy = detail::ParseDeadlinePolicyText(env); policy.has_value()) {
			_deadline_policy = *policy;
			Debug(Facility::Net, Severity::Error, "[prismatic-mcp] {} (PRISMATIC_MCP_APPROVAL_DEADLINE)", DescribeApprovalDeadline());
		} else {
			Debug(Facility::Net, Severity::Error, "[prismatic-mcp] PRISMATIC_MCP_APPROVAL_DEADLINE='{}' is not <game days 0-3650>[:keep|approve]; the default stays: {}", env, DescribeApprovalDeadline());
		}
	}
}

/**
 * Does something already accept TCP connections on loopback `port`? Listen() sets SO_REUSEPORT
 * (os_abstraction.cpp), so on macOS and Linux a second process can listen on the same port and the
 * kernel then spreads connections between the two: a client would reach either server at random
 * (seen 2026-11-16, two OpenTTD MCP servers on 8731). A connect probe before Listen catches the
 * real case; two processes starting in the same instant can still race.
 */
static bool PortHasListener(uint16_t port)
{
	for (const char *host : {"127.0.0.1", "::1"}) {
		NetworkAddress addr(host, port);
		const sockaddr_storage *ss = addr.GetAddress();
		SOCKET s = socket(ss->ss_family, SOCK_STREAM, 0);
		if (s == INVALID_SOCKET) continue;
		bool accepted = connect(s, reinterpret_cast<const sockaddr *>(ss), addr.GetAddressLength()) == 0;
		closesocket(s);
		if (accepted) return true;
	}
	return false;
}

bool Start(uint16_t port, std::string &error)
{
	if (_mcp.running) { error = "already running"; return false; }
	_mcp.port = (port == 0) ? DEFAULT_PORT : port;
	if (PortHasListener(_mcp.port)) {
		error = fmt::format("port {} already has a listener (another OpenTTD with MCP?); pick a free port", _mcp.port);
		_mcp.last_error = error;
		return false;
	}

	/* Bind loopback only: construct explicit v4 + v6 loopback addresses rather
	 * than using the all-interfaces default in GetBindAddresses(). */
	_mcp.listeners.clear();
	NetworkAddress v4("127.0.0.1", _mcp.port);
	v4.Listen(SOCK_STREAM, &_mcp.listeners);
	/* The advertised endpoint is 127.0.0.1: without the IPv4 listener a client would reach
	 * whatever else holds that port, so a failed IPv4 bind is a failed start. */
	if (_mcp.listeners.empty()) {
		error = "could not bind 127.0.0.1:" + fmt::format("{}", _mcp.port) + " (in use by another process?)";
		_mcp.last_error = error;
		return false;
	}
	NetworkAddress v6("::1", _mcp.port);
	v6.Listen(SOCK_STREAM, &_mcp.listeners);

	RandomBytesWithFallback(_mcp.token);
	_mcp.token_valid = true;
	_mcp.running = true;
	_mcp.requests_total = 0;
	_mcp.requests_denied = 0;
	Debug(Facility::Net, Severity::Error, "[prismatic-mcp] RESEARCH-ONLY MCP server listening on {} (loopback only)", GetEndpoint());
	/* The owner's persisted standing rules, each said aloud (ADR 0041). */
	LoadApprovalRules();
	Registry::Announce(GetEndpoint(), _mcp.port); // discoverable by any reader of the instance registry
	return true;
}

/**
 * Forget every approval, granted or pending. An approval binds an action to the operator's consent in
 * one session of one world: it must not outlive the server, its token, or the game it was given in.
 * This is the one point where session- and world-bound decision state is dropped: anything else that
 * waits on the old session or world (results still to be observed) is cleared here too.
 */
static void ClearApprovals(detail::ResetReason reason)
{
	const detail::ResetEffect effect = detail::EffectOf(reason);
	const std::string_view why = detail::ReasonText(reason);
	if (effect.clear_approvals && !_approvals.empty()) {
		Debug(Facility::Net, Severity::Error, "[prismatic-mcp] {} approval(s) cleared: {}", _approvals.size(), why);
		_approvals.clear();
	}
	/* A standing rule may name a company or a tile of this world: when the world is gone, only the
	 * owner's persisted rules (--persist) stay (ADR 0041). */
	if (effect.invalidate_awaiting) {
		size_t before = _rules.size();
		std::erase_if(_rules, [](const Rules::Rule &r) { return !r.persisted; });
		if (_rules.size() != before) Debug(Facility::Net, Severity::Error, "[prismatic-mcp] {} standing rule(s) of this game ended: {}", before - _rules.size(), why);
	}
	/* Commands already posted still run in the same world after a stop or a token rotation, and
	 * their results still arrive. Only when the world itself is gone will they never come: the
	 * decisions that wait for them are closed as INVALIDATED, so none waits forever. */
	if (!effect.invalidate_awaiting || _awaiting.empty()) return;
	size_t n = 0;
	for (uint64_t id : _awaiting) {
		ProvenanceRecord *p = FindDecision(id);
		if (p == nullptr || !detail::ClosedByReset(p->verdict)) continue;
		p->verdict = "INVALIDATED";
		p->command_result = "unobserved";
		p->verification = "none";
		p->error = std::string(why);
		p->callbacks_left = 0;
		p->observed_tick = (uint64_t)TimerGameTick::counter;
		n++;
	}
	_awaiting.clear();
	Debug(Facility::Net, Severity::Error, "[prismatic-mcp] {} decision(s) invalidated: {}", n, why);
}

void Stop()
{
	if (!_mcp.running && _mcp.listeners.empty()) return;
	Registry::Withdraw();
	for (auto &c : _mcp.conns) CloseConnection(c);
	_mcp.conns.clear();
	for (auto &pair : _mcp.listeners) closesocket(pair.first);
	_mcp.listeners.clear();
	_mcp.token.fill(0);
	_mcp.token_valid = false;
	_mcp.running = false;
	ClearApprovals(detail::ResetReason::ServerStopped);
	Debug(Facility::Net, Severity::Error, "[prismatic-mcp] MCP server stopped");
}

void Shutdown()
{
	Stop();
}

void OnGameModeChanged()
{
	ClearApprovals(detail::ResetReason::GameModeChanged);
}

/** The tick the last poll saw; a smaller one means another world (detail::WorldChanged). */
static uint64_t _last_poll_tick = 0;

static void SettleDueApprovals(); // below, with the approvers
static void RecordApproverSeatChanges(); // below, with the approvers

void Poll()
{
	if (!_mcp.running || _mcp.listeners.empty()) return;
	const uint64_t now_tick = TimerGameTick::counter;
	if (detail::WorldChanged(_last_poll_tick, now_tick)) ClearApprovals(detail::ResetReason::WorldReplaced);
	_last_poll_tick = now_tick;

	fd_set read_fds;
	FD_ZERO(&read_fds);
	SOCKET max_fd = 0;
	for (auto &pair : _mcp.listeners) {
		FD_SET(pair.first, &read_fds);
		if (pair.first > max_fd) max_fd = pair.first;
	}
	for (auto &c : _mcp.conns) {
		if (c.sock == INVALID_SOCKET) continue;
		FD_SET(c.sock, &read_fds);
		if (c.sock > max_fd) max_fd = c.sock;
	}

	timeval tv = {0, 0}; // never block the game loop
	if (select(max_fd + 1, &read_fds, nullptr, nullptr, &tv) <= 0) {
		/* Still service any connections that already have buffered work. */
	}

	/* Accept new connections. */
	for (auto &pair : _mcp.listeners) {
		if (!FD_ISSET(pair.first, &read_fds)) continue;
		for (;;) {
			sockaddr_storage ss;
			socklen_t slen = sizeof(ss);
			SOCKET s = accept(pair.first, (sockaddr *)&ss, &slen);
			if (s == INVALID_SOCKET) break;
			if (_mcp.conns.size() >= MAX_CONNECTIONS) { closesocket(s); break; }
			SetNonBlocking(s);
			SetNoDelay(s);
			Connection c;
			c.sock = s;
			_mcp.conns.push_back(std::move(c));
		}
	}

	/* Service existing connections. */
	for (auto &c : _mcp.conns) ServiceConnection(c);

	/* Who could approve from where, recorded as it changes; then an approval nobody answered is
	 * settled at its deadline, so no requester waits forever. */
	RecordApproverSeatChanges();
	SettleDueApprovals();

	/* Reap closed connections. */
	_mcp.conns.erase(
		std::remove_if(_mcp.conns.begin(), _mcp.conns.end(), [](const Connection &c) { return c.sock == INVALID_SOCKET; }),
		_mcp.conns.end());
}

bool IsRunning() { return _mcp.running; }

std::string GetEndpoint()
{
	if (!_mcp.running) return "";
	return "http://127.0.0.1:" + fmt::format("{}", _mcp.port) + "/mcp";
}

std::string GetToken()
{
	if (!_mcp.token_valid) return "";
	return ToHex(_mcp.token);
}

void RotateToken()
{
	if (!_mcp.running) return;
	RandomBytesWithFallback(_mcp.token);
	_mcp.token_valid = true;
	/* No session table yet; rotation invalidates the old bearer value and what was approved under it. */
	ClearApprovals(detail::ResetReason::TokenRotated);
	Debug(Facility::Net, Severity::Error, "[prismatic-mcp] bearer token rotated");
}

void OnCommandResult(Commands cmd, const CommandCost &result)
{
	for (auto it = _awaiting.begin(); it != _awaiting.end(); ++it) {
		ProvenanceRecord *p = FindDecision(*it);
		if (p == nullptr || p->cmd != cmd) continue;
		_awaiting.erase(it);
		p->cost += static_cast<int64_t>(result.GetCost());
		if (result.Failed()) {
			p->failed = true;
			std::string e = cmds::ErrorText(result);
			p->error = p->error.empty() ? e : p->error + "; " + e;
		}
		if (p->callbacks_left > 0) p->callbacks_left--;
		if (p->callbacks_left == 0) Conclude(*p);
		return;
	}
	Debug(Facility::Net, Severity::Error, "[prismatic-mcp] executed result for command {} that no decision awaits", to_underlying(cmd));
}


std::string ListApprovals()
{
	if (_approvals.empty()) return "no approvals";
	uint64_t now = (uint64_t)TimerGameTick::counter;
	std::string s;
	for (const auto &a : _approvals) {
		const char *st = a.state == ApprovalState::Pending ? "PENDING" :
			a.state == ApprovalState::Approved ? "APPROVED" :
			a.state == ApprovalState::Denied ? "DENIED" : "CONSUMED";
		bool expired = now > a.expiry_tick && a.state == ApprovalState::Pending;
		auto due = detail::DeadlineTick(a.created_tick, _deadline_policy.ticks);
		s += fmt::format("#{} {} {}tool={} args={} expires_tick={}{}{}\n", a.id, st, expired ? "(EXPIRED) " : "",
			a.tool, a.args_summary, a.expiry_tick,
			a.state == ApprovalState::Pending && due.has_value() ? fmt::format(" deadline_tick={} ({})", *due, detail::DeadlineVerdictName(_deadline_policy.verdict)) : "",
			a.settled_by.empty() ? "" : " settled_by=" + a.settled_by);
	}
	return s;
}

/* The pending queue, read rather than parsed out of ListApprovals()' prose: the console needs
 * the ids for 'deny all' and the tool and arguments to print for an id the operator names, the
 * control window its rows. This decides nothing: every id still goes through ApproveById or
 * DenyById, one at a time, also in bulk (DecideAll) and when a standing rule decides it. */
std::vector<PendingApprovalInfo> PendingApprovals()
{
	uint64_t now = (uint64_t)TimerGameTick::counter;
	std::vector<PendingApprovalInfo> out;
	for (const auto &a : _approvals) {
		if (a.state != ApprovalState::Pending) continue;
		PendingApprovalInfo info{a.id, a.tool, a.canonical_args, now > a.expiry_tick, a.mcp_tool};
		Rules::Verdict v = RuleVerdict(a, json::parse(a.canonical_args, nullptr, false));
		if (const Rules::Rule *r = v.decides != nullptr ? v.decides : v.blocked; r != nullptr) {
			info.matched_rule = r->id;
			info.rule_decides = v.decides == r && RuleDecidesNow(a, v, now);
		}
		out.push_back(std::move(info));
	}
	return out;
}

/* ---- bulk consent and the rules' lifecycle (ADR 0041) ------------------------------------- */

std::string RulesFile()
{
	return fmt::format("{}prismatic_mcp_approval_rules.{}.json", _personal_dir, _settings_client.network.server_port);
}

/** Write every persisted rule through a temporary file renamed over the old one, readable by its owner only. */
static bool SaveRules()
{
	json list = json::array();
	for (const auto &r : _rules) {
		if (!r.persisted) continue;
		json j = Rules::ToJson(r);
		j.erase("hits");
		j.erase("text");
		j.erase("id");
		j.erase("persisted");
		list.push_back(std::move(j));
	}
	std::string tmp = RulesFile() + ".tmp";
	std::error_code ec;
	std::filesystem::remove(tmp, ec);
	{
#if !defined(_WIN32)
		int fd = open(tmp.c_str(), O_WRONLY | O_CREAT | O_EXCL, 0600);
		if (fd < 0) return false;
		close(fd);
#endif
		auto f = FileHandle::Open(tmp, "w");
		bool written = f.has_value();
		if (written) {
			fmt::print(*f, "{}\n", json{{"schema", "prismatic-approval-rules/v1"},
				{"note", "RESEARCH-ONLY (ADR 0041): the owner's standing approval rules, made with --persist; prismatic_mcp rules lists them"},
				{"rules", std::move(list)}}.dump(2));
			written = fflush(*f) == 0;
		}
		if (!written) {
			f.reset();
			std::filesystem::remove(tmp, ec);
			return false;
		}
	}
	std::filesystem::rename(tmp, RulesFile(), ec);
	if (ec) std::filesystem::remove(tmp, ec);
	return !ec;
}

static std::string _rules_loaded_from; ///< the rules file read already; another port's file is read afresh

std::vector<std::string> LoadApprovalRules()
{
	std::vector<std::string> said;
	if (_rules_loaded_from == RulesFile()) return said;
	_rules_loaded_from = RulesFile();
	auto say = [&said]() {
		for (const auto &line : said) {
			Debug(Facility::Net, Severity::Error, "[prismatic-mcp] {}", line);
			IConsolePrint(CC_WARNING, "[prismatic-mcp] {}", line);
		}
		return said;
	};
#if !defined(_WIN32)
	/* Only the owner's own file, readable by nobody else, is the owner's standing consent. */
	struct stat st{};
	if (stat(RulesFile().c_str(), &st) != 0) return said;
	if (st.st_uid != getuid() || (st.st_mode & 077) != 0) {
		said.push_back(fmt::format("rules file {} refused: it must belong to this user and be readable by nobody else (mode 0600); no rule loaded from it", RulesFile()));
		return say();
	}
#endif
	auto f = FileHandle::Open(RulesFile(), "r");
	if (!f.has_value()) return said;
	std::string text;
	char buf[4096];
	for (size_t n; (n = fread(buf, 1, sizeof(buf), *f)) > 0;) text.append(buf, n);
	json doc = json::parse(text, nullptr, false);
	const json list = doc.is_object() && doc.contains("rules") && doc["rules"].is_array() ? doc["rules"] : json::array();
	for (const json &j : list) {
		auto r = Rules::FromJson(j);
		if (!r.has_value()) {
			said.push_back(fmt::format("rules file {}: skipped an entry that is not a rule: {}", RulesFile(), j.dump()));
			continue;
		}
		r->id = _next_rule_id++;
		const std::string made_via = r->created_via;
		/* A rule read from a file is the file's: it never decides an owner-only tool (Rules::OwnersOwnChannel). */
		r->created_via = "file";
		_rules.push_back(*r);
		said.push_back(fmt::format("standing approval {} loaded from {} (made via {} at {})", Rules::Describe(*r), RulesFile(), made_via, r->created_at));
		/* Anyone running as this user can write the file: its allow rules are held like the auto channel's. */
		if (r->effect == Rules::Effect::Allow) said.push_back(Rules::FileRuleNote(r->id));
	}
	if (doc.is_discarded()) said.push_back(fmt::format("rules file {} is not JSON; no rule loaded from it", RulesFile()));
	return say();
}

std::vector<Rules::Rule> ApprovalRules() { return _rules; }

/** The record of one approval acted on: its id, what happened, its tool and its arguments. */
static std::string OutcomeLine(const PendingApproval &a, std::string_view what)
{
	return fmt::format("#{}: {} -- tool={} args={}", a.id, what, a.mcp_tool.empty() ? a.tool : a.mcp_tool, a.canonical_args);
}

BulkOutcome AddApprovalRule(Rules::Effect effect, const Rules::Spec &spec)
{
	BulkOutcome out;
	if (!spec.error.empty()) { out.ok = false; out.lines.push_back(spec.error); return out; }
	if (auto err = Rules::MatcherError(spec.match); err.has_value()) { out.ok = false; out.lines.push_back(*err); return out; }
	if (auto err = Rules::BroadAllowRefusal(effect, spec); err.has_value()) { out.ok = false; out.lines.push_back(*err); return out; }
	Rules::Rule r;
	r.id = _next_rule_id++;
	r.effect = effect;
	r.tool = spec.tool.empty() ? "*" : spec.tool;
	r.match = spec.match;
	r.created_via = spec.via.empty() ? "console" : spec.via;
	r.created_at = NowUtc();
	r.created_tick = (uint64_t)TimerGameTick::counter;
	r.persisted = spec.persist;
	_rules.push_back(r);
	if (r.persisted && !SaveRules()) {
		_rules.back().persisted = false;
		out.lines.push_back(fmt::format("could not write {}: the rule holds for this process only", RulesFile()));
	}
	out.rule_id = r.id;
	out.lines.insert(out.lines.begin(), fmt::format("added {} (via {}{})", Rules::Describe(r), r.created_via, _rules.back().persisted ? ", persisted in " + RulesFile() : ""));
	Debug(Facility::Net, Severity::Error, "[prismatic-mcp] {}", out.lines.front());
	/* Every approval already waiting, at once. */
	for (auto &a : _approvals) {
		if (a.state != ApprovalState::Pending) continue;
		Rules::Verdict v = RuleVerdict(a, json::parse(a.canonical_args, nullptr, false));
		if (v.decides == nullptr || v.decides->id != r.id) {
			if (v.blocked != nullptr && v.blocked->id == r.id) {
				out.skipped++;
				out.skipped_ids.push_back({a.id, "an owner-only tool: decided only at the game's console or by the owner's click in the game"});
				out.lines.push_back(OutcomeLine(a, "skipped: " + out.skipped_ids.back().error));
			}
			continue;
		}
		const std::string hold = r.effect == Rules::Effect::Allow ? UnattendedHold(a, r.created_via) : std::string{};
		auto ruled = hold.empty() ? ApplyRules(a) : std::nullopt;
		if (!ruled.has_value()) {
			out.skipped++;
			out.skipped_ids.push_back({a.id, hold.empty() ? std::string{"expired"} : hold});
			out.lines.push_back(OutcomeLine(a, "skipped: " + out.skipped_ids.back().error));
			continue;
		}
		const std::string status = (*ruled)["status"].get<std::string>();
		if (status == "rejected") {
			out.refused++;
			out.rejected_ids.push_back({a.id, ruled->value("error", std::string{"the command was refused"})});
		} else {
			out.decided++;
			out.decided_ids.push_back(a.id);
		}
		out.lines.push_back(OutcomeLine(a, status + (ruled->contains("error") ? " -- " + (*ruled)["error"].get<std::string>() : "")));
	}
	return out;
}

bool RemoveApprovalRule(uint64_t id, std::string &message)
{
	auto it = std::ranges::find_if(_rules, [id](const Rules::Rule &r) { return r.id == id; });
	if (it == _rules.end()) { message = fmt::format("no rule #{} ('prismatic_mcp rules' lists them)", id); return false; }
	Rules::Rule removed = *it;
	_rules.erase(it);
	message = fmt::format("removed {} after {} decision(s)", Rules::Describe(removed), removed.hits);
	if (removed.persisted && !SaveRules()) message += fmt::format("; could not rewrite {}, so it loads again at the next start", RulesFile());
	Debug(Facility::Net, Severity::Error, "[prismatic-mcp] {}", message);
	return true;
}

void Tally(BulkOutcome &out, uint64_t id, bool was_pending, bool ok, const std::string &msg)
{
	if (ok) {
		out.decided++;
		out.decided_ids.push_back(id);
		return;
	}
	out.refused++;
	(was_pending && HasRun(id) ? out.rejected_ids : out.refused_ids).push_back({id, msg});
}

std::string UnattendedHoldOf(uint64_t id, std::string_view via)
{
	const PendingApproval *a = FindApproval(id);
	return a == nullptr || a->state != ApprovalState::Pending ? std::string{} : UnattendedHold(*a, via);
}

bool HasRun(uint64_t id)
{
	const PendingApproval *a = FindApproval(id);
	return a != nullptr && (a->state == ApprovalState::Consumed || a->state == ApprovalState::Approved);
}

std::string ResultLine(std::string_view nonce, const BulkOutcome &out, std::string_view error)
{
	auto errors = [](const std::vector<IdError> &list) {
		json arr = json::array();
		for (const IdError &e : list) arr.push_back({{"id", e.id}, {"error", e.error}});
		return arr;
	};
	json j = {{"nonce", nonce}, {"ok", out.ok && error.empty()}, {"decided", out.decided_ids}, {"refused", errors(out.refused_ids)},
		{"rejected", errors(out.rejected_ids)}, {"skipped", errors(out.skipped_ids)}, {"rule_id", out.rule_id}, {"error", error}};
	return "[approval-core] " + j.dump();
}

BulkOutcome DecideAll(bool approve, const Rules::Spec &spec)
{
	BulkOutcome out;
	if (!spec.error.empty()) { out.ok = false; out.lines.push_back(spec.error); return out; }
	const uint64_t now = (uint64_t)TimerGameTick::counter;
	const std::string via = fmt::format("{} ({} all {}{})", spec.via.empty() ? "console" : spec.via, approve ? "approve" : "deny",
		spec.tool, spec.match.is_null() ? "" : " " + spec.match.dump());
	Rules::Rule filter;
	filter.tool = spec.tool;
	filter.match = spec.match;
	std::vector<uint64_t> ids;
	for (const auto &a : _approvals) {
		if (a.state != ApprovalState::Pending) continue;
		if (!Rules::Applies(filter, Rules::Subject{a.mcp_tool, a.tool, json::parse(a.canonical_args, nullptr, false)})) continue;
		ids.push_back(a.id);
	}
	for (uint64_t id : ids) {
		PendingApproval *a = FindApproval(id);
		if (a == nullptr || a->state != ApprovalState::Pending) continue; // decided by an earlier one of these (a superseding plan)
		if (approve && now > a->expiry_tick) { out.skipped++; out.skipped_ids.push_back({id, "expired"}); out.lines.push_back(OutcomeLine(*a, "skipped: expired")); continue; }
		if (approve && !Rules::MayDecide(Rules::Effect::Allow, spec.via, a->tool)) {
			out.skipped++;
			out.skipped_ids.push_back({id, "an owner-only tool: decided only at the game's console or by the owner's click in the game"});
			out.lines.push_back(OutcomeLine(*a, "skipped: " + out.skipped_ids.back().error));
			continue;
		}
		if (approve) {
			if (std::string hold = UnattendedHold(*a, spec.via); !hold.empty()) {
				out.skipped++;
				out.skipped_ids.push_back({id, hold});
				out.lines.push_back(OutcomeLine(*a, "skipped: " + hold));
				continue;
			}
		}
		std::string msg;
		bool ok = approve ? ApproveById(id, msg, via) : DenyById(id, msg, via);
		Tally(out, id, true, ok, msg);
		if (PendingApproval *now_a = FindApproval(id)) out.lines.push_back(OutcomeLine(*now_a, msg));
	}
	return out;
}

bool ApproveById(uint64_t id, std::string &message, std::string_view via)
{
	PendingApproval *a = FindApproval(id);
	if (a == nullptr) { message = fmt::format("no approval #{}", id); return false; }
	uint64_t now = (uint64_t)TimerGameTick::counter;
	if (a->state != ApprovalState::Pending) { message = fmt::format("approval #{} is not pending", id); return false; }
	if (now > a->expiry_tick) { message = fmt::format("approval #{} expired", id); return false; }
	a->state = ApprovalState::Approved;
	Debug(Facility::Net, Severity::Error, "[prismatic-mcp] approved #{} ({}) via {}", id, a->tool, via);
	/* Approving carries it out, here and now: no channel has to resubmit the call (ADR 0038). */
	json out = ExecuteApproval(*a, via);
	bool executed = out["status"] == "executed";
	message = fmt::format("approved #{} ({}): {} {}{}", id, a->tool, out["status"].get<std::string>(),
		out["command_result"].get<std::string>(), out.contains("error") ? " -- " + out["error"].get<std::string>() : "");
	return executed;
}

bool HoldsPending(uint64_t id)
{
	const PendingApproval *a = FindApproval(id);
	return a != nullptr && a->state == ApprovalState::Pending;
}

/* ---- approvers: who may approve from inside the game (ADR 0038) --------------------------- *
 * Declared: the keys the server's operator registered ('prismatic_mcp approvers add <client id>'),
 * kept one per line in the personal directory so they survive a restart. Only these approve: a
 * company's allow list admits whoever plays it (an agent's armed client included), so it is never
 * an approver (the owner's decision, 2026-11-22). */
static std::vector<std::string> _approver_keys;
static std::string _approvers_loaded_from; ///< the file the list was read from; another port's file is read afresh

/** One file per server port: every instance on this machine keeps its own approvers. */
static std::string ApproversFile() { return fmt::format("{}prismatic_mcp_approvers.{}.txt", _personal_dir, _settings_client.network.server_port); }

static void LoadApprovers()
{
	if (_approvers_loaded_from == ApproversFile()) return;
	_approvers_loaded_from = ApproversFile();
	_approver_keys.clear();
	auto f = FileHandle::Open(ApproversFile(), "r");
	if (!f.has_value()) return;
	char buf[512];
	while (fgets(buf, sizeof(buf), *f) != nullptr) {
		std::string line = buf;
		while (!line.empty() && (line.back() == '\n' || line.back() == '\r' || line.back() == ' ')) line.pop_back();
		if (!line.empty() && line[0] != '#') _approver_keys.push_back(line);
	}
}

/** Write the list through a temporary file renamed over the old one, readable by its owner only. */
static bool SaveApprovers()
{
	std::string tmp = ApproversFile() + ".tmp";
	std::error_code ec;
	std::filesystem::remove(tmp, ec);
	{
#if !defined(_WIN32)
		/* Created readable by its owner only, before anything is written into it. */
		int fd = open(tmp.c_str(), O_WRONLY | O_CREAT | O_EXCL, 0600);
		if (fd < 0) return false;
		close(fd);
#endif
		auto f = FileHandle::Open(tmp, "w");
		if (!f.has_value()) return false;
		fmt::print(*f, "# Prismatic MCP approvers (RESEARCH-ONLY, ADR 0038): one client public key per line.\n");
		for (const auto &k : _approver_keys) fmt::print(*f, "{}\n", k);
		if (fflush(*f) != 0) return false;
	}
	std::filesystem::rename(tmp, ApproversFile(), ec);
	return !ec;
}

std::vector<std::string> Approvers() { LoadApprovers(); return _approver_keys; }

bool AddApprover(std::string_view key)
{
	LoadApprovers();
	if (key.empty() || std::find(_approver_keys.begin(), _approver_keys.end(), key) != _approver_keys.end()) return false;
	_approver_keys.emplace_back(key);
	if (SaveApprovers()) return true;
	_approver_keys.pop_back();
	return false;
}

std::string RemoveApprover(std::string_view key_or_prefix)
{
	LoadApprovers();
	if (key_or_prefix.size() < 8) return {};
	auto match = [&](const std::string &k) { return k.starts_with(key_or_prefix); };
	if (std::count_if(_approver_keys.begin(), _approver_keys.end(), match) != 1) return {};
	auto it = std::find_if(_approver_keys.begin(), _approver_keys.end(), match);
	std::string removed = *it;
	size_t at = static_cast<size_t>(it - _approver_keys.begin());
	_approver_keys.erase(it);
	if (SaveApprovers()) return removed;
	_approver_keys.insert(_approver_keys.begin() + at, removed); // not saved: still an approver
	return {};
}

/** Whether a chat line may decide this approval's action at all: one company's own AI or a command
 * that is neither a server nor an offline command; anything wider stays with the server's operator. */
static bool ChatDecidable(const PendingApproval &a)
{
	json d = json::object();
	if (a.tool.starts_with("command.")) {
		if (auto cmd = cmds::Find(std::string_view(a.tool).substr(8)); cmd.has_value()) d = cmds::Describe(*cmd);
	}
	return detail::ChatScoped(a.tool, d.value("flags", json::array()), d.value("category", ""));
}

/** The clients connected to this server as the approver seats see them; none when this process serves no game. */
static std::vector<detail::ClientSeat> ConnectedClientSeats()
{
	std::vector<detail::ClientSeat> clients;
	if (!_network_server) return clients;
	for (const NetworkClientInfo *ci : NetworkClientInfo::Iterate()) {
		int company = Company::IsValidID(ci->client_playas) ? ci->client_playas.base() : -1;
		clients.push_back({static_cast<uint32_t>(to_underlying(ci->client_id)), company, ci->public_key});
	}
	return clients;
}

static std::vector<detail::ApproverSeat> CurrentApproverSeats()
{
	LoadApprovers();
	return detail::ApproverSeats(ConnectedClientSeats(), _approver_keys);
}

/**
 * Record every change of the approver seats since the last poll in openttd://approvals and the log:
 * an approver who joined or left a company, and a registered key a second client connected with
 * (which then approves from neither). The audit trail of who could approve from where, and when.
 */
static void RecordApproverSeatChanges()
{
	static std::vector<detail::ApproverSeat> last;
	static std::vector<std::string> last_shared;
	if (!_network_server) return;
	LoadApprovers();
	std::vector<detail::ClientSeat> clients = ConnectedClientSeats();
	std::vector<detail::ApproverSeat> seats = detail::ApproverSeats(clients, _approver_keys);
	std::vector<std::string> shared;
	for (const auto &k : _approver_keys) {
		if (detail::ClientsWithKey(clients, k) > 1) shared.push_back(detail::KeyFingerprint(k));
	}
	if (seats == last && shared == last_shared) return;
	const uint64_t now = (uint64_t)TimerGameTick::counter;
	auto record = [now](std::string_view change, const detail::ApproverSeat &seat) {
		if (_approver_events.size() >= MAX_APPROVER_EVENTS) _approver_events.erase(_approver_events.begin());
		_approver_events.push_back({now, std::string(change), seat});
		Debug(Facility::Net, Severity::Error, "[prismatic-mcp] approver {}: key {} client {} company {}", change, seat.key_fp, seat.client_id, seat.company);
	};
	for (const auto &s : seats) {
		if (std::find(last.begin(), last.end(), s) == last.end()) record("joined", s);
	}
	for (const auto &s : last) {
		if (std::find(seats.begin(), seats.end(), s) == seats.end()) record("left", s);
	}
	for (const auto &fp : shared) {
		if (std::find(last_shared.begin(), last_shared.end(), fp) == last_shared.end()) record("shared", detail::ApproverSeat{-1, 0, fp});
	}
	last = std::move(seats);
	last_shared = std::move(shared);
}

/** The company an approval acts on, from the arguments it binds to; -1 when it names none. */
static int CompanyOf(const PendingApproval &a)
{
	json args = json::parse(a.canonical_args, nullptr, false);
	if (!args.is_object() || !args.contains("company") || !args["company"].is_number_integer()) return -1;
	int64_t c = args["company"].get<int64_t>();
	return c >= 0 && c < MAX_COMPANIES ? static_cast<int>(c) : -1;
}

bool ChatApproval(uint32_t client_id, int company, std::string_view key, bool injecting, std::string_view line, std::vector<std::string> &replies)
{
	detail::ChatRequest req = detail::ParseChatApproval(line);
	if (req.verb == detail::ChatRequest::Verb::None) return false;
	const size_t max = NETWORK_CHAT_LENGTH - 1;
	if (req.verb == detail::ChatRequest::Verb::Bad) { replies.push_back(detail::CapLine(req.error, max)); return true; }
	bool valid_company = company >= 0 && company < MAX_COMPANIES && Company::IsValidID(CompanyID(company));
	uint64_t now = (uint64_t)TimerGameTick::counter;
	if (req.verb == detail::ChatRequest::Verb::List) {
		/* Machine-readable, one approval per line, so a client can show the server's queue: each line
		 * starts "[approvals] " with a JSON object, the last one is the count and page. */
		constexpr size_t PAGE = 10;
		std::vector<const PendingApproval *> mine;
		for (const auto &a : _approvals) {
			if (a.state == ApprovalState::Pending && now <= a.expiry_tick && valid_company && CompanyOf(a) == company) mine.push_back(&a);
		}
		size_t pages = std::max<size_t>(1, (mine.size() + PAGE - 1) / PAGE);
		size_t from = static_cast<size_t>(req.page - 1) * PAGE;
		for (size_t i = from; i < std::min(mine.size(), from + PAGE); i++) {
			const PendingApproval &a = *mine[i];
			/* A line is whole JSON or it says the arguments did not fit; a cut line would not parse. */
			json row = {{"id", a.id}, {"tool", a.tool}, {"args", a.args_summary}, {"state", "pending"},
				{"expires_tick", a.expiry_tick}, {"chat_decidable", ChatDecidable(a)}};
			std::string line = "[approvals] " + row.dump();
			for (size_t keep = a.args_summary.size(); line.size() > max && keep > 0; ) {
				keep = keep > 64 ? keep / 2 : 0;
				row["args"] = detail::CapLine(a.args_summary, keep);
				row["args_truncated"] = true;
				line = "[approvals] " + row.dump();
			}
			if (line.size() > max) { row.erase("args"); row["args_truncated"] = true; line = "[approvals] " + row.dump(); }
			replies.push_back(std::move(line));
		}
		replies.push_back(json({{"count", mine.size()}, {"page", req.page}, {"pages", pages}}).dump().insert(0, "[approvals] "));
		return true;
	}
	PendingApproval *a = FindApproval(req.id);
	LoadApprovers();
	detail::ChatFacts f;
	f.exists = a != nullptr;
	f.pending = f.exists && a->state == ApprovalState::Pending;
	f.expired = f.exists && now > a->expiry_tick;
	f.chat_decidable = f.exists && ChatDecidable(*a);
	f.valid_company = valid_company;
	f.same_company = f.exists && valid_company && CompanyOf(*a) == company;
	f.key_registered = !key.empty() && std::find(_approver_keys.begin(), _approver_keys.end(), key) != _approver_keys.end();
	/* Whether the key is in a human company's allow list: said in the audit line, never enough to
	 * approve (DecideChat counts registered keys only). */
	if (!key.empty() && valid_company && Company::IsValidHumanID(CompanyID(company))) {
		const Company *c = Company::Get(CompanyID(company));
		f.key_in_allow_list = !c->allow_any && c->allow_list.Contains(key);
	}
	f.injecting = injecting;
	f.key_shared = f.key_registered && detail::ClientsWithKey(ConnectedClientSeats(), key) > 1;
	bool approving = req.verb == detail::ChatRequest::Verb::Approve;
	detail::ChatVerdict v = detail::DecideChat(f, approving);
	if (v != detail::ChatVerdict::Allow) {
		Debug(Facility::Net, Severity::Error, "[prismatic-mcp] chat {} #{} from client {} refused: {}", approving ? "approve" : "deny", req.id, client_id, detail::ChatVerdictText(v));
		replies.push_back(detail::CapLine(fmt::format("#{}: {}", req.id, detail::ChatVerdictText(v)), max));
		return true;
	}
	std::string m;
	std::string via = fmt::format("chat client {} company {} key {} ({})", client_id, company, detail::KeyFingerprint(key),
		f.key_registered ? "registered approver" : f.key_in_allow_list ? "company allow list, deny only" : "company member, deny only");
	if (approving) {
		ApproveById(req.id, m, via);
	} else if (DenyById(req.id, m)) {
		Debug(Facility::Net, Severity::Error, "[prismatic-mcp] denied #{} via {}", req.id, via);
	}
	replies.push_back(detail::CapLine(m, max));
	return true;
}

/**
 * The approval deadline (detail::DeadlineDue, research/mechanisms/approval-gate.md "The deadline"):
 * every pending approval past it is settled with the operator's standing verdict, on the main thread
 * once a poll. Keep retires it unanswered, enacting nothing; approve executes it through the one
 * execution path (ApproveById), and only when a chat approver could have decided it. Each settled
 * approval says so in the log and in openttd://approvals (settled_by).
 */
static void SettleDueApprovals()
{
	if (_deadline_policy.ticks == 0) return;
	const uint64_t now = (uint64_t)TimerGameTick::counter;
	const uint64_t days = _deadline_policy.ticks / detail::DEADLINE_DAY_TICKS;
	std::optional<std::vector<detail::ApproverSeat>> seats; // read once, and only when one is due
	for (auto &a : _approvals) {
		/* Due at all? ChatDecidable reads the command catalogue, so it is asked only of a due one. */
		if (detail::DeadlineDue(a.state == ApprovalState::Pending, a.created_tick, now, _deadline_policy, false, false) == detail::DeadlineAction::None) continue;
		if (!seats.has_value()) seats = CurrentApproverSeats();
		const int company = CompanyOf(a);
		const bool chat_scoped = ChatDecidable(a);
		const bool approver_company = a.approver_at_creation && detail::ApproverPlays(*seats, company);
		if (detail::DeadlineDue(true, a.created_tick, now, _deadline_policy, chat_scoped, approver_company) == detail::DeadlineAction::Execute) {
			a.settled_by = fmt::format("deadline approve (approver in company {})", company);
			std::string message;
			bool executed = ApproveById(a.id, message, fmt::format("deadline (the operator's standing verdict: approve; a registered approver plays company {})", company));
			Debug(Facility::Net, Severity::Error, "[prismatic-mcp] deadline: approval #{} unanswered after {} game day(s): {}", a.id, days, message);
			if (executed || a.state != ApprovalState::Pending) continue;
			/* Refused (expired, say): retired, never tried again at every poll. */
			a.settled_by = "deadline keep (approve refused: " + message + ")";
		} else if (_deadline_policy.verdict == detail::DeadlineVerdict::Approve && chat_scoped) {
			/* approve stands in only for an approver of the company it names, when asked and now. */
			a.settled_by = company < 0 ? "deadline keep (it names no company)"
				: fmt::format("deadline keep (no registered approver played company {} {})", company, a.approver_at_creation ? "at the deadline" : "when it was asked");
		} else {
			a.settled_by = "deadline keep";
		}
		a.state = ApprovalState::Denied;
		Debug(Facility::Net, Severity::Error, "[prismatic-mcp] deadline: approval #{} ({}) retired unanswered after {} game day(s); nothing enacted",
			a.id, a.tool, days);
	}
}

bool SetApprovalDeadline(std::string_view days, std::string_view verdict, std::string &message)
{
	auto policy = detail::ParseDeadlinePolicy(days, verdict);
	if (!policy.has_value()) {
		message = "Usage: prismatic_mcp approval_deadline <game days 0-3650, 0 = off> [keep|approve]";
		return false;
	}
	_deadline_policy = *policy;
	message = DescribeApprovalDeadline();
	return true;
}

std::string DescribeApprovalDeadline()
{
	if (_deadline_policy.ticks == 0) return "approval deadline: off (a pending approval waits until it expires)";
	return fmt::format("approval deadline: {} game day(s), then {} ({})", _deadline_policy.ticks / detail::DEADLINE_DAY_TICKS,
		detail::DeadlineVerdictName(_deadline_policy.verdict),
		_deadline_policy.verdict == detail::DeadlineVerdict::Approve
			? "executes what a chat approver may decide for a company a registered approver plays, retires the rest"
			: "retires it unanswered, enacting nothing");
}

bool DenyById(uint64_t id, std::string &message, std::string_view via)
{
	PendingApproval *a = FindApproval(id);
	if (a == nullptr) { message = fmt::format("no approval #{}", id); return false; }
	/* A decided approval stays as it was decided: denying a consumed one would relabel an action
	 * that ran. */
	if (a->state != ApprovalState::Pending) { message = fmt::format("approval #{} is not pending", id); return false; }
	a->state = ApprovalState::Denied;
	if (!via.empty()) a->settled_by = fmt::format("denied via {}", via);
	message = fmt::format("denied #{}", id);
	return true;
}

std::string DecisionTrace(int max_rows)
{
	if (_provenance.empty()) return "no decisions executed";
	std::string s;
	int start = (int)_provenance.size() - max_rows;
	if (start < 0) start = 0;
	for (size_t i = (size_t)start; i < _provenance.size(); i++) {
		const auto &p = _provenance[i];
		s += fmt::format("decision #{} approval #{} {} result={} verdict={} req_tick={} obs_tick={}{}\n",
			p.decision_id, p.approval_id, p.action_type, p.command_result, p.verdict,
			p.requested_tick, p.observed_tick, p.approved_via.empty() ? "" : " via=" + p.approved_via);
	}
	return s;
}

std::string GetStatusLine()
{
	std::string s = "prismatic_mcp";
	s += _mcp.running ? " running" : " stopped";
	if (_mcp.running) {
		s += " endpoint=" + GetEndpoint();
		s += " requests=" + fmt::format("{}", _mcp.requests_total);
		s += " denied=" + fmt::format("{}", _mcp.requests_denied);
		s += " conns=" + fmt::format("{}", _mcp.conns.size());
	}
	/* The same derived values as openttd://mcp/status: what this build and process can do, running or not. */
	s += " " + detail::CapabilitiesLine(ServerCapabilities());
	return s;
}

json Dispatch(const json &request)
{
	auto reply = DispatchOne(request);
	return reply.has_value() ? *reply : json(nullptr);
}

} // namespace PrismaticMCP

#endif /* OTTD_PRISMATIC_MCP_SERVER */
