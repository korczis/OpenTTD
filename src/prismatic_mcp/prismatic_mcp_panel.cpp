/*
 * This file is part of OpenTTD.
 * OpenTTD is free software; you can redistribute it and/or modify it under the terms of the GNU General Public License as published by the Free Software Foundation, version 2.
 * OpenTTD is distributed in the hope that it will be useful, but WITHOUT ANY WARRANTY; without even the implied warranty of MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.
 * See the GNU General Public License for more details. You should have received a copy of the GNU General Public License along with OpenTTD. If not, see <https://www.gnu.org/licenses/old-licenses/gpl-2.0>.
 */

/*
 * RESEARCH-ONLY / UNSTABLE. Compiled only with OPTION_PRISMATIC_MCP_SERVER (ADR 0008, ADR 0030).
 *
 * The Prismatic control window: the fork's state where the owner plays, in one list. The server's
 * status, every approval that waits for the owner, the latest decisions and what the game observed
 * after each, the questions asked from the toolbar and their answers, and the menu's switches. It
 * holds no state of its own and writes no list of its own: Snapshot() reads the functions the
 * console reads (GetStatusLine, PendingApprovals, DecisionTrace) and the menu's own table
 * (Menu::Describe, Menu::Requests), and openttd.ui.control answers that same Snapshot() over MCP.
 *
 * Every approval row has its own Approve, Deny and Always... buttons at the same x on every row,
 * a check box for Approve selected / Deny selected, and the keyboard goes through a queue without the
 * mouse (Up/Down or J/K, A or Y approve, D or N deny, Space checks). The owner's order of
 * 2026-11-23 ("tlacitko u kazdeho radku, approve/deny all, selectory; next"), ADR 0041.
 *
 * Every consent runs the console's `prismatic_mcp approve|deny <id> ... --via window` or
 * `prismatic_mcp rule allow|deny ... --via window`, so it consents exactly as the owner typing it
 * would, through whatever path that command takes. An Approve, an Approve selected/all or an allow
 * rule injected by an openttd.input.* tool (UI::InjectingInput) is refused: the window exists for the
 * owner's hand, and no script, bot or MCP client approves its own request by moving the owner's
 * mouse. Deny, which only withdraws a request, stays open to anyone, as it is in the panel.
 */

/** @file prismatic_mcp_panel.cpp The Prismatic control window: the fork's state in the game, read from the same functions the console and MCP read. */

#include "../stdafx.h"

#ifdef OTTD_PRISMATIC_MCP_SERVER

#include "prismatic_mcp_panel.h"
#include "prismatic_mcp.h"
#include "prismatic_mcp_detail.h"
#include "prismatic_mcp_menu.h"
#include "prismatic_mcp_ui.h"

#include "../company_base.h"
#include "../company_gui.h"
#include "../company_type.h"
#include "../console_func.h"
#include "../console_gui.h"
#include "../dropdown_func.h"
#include "../dropdown_type.h"
#include "../gfx_func.h"
#include "../map_func.h"
#include "../network/network.h"
#include "../network/network_base.h"
#include "../network/network_func.h"
#include "../openttd.h"
#include "../palette_func.h"
#include "../strings_func.h"
#include "../table/sprites.h"
#include "../table/strings.h"
#include "../textbuf_gui.h"
#include "../timer/timer.h"
#include "../timer/timer_window.h"
#include "../viewport_func.h"
#include "../widget_type.h"
#include "../window_func.h"
#include "../window_gui.h"
#include "../zoom_func.h"

#include <algorithm>
#include <chrono>
#include <map>
#include <set>
#include <vector>

#include "../safeguards.h"

using json = nlohmann::json;

