/*
 * This file is part of korczis's research fork of OpenTTD and is NOT part of upstream OpenTTD.
 * RESEARCH-ONLY / UNSTABLE.
 */

/** @file prismatic_mcp_ui.h The user interface as data and as input: windows, widgets and tiles a GUI process shows, and the pointer and keys that drive it. */

#ifndef PRISMATIC_MCP_UI_H
#define PRISMATIC_MCP_UI_H

#ifdef OTTD_PRISMATIC_MCP_SERVER

#include "../3rdparty/nlohmann/json.hpp"

#include <optional>
#include <string>

namespace PrismaticMCP::UI {

/**
 * The UI tools' descriptors for tools/list: openttd.ui.* (read the interface) and
 * openttd.input.* (drive it).
 * @return An array of MCP tool descriptors (name, description, inputSchema).
 */
nlohmann::json Tools();

/**
 * Whether this process is armed for interface input: started with PRISMATIC_MCP_INPUT=1, the
 * owner's switch. Read once; openttd://mcp/status reports it. Armed, a script can type into this
 * process's chat and console, so an approval may not leave it that way (ADR 0038).
 * @return True when the openttd.input.* and acting openttd.ui.* tools may run.
 */
bool InputArmed();

/**
 * Run a UI tool. Main thread only (called from the MCP poll inside GameLoop()).
 *
 * openttd.ui.* reads what the process shows: every window with its widgets, their screen
 * rectangles and their resolved text, derived from the window system itself rather than from a
 * hand-written list, and the projection between screen pixels and map tiles.
 *
 * openttd.input.* feeds the pointer, the mouse buttons, the wheel and keys into the same entry
 * points a video driver calls (HandleMouseEvents(), HandleKeypress()), so the game reacts exactly
 * as to a person: the cursor sprite moves, tools highlight tiles, a click or a drag issues the
 * command a person's would, through the network like any client. Input is refused unless the
 * process was started with PRISMATIC_MCP_INPUT=1 (the owner's switch for their own client), and
 * it pauses for a few seconds whenever the real mouse moves, so the owner can take over at once.
 *
 * In a dedicated server, which has no interface, every UI tool answers an error.
 * @param name The tool name.
 * @param args The tool arguments.
 * @return The tool's JSON answer, or std::nullopt when \a name is not a UI tool.
 */
std::optional<nlohmann::json> Call(const std::string &name, const nlohmann::json &args);

/**
 * Whether the input being handled right now was injected by a UI tool (openttd.input.*, and the
 * openttd.ui.* tools that press, choose, type, close or click a tile) rather than typed or clicked
 * by a person. A window whose buttons consent on the owner's behalf (the
 * approvals of the Prismatic control window) refuses a click while this is true: no script, bot
 * or MCP client approves its own request by driving the owner's mouse.
 */
bool InjectingInput();

/** Marks the input handled during its lifetime as injected (Call() holds one around every UI tool; tests hold one to act as a script). */
class InjectionScope {
public:
	explicit InjectionScope(bool on = true);
	~InjectionScope();
	InjectionScope(const InjectionScope &) = delete;
	InjectionScope &operator=(const InjectionScope &) = delete;
private:
	bool previous;
};

} // namespace PrismaticMCP::UI

#endif /* OTTD_PRISMATIC_MCP_SERVER */

#endif /* PRISMATIC_MCP_UI_H */
