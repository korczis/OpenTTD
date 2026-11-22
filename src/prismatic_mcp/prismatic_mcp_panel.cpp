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
 * Approve and Deny run the console's `prismatic_mcp approve|deny <id>`, so they consent exactly as
 * the owner typing it would, through whatever path that command takes. An Approve click injected by
 * an openttd.input.* tool (UI::InjectingInput) is refused: the window exists for the owner's hand,
 * and no script, bot or MCP client approves its own request by moving the owner's mouse. Deny, which
 * only withdraws a request, stays open to anyone, as it is in the panel.
 */

#include "../stdafx.h"

#ifdef OTTD_PRISMATIC_MCP_SERVER

#include "prismatic_mcp_panel.h"
#include "prismatic_mcp.h"
#include "prismatic_mcp_menu.h"
#include "prismatic_mcp_ui.h"

#include "../console_func.h"
#include "../console_gui.h"
#include "../gfx_func.h"
#include "../network/network.h"
#include "../openttd.h"
#include "../palette_func.h"
#include "../strings_func.h"
#include "../table/strings.h"
#include "../textbuf_gui.h"
#include "../timer/timer.h"
#include "../timer/timer_window.h"
#include "../widget_type.h"
#include "../window_func.h"
#include "../window_gui.h"
#include "../zoom_func.h"

#include <algorithm>
#include <vector>

#include "../safeguards.h"

using json = nlohmann::json;

