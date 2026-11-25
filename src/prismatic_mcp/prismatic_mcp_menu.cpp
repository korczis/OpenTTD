/*
 * This file is part of OpenTTD.
 * OpenTTD is free software; you can redistribute it and/or modify it under the terms of the GNU General Public License as published by the Free Software Foundation, version 2.
 * OpenTTD is distributed in the hope that it will be useful, but WITHOUT ANY WARRANTY; without even the implied warranty of MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.
 * See the GNU General Public License for more details. You should have received a copy of the GNU General Public License along with OpenTTD. If not, see <https://www.gnu.org/licenses/old-licenses/gpl-2.0>.
 */

/*
 * RESEARCH-ONLY / UNSTABLE. Compiled only with OPTION_PRISMATIC_MCP_SERVER (ADR 0008, ADR 0030).
 *
 * The fork's own main-toolbar menu. One table (Items) is the menu: the toolbar draws it as its
 * dropdown, openttd://ui/menu describes it, openttd.ui.menu.run runs an item as a click does. A
 * question typed through "Ask the planner...", and every Request item (a fixed kind such as
 * serve_here, with the tile under the main view's centre), becomes a request in openttd://ui/requests;
 * any MCP client (the hub, a Rhai script, mcp-human, another AI) answers it with
 * openttd.ui.requests.answer, and the answer is shown in this game as a news message. A Toggle item
 * calls an MCP tool with a boolean it flips. Nothing here changes the game: a console command
 * runs as typed, a URL opens in the browser, a request and its answer are local to this process.
 */

/** @file prismatic_mcp_menu.cpp The fork's own main-toolbar menu: one table drawn as the dropdown, described over MCP, run by both. */

#include "../stdafx.h"

#ifdef OTTD_PRISMATIC_MCP_SERVER

#include "prismatic_mcp_menu.h"
#include "prismatic_mcp_panel.h"
#include "prismatic_mcp_view.h"

#include "../console_func.h"
#include "../console_gui.h"
#include "../dropdown_func.h"
#include "../dropdown_common_type.h"
#include "../map_func.h"
#include "../network/network.h"
#include "../openttd.h"
#include "../timer/timer_game_calendar.h"
#include "../viewport_func.h"
#include "../window_func.h"
#include "../window_gui.h"

#include <algorithm>
#include <array>
#include <span>
#include <cstdlib>
#include <deque>

#include "../safeguards.h"

using json = nlohmann::json;

