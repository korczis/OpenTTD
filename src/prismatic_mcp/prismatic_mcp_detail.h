/*
 * Part of korczis's PRIVATE RESEARCH FORK of OpenTTD. RESEARCH-ONLY.
 * Compiled only when OTTD_PRISMATIC_MCP_SERVER is defined.
 */

/** @file prismatic_mcp_detail.h Pure, side-effect-free MCP helpers (unit-testable, no game state). */

#ifndef PRISMATIC_MCP_DETAIL_H
#define PRISMATIC_MCP_DETAIL_H

#ifdef OTTD_PRISMATIC_MCP_SERVER

#include <cstdint>
#include <algorithm>
#include <array>
#include <optional>
#include <string>
#include <string_view>
#include <vector>

#include "../3rdparty/monocypher/monocypher.h"
#include "../3rdparty/nlohmann/json.hpp"
#include "../core/format.hpp"

/**
 * Pure helpers with no OpenTTD or game-state dependency, factored out of the
 * server so the security-critical logic (digest binding, header parsing, bearer
 * extraction, action-type gating) can be unit-tested in isolation. Nothing here
 * touches a pool, a socket, or a global.
 */
namespace PrismaticMCP::detail {

/** FNV-1a 64-bit over `tool + "|" + canonical_args`, rendered as 16 hex chars.
 * Deterministic; identical input -> identical digest; any difference -> different digest. */
inline std::string ArgDigestHex(std::string_view tool, std::string_view canonical_args)
{
	uint64_t h = 1469598103934665603ull;
	auto mix = [&h](std::string_view s) { for (unsigned char c : s) { h ^= c; h *= 1099511628211ull; } };
	mix(tool);
	mix("|");
	mix(canonical_args);
	static const char *d = "0123456789abcdef";
	std::string out(16, '0');
	for (int i = 15; i >= 0; i--) { out[i] = d[h & 0xF]; h >>= 4; }
	return out;
}

/** Why session- and world-bound decision state is being dropped. */
enum class ResetReason : uint8_t {
	ServerStopped,   ///< the MCP server stopped; the world goes on
	TokenRotated,    ///< the bearer token changed; the world goes on
	GameModeChanged, ///< the game mode changed (menu, editor, a game): the world is gone
	WorldReplaced,   ///< a new or loaded world (the tick counter went back): the world is gone
};

/** What a reset drops. */
struct ResetEffect {
	bool clear_approvals;     ///< every approval, granted or pending
	bool invalidate_awaiting; ///< decisions still waiting for their commands' results
};

/**
 * The rule: an approval binds consent to one session of one world, so every reset drops it.
 * A posted command still runs in the same world after a stop or a token rotation and its result
 * still arrives; only when the world is gone will it never come, so only then are waiting
 * decisions closed.
 */
constexpr ResetEffect EffectOf(ResetReason r)
{
	bool world_gone = r == ResetReason::GameModeChanged || r == ResetReason::WorldReplaced;
	return ResetEffect{true, world_gone};
}

/** @return The words a reset is logged and recorded with. */
constexpr std::string_view ReasonText(ResetReason r)
{
	switch (r) {
		case ResetReason::ServerStopped: return "server stopped";
		case ResetReason::TokenRotated: return "token rotated";
		case ResetReason::GameModeChanged: return "game mode changed";
		case ResetReason::WorldReplaced: return "a new or loaded world";
	}
	return "reset";
}

/** @return Whether a reset that invalidates waiting decisions closes a decision with this verdict. */
inline bool ClosedByReset(std::string_view verdict)
{
	return verdict == "PENDING_OBSERVATION";
}

/** The fixed typed action types this build can execute, the one place they are named; the game
 * commands add one "command.<Name>" type each, derived from the command table (prismatic_mcp_commands).
 * The validator, openttd.decision.list_action_types and the decision tools' input schemas read both. */
inline constexpr std::array<std::string_view, 6> ACTION_TYPES = {"game.set_pause", "ai.set_setting", "ai.set_settings", "ai.start", "ai.stop", "ai.reload"};

/** Whether this build can execute a typed action of this type. Anything else is rejected. */
inline bool IsKnownActionType(std::string_view type)
{
	if (std::find(ACTION_TYPES.begin(), ACTION_TYPES.end(), type) != ACTION_TYPES.end()) return true;
	/* "command.<Name>": a game command by its Cmd* name; whether it exists is the catalogue's call. */
	if (!type.starts_with("command.") || type.size() == 8) return false;
	for (char c : type.substr(8)) {
		if (!((c >= 'A' && c <= 'Z') || (c >= 'a' && c <= 'z') || (c >= '0' && c <= '9'))) return false;
	}
	return true;
}

/** Case-insensitive Content-Length parse from a raw header block. nullopt if absent/invalid. */
inline std::optional<size_t> ParseContentLength(std::string_view header_block)
{
	std::string lower(header_block);
	for (char &c : lower) c = (char)((c >= 'A' && c <= 'Z') ? c - 'A' + 'a' : c);
	size_t p = lower.find("content-length:");
	if (p == std::string::npos) return std::nullopt;
	p += 15; // strlen("content-length:")
	while (p < lower.size() && (lower[p] == ' ' || lower[p] == '\t')) p++;
	size_t start = p;
	while (p < lower.size() && lower[p] >= '0' && lower[p] <= '9') p++;
	if (p == start) return std::nullopt;
	size_t value = 0;
	for (size_t i = start; i < p; i++) value = value * 10 + (size_t)(lower[i] - '0');
	return value;
}

/** Extract the token from an `Authorization: Bearer <tok>` value (case-insensitive scheme).
 * Returns empty view if the scheme is missing. */
inline std::string_view ExtractBearer(std::string_view authorization)
{
	if (authorization.size() <= 7) return {};
	std::string_view scheme = authorization.substr(0, 7);
	if (scheme != "Bearer " && scheme != "bearer ") return {};
	std::string_view tok = authorization.substr(7);
	while (!tok.empty() && (tok.front() == ' ' || tok.front() == '\t')) tok.remove_prefix(1);
	while (!tok.empty() && (tok.back() == ' ' || tok.back() == '\t' || tok.back() == '\r')) tok.remove_suffix(1);
	return tok;
}

/** Constant-time equality of two equal-length-or-not hex strings. */
inline bool ConstantTimeEquals(std::string_view a, std::string_view b)
{
	if (a.size() != b.size()) return false;
	unsigned diff = 0;
	for (size_t i = 0; i < a.size(); i++) diff |= (unsigned)(a[i] ^ b[i]);
	return diff == 0;
}

/**
 * A short, one-way name for a bearer token that the console may print in its place:
 * "blake2b:" and the first 8 bytes of BLAKE2b-256 over the token's text, in hex. It lets the
 * operator tell which token a file holds without the secret reaching the console or its log.
 * @param token The token in hex.
 * @return The fingerprint, or empty for an empty token.
 */
inline std::string TokenFingerprint(std::string_view token)
{
	if (token.empty()) return {};
	std::array<uint8_t, 32> hash{};
	crypto_blake2b(hash.data(), hash.size(), reinterpret_cast<const uint8_t *>(token.data()), token.size());
	std::string out = "blake2b:";
	for (size_t i = 0; i < 8; i++) out += fmt::format("{:02x}", hash[i]);
	return out;
}

/**
 * The name of the token file `prismatic_mcp start` writes into the personal directory when it is
 * given no file of its own: one per port, so two servers sharing a personal directory never
 * overwrite each other's token.
 * @param port The port the server listens on.
 * @return The file name, without a directory.
 */
inline std::string DefaultTokenFileName(uint16_t port)
{
	return fmt::format("prismatic_mcp_token_{}.json", port);
}

/**
 * Has the world changed under the server since the last poll? The tick counter only moves forward
 * within one game; a new game or a load restarts it, so a smaller value means another world, whose
 * approvals must not carry over (their expiry is counted in those ticks).
 */
inline bool WorldChanged(uint64_t last_tick, uint64_t now_tick)
{
	return now_tick < last_tick;
}

/** The approval state machine's decision, factored out of the server so it is unit-testable
 * without a game. An approval binds to the submission's CANONICAL ARGUMENTS -- the exact
 * serialized arguments -- not to a 64-bit digest that could collide: a changed argument never
 * matches an approval granted for the original, and no two different argument sets can ever share
 * one approval. */
enum class SubmitDecision : uint8_t {
	CreatePending, ///< no live approval for these exact args: ask the operator
	ReusePending,  ///< a pending approval for these exact args already exists: wait on it
	Execute,       ///< an approved, unexpired, unconsumed approval for these exact args: run once
};

/** One approval as the pure classifier sees it: its state, the action (tool) and the canonical args
 * it was granted for, and when it expires. `state` mirrors ApprovalState (0 Pending, 1 Approved,
 * 2 Denied, 3 Consumed). */
struct ApprovalView {
	uint8_t state = 0;
	std::string tool;
	std::string canonical_args;
	uint64_t expiry_tick = 0;
};

/** Classify a submission of action `tool` with canonical arguments `canonical_args`, at tick `now`,
 * against the live `approvals` (scanned in order). An approval binds to the action AND its exact
 * arguments: an approval granted for one tool is never spent by another tool with the same
 * arguments (e.g. ai.reload {company:N} never runs ai.stop {company:N}). Returns the decision; for
 * ReusePending/Execute it sets `matched_index` to the matched approval's index. Pure: no globals, no
 * game state, no side effects -- the caller consumes the approval and runs the command. */
inline SubmitDecision ClassifySubmit(const std::vector<ApprovalView> &approvals, std::string_view tool,
	std::string_view canonical_args, uint64_t now, size_t &matched_index)
{
	/* An approved, unexpired, unconsumed approval bound to this action and these exact args executes once. */
	for (size_t i = 0; i < approvals.size(); i++) {
		const ApprovalView &a = approvals[i];
		if (a.state == 1 && a.tool == tool && a.canonical_args == canonical_args && now <= a.expiry_tick) {
			matched_index = i;
			return SubmitDecision::Execute;
		}
	}
	/* Otherwise reuse a still-pending approval for this action and these exact args. */
	for (size_t i = 0; i < approvals.size(); i++) {
		const ApprovalView &a = approvals[i];
		if (a.state == 0 && a.tool == tool && a.canonical_args == canonical_args && now <= a.expiry_tick) {
			matched_index = i;
			return SubmitDecision::ReusePending;
		}
	}
	return SubmitDecision::CreatePending;
}

/** At most this many settings one openttd.ai.set_settings call changes: a bound on one approval. */
inline constexpr size_t SET_SETTINGS_MAX = 32;

/**
 * What openttd.ai.set_settings `{company, settings: {name: int, ...}, expires_tick?}` asks, checked for
 * shape before any AI is looked at. Every name and value is then checked against the company's AI
 * (prismatic_mcp_ai.cpp); this is the part that needs no game.
 */
struct SetSettingsPlan {
	std::vector<std::string> errors;                        ///< every reason the shape is wrong; empty: well formed
	std::vector<std::pair<std::string, int64_t>> settings;  ///< the changes, in canonical (name) order
	std::optional<uint64_t> expires_tick;                   ///< the last game tick the decision may run at
};

/**
 * Check the shape of openttd.ai.set_settings arguments: `settings` an object of 1..SET_SETTINGS_MAX
 * integer values, `expires_tick` (when given) a non-negative integer. Every wrong value is named.
 * Pure: the company is the caller's to check.
 */
inline SetSettingsPlan PlanSetSettings(const nlohmann::json &args)
{
	SetSettingsPlan plan;
	const nlohmann::json *settings = args.is_object() && args.contains("settings") ? &args["settings"] : nullptr;
	if (settings == nullptr || !settings->is_object()) {
		plan.errors.push_back("settings (an object {name: integer}) required");
	} else if (settings->empty()) {
		plan.errors.push_back("settings is empty: name at least one setting");
	} else if (settings->size() > SET_SETTINGS_MAX) {
		plan.errors.push_back(fmt::format("at most {} settings per call, got {}", SET_SETTINGS_MAX, settings->size()));
	} else {
		for (const auto &[name, value] : settings->items()) {
			if (!value.is_number_integer()) {
				plan.errors.push_back(fmt::format("{}: value must be an integer", name));
				continue;
			}
			plan.settings.emplace_back(name, value.get<int64_t>());
		}
	}
	if (args.is_object() && args.contains("expires_tick")) {
		const nlohmann::json &e = args["expires_tick"];
		if (!e.is_number_integer() || (!e.is_number_unsigned() && e.get<int64_t>() < 0)) {
			plan.errors.push_back("expires_tick must be a non-negative integer (a game tick)");
		} else {
			plan.expires_tick = e.get<uint64_t>();
		}
	}
	return plan;
}

/** The expiry tick an action binds through its arguments: only ai.set_settings has one (`expires_tick`). */
inline std::optional<uint64_t> BoundExpiresTick(std::string_view type, const nlohmann::json &args)
{
	if (type != "ai.set_settings") return std::nullopt;
	return PlanSetSettings(args).expires_tick;
}

/** When an approval created at `now` expires: after `ttl` ticks, or at the action's own expires_tick if that is sooner. */
inline uint64_t ApprovalExpiry(uint64_t now, uint64_t ttl, std::optional<uint64_t> expires_tick)
{
	uint64_t by_ttl = now + ttl;
	return expires_tick.has_value() ? std::min(by_ttl, *expires_tick) : by_ttl;
}

/** Whether an action bound to `expires_tick` may still run at tick `now` (the last tick it names included). */
inline bool StillFresh(uint64_t now, std::optional<uint64_t> expires_tick)
{
	return !expires_tick.has_value() || now <= *expires_tick;
}

/** At most this many resources one openttd.resources.read_many call reads: a bound on one main-thread poll. */
inline constexpr size_t READ_MANY_MAX_URIS = 32;
/** The only scheme openttd.resources.read_many reads; anything else is refused per entry. */
inline constexpr std::string_view READ_MANY_SCHEME = "openttd://";

/** What openttd.resources.read_many will do with its arguments, decided before any resource is built. */
struct ReadManyPlan {
	/** One requested uri, in the order asked; `error` empty means it is read. */
	struct Entry {
		std::string uri;   ///< as given (a non-string element as its JSON text)
		std::string error; ///< why it is not read, or empty
	};
	std::string error;          ///< non-empty: the whole call is refused (malformed or over the cap)
	std::vector<Entry> entries; ///< one per requested uri, when `error` is empty
};

/**
 * Plan a read of several resources at one tick from the tool's arguments `{uris: [...]}`. The call
 * is refused as a whole only when `uris` is missing, not an array, empty or longer than
 * READ_MANY_MAX_URIS; otherwise each entry stands alone: not a string, another scheme than
 * openttd://, a uri `is_known` does not know, or a uri already asked for earlier in the call is an
 * error for that entry only. Pure: `is_known` is the caller's view of the resource table.
 */
template <class IsKnown>
ReadManyPlan PlanReadMany(const nlohmann::json &args, IsKnown &&is_known)
{
	ReadManyPlan plan;
	const nlohmann::json *uris = args.is_object() && args.contains("uris") ? &args["uris"] : nullptr;
	if (uris == nullptr || !uris->is_array()) {
		plan.error = "uris (an array of openttd:// resource uris) required";
		return plan;
	}
	if (uris->empty()) {
		plan.error = "uris is empty";
		return plan;
	}
	if (uris->size() > READ_MANY_MAX_URIS) {
		plan.error = "at most " + std::to_string(READ_MANY_MAX_URIS) + " uris per call";
		return plan;
	}
	for (const auto &u : *uris) {
		ReadManyPlan::Entry &e = plan.entries.emplace_back();
		if (!u.is_string()) {
			e.uri = u.dump();
			e.error = "not a string";
			continue;
		}
		e.uri = u.get<std::string>();
		if (!std::string_view(e.uri).starts_with(READ_MANY_SCHEME)) {
			e.error = "only openttd:// resources";
		} else if (!is_known(e.uri)) {
			e.error = "unknown resource uri";
		} else if (std::any_of(plan.entries.begin(), plan.entries.end() - 1, [&e](const auto &o) { return o.error.empty() && o.uri == e.uri; })) {
			e.error = "duplicate: read once, earlier in this call";
		}
	}
	return plan;
}

/** The tools/list entry of openttd.resources.read_many: read-only, its cap and scheme from the constants above. */
inline nlohmann::json ReadManyTool()
{
	nlohmann::json uri = {{"type", "string"}, {"pattern", "^" + std::string(READ_MANY_SCHEME)}};
	nlohmann::json uris = {{"type", "array"}, {"minItems", 1}, {"maxItems", READ_MANY_MAX_URIS}, {"items", uri},
		{"description", "openttd:// resource uris as resources/list names them; an unknown one is an error for its entry only"}};
	return {{"name", "openttd.resources.read_many"},
		{"description", "Read several resources at one game tick, without pausing: every one is built in the same "
			"main-thread call, so their game_tick agree (a coherent snapshot). Answers {ok, game_tick, game_date, "
			"resources: [{uri, ok, data | error}]}, data being what resources/read answers for that uri"},
		{"inputSchema", {{"type", "object"}, {"required", nlohmann::json::array({"uris"})}, {"properties", {{"uris", uris}}}}},
		{"annotations", {{"readOnlyHint", true}, {"idempotentHint", true}, {"openWorldHint", false}}}};
}

/**
 * One way the server can change something, as openttd://mcp/status and `prismatic_mcp status` name
 * it: the tools/list names that carry it (name prefixes) and whether it also needs this process to
 * be armed (PRISMATIC_MCP_INPUT=1). A tool counts only when its readOnlyHint is not true, so a
 * reader under the same prefix (openttd.ai.list, openttd.ui.windows) never makes a capability.
 */
struct CapabilityRule {
	std::string_view field;                  ///< The status field.
	std::array<std::string_view, 2> prefixes; ///< Tool-name prefixes; an empty one matches nothing.
	bool needs_arming;                       ///< Present tools count only in an armed process.
};

/** Every mutating capability, in the order status reports them; a tool takes the first rule it
 * matches, so the toolbar menu's openttd.ui.menu.* and openttd.ui.requests.* are not interface input.
 * openttd.view.* moves only the local viewport and openttd.decision.submit runs only the action
 * types the tools below carry, so neither is a capability of its own. */
inline constexpr std::array<CapabilityRule, 6> MUTATING_CAPABILITIES = {{
	{"approval_gated_commands", {"openttd.cmd.", ""}, false},
	{"script_api", {"openttd.api.", ""}, false},
	{"ai_tools", {"openttd.ai.", ""}, false},
	{"pause", {"openttd.game.set_pause", ""}, false},
	{"toolbar_menu", {"openttd.ui.menu.", "openttd.ui.requests."}, false},
	{"interface_input", {"openttd.input.", "openttd.ui."}, true},
}};

/** What a server can do, derived from its tool registry and its process (CapabilitiesOf). */
struct Capabilities {
	bool reads = false;       ///< A resource or a read-only tool is served.
	std::array<bool, MUTATING_CAPABILITIES.size()> mutating{}; ///< One per MUTATING_CAPABILITIES rule.
	bool input_armed = false; ///< The process was started with PRISMATIC_MCP_INPUT=1.

