/*
 * This file is part of OpenTTD.
 * OpenTTD is free software; you can redistribute it and/or modify it under the terms of the GNU General Public License as published by the Free Software Foundation, version 2.
 * OpenTTD is distributed in the hope that it will be useful, but WITHOUT ANY WARRANTY; without even the implied warranty of MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.
 * See the GNU General Public License for more details. You should have received a copy of the GNU General Public License along with OpenTTD. If not, see <https://www.gnu.org/licenses/old-licenses/gpl-2.0>.
 */

/*
 * RESEARCH-ONLY / UNSTABLE. Compiled only with OPTION_PRISMATIC_MCP_SERVER (ADR 0008, ADR 0030).
 */

/** @file prismatic_mcp_menu.h The fork's own main-toolbar menu: one table drawn as the dropdown, described over MCP, run by both. */

#ifndef PRISMATIC_MCP_MENU_H
#define PRISMATIC_MCP_MENU_H

#ifdef OTTD_PRISMATIC_MCP_SERVER

#include "../3rdparty/nlohmann/json.hpp"
#include "../dropdown_type.h"

#include <optional>
#include <string>
#include <string_view>
#include <vector>

namespace PrismaticMCP::Menu {

/** What a menu item does. */
enum class Action : uint8_t {
	Console, ///< Run a console command and show the console.
	Browse,  ///< Open a URL of the hub in the browser.
	Ask,     ///< Ask a question in plain language (queued as a request of kind "ask" for any MCP client to answer).
	Request, ///< Queue a request of a fixed kind (arg) for the tile under the main view's centre, no text typed.
	Toggle,  ///< Call an MCP tool (arg) with {key: on|off}, flipping a switch the item shows as checked.
	Window,  ///< Open one of the fork's own windows (arg names it: "control").
};

/** One item of the menu. The table of these is the only place the menu is written down. */
struct Item {
	std::string_view label; ///< What the dropdown shows.
	Action action;          ///< What it does.
	std::string_view arg;   ///< The console command, the hub path to open, the request kind, or the tool to call.
	std::string_view key = {}; ///< Toggle only: the boolean argument the tool takes.
	std::string_view group = {}; ///< The submenu the item sits in; empty: the top level of the menu.
};

/**
 * Results of the dropdown rows that are not items: a group row opens its submenu in place, the
 * Back row returns to the top level. Item rows report their index into Items(), always below these.
 */
static constexpr int MENU_BACK_ROW = 999;
static constexpr int MENU_GROUP_ROW = 1000;

/** The items, in menu order. */
std::span<const Item> Items();

/** The hub's base URL: $PRISMATIC_HUB_URL, else the panel's (PanelUrl), which a co-play stack starts where it starts no hub. */
std::string HubUrl();

/**
 * The panel's base URL: $PRISMATIC_PANEL_URL, else http://127.0.0.1:8800 (the port mcp-e2e starts the
 * panel on), without a trailing slash.
 */
std::string PanelUrl();

/** Where a Browse item leads: a target "panel:<path>" is the panel's, any other the hub's. */
std::string BrowseUrl(std::string_view target);

/** The submenus in the order their first item appears in Items(); each holds at least one item. */
std::vector<std::string_view> Groups();

/** One row of the dropdown, before it is drawn: what it reports when chosen and what it shows. */
struct Row {
	int result;            ///< Items() index, MENU_GROUP_ROW + n for a group, MENU_BACK_ROW for Back.
	std::string label;     ///< The text shown.
	bool toggle = false;   ///< A Toggle item: drawn with a check box.
	bool checked = false;  ///< Its switch is on.
};

/**
 * The rows of the toolbar button's dropdown, from Items(): with no group, the top-level items and one
 * row per group (result MENU_GROUP_ROW + its place in Groups()); with a group, a Back row
 * (MENU_BACK_ROW) and that group's items. Pure data, so it is tested without a window or a font.
 */
std::vector<Row> Rows(std::string_view group = {});

/** The dropdown of the toolbar button: Rows(group), drawn. */
DropDownList DropDown(std::string_view group = {});

/** The group a dropdown result opens (MENU_GROUP_ROW + n), if it is a group row. */
std::optional<std::string_view> GroupOfRow(int result);

/**
 * Run item `index`, as a click on it does.
 * @return true when the item asks for text, so the caller shows the query window.
 */
bool Run(int index);

/** A question typed in the game: queued as a request until an MCP client answers it. */
uint64_t Ask(std::string_view text);

/**
 * Queue a request: {id, kind, text, date, tile, x, y}; the tile is the one under the main view's
 * centre (null without a main window).
 * @return The request's id.
 */
uint64_t Queue(std::string_view kind, std::string_view text);

/** openttd://ui/menu: every item with its action and target, and the hub URL. */
nlohmann::json Describe();

/** openttd://ui/requests: every request typed in the game, answered or not. */
nlohmann::json Requests();

/** The tools of this module: openttd.ui.control (in every process), openttd.ui.menu.run and openttd.ui.requests.answer (where there is a toolbar). */
nlohmann::json Tools();

/**
 * Call any published MCP tool in this process, the way tools/call does: the menu's Toggle items and
 * nothing else use it. Defined by the server (prismatic_mcp_server.cpp), which owns the dispatch.
 */
nlohmann::json CallTool(std::string_view name, const nlohmann::json &args);

/** Run one of Tools() by name; nullopt when the name is not this module's. */
std::optional<nlohmann::json> Call(std::string_view name, const nlohmann::json &args);

} // namespace PrismaticMCP::Menu

#endif /* OTTD_PRISMATIC_MCP_SERVER */

#endif /* PRISMATIC_MCP_MENU_H */