namespace PrismaticMCP::Panel {

/** How many of the latest decisions and requests the window lists. */
static constexpr int RECENT = 8;

/* ---- the server's queue on a network client (ADR 0038) ------------------------------------ *
 * A network client's own MCP server holds none of the server's approvals, so on a client the window
 * shows the server's queue as the server answers '!approvals' (detail::ServerApprovalsReader). The
 * client asks quietly every ASK_EVERY while it is in a game, and the answers to its own asks are
 * read, not printed in the chat. When an approval the client has not seen yet arrives, the window
 * opens: a decision that waits is shown as a decision (Approve, Deny), not as news. Approve and Deny
 * still run the console's command, which sends '!approve <id>' or '!deny <id>' to the server, where
 * only a registered approver's key approves (prismatic_mcp approvers, research/coplay.md). */

/** The server's queue as this client last heard it. */
static detail::ServerApprovalsReader _server_queue;
/** Replies to this client's own quiet asks still to arrive: those are read, not printed. */
static uint64_t _quiet_asks = 0;
/** The highest approval id seen in a reply: a higher one opens the window. */
static uint64_t _highest_seen = 0;
/** When '!approvals' was last sent, so that the server is asked at most every ASK_EVERY. */
static std::chrono::steady_clock::time_point _last_ask{};
static constexpr auto ASK_EVERY = std::chrono::seconds(5);

/** A network client that has joined a game: the queue it can decide is the server's. */
static bool OnJoinedNetworkClient()
{
	return _networking && !_network_server && NetworkClientInfo::GetByClientID(_network_own_client_id) != nullptr;
}

/** Ask the server for this company's queue, at most every ASK_EVERY; its reply is read quietly. */
static void AskServerQuietly()
{
	if (!OnJoinedNetworkClient()) return;
	auto now = std::chrono::steady_clock::now();
	if (_last_ask != std::chrono::steady_clock::time_point{} && now - _last_ask < ASK_EVERY) return;
	_last_ask = now;
	_quiet_asks++;
	NetworkClientSendChat(NetworkAction::ChatClient, NetworkChatDestinationType::Client, to_underlying(ClientID::Server), "!approvals");
}

bool OnServerChat(std::string_view msg)
{
	uint64_t answers = _server_queue.answers;
	if (!_server_queue.Feed(msg)) return false;
	const bool quiet = _quiet_asks > 0;
	if (_server_queue.answers == answers) return quiet; // a row; the answer goes on
	if (quiet) _quiet_asks--;
	/* Every page of the server's queue, not the first ten rows only: the next page is asked quietly. */
	if (_server_queue.next_page != 0) {
		if (OnJoinedNetworkClient()) {
			_quiet_asks++;
			NetworkClientSendChat(NetworkAction::ChatClient, NetworkChatDestinationType::Client, to_underlying(ClientID::Server),
				fmt::format("!approvals {}", _server_queue.next_page));
		}
		return quiet;
	}
	uint64_t highest = _highest_seen;
	for (const json &r : _server_queue.rows) highest = std::max<uint64_t>(highest, r.value("id", uint64_t{0}));
	if (highest > _highest_seen) {
		_highest_seen = highest;
		Show();
	}
	return quiet;
}

/** The quiet ask runs whether or not the window is open, so that a new decision opens it. */
static const IntervalTimer<TimerWindow> _server_queue_poll(ASK_EVERY, [](auto) { AskServerQuietly(); });

json Snapshot()
{
	json approvals = json::array();
	json rules = json::array();
	if (OnJoinedNetworkClient()) {
		/* The server's queue as it answered '!approvals'; its rules stay on the server. */
		for (const json &r : _server_queue.rows) {
			approvals.push_back({{"id", r.value("id", uint64_t{0})}, {"tool", r.value("tool", "")}, {"args", r.value("args", "")},
				{"expired", false}, {"on_server", true}, {"chat_decidable", r.value("chat_decidable", false)}});
		}
	} else {
		for (const PendingApprovalInfo &a : PendingApprovals()) {
			json row = {{"id", a.id}, {"tool", a.tool}, {"mcp_tool", a.mcp_tool}, {"args", a.canonical_args}, {"expired", a.expired}};
			if (a.matched_rule != 0) {
				for (const Rules::Rule &r : ApprovalRules()) {
					if (r.id == a.matched_rule) row["matched_rule"] = {{"id", r.id}, {"effect", Rules::EffectName(r.effect)}, {"decides", a.rule_decides}};
				}
			}
			approvals.push_back(std::move(row));
		}
		for (const Rules::Rule &r : ApprovalRules()) rules.push_back(Rules::ToJson(r));
	}
	json decisions = json::array();
	const std::string trace = DecisionTrace(RECENT);
	if (trace.rfind("decision #", 0) == 0) { // "no decisions executed" otherwise
		for (size_t start = 0; start < trace.size();) {
			size_t end = std::min(trace.find('\n', start), trace.size());
			if (end > start) decisions.push_back(trace.substr(start, end - start));
			start = end + 1;
		}
	}
	json requests = json::array();
	const json all = Menu::Requests()["requests"];
	for (size_t i = all.size() > RECENT ? all.size() - RECENT : 0; i < all.size(); i++) requests.push_back(all[i]);
	json switches = json::array(), links = json::array();
	const json menu = Menu::Describe(); // held: a range-for over a temporary's member dangles
	for (const json &item : menu["items"]) {
		const std::string action = item.value("action", "");
		if (action == "toggle") switches.push_back({{"index", item["index"]}, {"label", item["label"]}, {"on", item.value("on", false)}});
		if (action == "browse") links.push_back({{"index", item["index"]}, {"label", item["label"]}, {"url", item["target"]}});
	}
	return {{"status", GetStatusLine()}, {"approvals", std::move(approvals)}, {"rules", std::move(rules)}, {"decisions", std::move(decisions)},
		{"requests", std::move(requests)}, {"switches", std::move(switches)}, {"links", std::move(links)}};
}

std::optional<std::string> ConsentRefusal(bool approve)
{
	if (approve && UI::InjectingInput()) return "an approval needs the owner's own click; input injected through a UI tool (openttd.input.*, openttd.ui.press, ...) never consents";
	return std::nullopt;
}

std::optional<std::string> RuleRefusal(Rules::Effect effect, const Rules::Spec &spec)
{
	if (auto refused = ConsentRefusal(effect == Rules::Effect::Allow); refused.has_value()) return refused;
	/* The core refuses a broad allow from the window (Rules::BroadAllowRefusal, IsBroadMatcher):
	 * the window says so with the same words, before it asks the owner to confirm the rule. */
	Rules::Spec from_window = spec;
	from_window.via = "window";
	return Rules::BroadAllowRefusal(effect, from_window);
}

std::string ConsentCommand(bool approve, const std::vector<uint64_t> &ids)
{
	std::string line = fmt::format("prismatic_mcp {}", approve ? "approve" : "deny");
	for (uint64_t id : ids) line += fmt::format(" {}", id);
	return line + " --via window";
}

std::string ConsentCommand(bool approve, uint64_t id)
{
	return ConsentCommand(approve, std::vector<uint64_t>{id});
}

json Tool()
{
	return {{"name", "openttd.ui.control"},
		{"description", "What the Prismatic control window shows, as data: the MCP server's status, every approval still waiting (id, tool, MCP tool, the exact arguments, expired, the standing rule that applies to it as matched_rule), the owner's standing approval rules with their hits (ADR 0041), the latest decisions with their verdicts and channels, the questions asked in this game, and the toolbar's switches and hub links. Read-only: it decides nothing"},
		{"inputSchema", {{"type", "object"}}},
		{"annotations", {{"readOnlyHint", true}, {"idempotentHint", true}, {"openWorldHint", false}}}};
}

/** The array under \p key, or an empty one: a partial snapshot (a client's cached copy) never throws. */
static const json &ArrayAt(const json &s, const char *key)
{
	static const json empty = json::array();
	auto it = s.is_object() ? s.find(key) : s.end();
	return (s.is_object() && it != s.end() && it->is_array()) ? *it : empty;
}

/** The string under \p key of \p o, or \p fallback when it is absent or not a string. */
static std::string StringAt(const json &o, const char *key, std::string_view fallback = {})
{
	auto it = o.is_object() ? o.find(key) : o.end();
	return (o.is_object() && it != o.end() && it->is_string()) ? it->get<std::string>() : std::string{fallback};
}

/** The unsigned number under \p key of \p o, or 0. */
static uint64_t IdAt(const json &o, const char *key)
{
	auto it = o.is_object() ? o.find(key) : o.end();
	return (o.is_object() && it != o.end() && it->is_number_unsigned()) ? it->get<uint64_t>()
		: (o.is_object() && it != o.end() && it->is_number_integer() && it->get<int64_t>() > 0) ? static_cast<uint64_t>(it->get<int64_t>()) : 0;
}

/** The boolean under \p key of \p o, or false. */
static bool FlagAt(const json &o, const char *key)
{
	auto it = o.is_object() ? o.find(key) : o.end();
	return o.is_object() && it != o.end() && it->is_boolean() && it->get<bool>();
}

/** How much of an approval's arguments a row spells; the details area shows them whole. */
static constexpr size_t ROW_ARGS = 160;

std::vector<Row> Rows(const json &s, const View &view)
{
	std::vector<Row> rows;
	auto heading = [&rows](std::string text) { rows.push_back({Row::Kind::Heading, std::move(text)}); };
	auto text = [&rows](std::string text) { rows.push_back({Row::Kind::Text, std::move(text)}); };
	const json &approvals = ArrayAt(s, "approvals"), &decisions = ArrayAt(s, "decisions"), &rules = ArrayAt(s, "rules");
	const json &requests = ArrayAt(s, "requests"), &switches = ArrayAt(s, "switches");
	heading("MCP server");
	text(StringAt(s, "status", "status unknown"));
	std::vector<Row> shown;
	for (const json &a : approvals) {
		const std::string tool = StringAt(a, "tool"), args = StringAt(a, "args");
		const uint64_t id = IdAt(a, "id");
		const bool expired = FlagAt(a, "expired");
		if (!view.tool.empty() && tool != view.tool) continue;
		if (view.hide_expired && expired) continue;
		/* A server's approval that a chat approver may not decide stays with the server's operator. */
		const bool operators = FlagAt(a, "on_server") && !FlagAt(a, "chat_decidable");
		std::string rule;
		if (auto it = a.is_object() ? a.find("matched_rule") : a.end(); a.is_object() && it != a.end() && it->is_object()) {
			rule = fmt::format("  (rule #{} {}{})", IdAt(*it, "id"), StringAt(*it, "effect"), FlagAt(*it, "decides") ? "" : ", held: you decide it");
		}
		const bool sent = view.sent.contains(id);
		Row r{Row::Kind::Approval, fmt::format("#{}  {}  {}{}{}{}", id, tool, detail::CapLine(args, ROW_ARGS),
			operators ? "  (the server's operator decides)" : "", rule, sent ? "  (sent, waiting for the server)" : ""), id, expired};
		r.sent = sent;
		r.checked = view.checked.contains(id);
		r.decidable = !operators;
		r.tool = tool;
		r.args = args;
		shown.push_back(std::move(r));
	}
	heading(shown.size() == approvals.size() ? fmt::format("Waiting for your approval ({})", approvals.size())
		: fmt::format("Waiting for your approval ({} of {} shown)", shown.size(), approvals.size()));
	if (approvals.empty()) text("nothing waits: a change an agent asks for appears here");
	else if (shown.empty()) text("nothing the selectors show: choose Tool: all, or show expired");
	for (Row &r : shown) rows.push_back(std::move(r));
	heading(fmt::format("Standing rules ({})", rules.size()));
	if (rules.empty()) text("none: every approval waits for you; Always... on an approval makes one");
	for (const json &r : rules) {
		const bool allow = StringAt(r, "effect") == "allow";
		const bool from_file = StringAt(r, "created_via") == "file";
		rows.push_back({Row::Kind::Rule, fmt::format("{}  hits={} via={}{}{}", StringAt(r, "text"), IdAt(r, "hits"), StringAt(r, "created_via"),
			FlagAt(r, "persisted") ? " persisted" : "", allow && from_file ? "  -- " + Rules::FileRuleNote(IdAt(r, "id")) : ""),
			IdAt(r, "id"), allow});
	}
	heading("Latest decisions: what ran, and what the game showed afterwards");
	if (decisions.empty()) text("no decision has run yet");
	for (const json &d : decisions) text(d.is_string() ? d.get<std::string>() : d.dump());
	heading("Questions from this game");
	if (requests.empty()) text("none yet: use Ask, or a request item of the toolbar menu");
	for (const json &r : requests) {
		std::string asked = StringAt(r, "text");
		text(fmt::format("#{} {}{}{} - {}", IdAt(r, "id"), StringAt(r, "kind"), asked.empty() ? "" : ": ", asked,
			FlagAt(r, "answered") ? StringAt(r, "answer") : std::string{"waiting for an answer"}));
	}
	if (!switches.empty()) heading("Switches (click to flip)");
	for (const json &sw : switches) {
		rows.push_back({Row::Kind::Switch, StringAt(sw, "label"), IdAt(sw, "index"), FlagAt(sw, "on")});
	}
	return rows;
}

std::vector<std::string> ToolNames(const json &snapshot)
{
	std::set<std::string> names;
	for (const json &a : ArrayAt(snapshot, "approvals")) {
		if (std::string t = StringAt(a, "tool"); !t.empty()) names.insert(std::move(t));
	}
	return {names.begin(), names.end()};
}

std::vector<uint64_t> Shown(const std::vector<Row> &rows)
{
	std::vector<uint64_t> ids;
	for (const Row &r : rows) if (r.kind == Row::Kind::Approval) ids.push_back(r.id);
	return ids;
}

bool AnyChecked(const std::vector<Row> &rows)
{
	return std::ranges::any_of(rows, [](const Row &r) { return r.kind == Row::Kind::Approval && r.checked; });
}

std::vector<uint64_t> BulkTargets(const std::vector<Row> &rows, bool approve)
{
	const bool checked = AnyChecked(rows);
	std::vector<uint64_t> ids;
	for (const Row &r : rows) {
		if (r.kind != Row::Kind::Approval || r.sent || (checked && !r.checked)) continue;
		if (approve && (r.flag || !r.decidable)) continue;
		ids.push_back(r.id);
	}
	return ids;
}

std::vector<uint64_t> Selected(const json &snapshot, const Rules::Spec &spec, bool skip_expired)
{
	Rules::Rule filter;
	filter.tool = spec.tool;
	filter.match = spec.match;
	std::vector<uint64_t> ids;
	for (const json &a : ArrayAt(snapshot, "approvals")) {
		const std::string tool = StringAt(a, "tool"), mcp_tool = StringAt(a, "mcp_tool");
		const json args = json::parse(StringAt(a, "args"), nullptr, false);
		if (skip_expired && FlagAt(a, "expired")) continue;
		if (Rules::Applies(filter, Rules::Subject{mcp_tool, tool, args})) ids.push_back(IdAt(a, "id"));
	}
	return ids;
}

bool KeyTaken(KeyHistory &history, uint16_t key, std::chrono::steady_clock::time_point now)
{
	const bool repeat = key == history.key && history.at != std::chrono::steady_clock::time_point{} && now - history.at < KEY_QUIET;
	history.key = key;
	history.at = now;
	return !repeat;
}

std::optional<uint64_t> NextSelection(const std::vector<uint64_t> &before, const std::set<uint64_t> &decided, const std::vector<uint64_t> &after)
{
	auto first = std::ranges::find_if(before, [&decided](uint64_t id) { return decided.contains(id); });
	if (first == before.end()) return std::nullopt;
	auto waits = [&](uint64_t id) { return !decided.contains(id) && std::ranges::find(after, id) != after.end(); };
	for (auto it = first; it != before.end(); ++it) if (waits(*it)) return *it;
	for (auto it = std::make_reverse_iterator(first); it != before.rend(); ++it) if (waits(*it)) return *it;
	return std::nullopt;
}

RowPart HitPart(int left, int right, int x, int check, int button, int always)
{
	const int always_left = right - always + 1;
	const int deny_left = always_left - button;
	const int approve_left = deny_left - button;
	if (x >= always_left) return RowPart::Always;
	if (x >= deny_left) return RowPart::Deny;
	if (x >= approve_left) return RowPart::Approve;
	if (x < left + check) return RowPart::Check;
	return RowPart::Text;
}

std::optional<uint32_t> TileOf(const json &args, uint64_t map_size)
{
	for (const char *key : {"location", "tile"}) {
		if (!args.is_object() || !args.contains(key) || !args[key].is_number_integer()) continue;
		const int64_t tile = args[key].get<int64_t>();
		if (tile >= 0 && static_cast<uint64_t>(tile) < map_size) return static_cast<uint32_t>(tile);
	}
	return std::nullopt;
}

std::optional<int> CompanyNamed(const json &args)
{
	if (!args.is_object() || !args.contains("company") || !args["company"].is_number_integer()) return std::nullopt;
	int64_t c = args["company"].get<int64_t>();
	return c >= 0 && c < MAX_COMPANIES ? std::optional<int>(static_cast<int>(c)) : std::nullopt;
}

namespace {

/** Widgets of the control window. */
enum ControlWidgets : WidgetID {
	WID_PC_CAPTION, ///< The caption: how many approvals wait, how many rules stand.
	WID_PC_TOOL, ///< The tool selector: show the approvals of one tool, or all.
	WID_PC_EXPIRED, ///< Hide expired approvals (a toggle).
	WID_PC_CHECK_SHOWN, ///< Check every approval shown.
	WID_PC_CHECK_NONE, ///< Uncheck every approval.
	WID_PC_LIST, ///< Every section, one row per line; an approval row has its own buttons.
	WID_PC_SCROLLBAR, ///< The list's scrollbar.
	WID_PC_DETAILS, ///< The selected approval's whole arguments, wrapped.
	WID_PC_APPROVE_ALL, ///< Approve the checked approvals, or every one shown.
	WID_PC_DENY_ALL, ///< Deny the checked approvals, or every one shown.
	WID_PC_GOTO, ///< Scroll to the tile, or open the company, the selected approval names.
	WID_PC_ASK, ///< Ask the planner in plain language.
	WID_PC_HUB, ///< Open the hub in the browser.
	WID_PC_CONSOLE, ///< Open the console.
};

/** The items of an approval row's Always... menu. */
enum AlwaysItem : int {
	AI_ALLOW_TOOL, ///< Always allow this tool, any arguments.
	AI_ALLOW_EDIT, ///< Always allow this tool with a matcher, edited first.
	AI_DENY_TOOL, ///< Always deny this tool, any arguments.
	AI_DENY_EDIT, ///< Always deny this tool with a matcher, edited first.
};

/** What the open query string is for. */
enum class QueryFor : uint8_t { Ask, AllowRule, DenyRule };

/** How long a row this window decided is marked sent while a server has not yet answered again. */
static constexpr auto SENT_FOR = std::chrono::seconds(12);
/** How long rows that moved, or a selection that changed, by themselves hold the decisive clicks and keys. */
static constexpr auto RECENTLY = std::chrono::milliseconds(400);

/** Whether \p at was less than \p span ago. */
static bool Within(std::chrono::steady_clock::time_point at, std::chrono::milliseconds span)
{
	return std::chrono::steady_clock::now() - at < span;
}

struct ControlWindow : Window {
	Scrollbar *vscroll = nullptr;
	json snapshot;
	std::vector<Row> rows;
	View view;
	std::optional<uint64_t> selected; ///< The approval the owner selected: the keyboard acts on it, the details show it.
	std::optional<uint64_t> anchor; ///< The approval last checked, where a Shift+click range starts.
	std::map<uint64_t, std::chrono::steady_clock::time_point> decided_at; ///< Ids sent to a server, marked sent until it lists them no more.
	std::vector<uint64_t> pending_bulk; ///< The ids Approve selected/all asks to confirm.
	std::vector<std::string> tool_choices; ///< The tool selector's choices when it opened.
	uint64_t always_for = 0; ///< The approval whose Always... menu is open.
	bool always_injected = false; ///< Whether the click that opened it was injected: a dropdown's choice arrives frames later, outside any injection scope.
	QueryFor query = QueryFor::Ask;
	bool keys_armed = false; ///< The owner clicked in this window since it got the focus: only then do its keys decide.
	std::vector<std::pair<Row::Kind, uint64_t>> layout; ///< What each row of the list was at the last refresh.
	std::chrono::steady_clock::time_point moved_at{}; ///< When rows moved without the owner's doing (a refresh).
	std::chrono::steady_clock::time_point auto_selected_at{}; ///< When the selection changed without the owner's doing.
	KeyHistory keys; ///< The last decisive key seen, to ignore a held key's repeats (KeyTaken).
	std::optional<uint64_t> pending_remove; ///< The rule Remove asks to confirm.
	std::optional<std::pair<Rules::Effect, Rules::Spec>> pending_rule; ///< The allow rule Always... asks to confirm.

