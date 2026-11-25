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

/* ---- the approval deadline (research/mechanisms/approval-gate.md "The deadline") ----------- *
 * A pending approval nobody answers used to wait until its expiry (APPROVAL_TTL_TICKS, about 3.7
 * game years), and every requester that holds while it waits (replan.rhai) stopped planning: the
 * owner's game of 2026-11-22 stopped building. The deadline settles it after a number of game
 * ticks with the operator's standing verdict. `Keep` (the default) retires it: nothing is enacted,
 * the game keeps what it has, and the requester may ask again. `Approve` executes it, and only
 * when the action is one a chat approver may decide (ChatScoped: one company's own AI or a
 * company-level command); anything wider is retired as under `Keep`. The verdict is given in
 * advance by the operator, so consent stays a decision someone made, never a default of the code. */

/** The operator's standing verdict for an approval that reaches its deadline unanswered. */
enum class DeadlineVerdict : uint8_t {
	Keep,    ///< retire it: deny, enact nothing (the default)
	Approve, ///< execute it, when ChatScoped; else Keep
};

/** What the deadline does to one approval now. */
enum class DeadlineAction : uint8_t {
	None,    ///< not due, not pending, or no deadline
	Retire,  ///< deny it, with the deadline as the reason
	Execute, ///< approve and execute it, with the deadline as the channel
};

/** Ticks per game day (TimerGameTick::DAY_TICKS, src/timer/timer_game_tick.h), kept here so the pure helpers need no game header. */
inline constexpr uint64_t DEADLINE_DAY_TICKS = 74;
/** The deadline a server starts with: one game month (30 days), the cadence replan.rhai plans at,
 * so a plan nobody answered is settled before the next one is asked for. 0 turns the deadline off. */
inline constexpr uint64_t DEFAULT_DEADLINE_TICKS = 30 * DEADLINE_DAY_TICKS;

/** The operator's deadline policy: after `ticks` game ticks pending, the `verdict`; `ticks` 0 is off. */
struct DeadlinePolicy {
	uint64_t ticks = DEFAULT_DEADLINE_TICKS;
	DeadlineVerdict verdict = DeadlineVerdict::Keep;
};

/** The tick an approval created at `created` reaches its deadline under `ticks`, or nullopt when the deadline is off. */
inline std::optional<uint64_t> DeadlineTick(uint64_t created, uint64_t ticks)
{
	if (ticks == 0) return std::nullopt;
	return created + ticks;
}

/**
 * What the deadline does at tick `now` to an approval created at `created`, pending or not, whose
 * action a chat approver may (`chat_scoped`) or may not decide. `approver_company`: a registered
 * approver played the company the approval names both when it was asked and now (ApproverSeats).
 * `approve` stands in only for that approver, so it executes nothing for a company no approver
 * plays, nor for one that names no company (the owner's decision, 2026-11-23): retired as under keep.
 */
inline DeadlineAction DeadlineDue(bool pending, uint64_t created, uint64_t now, const DeadlinePolicy &policy, bool chat_scoped, bool approver_company)
{
	auto due = DeadlineTick(created, policy.ticks);
	if (!pending || !due.has_value() || now < *due) return DeadlineAction::None;
	return policy.verdict == DeadlineVerdict::Approve && chat_scoped && approver_company ? DeadlineAction::Execute : DeadlineAction::Retire;
}

/** The verdict's name, as the console, the resource and the log spell it. */
inline std::string_view DeadlineVerdictName(DeadlineVerdict v)
{
	return v == DeadlineVerdict::Approve ? "approve" : "keep";
}

/** The policy `prismatic_mcp approval_deadline <days> [keep|approve]` asks for, or nullopt for a malformed line. Days are whole game days, 0 (off) to 3650; the verdict defaults to keep. */
inline std::optional<DeadlinePolicy> ParseDeadlinePolicy(std::string_view days, std::string_view verdict)
{
	if (days.empty() || days.size() > 4 || !std::all_of(days.begin(), days.end(), [](char c) { return c >= '0' && c <= '9'; })) return std::nullopt;
	uint64_t d = 0;
	for (char c : days) d = d * 10 + (uint64_t)(c - '0');
	if (d > 3650) return std::nullopt;
	DeadlinePolicy p;
	p.ticks = d * DEADLINE_DAY_TICKS;
	if (verdict.empty() || verdict == "keep") {
		p.verdict = DeadlineVerdict::Keep;
	} else if (verdict == "approve") {
		p.verdict = DeadlineVerdict::Approve;
	} else {
		return std::nullopt;
	}
	return p;
}

