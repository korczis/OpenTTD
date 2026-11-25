/*
 * This file is part of OpenTTD.
 * OpenTTD is free software; you can redistribute it and/or modify it under the terms of the GNU General Public License as published by the Free Software Foundation, version 2.
 * OpenTTD is distributed in the hope that it will be useful, but WITHOUT ANY WARRANTY; without even the implied warranty of MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.
 * See the GNU General Public License for more details. You should have received a copy of the GNU General Public License along with OpenTTD. If not, see <https://www.gnu.org/licenses/old-licenses/gpl-2.0>.
 */

/*
 * RESEARCH-ONLY / UNSTABLE. Compiled only with OPTION_PRISMATIC_MCP_SERVER (ADR 0008, ADR 0030).
 */

/** @file prismatic_mcp_panel.h The Prismatic control window: the fork's state in the game, read from the same functions the console and MCP read. */

#ifndef PRISMATIC_MCP_PANEL_H
#define PRISMATIC_MCP_PANEL_H

#ifdef OTTD_PRISMATIC_MCP_SERVER

#include "../3rdparty/nlohmann/json.hpp"
#include "prismatic_mcp_rules.h"

#include <chrono>
#include <optional>
#include <set>
#include <string>
#include <string_view>
#include <vector>

namespace PrismaticMCP::Panel {

/**
 * What the control window shows, as data: the server's status line, every approval still waiting
 * (id, tool, MCP tool, the exact arguments it binds to, whether it expired, the standing rule that
 * applies to it), the owner's standing rules with their hits, the latest decisions with their
 * verdicts and channels, the questions asked in this game, and the menu's switches and hub links.
 * Built only from GetStatusLine(), PendingApprovals(), ApprovalRules(), DecisionTrace(),
 * Menu::Requests() and Menu::Describe(), so the window, the console and openttd.ui.control can
 * never disagree. What the window has checked, filtered or selected is window state (View), not here.
 */
nlohmann::json Snapshot();

/** One line of the control window's list. */
struct Row {
	enum class Kind : uint8_t {
		Heading,  ///< A section's title.
		Text,     ///< A line to read.
		Approval, ///< An approval waiting: its own Approve, Deny and Always... buttons; a click selects it, its box checks it.
		Switch,   ///< A menu switch: a click flips it.
		Rule,     ///< A standing rule: its Remove button ends it.
	};
	Kind kind;            ///< What the line is.
	std::string text;     ///< What it says.
	uint64_t id = 0;      ///< Approval: its id. Switch: the menu item's index. Rule: its id.
	bool flag = false;    ///< Approval: expired. Switch: on. Rule: an allow rule.
	bool checked = false; ///< Approval: checked for Approve selected / Deny selected.
	bool decidable = true; ///< Approval: this window may decide it (false: the server's operator does).
	bool sent = false;     ///< Approval: decided here and sent to the server, which has not answered yet.
	std::string tool{};   ///< Approval: its tool, as the filter and "Always..." name it.
	std::string args{};   ///< Approval: its whole arguments, for the details area and the matcher.
};

/** The window's own state over the snapshot: the selectors and the checked rows. */
struct View {
	std::string tool;             ///< Show only approvals of this tool; empty: every tool.
	bool hide_expired = false;    ///< Hide approvals past their expiry.
	std::set<uint64_t> checked;   ///< Approvals checked for a bulk decision.
	std::set<uint64_t> sent;      ///< Approvals this window decided that a server has not yet stopped listing (a network client's queue refreshes every few seconds).
};

/**
 * Every line the window lists for \p snapshot as \p view selects it, in order: the server's
 * status, the approvals the selectors show (each marked checked as in the view), the standing rules,
 * the decisions, the questions, the switches. A snapshot with keys missing or of the wrong type (a
 * client's partial copy of a server's queue) gives the sections empty, never an exception.
 */
std::vector<Row> Rows(const nlohmann::json &snapshot, const View &view = {});

/** The distinct tools of the waiting approvals, sorted: the tool selector's choices after "All". */
std::vector<std::string> ToolNames(const nlohmann::json &snapshot);

/** The ids of the approval rows, in list order. */
std::vector<uint64_t> Shown(const std::vector<Row> &rows);

/**
 * What Approve selected / Deny selected (or, with nothing checked, Approve all / Deny all) acts on:
 * the checked approval rows shown, or every approval row shown when none is checked. A row already
 * sent to the server is left out; approving leaves out expired rows and those the server's
 * operator decides as well.
 */
std::vector<uint64_t> BulkTargets(const std::vector<Row> &rows, bool approve);

/** Whether any approval row shown is checked: the bulk buttons then say "selected". */
bool AnyChecked(const std::vector<Row> &rows);

/**
 * The ids in a snapshot's approvals that the tool glob and matcher of a spec select: what a network
 * client's 'prismatic_mcp approve all|deny all' sends to the server, one id per line, and how many
 * waiting approvals an allow rule decides at once.
 * @param snapshot The snapshot.
 * @param spec The glob and matcher.
 * @param skip_expired Leave out expired approvals, which no allow rule approves.
 * @return The ids, in queue order.
 */
std::vector<uint64_t> Selected(const nlohmann::json &snapshot, const Rules::Spec &spec, bool skip_expired = false);

/** The last decisive key the window saw, to tell a held key's repeats from a new press. */
struct KeyHistory {
	uint16_t key = 0;                          ///< The key.
	std::chrono::steady_clock::time_point at{}; ///< When it was last seen, taken or not.
};

/** How long the same key must rest before it decides again: longer than a keyboard's repeat interval, so a held key decides once. */
inline constexpr std::chrono::milliseconds KEY_QUIET{600};

/**
 * Whether a decisive key press is taken. The same key seen again within KEY_QUIET of its last
 * sighting is a repeat of a held key and is not taken; every sighting, taken or not, restarts that
 * time, so a key held through the keyboard's repeats decides exactly once.
 * @param history The last key seen; updated.
 * @param key The key pressed.
 * @param now When.
 * @return Whether it decides.
 */
bool KeyTaken(KeyHistory &history, uint16_t key, std::chrono::steady_clock::time_point now);

/**
 * The approval to select after \p decided were decided: the first row at or after the first decided
 * one in \p before that still waits in \p after (the next row under the cursor), else the last one
 * before it, else none.
 */
std::optional<uint64_t> NextSelection(const std::vector<uint64_t> &before, const std::set<uint64_t> &decided, const std::vector<uint64_t> &after);

/** One part of an approval row the mouse can hit. */
enum class RowPart : uint8_t { Check, Text, Approve, Deny, Always };

/**
 * Which part of an approval row spanning \p left .. \p right is at \p x: the check box (\p check
 * wide) at the left, the Approve, Deny (\p button wide each) and Always... (\p always wide) buttons at
 * the right end, the same x on every row, so the mouse stays put while the rows advance.
 */
RowPart HitPart(int left, int right, int x, int check, int button, int always);

/** The tile an approval's arguments name ("location" or "tile"), if any; checked against \p map_size. */
std::optional<uint32_t> TileOf(const nlohmann::json &args, uint64_t map_size);
/** The company an approval's arguments name ("company", 0-14), if any. */
std::optional<int> CompanyNamed(const nlohmann::json &args);

/**
 * Why a click on Approve (\p approve true), Approve selected/all or an allow rule in the control
 * window is refused, or nullopt when it may run: input injected by a UI tool (openttd.input.*,
 * openttd.ui.press, ...) never approves for the owner. Denying, which withdraws a request and
 * changes nothing, stays open to anyone, as in the panel.
 */
std::optional<std::string> ConsentRefusal(bool approve);

/**
 * Why the window will not make a standing rule, or nullopt when it may ask the owner to confirm it:
 * an allow rule from injected input (ConsentRefusal), or a broad allow, which the core refuses from
 * the window (Rules::BroadAllowRefusal with via "window"), said with the core's own words.
 * @param effect The rule's effect.
 * @param spec The tool glob and matcher, as Always... or its query gives them.
 * @return Why not; nullopt when it may.
 */
std::optional<std::string> RuleRefusal(Rules::Effect effect, const Rules::Spec &spec);

/**
 * The console command a consent from the window runs: `prismatic_mcp approve <id> [<id> ...] --via window`
 * or `prismatic_mcp deny ...`. The window goes through the console's own path, so wherever that path
 * leads (this process's queue, or the game server's) the window follows.
 */
std::string ConsentCommand(bool approve, uint64_t id);
/** The same for several ids at once: Approve selected / Approve all of the window. */
std::string ConsentCommand(bool approve, const std::vector<uint64_t> &ids);

/**
 * Open the control window, or bring it to the front.
 * @return false in a process without an interface (a dedicated server, a unit test).
 */
bool Show();

/**
 * A private chat line from the game server, on a network client. True when it is part of the
 * server's answer to '!approvals' that this client asked for itself (the window's quiet refresh):
 * the line then updates the server's queue the window shows and is not printed. Any other
 * "[approvals]" line updates the queue as well and is printed as usual (false).
 */
bool OnServerChat(std::string_view msg);

/** openttd.ui.control: the descriptor of the read-only tool that answers Snapshot(). */
nlohmann::json Tool();

} // namespace PrismaticMCP::Panel

#endif /* OTTD_PRISMATIC_MCP_SERVER */

#endif /* PRISMATIC_MCP_PANEL_H */