	ControlWindow(WindowDesc &desc, WindowNumber number) : Window(desc)
	{
		this->CreateNestedTree();
		this->vscroll = this->GetScrollbar(WID_PC_SCROLLBAR);
		this->FinishInitNested(number);
		this->Refresh();
	}

	/** The approval row with this id, if shown. */
	const Row *Find(std::optional<uint64_t> id) const
	{
		if (!id.has_value()) return nullptr;
		for (const Row &r : this->rows) if (r.kind == Row::Kind::Approval && r.id == *id) return &r;
		return nullptr;
	}

	/**
	 * Read the snapshot again. The selection stays while that approval still shows; when it went
	 * (decided elsewhere, expired), the row now at its place is selected. Rows that moved, or a
	 * selection that changed, without the owner's doing (\p owner false: the 1 s refresh) hold the
	 * decisive clicks and keys for a moment (RECENTLY), so that a click aimed at one row never
	 * decides the one that slid under the pointer.
	 */
	void Refresh(bool owner = true)
	{
		const std::vector<uint64_t> before = Shown(this->rows);
		this->snapshot = Snapshot();
		std::set<uint64_t> waiting;
		for (const json &a : ArrayAt(this->snapshot, "approvals")) waiting.insert(IdAt(a, "id"));
		auto now = std::chrono::steady_clock::now();
		this->view.sent.clear();
		std::erase_if(this->decided_at, [&](const auto &e) { return !waiting.contains(e.first) || now - e.second > SENT_FOR; });
		for (const auto &e : this->decided_at) this->view.sent.insert(e.first);
		std::erase_if(this->view.checked, [&](uint64_t id) { return !waiting.contains(id); });
		this->rows = Rows(this->snapshot, this->view);
		std::vector<std::pair<Row::Kind, uint64_t>> layout;
		for (const Row &r : this->rows) layout.emplace_back(r.kind, r.id);
		if (!owner && !this->layout.empty() && layout != this->layout) this->moved_at = now;
		this->layout = std::move(layout);
		const std::vector<uint64_t> shown = Shown(this->rows);
		if (this->selected.has_value() && this->Find(this->selected) == nullptr) {
			const uint64_t gone = *this->selected;
			this->selected = NextSelection(before, {gone}, shown);
			if (!owner) this->auto_selected_at = now;
		}
		if (!this->selected.has_value() && !shown.empty()) this->selected = shown.front();
		this->vscroll->SetCount(this->rows.size());
		this->SetWidgetDisabledState(WID_PC_APPROVE_ALL, BulkTargets(this->rows, true).empty());
		this->SetWidgetDisabledState(WID_PC_DENY_ALL, BulkTargets(this->rows, false).empty());
		this->SetWidgetDisabledState(WID_PC_GOTO, !this->GotoTarget().has_value());
		this->SetWidgetLoweredState(WID_PC_EXPIRED, this->view.hide_expired);
		this->SetDirty();
	}

