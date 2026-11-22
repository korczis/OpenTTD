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

namespace PrismaticMCP::Panel {

/**
 * What the control window shows, as data: the server's status line, every approval still waiting
 * (id, tool, the exact arguments it binds to, whether it expired), the latest decisions with their
 * verdicts, the questions asked in this game, and the menu's switches and hub links. Built only
 * from GetStatusLine(), PendingApprovals(), DecisionTrace(), Menu::Requests() and Menu::Describe(),
 * so the window, the console and openttd.ui.control can never disagree.
 */
nlohmann::json Snapshot();

/**
 * Why a click on Approve (\p approve true) or Deny in the control window is refused, or nullopt
 * when it may run: input injected by an openttd.input.* tool never approves for the owner. Denying,
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

/** openttd.ui.control: the descriptor of the read-only tool that answers Snapshot(). */
nlohmann::json Tool();

} // namespace PrismaticMCP::Panel

#endif /* OTTD_PRISMATIC_MCP_SERVER */

#endif /* PRISMATIC_MCP_PANEL_H */