	/** @return Whether no mutating capability is present: the negation of every one of them. */
	bool ReadOnly() const
	{
		return std::none_of(this->mutating.begin(), this->mutating.end(), [](bool b) { return b; });
	}
};

/**
 * Derive the capabilities from what the server registers.
 * @param tools The tools/list "tools" array.
 * @param resources How many resources resources/list serves.
 * @param input_armed Whether this process is armed for interface input.
 * @return Every capability, each true only when a tool that carries it is registered (and armed).
 */
inline Capabilities CapabilitiesOf(const nlohmann::json &tools, size_t resources, bool input_armed)
{
	Capabilities caps;
	caps.input_armed = input_armed;
	caps.reads = resources > 0;
	for (const auto &t : tools) {
		const std::string name = t.value("name", "");
		bool read_only = t.contains("annotations") && t["annotations"].value("readOnlyHint", false);
		if (read_only) {
			caps.reads = true;
			continue;
		}
		for (size_t i = 0; i < MUTATING_CAPABILITIES.size(); i++) {
			const CapabilityRule &r = MUTATING_CAPABILITIES[i];
			bool match = std::any_of(r.prefixes.begin(), r.prefixes.end(), [&name](std::string_view p) { return !p.empty() && name.starts_with(p); });
			if (!match) continue;
			if (!r.needs_arming || input_armed) caps.mutating[i] = true;
			break;
		}
	}
	return caps;
}

/** @return The status resource's `capabilities` object: reads, then every MUTATING_CAPABILITIES field. */
inline nlohmann::json CapabilitiesJson(const Capabilities &caps)
{
	nlohmann::json j = {{"reads", caps.reads}};
	for (size_t i = 0; i < MUTATING_CAPABILITIES.size(); i++) j[std::string(MUTATING_CAPABILITIES[i].field)] = caps.mutating[i];
	return j;
}

/** @return The console's words for the same values: "read_only=<b> capabilities=<present, comma-separated, or none> input_armed=<b>". */
inline std::string CapabilitiesLine(const Capabilities &caps)
{
	std::string present;
	auto add = [&present](std::string_view f) { if (!present.empty()) present += ','; present += f; };
	if (caps.reads) add("reads");
	for (size_t i = 0; i < MUTATING_CAPABILITIES.size(); i++) if (caps.mutating[i]) add(MUTATING_CAPABILITIES[i].field);
	return fmt::format("read_only={} capabilities={} input_armed={}", caps.ReadOnly(), present.empty() ? "none" : present, caps.input_armed);
}

} // namespace PrismaticMCP::detail

#endif /* OTTD_PRISMATIC_MCP_SERVER */
#endif /* PRISMATIC_MCP_DETAIL_H */