	/** Where Go to leads for the selected approval: a tile (scroll there) or a company (-1 - id). */
	std::optional<int64_t> GotoTarget() const
	{
		const Row *r = this->Find(this->selected);
		if (r == nullptr) return std::nullopt;
		const json args = json::parse(r->args, nullptr, false);
		if (auto tile = TileOf(args, Map::Size()); tile.has_value()) return static_cast<int64_t>(*tile);
		if (auto c = CompanyNamed(args); c.has_value() && Company::IsValidID(CompanyID(*c))) return -1 - static_cast<int64_t>(*c);
		return std::nullopt;
	}

	std::string GetWidgetString(WidgetID widget, StringID stringid) const override
	{
		switch (widget) {
			case WID_PC_CAPTION: {
				size_t waiting = ArrayAt(this->snapshot, "approvals").size(), rules = ArrayAt(this->snapshot, "rules").size();
				if (waiting == 0 && rules == 0) return "Prismatic control";
				return fmt::format("Prismatic control - {} waiting for you, {} standing rule{}", waiting, rules, rules == 1 ? "" : "s");
			}
			case WID_PC_TOOL: return this->view.tool.empty() ? std::string{"Tool: all"} : "Tool: " + this->view.tool;
			case WID_PC_EXPIRED: return "Hide expired";
			case WID_PC_CHECK_SHOWN: return "Check shown";
			case WID_PC_CHECK_NONE: return "Check none";
			case WID_PC_APPROVE_ALL: return fmt::format("Approve {} ({})", AnyChecked(this->rows) ? "selected" : "all", BulkTargets(this->rows, true).size());
			case WID_PC_DENY_ALL: return fmt::format("Deny {} ({})", AnyChecked(this->rows) ? "selected" : "all", BulkTargets(this->rows, false).size());
			case WID_PC_GOTO: return "Go to";
			case WID_PC_ASK: return "Ask the planner...";
			case WID_PC_HUB: return "Open the hub";
			case WID_PC_CONSOLE: return "Console";
		}
		return this->Window::GetWidgetString(widget, stringid);
	}