/** The policy the environment names at start-up, PRISMATIC_MCP_APPROVAL_DEADLINE="<days>[:keep|approve]" (the launcher's --approval-deadline), or nullopt for a malformed value. The operator chooses it when starting the server, so it is consent given in advance as on the console. */
inline std::optional<DeadlinePolicy> ParseDeadlinePolicyText(std::string_view text)
{
	size_t colon = text.find(':');
	if (colon == std::string_view::npos) return ParseDeadlinePolicy(text, "");
	return ParseDeadlinePolicy(text.substr(0, colon), text.substr(colon + 1));
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

/* ---- the in-game approval channel (ADR 0038): pure, so every branch is unit-tested ---------- */

/** One chat line of the in-game approval channel, parsed: '!approvals [page]', '!approve <id>',
 * '!deny <id>'. One id per line, never a list: a list approved decisions nobody read one by one,
 * and a long line is cut by the network, which could leave an id nobody typed. */
struct ChatRequest {
	enum class Verb : uint8_t { None, List, Approve, Deny, Bad };
	Verb verb = Verb::None;
	uint64_t id = 0;     ///< the approval, for Approve/Deny
	uint32_t page = 1;   ///< the page, for List (1-based)
	std::string error;   ///< why the line is Bad
};

inline ChatRequest ParseChatApproval(std::string_view line)
{
	std::vector<std::string_view> w;
	for (size_t i = 0; i < line.size();) {
		size_t j = line.find(' ', i);
		if (j == std::string_view::npos) j = line.size();
		if (j > i) w.push_back(line.substr(i, j - i));
		i = j + 1;
	}
	ChatRequest r;
	if (w.empty() || (w[0] != "!approvals" && w[0] != "!approve" && w[0] != "!deny")) return r;
	auto number = [](std::string_view t) -> std::optional<uint64_t> {
		if (t.empty() || t.size() > 18) return std::nullopt;
		uint64_t v = 0;
		for (char c : t) { if (c < '0' || c > '9') return std::nullopt; v = v * 10 + static_cast<uint64_t>(c - '0'); }
		return v;
	};
	if (w[0] == "!approvals") {
		r.verb = ChatRequest::Verb::List;
		if (w.size() > 2) { r.verb = ChatRequest::Verb::Bad; r.error = "usage: !approvals [page]"; return r; }
		if (w.size() == 2) {
			auto n = number(w[1]);
			if (!n || *n == 0 || *n > 100000) { r.verb = ChatRequest::Verb::Bad; r.error = "usage: !approvals [page]"; return r; }
			r.page = static_cast<uint32_t>(*n);
		}
		return r;
	}
	r.verb = w[0] == "!approve" ? ChatRequest::Verb::Approve : ChatRequest::Verb::Deny;
	if (w.size() != 2) {
		r.verb = ChatRequest::Verb::Bad;
		r.error = fmt::format("usage: {} <id> -- one id per line (!approvals lists them)", w[0]);
		return r;
	}
	auto n = number(w[1]);
	if (!n) { r.verb = ChatRequest::Verb::Bad; r.error = fmt::format("not an approval id: '{}'", w[1]); return r; }
	r.id = *n;
	return r;
}

/** What the in-game sink knows when a client asks to decide an approval. */
struct ChatFacts {
	bool exists = false;           ///< the server holds the approval
	bool pending = false;          ///< it is still pending
	bool expired = false;          ///< past its expiry
	bool chat_decidable = false;   ///< its action is company-scoped (ai.* or a command without the Server/Offline flags)
	bool valid_company = false;    ///< the client plays a company (a spectator does not)
	bool same_company = false;     ///< the approval's arguments name the client's company
	bool key_registered = false;   ///< the client's key is in the server's approver list (declared)
	bool key_in_allow_list = false; ///< the client's key is in that company's allow list: told to the asker, never enough to approve
	bool injecting = false;        ///< the line came from injected UI input, not the person
	bool key_shared = false;       ///< another connected client presents the same key (ClientsWithKey > 1)
};

/** The verdict on one chat decision, with the condition that failed. */
enum class ChatVerdict : uint8_t { Allow, NoSuchApproval, NotPending, Expired, ServerOnly, Spectator, NotYourCompany, NotAnApprover, Injected, SharedKey };

/**
 * Decide whether a client may approve (or deny) an approval from inside the game. A deny withdraws
 * and executes nothing, so it needs only the approval's own company; an approval executes, so it
 * also needs an approver -- a key the server's operator registered, and only that: a company's allow
 * list admits whoever plays it, an agent's own client included, so it never makes an approver
 * (the owner's decision, 2026-11-22, ADR 0038) -- and the person's own input (never injected input).
 * A registered key that a second connected client presents at the same time approves from neither:
 * OpenTTD admits both connections, and a copied key must not make its holder an approver.
 */
inline ChatVerdict DecideChat(const ChatFacts &f, bool approving)
{
	if (!f.exists) return ChatVerdict::NoSuchApproval;
	if (!f.pending) return ChatVerdict::NotPending;
	if (f.expired) return ChatVerdict::Expired;
	if (!f.chat_decidable) return ChatVerdict::ServerOnly;
	if (!f.valid_company) return ChatVerdict::Spectator;
	if (!f.same_company) return ChatVerdict::NotYourCompany;
	if (!approving) return ChatVerdict::Allow;
	if (f.injecting) return ChatVerdict::Injected;
	if (!f.key_registered) return ChatVerdict::NotAnApprover;
	if (f.key_shared) return ChatVerdict::SharedKey;
	return ChatVerdict::Allow;
}

/** The reason a verdict gives the asker, naming the condition that failed. */
inline std::string_view ChatVerdictText(ChatVerdict v)
{
	switch (v) {
		case ChatVerdict::Allow: return "allowed";
		case ChatVerdict::NoSuchApproval: return "the server holds no such approval";
		case ChatVerdict::NotPending: return "it is not pending any more";
		case ChatVerdict::Expired: return "it expired";
		case ChatVerdict::ServerOnly: return "it acts beyond one company, so only the server's operator decides it (the panel or the server console)";
		case ChatVerdict::Spectator: return "a spectator decides nothing; join the company it names";
		case ChatVerdict::NotYourCompany: return "it is not your company's to decide";
		case ChatVerdict::NotAnApprover: return "you are not an approver: only a key the server's operator registered approves ('prismatic_mcp approvers add <your client id>'); playing the company, or being in its allow list, is not enough";
		case ChatVerdict::Injected: return "it came from injected input, not your own keyboard or mouse";
		case ChatVerdict::SharedKey: return "your approver key is connected twice; an approver key approves only while one client holds it (disconnect the other client)";
	}
	return "refused";
}

/** A line of at most `max` bytes, cut at a UTF-8 boundary with an ellipsis when it was longer. */
inline std::string CapLine(std::string s, size_t max)
{
	if (s.size() <= max) return s;
	bool mark = max >= 3; // room for the ellipsis; below that the cut text alone, never over max
	size_t cut = mark ? max - 3 : max;
	while (cut > 0 && (static_cast<unsigned char>(s[cut]) & 0xC0) == 0x80) cut--;
	s.resize(cut);
	return mark ? s + "..." : s;
}

/** The AI setting names an ai.set_setting or ai.set_settings action changes (empty for any other). */
inline std::vector<std::string> SettingKeys(std::string_view type, const nlohmann::json &args)
{
	std::vector<std::string> keys;
	if (!args.is_object()) return keys;
	if (type == "ai.set_setting" && args.contains("name") && args["name"].is_string()) keys.push_back(args["name"].get<std::string>());
	if (type == "ai.set_settings" && args.contains("settings") && args["settings"].is_object()) {
		for (const auto &[k, v] : args["settings"].items()) keys.push_back(k);
	}
	return keys;
}

/**
 * Whether a new pending approval supersedes an older pending one: both change AI settings
 * (ai.set_setting or ai.set_settings) of the same company and share a setting. Approving the older
 * one afterwards would undo the newer plan for that setting, so it is withdrawn when the newer
 * one arrives.
 */
inline bool Supersedes(std::string_view new_type, const nlohmann::json &new_args, std::string_view old_type, const nlohmann::json &old_args)
{
	if (!new_args.is_object() || !old_args.is_object() || !new_args.contains("company") || new_args["company"] != old_args.value("company", nlohmann::json())) return false;
	std::vector<std::string> a = SettingKeys(new_type, new_args), b = SettingKeys(old_type, old_args);
	return std::any_of(a.begin(), a.end(), [&](const std::string &k) { return std::find(b.begin(), b.end(), k) != b.end(); });
}

/**
 * Whether a chat line may decide an action at all (ADR 0038): only what acts on one company. That
 * is a change of one company's AI settings, or a game command that is neither a server, an offline
 * nor a deity command and not of the server-setting category (CompanyControl, which creates and
 * deletes companies, is one: OpenTTD makes it server-only in the network layer, not by its flag).
 * Everything else -- pausing, starting or stopping an AI, script API calls -- is the server
 * operator's. `flags` and `category` are the command's own, as cmds::Describe gives them.
 */
inline bool ChatScoped(std::string_view type, const nlohmann::json &flags, std::string_view category)
{
	if (type == "ai.set_setting" || type == "ai.set_settings") return true;
	if (!type.starts_with("command.") || !flags.is_array()) return false;
	for (const auto &f : flags) {
		if (f == "server" || f == "offline" || f == "deity") return false;
	}
	return category != "server_setting" && category != "cheat" && !category.empty();
}

/** An approver's key as the log and the audit trail show it: its first 12 characters. */
inline std::string KeyFingerprint(std::string_view key) { return std::string(key.substr(0, 12)); }

/** One connected client as the approver seats see it: its id, the company it plays (-1 for a spectator) and its public key. */
struct ClientSeat {
	uint32_t client_id = 0;
	int company = -1;
	std::string key;
};

/** A company a registered approver plays now: the company, the approver's client and its key's fingerprint. */
struct ApproverSeat {
	int company = -1;
	uint32_t client_id = 0;
	std::string key_fp;
	bool operator==(const ApproverSeat &) const = default;
};

/** How many connected clients present `key`. OpenTTD admits two connections with one key, so a key is one person only while one client holds it. */
inline size_t ClientsWithKey(const std::vector<ClientSeat> &clients, std::string_view key)
{
	if (key.empty()) return 0;
	return static_cast<size_t>(std::count_if(clients.begin(), clients.end(), [key](const ClientSeat &c) { return c.key == key; }));
}

/**
 * The companies registered approvers play now (openttd://approvals approver_companies): one seat per
 * registered key that exactly one connected client presents, while that client plays a company. A
 * key two clients present at once is no approver at all -- a copied key used from a second client
 * must not approve, and the server cannot tell which of the two is the person -- and a spectator
 * plays no company. Sorted by company, then client.
 */
inline std::vector<ApproverSeat> ApproverSeats(const std::vector<ClientSeat> &clients, const std::vector<std::string> &registered)
{
	std::vector<ApproverSeat> seats;
	for (const auto &key : registered) {
		if (ClientsWithKey(clients, key) != 1) continue;
		auto c = std::find_if(clients.begin(), clients.end(), [&key](const ClientSeat &s) { return s.key == key; });
		if (c->company < 0) continue;
		seats.push_back({c->company, c->client_id, KeyFingerprint(key)});
	}
	std::sort(seats.begin(), seats.end(), [](const ApproverSeat &a, const ApproverSeat &b) {
		return a.company != b.company ? a.company < b.company : a.client_id < b.client_id;
	});
	return seats;
}

/** Whether a registered approver plays `company` among `seats`; a company of -1 (none named) never is. */
inline bool ApproverPlays(const std::vector<ApproverSeat> &seats, int company)
{
	return company >= 0 && std::any_of(seats.begin(), seats.end(), [company](const ApproverSeat &s) { return s.company == company; });
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

/**
 * The server's approval queue as a network client hears it (ADR 0038): the server answers
 * '!approvals' with one private line per pending approval, "[approvals] {json}" with an "id", and a
 * last line "[approvals] {"count": n, ...}". Feed() reads each line; the count line ends a reply,
 * and the rows of that reply become the queue (`rows`, and `count`, the server's total, which may be
 * more than one page). A line that is not "[approvals] " followed by a JSON object is not one.
 * The control window of a network client shows this queue (Panel::Snapshot), since the client's
 * own MCP server holds none of the server's approvals.
 */
struct ServerApprovalsReader {
	nlohmann::json rows = nlohmann::json::array(); ///< the approvals of the last whole reply
	uint64_t count = 0;                            ///< how many the server holds for the company
	uint64_t replies = 0;                          ///< whole replies read (every page of one)
	uint64_t answers = 0;                          ///< answers read: one per page
	uint64_t next_page = 0;                        ///< the page to ask for next, while a reply has more; else 0
	nlohmann::json page = nlohmann::json::array(); ///< the rows of the answer being read
	nlohmann::json pages = nlohmann::json::array(); ///< the rows of the earlier pages of the reply being read

	/** Read one chat line; true when it is an "[approvals]" line (and then it is consumed here). */
	bool Feed(std::string_view line)
	{
		constexpr std::string_view PREFIX = "[approvals] ";
		if (!line.starts_with(PREFIX)) return false;
		nlohmann::json j = nlohmann::json::parse(line.substr(PREFIX.size()), nullptr, false);
		if (!j.is_object()) return false;
		if (j.contains("id")) {
			this->page.push_back(std::move(j));
			return true;
		}
		if (j.contains("count") && j["count"].is_number_unsigned()) {
			/* A queue longer than one answer comes a page at a time: the reply is whole at its last page. */
			const uint64_t at = j.value("page", uint64_t{1}), of = j.value("pages", uint64_t{1});
			if (at <= 1) this->pages = nlohmann::json::array();
			for (auto &row : this->page) this->pages.push_back(std::move(row));
			this->page = nlohmann::json::array();
			this->answers++;
			if (at < of) {
				this->next_page = at + 1;
				return true;
			}
			this->next_page = 0;
			this->rows = std::move(this->pages);
			this->pages = nlohmann::json::array();
			this->count = j["count"].get<uint64_t>();
			this->replies++;
			return true;
		}
		return false;
	}
};

} // namespace PrismaticMCP::detail

#endif /* OTTD_PRISMATIC_MCP_SERVER */
#endif /* PRISMATIC_MCP_DETAIL_H */