namespace PrismaticMCP::Panel {

/** How many of the latest decisions and requests the window lists. */
static constexpr int RECENT = 8;

json Snapshot()
{
	json approvals = json::array();
	for (const PendingApprovalInfo &a : PendingApprovals()) {
		approvals.push_back({{"id", a.id}, {"tool", a.tool}, {"args", a.canonical_args}, {"expired", a.expired}});
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
	return {{"status", GetStatusLine()}, {"approvals", std::move(approvals)}, {"decisions", std::move(decisions)},
		{"requests", std::move(requests)}, {"switches", std::move(switches)}, {"links", std::move(links)}};
}

std::optional<std::string> ConsentRefusal(bool approve)
{
	if (approve && UI::InjectingInput()) return "an approval needs the owner's own click; input injected through a UI tool (openttd.input.*, openttd.ui.press, ...) never consents";
	return std::nullopt;
}

std::string ConsentCommand(bool approve, uint64_t id)
{
	return fmt::format("prismatic_mcp {} {}", approve ? "approve" : "deny", id);
}

json Tool()
{
	return {{"name", "openttd.ui.control"},
		{"description", "What the Prismatic control window shows, as data: the MCP server's status, every approval still waiting (id, tool, the exact arguments, expired), the latest decisions with their verdicts, the questions asked in this game, and the toolbar's switches and hub links. Read-only"},
		{"inputSchema", {{"type", "object"}}},
		{"annotations", {{"readOnlyHint", true}, {"idempotentHint", true}, {"openWorldHint", false}}}};
}

namespace {

/** Widgets of the control window. */
enum ControlWidgets : WidgetID {
	WID_PC_CAPTION, ///< The caption, with the number of approvals waiting.
	WID_PC_LIST, ///< Every section, one row per line.
	WID_PC_SCROLLBAR, ///< The list's scrollbar.
	WID_PC_APPROVE, ///< Approve the selected approval.
	WID_PC_DENY, ///< Deny the selected approval.
	WID_PC_ASK, ///< Ask the planner in plain language.
	WID_PC_HUB, ///< Open the hub in the browser.
	WID_PC_CONSOLE, ///< Open the console.
};

/** One line of the list. */
struct Row {
	enum class Kind : uint8_t { Heading, Text, Approval, Switch };
	Kind kind;
	std::string text;
	uint64_t id = 0; ///< Approval: its id. Switch: the menu item's index.
	bool flag = false; ///< Approval: expired. Switch: on.
};

/** The rows of a snapshot, in the order the window lists them. */
std::vector<Row> RowsOf(const json &s)
{
	std::vector<Row> rows;
	auto heading = [&rows](std::string text) { rows.push_back({Row::Kind::Heading, std::move(text)}); };
	auto text = [&rows](std::string text) { rows.push_back({Row::Kind::Text, std::move(text)}); };
	heading("MCP server");
	text(s.value("status", ""));
	heading(fmt::format("Waiting for your approval ({})", s["approvals"].size()));
	if (s["approvals"].empty()) text("nothing waits: a change an agent asks for appears here");
	for (const json &a : s["approvals"]) {
		rows.push_back({Row::Kind::Approval, fmt::format("#{}  {}  {}", a.value("id", uint64_t{0}), a.value("tool", ""), a.value("args", "")),
			a.value("id", uint64_t{0}), a.value("expired", false)});
	}
	heading("Latest decisions: what ran, and what the game showed afterwards");
	if (s["decisions"].empty()) text("no decision has run yet");
	for (const json &d : s["decisions"]) text(d.get<std::string>());
	heading("Questions from this game");
	if (s["requests"].empty()) text("none yet: use Ask, or a request item of the toolbar menu");
	for (const json &r : s["requests"]) {
		std::string asked = r.value("text", "");
		text(fmt::format("#{} {}{}{} - {}", r.value("id", uint64_t{0}), r.value("kind", ""), asked.empty() ? "" : ": ", asked,
			r.value("answered", false) ? r.value("answer", std::string{}) : std::string{"waiting for an answer"}));
	}
	if (!s["switches"].empty()) heading("Switches (click to flip)");
	for (const json &sw : s["switches"]) {
		rows.push_back({Row::Kind::Switch, sw.value("label", ""), sw.value("index", uint64_t{0}), sw.value("on", false)});
	}
	return rows;
}

struct ControlWindow : Window {
	Scrollbar *vscroll = nullptr;
	json snapshot;
	std::vector<Row> rows;
	std::optional<uint64_t> selected; ///< The approval the owner selected.

	ControlWindow(WindowDesc &desc, WindowNumber number) : Window(desc)
	{
		this->CreateNestedTree();
		this->vscroll = this->GetScrollbar(WID_PC_SCROLLBAR);
		this->FinishInitNested(number);
		this->Refresh();
	}

	/** Read the snapshot again; keep the selection only while that approval still waits. */
	void Refresh()
	{
		this->snapshot = Snapshot();
		this->rows = RowsOf(this->snapshot);
		if (this->selected.has_value()) {
			bool still = std::ranges::any_of(this->rows, [this](const Row &r) { return r.kind == Row::Kind::Approval && r.id == *this->selected; });
			if (!still) this->selected.reset();
		}
		this->vscroll->SetCount(this->rows.size());
		this->SetWidgetDisabledState(WID_PC_APPROVE, !this->selected.has_value());
		this->SetWidgetDisabledState(WID_PC_DENY, !this->selected.has_value());
		this->SetDirty();
	}

	std::string GetWidgetString(WidgetID widget, StringID stringid) const override
	{
		switch (widget) {
			case WID_PC_CAPTION: {
				size_t waiting = this->snapshot.is_object() ? this->snapshot["approvals"].size() : 0;
				return waiting == 0 ? std::string{"Prismatic control"} : fmt::format("Prismatic control - {} waiting for you", waiting);
			}
			case WID_PC_APPROVE: return "Approve";
			case WID_PC_DENY: return "Deny";
			case WID_PC_ASK: return "Ask the planner...";
			case WID_PC_HUB: return "Open the hub";
			case WID_PC_CONSOLE: return "Console";
		}
		return this->Window::GetWidgetString(widget, stringid);
	}

	void UpdateWidgetSize(WidgetID widget, Dimension &size, [[maybe_unused]] const Dimension &padding, Dimension &fill, Dimension &resize) override
	{
		if (widget != WID_PC_LIST) return;
		resize.width = 1;
		fill.height = resize.height = GetCharacterHeight(FontSize::Normal) + WidgetDimensions::scaled.vsep_normal;
		size.width = std::max<uint>(size.width, ScaleGUITrad(460));
		size.height = std::max<uint>(size.height, resize.height * 14 + WidgetDimensions::scaled.framerect.Vertical());
	}

	void DrawWidget(const Rect &r, WidgetID widget) const override
	{
		if (widget != WID_PC_LIST) return;
		const int line = GetCharacterHeight(FontSize::Normal) + WidgetDimensions::scaled.vsep_normal;
		Rect ir = r.Shrink(WidgetDimensions::scaled.framerect);
		auto [first, last] = this->vscroll->GetVisibleRangeIterators(this->rows);
		for (auto it = first; it != last; ++it) {
			const Row &row = *it;
			Rect lr = ir.WithHeight(line);
			switch (row.kind) {
				case Row::Kind::Heading:
					DrawString(lr.left, lr.right, lr.top, row.text, TextColour::Gold);
					break;
				case Row::Kind::Text:
					DrawString(lr.left + WidgetDimensions::scaled.hsep_indent, lr.right, lr.top, row.text, TextColour::White);
					break;
				case Row::Kind::Approval: {
					bool on = this->selected == row.id;
					if (on) GfxFillRect(lr, PC_DARK_GREY);
					DrawString(lr.left + WidgetDimensions::scaled.hsep_indent, lr.right, lr.top,
						row.flag ? row.text + "  (expired)" : row.text, row.flag ? TextColour::Grey : (on ? TextColour::White : TextColour::Orange));
					break;
				}
				case Row::Kind::Switch:
					DrawString(lr.left + WidgetDimensions::scaled.hsep_indent, lr.right, lr.top,
						fmt::format("[{}] {}", row.flag ? "x" : " ", row.text), row.flag ? TextColour::Green : TextColour::White);
					break;
			}
			ir.top += line;
		}
	}

	/** Consent to the selected approval, unless the click was not the owner's own. */
	void Consent(bool approve)
	{
		if (!this->selected.has_value()) return;
		if (auto refused = ConsentRefusal(approve); refused.has_value()) {
			IConsolePrint(CC_ERROR, "Prismatic control: {}", *refused);
			return;
		}
		IConsoleCmdExec(ConsentCommand(approve, *this->selected));
		this->selected.reset();
		this->Refresh();
	}

	void OnClick([[maybe_unused]] Point pt, WidgetID widget, [[maybe_unused]] int click_count) override
	{
		switch (widget) {
			case WID_PC_LIST: {
				auto it = this->vscroll->GetScrolledItemFromWidget(this->rows, pt.y, this, WID_PC_LIST, WidgetDimensions::scaled.framerect.top);
				if (it == this->rows.end()) return;
				if (it->kind == Row::Kind::Approval) {
					this->selected = (this->selected == it->id) ? std::nullopt : std::optional<uint64_t>{it->id};
					this->Refresh();
				} else if (it->kind == Row::Kind::Switch) {
					Menu::Run(static_cast<int>(it->id));
					this->Refresh();
				}
				break;
			}
			case WID_PC_APPROVE: this->Consent(true); break;
			case WID_PC_DENY: this->Consent(false); break;
			case WID_PC_ASK: ShowQueryString({}, STR_PRISMATIC_MCP_ASK_CAPTION, 256, this, CS_ALPHANUMERAL, {}); break;
			case WID_PC_HUB: OpenBrowser(Menu::HubUrl() + "/"); break;
			case WID_PC_CONSOLE: if (_iconsole_mode == IConsoleMode::Closed) IConsoleSwitch(); break;
		}
	}

	void OnQueryTextFinished(std::optional<std::string> str) override
	{
		if (str.has_value() && !str->empty()) Menu::Ask(*str);
		this->Refresh();
	}

	void OnResize() override
	{
		this->vscroll->SetCapacityFromWidget(this, WID_PC_LIST, WidgetDimensions::scaled.framerect.Vertical());
	}

	/** Approvals arrive and expire, decisions run, answers come back: read it all again every second. */
	const IntervalTimer<TimerWindow> refresh_interval = {std::chrono::seconds(1), [this](auto) {
		this->Refresh();
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
		NWidget(WWT_PANEL, Colours::Grey, WID_PC_LIST), SetScrollbar(WID_PC_SCROLLBAR), SetResize(1, 1), SetFill(1, 1),
		EndContainer(),
		NWidget(NWID_VSCROLLBAR, Colours::Grey, WID_PC_SCROLLBAR),
	EndContainer(),
	NWidget(NWID_HORIZONTAL),
		NWidget(NWID_HORIZONTAL, NWidContainerFlag::EqualSize),
			NWidget(WWT_PUSHTXTBTN, Colours::Grey, WID_PC_APPROVE), SetFill(1, 0), SetResize(1, 0),
			NWidget(WWT_PUSHTXTBTN, Colours::Grey, WID_PC_DENY), SetFill(1, 0), SetResize(1, 0),
			NWidget(WWT_PUSHTXTBTN, Colours::Grey, WID_PC_ASK), SetFill(1, 0), SetResize(1, 0),
			NWidget(WWT_PUSHTXTBTN, Colours::Grey, WID_PC_HUB), SetFill(1, 0), SetResize(1, 0),
			NWidget(WWT_PUSHTXTBTN, Colours::Grey, WID_PC_CONSOLE), SetFill(1, 0), SetResize(1, 0),
		EndContainer(),
		NWidget(WWT_RESIZEBOX, Colours::Grey),
	EndContainer(),
};

WindowDesc _control_desc(
	WindowPosition::Automatic, "prismatic_control", 480, 260,
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