	/** The height of one row of the list: tall enough for its buttons. */
	static int LineHeight()
	{
		return GetCharacterHeight(FontSize::Normal) + WidgetDimensions::scaled.framerect.Vertical() + WidgetDimensions::scaled.vsep_normal;
	}

	/** The width of a row's Approve and Deny buttons: the same on every row, big enough for a quick click. */
	static int ButtonWidth()
	{
		uint w = std::max(GetStringBoundingBox("Approve").width, GetStringBoundingBox("Deny").width);
		return std::max<int>(w + WidgetDimensions::scaled.framerect.Horizontal() + ScaleGUITrad(12), ScaleGUITrad(64));
	}

	/** The width of a row's Always... button (and a rule's Remove). */
	static int AlwaysWidth()
	{
		uint w = std::max(GetStringBoundingBox("Always...").width, GetStringBoundingBox("Remove").width);
		return w + WidgetDimensions::scaled.framerect.Horizontal() + ScaleGUITrad(12);
	}

	/** The width of a row's check box. */
	static int CheckWidth()
	{
		return GetSpriteSize(SPR_BOX_EMPTY).width + WidgetDimensions::scaled.hsep_normal * 2;
	}

	void UpdateWidgetSize(WidgetID widget, Dimension &size, [[maybe_unused]] const Dimension &padding, Dimension &fill, Dimension &resize) override
	{
		switch (widget) {
			case WID_PC_LIST:
				resize.width = 1;
				fill.height = resize.height = LineHeight();
				size.width = std::max<uint>(size.width, ScaleGUITrad(560));
				size.height = std::max<uint>(size.height, resize.height * 14 + WidgetDimensions::scaled.framerect.Vertical());
				break;
			case WID_PC_DETAILS:
				size.height = std::max<uint>(size.height, GetCharacterHeight(FontSize::Normal) * 6 + WidgetDimensions::scaled.framerect.Vertical());
				break;
			case WID_PC_APPROVE_ALL:
			case WID_PC_DENY_ALL:
				size.width = std::max<uint>(size.width, GetStringBoundingBox("Approve selected (999)").width + padding.width);
				break;
			case WID_PC_TOOL:
				size.width = std::max<uint>(size.width, ScaleGUITrad(180));
				break;
		}
	}

	/** Draw a button inside a row: a raised frame and its label, centred. */
	static void DrawRowButton(const Rect &r, std::string_view label, Colours colour, bool usable)
	{
		DrawFrameRect(r, colour, {});
		DrawString(r.left, r.right, r.top + (r.Height() - GetCharacterHeight(FontSize::Normal)) / 2, label,
			usable ? TextColour::White : TextColour::Grey, AlignmentH::Centre);
	}

	void DrawWidget(const Rect &r, WidgetID widget) const override
	{
		if (widget == WID_PC_DETAILS) {
			Rect ir = r.Shrink(WidgetDimensions::scaled.framerect);
			const Row *sel = this->Find(this->selected);
			std::string text = sel == nullptr
				? std::string{"Select an approval to read its whole arguments. Keys (click the window first): Up/Down or J/K move, A or Y approve, D or N deny, Space checks the row."}
				: fmt::format("#{} {}{}: {}", sel->id, sel->tool, sel->flag ? " (expired)" : "", sel->args);
			DrawStringMultiLineWithClipping(ir.left, ir.right, ir.top, ir.bottom, text, sel == nullptr ? TextColour::Grey : TextColour::White);
			return;
		}
		if (widget != WID_PC_LIST) return;
		const int line = LineHeight();
		const int button = ButtonWidth(), always = AlwaysWidth(), check = CheckWidth();
		const int text_y = (line - GetCharacterHeight(FontSize::Normal)) / 2;
		Rect ir = r.Shrink(WidgetDimensions::scaled.framerect);
		auto [first, last] = this->vscroll->GetVisibleRangeIterators(this->rows);
		for (auto it = first; it != last; ++it) {
			const Row &row = *it;
			Rect lr = ir.WithHeight(line);
			const Rect br = lr.Shrink(0, WidgetDimensions::scaled.vsep_normal / 2 + 1);
			const Rect always_r = br.WithWidth(always, true);
			switch (row.kind) {
				case Row::Kind::Heading:
					DrawString(lr.left, lr.right, lr.top + text_y, row.text, TextColour::Gold);
					break;
				case Row::Kind::Text:
					DrawString(lr.left + WidgetDimensions::scaled.hsep_indent, lr.right, lr.top + text_y, row.text, TextColour::White);
					break;
				case Row::Kind::Approval: {
					const bool on = this->selected == row.id;
					if (on) GfxFillRect(lr, PC_DARK_GREY);
					const Rect deny_r = always_r.Translate(-button, 0).WithWidth(button, false);
					const Rect approve_r = deny_r.Translate(-button, 0);
					Dimension box = GetSpriteSize(SPR_BOX_EMPTY);
					DrawSprite(row.checked ? SPR_BOX_CHECKED : SPR_BOX_EMPTY, PAL_NONE, lr.left + WidgetDimensions::scaled.hsep_normal, lr.top + (line - (int)box.height) / 2);
					DrawString(lr.left + check, approve_r.left - WidgetDimensions::scaled.hsep_normal, lr.top + text_y,
						row.flag ? row.text + "  (expired)" : row.text, row.flag ? TextColour::Grey : (on ? TextColour::White : TextColour::Orange));
					const bool may_approve = !row.flag && row.decidable;
					DrawRowButton(approve_r, "Approve", may_approve ? Colours::Green : Colours::Grey, may_approve);
					DrawRowButton(deny_r, "Deny", Colours::Red, true);
					DrawRowButton(always_r, "Always...", Colours::Grey, !OnJoinedNetworkClient());
					break;
				}
				case Row::Kind::Rule:
					DrawString(lr.left + WidgetDimensions::scaled.hsep_indent, always_r.left - WidgetDimensions::scaled.hsep_normal, lr.top + text_y,
						row.text, row.flag ? TextColour::Yellow : TextColour::LightBlue);
					DrawRowButton(always_r, "Remove", Colours::Grey, true);
					break;
				case Row::Kind::Switch:
					DrawString(lr.left + WidgetDimensions::scaled.hsep_indent, lr.right, lr.top + text_y,
						fmt::format("[{}] {}", row.flag ? "x" : " ", row.text), row.flag ? TextColour::Green : TextColour::White);
					break;
			}
			ir.top += line;
		}
	}

