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

#include <optional>
#include <string>
#include <string_view>
#include <vector>

namespace PrismaticMCP::Panel {

/**
 * What the control window shows, as data: the server's status line, every approval still waiting
 * (id, tool, the exact arguments it binds to, whether it expired), the latest decisions with their
 * verdicts, the questions asked in this game, and the menu's switches and hub links. Built only
 * from GetStatusLine(), PendingApprovals(), DecisionTrace(), Menu::Requests() and Menu::Describe(),
 * so the window, the console and openttd.ui.control can never disagree.
 */
nlohmann::json Snapshot();

/** One line of the control window's list. */
struct Row {
	enum class Kind : uint8_t {
		Heading,  ///< A section's title.
		Text,     ///< A line to read.
		Approval, ///< An approval waiting: a click selects it for Approve or Deny.
		Switch,   ///< A menu switch: a click flips it.
	};
	Kind kind;         ///< What the line is.
	std::string text;  ///< What it says.
	uint64_t id = 0;   ///< Approval: its id. Switch: the menu item's index.
	bool flag = false; ///< Approval: expired. Switch: on.
};

/**
 * Every line the window lists for \p snapshot, in order: the server's status, the approvals, the
 * decisions, the questions, the switches. A snapshot with keys missing or of the wrong type (a
 * client's partial copy of a server's queue) gives the sections empty, never an exception.
 */
std::vector<Row> Rows(const nlohmann::json &snapshot);

/**
 * Why a click on Approve (\p approve true) or Deny in the control window is refused, or nullopt
 * when it may run: input injected by a UI tool (openttd.input.*, openttd.ui.press, ...) never approves for the owner. Denying,
 * which withdraws a request and changes nothing, stays open to anyone, as in the panel.
 */
std::optional<std::string> ConsentRefusal(bool approve);

/**
 * The console command a consent from the window runs: `prismatic_mcp approve <id>` or
 * `prismatic_mcp deny <id>`. The window goes through the console's own path, so wherever that path
 * leads (this process's queue, or the game server's) the window follows.
 */
std::string ConsentCommand(bool approve, uint64_t id);

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