namespace PrismaticMCP::Menu {

/* The only place the menu is written down: the dropdown, openttd://ui/menu, the tests and the menu
 * table of research/prismatic-mcp/README.md (tools/research/prismatic-mcp-menu) all derive from it.
 * An item with a group sits in that submenu; the top level lists the other items and one row per
 * group, in the order the groups first appear here. */
static constexpr std::array<Item, 15> ITEMS = {{
	{"Prismatic control...", Action::Window, "control"},
	{"Ask the planner...", Action::Ask, "ask"},
	/* Rows of the human-like player (tools/research/mcp-human, openttd-07): it answers these kinds. */
	{"Serve the towns here", Action::Request, "serve_here", {}, "Requests here"},
	{"Plan a rail line here", Action::Request, "plan_rail", {}, "Requests here"},
	{"Watch my buses", Action::Request, "watch", {}, "Requests here"},
	{"Human player: paused", Action::Toggle, "openttd.input.yield", "paused", "Human player"},
	{"MCP server: status", Action::Console, "prismatic_mcp status", {}, "MCP server"},
	{"MCP server: start", Action::Console, "prismatic_mcp start", {}, "MCP server"},
	{"MCP approvals waiting", Action::Console, "prismatic_mcp approvals", {}, "MCP server"},
	{"MCP decisions", Action::Console, "prismatic_mcp decisions", {}, "MCP server"},
	{"Open the hub", Action::Browse, "/", {}, "Hub and API"},
	{"World view", Action::Browse, "/viz", {}, "Hub and API"},
	{"API (Swagger)", Action::Browse, "/docs", {}, "Hub and API"},
	{"GraphQL schema", Action::Browse, "/graphql/schema", {}, "Hub and API"},
	{"Open approvals in the browser", Action::Browse, "panel:/#approvals", {}, "Hub and API"},
}};

std::span<const Item> Items()
{
	return ITEMS;
}

std::string HubUrl()
{
	/* A co-play stack (mcp-e2e) starts the panel and no hub: without PRISMATIC_HUB_URL the hub's
	 * pages lead to the panel, which answers them, rather than to a port where nothing listens. */
	const char *env = std::getenv("PRISMATIC_HUB_URL");
	if (env == nullptr || *env == '\0') return PanelUrl();
	std::string url = env;
	while (!url.empty() && url.back() == '/') url.pop_back();
	return url;
}

std::string PanelUrl()
{
	const char *env = std::getenv("PRISMATIC_PANEL_URL");
	std::string url = (env != nullptr && *env != '\0') ? env : "http://127.0.0.1:8800";
	while (!url.empty() && url.back() == '/') url.pop_back();
	return url;
}

std::string BrowseUrl(std::string_view target)
{
	if (target.starts_with("panel:")) return PanelUrl() + std::string{target.substr(6)};
	return HubUrl() + std::string{target};
}

/** The switches of the Toggle items, by index; off until flipped. */
static std::array<bool, ITEMS.size()> _toggled{};

std::vector<std::string_view> Groups()
{
	std::vector<std::string_view> groups;
	for (const Item &item : ITEMS) {
		if (item.group.empty() || std::find(groups.begin(), groups.end(), item.group) != groups.end()) continue;
		groups.push_back(item.group);
	}
	return groups;
}

std::optional<std::string_view> GroupOfRow(int result)
{
	auto groups = Groups();
	if (result < MENU_GROUP_ROW || static_cast<size_t>(result - MENU_GROUP_ROW) >= groups.size()) return std::nullopt;
	return groups[result - MENU_GROUP_ROW];
}

std::vector<Row> Rows(std::string_view group)
{
	std::vector<Row> rows;
	/* A submenu opens in place of the top level (the toolbar replaces the list), with a way back. */
	if (!group.empty()) rows.push_back({MENU_BACK_ROW, "< Back"});
	auto groups = Groups();
	std::vector<bool> listed(groups.size(), false);
	for (size_t i = 0; i < ITEMS.size(); i++) {
		int result = static_cast<int>(i);
		if (ITEMS[i].group != group) {
			/* At the top level, a group's row stands where its first item would. */
			if (!group.empty() || ITEMS[i].group.empty()) continue;
			size_t g = std::find(groups.begin(), groups.end(), ITEMS[i].group) - groups.begin();
			if (listed[g]) continue;
			listed[g] = true;
			rows.push_back({MENU_GROUP_ROW + static_cast<int>(g), fmt::format("{} >", ITEMS[i].group)});
			continue;
		}
		if (ITEMS[i].action == Action::Toggle) {
			rows.push_back({result, std::string{ITEMS[i].label}, true, _toggled[i]});
		} else if (ITEMS[i].action == Action::Window) {
			/* The window's row says, when the menu opens, how many approvals wait for the owner. */
			size_t waiting = Panel::Snapshot()["approvals"].size();
			std::string label{ITEMS[i].label};
			if (waiting > 0) label = fmt::format("{} ({} waiting)", label.substr(0, label.find("...")), waiting);
			rows.push_back({result, std::move(label)});
		} else {
			rows.push_back({result, std::string{ITEMS[i].label}});
		}
	}
	return rows;
}

DropDownList DropDown(std::string_view group)
{
	DropDownList list;
	for (Row &row : Rows(group)) {
		if (row.toggle) {
			list.push_back(std::make_unique<DropDownListCheckedItem>(0, row.checked, std::move(row.label), row.result));
		} else {
			list.push_back(MakeDropDownListStringItem(std::move(row.label), row.result));
		}
	}
	return list;
}

/** Flip the switch of Toggle item `index` through its tool; the switch moves only when the tool says ok. */
static json Flip(size_t index)
{
	const Item &item = ITEMS[index];
	json result = CallTool(item.arg, {{std::string{item.key}, !_toggled[index]}});
	bool ok = !result.value("isError", true);
	if (ok) _toggled[index] = !_toggled[index];
	IConsolePrint(ok ? CC_INFO : CC_ERROR, "{}: {}", item.label, ok ? (_toggled[index] ? "on" : "off") : result.dump());
	return json{{"ok", ok}, {"item", item.label}, {"on", _toggled[index]}, {"result", result}};
}

static std::string_view ActionName(Action a)
{
	switch (a) {
		case Action::Console: return "console";
		case Action::Browse: return "browse";
		case Action::Ask: return "ask";
		case Action::Request: return "request";
		case Action::Toggle: return "toggle";
		case Action::Window: return "window";
	}
	return "unknown";
}

bool Run(int index)
{
	if (index < 0 || static_cast<size_t>(index) >= ITEMS.size()) return false;
	const Item &item = ITEMS[index];
	switch (item.action) {
		case Action::Console:
			IConsoleCmdExec(item.arg);
			if (_iconsole_mode == IConsoleMode::Closed) IConsoleSwitch();
			return false;
		case Action::Browse:
			OpenBrowser(BrowseUrl(item.arg));
			return false;
		case Action::Ask:
			return true;
		case Action::Request:
			Queue(item.arg, "");
			return false;
		case Action::Toggle:
			Flip(index);
			return false;
		case Action::Window:
			Panel::Show();
			return false;
	}
	return false;
}

/** One question typed in the game, and its answer once an MCP client gave one. */
struct Request {
	uint64_t id = 0;
	std::string kind;
	std::string text;
	std::optional<TileIndex> tile;
	std::string date;
	std::optional<std::string> answer;
};

static std::deque<Request> _requests;
static uint64_t _next_request = 1;
static constexpr size_t MAX_REQUESTS = 64;

/** The tile under the main view's centre, if there is a main window. */
static std::optional<TileIndex> ViewCentreTile()
{
	const Window *w = FindWindowById(WindowClass::MainWindow, 0); // not GetMainWindow: that asserts one exists
	if (w == nullptr || w->viewport == nullptr) return std::nullopt;
	const Viewport &vp = *w->viewport;
	Point pt = TranslateXYToTileCoord(vp, vp.left + vp.width / 2, vp.top + vp.height / 2);
	if (pt.x < 0 || pt.y < 0) return std::nullopt;
	return TileVirtXY(pt.x, pt.y);
}

uint64_t Queue(std::string_view kind, std::string_view text)
{
	if (_requests.size() >= MAX_REQUESTS) _requests.pop_front();
	_requests.push_back({_next_request++, std::string{kind}, std::string{text}, ViewCentreTile(),
		fmt::format("{}", TimerGameCalendar::date.base()), std::nullopt});
	const Request &r = _requests.back();
	IConsolePrint(CC_INFO, "Request {} ({}{}{}): an MCP client answers it through openttd.ui.requests.answer", r.id, r.kind,
		text.empty() ? "" : ": ", text);
	return r.id;
}

uint64_t Ask(std::string_view text)
{
	return text.empty() ? 0 : Queue("ask", text);
}

json Describe()
{
	/* A dedicated server has no toolbar: nothing of the menu exists there. */
	if (_network_dedicated) return {{"items", json::array()}, {"hub", HubUrl()}, {"dedicated", true}};
	json items = json::array();
	for (size_t i = 0; i < ITEMS.size(); i++) {
		json target = ITEMS[i].action == Action::Browse ? json(BrowseUrl(ITEMS[i].arg)) : json(std::string{ITEMS[i].arg});
		json item = {{"index", i}, {"label", ITEMS[i].label}, {"action", ActionName(ITEMS[i].action)}, {"target", target},
			{"group", ITEMS[i].group.empty() ? json(nullptr) : json(std::string{ITEMS[i].group})}};
		if (ITEMS[i].action == Action::Toggle) { item["key"] = ITEMS[i].key; item["on"] = _toggled[i]; }
		items.push_back(std::move(item));
	}
	json groups = json::array();
	for (std::string_view g : Groups()) groups.push_back(std::string{g});
	return {{"items", std::move(items)}, {"groups", std::move(groups)}, {"hub", HubUrl()}};
}

json Requests()
{
	if (_network_dedicated) return {{"requests", json::array()}};
	json list = json::array();
	for (const Request &r : _requests) {
		json tile = nullptr, x = nullptr, y = nullptr;
		if (r.tile.has_value()) { tile = r.tile->base(); x = TileX(*r.tile); y = TileY(*r.tile); }
		list.push_back({{"id", r.id}, {"kind", r.kind}, {"text", r.text}, {"tile", tile}, {"x", x}, {"y", y}, {"date", r.date}, {"answered", r.answer.has_value()},
			{"answer", r.answer.has_value() ? json(*r.answer) : json(nullptr)}});
	}
	return {{"requests", std::move(list)}};
}

json Tools()
{
	json arr = json::array();
	/* What the control window shows is data in every process: a dedicated server holds the approvals. */
	arr.push_back(Panel::Tool());
	/* Each instance's catalogue is what it can do: a dedicated server has no toolbar to run. */
	if (_network_dedicated) return arr;
	arr.push_back({{"name", "openttd.ui.menu.run"},
		{"description", "Run an item of the fork's toolbar menu (openttd://ui/menu) as a click on it does: a console command, a hub page in the browser, a fork window (the Prismatic control window), or with text for the Ask item, a request. Local to this process"},
		{"inputSchema", {{"type", "object"}, {"required", json::array({"index"})},
			{"properties", {{"index", {{"type", "integer"}}}, {"text", {{"type", "string"}}}}}}},
		/* Not read-only: a row runs a console command, opens a browser, queues a request or flips a switch. */
		{"annotations", {{"readOnlyHint", false}, {"idempotentHint", false}}}});
	arr.push_back({{"name", "openttd.ui.requests.answer"},
		{"description", "Answer a question the player asked in the game (openttd://ui/requests): the answer is recorded and shown in this game as a news message. Local to this process"},
		{"inputSchema", {{"type", "object"}, {"required", json::array({"id", "answer"})},
			{"properties", {{"id", {{"type", "integer"}}}, {"answer", {{"type", "string"}}}}}}},
		{"annotations", {{"readOnlyHint", false}, {"idempotentHint", false}}}});
	return arr;
}

std::optional<json> Call(std::string_view name, const json &args)
{
	if (name == "openttd.ui.control") {
		json out = Panel::Snapshot();
		out["ok"] = true;
		return out;
	}
	if (_network_dedicated && (name == "openttd.ui.menu.run" || name == "openttd.ui.requests.answer")) {
		return json{{"ok", false}, {"error", "no toolbar on a dedicated server"}};
	}
	if (name == "openttd.ui.menu.run") {
		int index = args.value("index", -1);
		if (index < 0 || static_cast<size_t>(index) >= ITEMS.size()) return json{{"ok", false}, {"error", "no such menu item; see openttd://ui/menu"}};
		switch (ITEMS[index].action) {
			case Action::Ask: {
				std::string text = args.value("text", "");
				if (text.empty()) return json{{"ok", false}, {"error", "the Ask item needs text"}};
				return json{{"ok", true}, {"request", Queue("ask", text)}};
			}
			case Action::Request:
				return json{{"ok", true}, {"request", Queue(ITEMS[index].arg, args.value("text", ""))}};
			case Action::Toggle:
				return Flip(index);
			case Action::Window:
				if (!Panel::Show()) return json{{"ok", false}, {"error", "no interface in this process to open a window in"}};
				return json{{"ok", true}, {"opened", ITEMS[index].arg}};
			default:
				break;
		}
		Run(index);
		return json{{"ok", true}, {"ran", ITEMS[index].label}};
	}
	if (name == "openttd.ui.requests.answer") {
		uint64_t id = args.value("id", uint64_t{0});
		std::string answer = args.value("answer", "");
		if (answer.empty()) return json{{"ok", false}, {"error", "answer is required"}};
		for (Request &r : _requests) {
			if (r.id != id) continue;
			r.answer = answer;
			/* Shown the way every message from the fork reaches the player: an in-game news item. */
			/* A small news item needs a place: the tile the main view was centred on when the request was
			 * made. Without one (no main window then) it is a newspaper, which needs none. */
			json news = {{"text", fmt::format("{}\n{}", r.text, answer)}, {"style", r.tile.has_value() ? "small" : "normal"}};
			if (r.tile.has_value()) news["tile"] = r.tile->base();
			auto shown = View::Call("openttd.view.news.show", news);
			/* And in the console, where a request typed there (plan, ask, do) expects it. */
			IConsolePrint(CC_INFO, "Answer to request {} ({}):", r.id, r.text);
			for (size_t start = 0; start <= answer.size();) {
				size_t end = std::min(answer.find('\n', start), answer.size());
				IConsolePrint(CC_INFO, "  {}", std::string_view(answer).substr(start, end - start));
				start = end + 1;
			}
			return json{{"ok", true}, {"id", id}, {"shown", shown.has_value() && shown->value("ok", false)}};
		}
		return json{{"ok", false}, {"error", "no such request; see openttd://ui/requests"}};
	}
	return std::nullopt;
}

} // namespace PrismaticMCP::Menu

#endif /* OTTD_PRISMATIC_MCP_SERVER */