	/** Scroll the list so that the approval \p id is in view. */
	void ScrollTo(uint64_t id)
	{
		for (size_t i = 0; i < this->rows.size(); i++) {
			if (this->rows[i].kind == Row::Kind::Approval && this->rows[i].id == id) this->vscroll->ScrollTowards(static_cast<int>(i));
		}
	}

	/**
	 * Approve or deny \p ids through the console's own command, unless an approval is not the
	 * owner's own act; then select the next row still waiting, keeping the list where it is.
	 */
	void Consent(bool approve, const std::vector<uint64_t> &ids)
	{
		if (ids.empty()) return;
		if (auto refused = ConsentRefusal(approve); refused.has_value()) {
			IConsolePrint(CC_ERROR, "Prismatic control: {}", *refused);
			return;
		}
		const std::vector<uint64_t> before = Shown(this->rows);
		const std::set<uint64_t> decided(ids.begin(), ids.end());
		IConsoleCmdExec(ConsentCommand(approve, ids));
		/* A server's queue lists them until it is asked again; this window marks them sent meanwhile
		 * (unless this client cannot consent at all: then its console said why, and they stay as they were). */
		if (OnJoinedNetworkClient() && !(approve && UI::InputArmed())) {
			for (uint64_t id : ids) this->decided_at[id] = std::chrono::steady_clock::now();
		}
		for (uint64_t id : ids) this->view.checked.erase(id);
		this->selected.reset();
		this->Refresh();
		this->selected = NextSelection(before, decided, Shown(this->rows));
		if (this->selected.has_value()) this->ScrollTo(*this->selected);
		this->Refresh();
		/* The rows moved up under the pointer: the next click needs a moment, so a double click decides one row. */
		this->moved_at = std::chrono::steady_clock::now();
	}

	/**
	 * Make a standing rule through the console's own command, unless an allow rule is not the owner's
	 * own act. An allow rule is confirmed first, with how many waiting approvals it decides at once.
	 */
	void MakeRule(Rules::Effect effect, const Rules::Spec &spec, bool confirmed = false)
	{
		if (auto refused = RuleRefusal(effect, spec); refused.has_value()) {
			IConsolePrint(CC_ERROR, "Prismatic control: {}", *refused);
			return;
		}
		if (effect == Rules::Effect::Allow && !confirmed) {
			const size_t now = Selected(Snapshot(), spec, true).size(); // an allow rule never approves an expired one
			this->pending_rule = std::make_pair(effect, spec);
			ShowQuery(GetEncodedString(STR_JUST_RAW_STRING, fmt::format("Always allow {}?", spec.tool)),
				GetEncodedString(STR_JUST_RAW_STRING, fmt::format("{}{}: it approves {} waiting approval{} now and every new one it matches, until you remove it.",
					spec.tool, spec.match.is_null() ? " with any arguments" : " matching " + spec.match.dump(), now, now == 1 ? "" : "s")),
				this, &ControlWindow::RuleConfirmed);
			return;
		}
		IConsoleCmdExec(Rules::RuleCommand(effect, spec.tool, spec.match, spec.persist, "window"));
		this->Refresh();
	}

	/** Make the allow rule Always... named, once the owner confirmed it. */
	static void RuleConfirmed(Window *w, bool confirmed)
	{
		auto *cw = dynamic_cast<ControlWindow *>(w);
		if (cw == nullptr || !confirmed || !cw->pending_rule.has_value()) return;
		auto [effect, spec] = *cw->pending_rule;
		cw->pending_rule.reset();
		cw->MakeRule(effect, spec, true);
	}

	/**
	 * Remove rule \p id, once confirmed. Removing a deny rule lets an allow rule decide what it held
	 * back, so it is refused to injected input like an approval.
	 */
	void RemoveRule(uint64_t id, bool deny, bool confirmed)
	{
		if (auto refused = ConsentRefusal(deny); refused.has_value()) {
			IConsolePrint(CC_ERROR, "Prismatic control: {}", *refused);
			return;
		}
		if (!confirmed) {
			this->pending_remove = id;
			ShowQuery(GetEncodedString(STR_JUST_RAW_STRING, fmt::format("Remove rule #{}?", id)),
				GetEncodedString(STR_JUST_RAW_STRING, deny ? std::string{"What it denies then waits for you, or an allow rule decides it."}
					: std::string{"What it allows then waits for you again."}),
				this, &ControlWindow::RemoveConfirmed);
			return;
		}
		IConsoleCmdExec(fmt::format("prismatic_mcp rule remove {}", id));
		this->Refresh();
	}

	/** Remove the rule Remove named, once the owner confirmed it. */
	static void RemoveConfirmed(Window *w, bool confirmed)
	{
		auto *cw = dynamic_cast<ControlWindow *>(w);
		if (cw == nullptr || !confirmed || !cw->pending_remove.has_value()) return;
		const uint64_t id = *cw->pending_remove;
		cw->pending_remove.reset();
		bool deny = false;
		for (const Row &r : cw->rows) if (r.kind == Row::Kind::Rule && r.id == id) deny = !r.flag;
		cw->RemoveRule(id, deny, true);
	}

	/** Check every shown approval between the anchor and \p id, both included. */
	void CheckRange(uint64_t id)
	{
		const std::vector<uint64_t> shown = Shown(this->rows);
		auto a = std::ranges::find(shown, this->anchor.value_or(id)), b = std::ranges::find(shown, id);
		if (a == shown.end() || b == shown.end()) return;
		if (a > b) std::swap(a, b);
		for (auto it = a; it <= b; ++it) this->view.checked.insert(*it);
	}

	/** Move the selection by \p delta among the shown approvals. */
	void Move(int delta)
	{
		const std::vector<uint64_t> shown = Shown(this->rows);
		if (shown.empty()) return;
		auto it = this->selected.has_value() ? std::ranges::find(shown, *this->selected) : shown.end();
		int at = it == shown.end() ? (delta > 0 ? -1 : static_cast<int>(shown.size())) : static_cast<int>(it - shown.begin());
		at = std::clamp(at + delta, 0, static_cast<int>(shown.size()) - 1);
		this->selected = shown[at];
		this->ScrollTo(shown[at]);
		this->SetDirty();
	}

	/** Open the Always... menu of approval row \p id under its button. */
	void ShowAlways(const Row &row, const Rect &button)
	{
		this->always_for = row.id;
		this->always_injected = UI::InjectingInput();
		const bool masked = OnJoinedNetworkClient(); // the rules live on the game server
		DropDownList list;
		list.push_back(MakeDropDownListStringItem(fmt::format("Always allow {} (any arguments)", row.tool), AI_ALLOW_TOOL, masked));
		list.push_back(MakeDropDownListStringItem(fmt::format("Always allow {} matching...", row.tool), AI_ALLOW_EDIT, masked));
		list.push_back(MakeDropDownListStringItem(fmt::format("Always deny {} (any arguments)", row.tool), AI_DENY_TOOL, masked));
		list.push_back(MakeDropDownListStringItem(fmt::format("Always deny {} matching...", row.tool), AI_DENY_EDIT, masked));
		ShowDropDownListAt(this, std::move(list), -1, WID_PC_LIST, button, Colours::Grey);
	}

	void OnClickList(Point pt, int click_count)
	{
		auto it = this->vscroll->GetScrolledItemFromWidget(this->rows, pt.y, this, WID_PC_LIST, WidgetDimensions::scaled.framerect.top);
		if (it == this->rows.end()) return;
		const Row row = *it;
		const Rect ir = this->GetWidget<NWidgetBase>(WID_PC_LIST)->GetCurrentRect().Shrink(WidgetDimensions::scaled.framerect);
		const int button = ButtonWidth(), always = AlwaysWidth();
		/* A row that slid under the pointer by itself is not what the owner aimed at. */
		const bool moved = Within(this->moved_at, RECENTLY);
		switch (row.kind) {
			case Row::Kind::Approval: {
				const RowPart part = HitPart(ir.left, ir.right, pt.x, CheckWidth(), button, always);
				if (part != RowPart::Text && part != RowPart::Check && (moved || row.sent)) return;
				/* The second click of a double click lands on the row that moved under it: it decides nothing. */
				if (part != RowPart::Text && part != RowPart::Check && click_count > 1) return;
				switch (part) {
					case RowPart::Check:
						if (IsShiftPressed() && this->anchor.has_value()) {
							this->CheckRange(row.id);
						} else if (!this->view.checked.erase(row.id)) {
							this->view.checked.insert(row.id);
						}
						this->anchor = row.id;
						break;
					case RowPart::Text:
						this->selected = row.id;
						if (IsShiftPressed()) this->CheckRange(row.id);
						this->anchor = row.id;
						break;
					case RowPart::Approve:
						this->selected = row.id;
						if (!row.flag && row.decidable) this->Consent(true, {row.id});
						return;
					case RowPart::Deny:
						this->selected = row.id;
						this->Consent(false, {row.id});
						return;
					case RowPart::Always: {
						this->selected = row.id;
						const int top = pt.y - (pt.y - ir.top) % LineHeight();
						this->ShowAlways(row, Rect{ir.right - always + 1, top, ir.right, top + LineHeight() - 1});
						break;
					}
				}
				this->Refresh();
				break;
			}
			case Row::Kind::Rule:
				if (pt.x >= ir.right - always + 1 && !moved) this->RemoveRule(row.id, !row.flag, false);
				break;
			case Row::Kind::Switch:
				Menu::Run(static_cast<int>(row.id));
				this->Refresh();
				break;
			default:
				break;
		}
	}

	/** Approve what Approve selected/all named, once the owner confirmed it. */
	static void ApproveConfirmed(Window *w, bool confirmed)
	{
		auto *cw = dynamic_cast<ControlWindow *>(w);
		if (cw == nullptr || !confirmed) return;
		std::vector<uint64_t> ids = std::move(cw->pending_bulk);
		cw->pending_bulk.clear();
		cw->Consent(true, ids);
	}

	void OnClick(Point pt, WidgetID widget, int click_count) override
	{
		this->keys_armed = true;
		switch (widget) {
			case WID_PC_LIST: this->OnClickList(pt, click_count); break;
			case WID_PC_TOOL: {
				this->tool_choices = ToolNames(Snapshot()); // what waits now, not a second ago
				DropDownList list;
				list.push_back(MakeDropDownListStringItem(std::string{"All tools"}, -1));
				for (size_t i = 0; i < this->tool_choices.size(); i++) list.push_back(MakeDropDownListStringItem(std::string{this->tool_choices[i]}, static_cast<int>(i)));
				auto at = std::ranges::find(this->tool_choices, this->view.tool);
				ShowDropDownList(this, std::move(list), this->view.tool.empty() || at == this->tool_choices.end() ? -1 : static_cast<int>(at - this->tool_choices.begin()), WID_PC_TOOL);
				break;
			}
			case WID_PC_EXPIRED: this->view.hide_expired = !this->view.hide_expired; this->Refresh(); break;
			case WID_PC_CHECK_SHOWN: for (uint64_t id : Shown(this->rows)) this->view.checked.insert(id); this->Refresh(); break;
			case WID_PC_CHECK_NONE: this->view.checked.clear(); this->Refresh(); break;
			case WID_PC_APPROVE_ALL: {
				if (Within(this->moved_at, RECENTLY)) break; // the rows it would take just changed
				std::vector<uint64_t> ids = BulkTargets(this->rows, true);
				if (ids.empty()) break;
				if (auto refused = ConsentRefusal(true); refused.has_value()) {
					IConsolePrint(CC_ERROR, "Prismatic control: {}", *refused);
					break;
				}
				std::string list;
				for (size_t i = 0; i < ids.size() && i < 12; i++) {
					const Row *r = this->Find(ids[i]);
					list += fmt::format("{}#{} {}", i == 0 ? "" : ", ", ids[i], r != nullptr ? r->tool : "");
				}
				if (ids.size() > 12) list += fmt::format(" and {} more", ids.size() - 12);
				this->pending_bulk = ids;
				ShowQuery(GetEncodedString(STR_JUST_RAW_STRING, fmt::format("Approve {} approval{}?", ids.size(), ids.size() == 1 ? "" : "s")),
					GetEncodedString(STR_JUST_RAW_STRING, fmt::format("Each executes at once: {}.", list)), this, &ControlWindow::ApproveConfirmed);
				break;
			}
			case WID_PC_DENY_ALL:
				if (!Within(this->moved_at, RECENTLY)) this->Consent(false, BulkTargets(this->rows, false)); // not while the rows just changed
				break;
			case WID_PC_GOTO: {
				auto target = this->GotoTarget();
				if (!target.has_value()) break;
				if (*target >= 0) ScrollMainWindowToTile(TileIndex{static_cast<uint32_t>(*target)});
				else ShowCompany(CompanyID(static_cast<int>(-1 - *target)));
				break;
			}
			case WID_PC_ASK:
				this->query = QueryFor::Ask;
				ShowQueryString({}, STR_PRISMATIC_MCP_ASK_CAPTION, 256, this, CS_ALPHANUMERAL, {});
				break;
			case WID_PC_HUB: OpenBrowser(Menu::HubUrl() + "/"); break;
			case WID_PC_CONSOLE: if (_iconsole_mode == IConsoleMode::Closed) IConsoleSwitch(); break;
		}
	}

	void OnDropdownSelect(WidgetID widget, int index, [[maybe_unused]] int click_result) override
	{
		if (widget == WID_PC_TOOL) {
			this->view.tool = index >= 0 && static_cast<size_t>(index) < this->tool_choices.size() ? this->tool_choices[index] : std::string{};
			this->Refresh();
			return;
		}
		if (widget != WID_PC_LIST) return;
		const Row *row = this->Find(this->always_for);
		if (row == nullptr) return;
		/* The menu's choice comes frames after the click that opened it: a menu an injected click
		 * opened never makes an allow rule, whoever lets go over it. */
		if ((index == AI_ALLOW_TOOL || index == AI_ALLOW_EDIT) && this->always_injected) {
			IConsolePrint(CC_ERROR, "Prismatic control: {}", ConsentRefusal(true).value_or(
				"an approval needs the owner's own click; this menu was opened by input injected through a UI tool"));
			return;
		}
		Rules::Spec spec;
		spec.tool = row->tool;
		switch (index) {
			case AI_ALLOW_TOOL: this->MakeRule(Rules::Effect::Allow, spec); break;
			case AI_DENY_TOOL: this->MakeRule(Rules::Effect::Deny, spec); break;
			case AI_ALLOW_EDIT:
			case AI_DENY_EDIT: {
				this->query = index == AI_ALLOW_EDIT ? QueryFor::AllowRule : QueryFor::DenyRule;
				std::string prefill = row->tool + " " + Rules::MatcherFromArgs(json::parse(row->args, nullptr, false)).dump();
				ShowQueryString(prefill, index == AI_ALLOW_EDIT ? STR_PRISMATIC_MCP_RULE_ALLOW_CAPTION : STR_PRISMATIC_MCP_RULE_DENY_CAPTION, 1024, this, CS_ALPHANUMERAL, {});
				break;
			}
		}
	}

	void OnQueryTextFinished(std::optional<std::string> str) override
	{
		if (!str.has_value() || str->empty()) return;
		if (this->query == QueryFor::Ask) {
			Menu::Ask(*str);
			this->Refresh();
			return;
		}
		/* "<tool glob> [<json matcher>] [--persist]", read as the console reads a rule. */
		std::vector<std::string> words;
		for (size_t start = 0; start < str->size();) {
			size_t end = std::min(str->find(' ', start), str->size());
			if (end > start) words.push_back(str->substr(start, end - start));
			start = end + 1;
		}
		std::vector<std::string_view> tokens(words.begin(), words.end());
		Rules::Spec spec = Rules::ParseSpec(tokens);
		if (!spec.error.empty()) {
			IConsolePrint(CC_ERROR, "Prismatic control: {}", spec.error);
			return;
		}
		this->MakeRule(this->query == QueryFor::AllowRule ? Rules::Effect::Allow : Rules::Effect::Deny, spec);
	}

	EventState OnKeyPress([[maybe_unused]] char32_t key, uint16_t keycode) override
	{
		/* Only while the owner works in this window, after a click in it: its letters must not take the
		 * toolbar's hotkeys, and a window that opened by itself (a new approval on a client) must not
		 * decide on a key the owner meant for the game. */
		if (_focused_window != this || !this->keys_armed) return EventState::NotHandled;
		/* A held key repeats: one press decides one row. Enter is the chat key in a network game, so it decides nothing. */
		const bool decisive = keycode == 'A' || keycode == 'Y' || keycode == 'D' || keycode == 'N';
		if (decisive && !KeyTaken(this->keys, keycode, std::chrono::steady_clock::now())) return EventState::Handled;
		/* A selection the owner did not make (the selected row went by itself) is not decided by a key at once. */
		if (decisive && Within(this->auto_selected_at, RECENTLY)) return EventState::Handled;
		switch (keycode) {
			case WKC_UP: case 'K': this->Move(-1); return EventState::Handled;
			case WKC_DOWN: case 'J': this->Move(1); return EventState::Handled;
			case 'A': case 'Y': {
				const Row *r = this->Find(this->selected);
				if (r != nullptr && !r->flag && r->decidable && !r->sent) this->Consent(true, {r->id});
				return EventState::Handled;
			}
			case 'D': case 'N':
				if (const Row *r = this->Find(this->selected); r != nullptr && !r->sent) this->Consent(false, {r->id});
				return EventState::Handled;
			case WKC_SPACE:
				if (this->selected.has_value() && !this->view.checked.erase(*this->selected)) this->view.checked.insert(*this->selected);
				this->anchor = this->selected;
				this->Refresh();
				return EventState::Handled;
		}
		return EventState::NotHandled;
	}

	void OnFocusLost(bool closing) override
	{
		this->keys_armed = false;
		this->Window::OnFocusLost(closing);
	}

	void OnResize() override
	{
		this->vscroll->SetCapacityFromWidget(this, WID_PC_LIST, WidgetDimensions::scaled.framerect.Vertical());
	}

	/** Approvals arrive and expire, decisions run, answers come back: read it all again every second. */
	const IntervalTimer<TimerWindow> refresh_interval = {std::chrono::seconds(1), [this](auto) {
		this->Refresh(false);
	}};
};

constexpr std::initializer_list<NWidgetPart> _nested_control_widgets = {
	NWidget(NWID_HORIZONTAL),
		NWidget(WWT_CLOSEBOX, Colours::Grey),
		NWidget(WWT_CAPTION, Colours::Grey, WID_PC_CAPTION),
		NWidget(WWT_SHADEBOX, Colours::Grey),
		NWidget(WWT_DEFSIZEBOX, Colours::Grey),
		NWidget(WWT_STICKYBOX, Colours::Grey),
	EndContainer(),
	NWidget(NWID_HORIZONTAL),
		NWidget(WWT_DROPDOWN, Colours::Grey, WID_PC_TOOL), SetFill(1, 0), SetResize(1, 0),
		NWidget(WWT_TEXTBTN, Colours::Grey, WID_PC_EXPIRED), SetFill(0, 0),
		NWidget(WWT_PUSHTXTBTN, Colours::Grey, WID_PC_CHECK_SHOWN), SetFill(0, 0),
		NWidget(WWT_PUSHTXTBTN, Colours::Grey, WID_PC_CHECK_NONE), SetFill(0, 0),
	EndContainer(),
	NWidget(NWID_HORIZONTAL),
		NWidget(WWT_PANEL, Colours::Grey, WID_PC_LIST), SetScrollbar(WID_PC_SCROLLBAR), SetResize(1, 1), SetFill(1, 1),
		EndContainer(),
		NWidget(NWID_VSCROLLBAR, Colours::Grey, WID_PC_SCROLLBAR),
	EndContainer(),
	NWidget(WWT_PANEL, Colours::Grey, WID_PC_DETAILS), SetFill(1, 0), SetResize(1, 0),
	EndContainer(),
	NWidget(NWID_HORIZONTAL),
		NWidget(WWT_PUSHTXTBTN, Colours::Green, WID_PC_APPROVE_ALL), SetFill(0, 0),
		NWidget(WWT_PUSHTXTBTN, Colours::Red, WID_PC_DENY_ALL), SetFill(0, 0),
		NWidget(NWID_HORIZONTAL, NWidContainerFlag::EqualSize),
			NWidget(WWT_PUSHTXTBTN, Colours::Grey, WID_PC_GOTO), SetFill(1, 0), SetResize(1, 0),
			NWidget(WWT_PUSHTXTBTN, Colours::Grey, WID_PC_ASK), SetFill(1, 0), SetResize(1, 0),
			NWidget(WWT_PUSHTXTBTN, Colours::Grey, WID_PC_HUB), SetFill(1, 0), SetResize(1, 0),
			NWidget(WWT_PUSHTXTBTN, Colours::Grey, WID_PC_CONSOLE), SetFill(1, 0), SetResize(1, 0),
		EndContainer(),
		NWidget(WWT_RESIZEBOX, Colours::Grey),
	EndContainer(),
};

WindowDesc _control_desc(
	WindowPosition::Automatic, "prismatic_control", 600, 320,
	WindowClass::PrismaticControl, WindowClass::None,
	{},
	_nested_control_widgets
);

} // namespace

bool Show()
{
	if (_network_dedicated || FindWindowById(WindowClass::MainWindow, 0) == nullptr) return false;
	AllocateWindowDescFront<ControlWindow>(_control_desc, 0);
	return true;
}

} // namespace PrismaticMCP::Panel

#endif /* OTTD_PRISMATIC_MCP_SERVER */
