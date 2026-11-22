/*
 * Part of korczis's PRIVATE RESEARCH FORK of OpenTTD. RESEARCH-ONLY / UNSTABLE.
 * Compiled only when OTTD_PRISMATIC_MCP_SERVER is defined.
 */

/**
 * @file prismatic_mcp_ai_status.h Each AI company's latest heartbeat line, read from its own script log, as fields.
 *
 * An AI that logs `<Name> heartbeat: key=value key=value ...` (PrismaticAI does, main.nut) publishes
 * its state that way already: the server reads the newest such line from the company's script log
 * buffer (ScriptInstance::GetLogData, the deque the AI debug window shows) and splits it into
 * fields. Nothing is asked of the script, so its behaviour and timing do not change, and no field
 * name is known here: whatever the line carries is what the resource answers.
 */

#ifndef PRISMATIC_MCP_AI_STATUS_H
#define PRISMATIC_MCP_AI_STATUS_H

#ifdef OTTD_PRISMATIC_MCP_SERVER

#include "../3rdparty/nlohmann/json.hpp"
#include "../script/api/script_log_types.hpp"

#include <optional>
#include <string>
#include <string_view>

namespace PrismaticMCP::ai_status {

using json = nlohmann::json;

/** What separates the logging script's name from its fields in a heartbeat line. */
inline constexpr std::string_view HEARTBEAT_MARKER = " heartbeat: ";

/** One heartbeat line, split. */
struct Heartbeat {
	std::string source; ///< The word before the marker: who logged it (e.g. "PrismaticAI").
	json fields;        ///< Every `key=value` token: an integer when the value is one, else its text.
};

/**
 * @return A `key=value` token's value as JSON: an integer when the whole value is a decimal
 * integer that fits 64 bits, else the text itself ("3/8", "growth-on-plan", "").
 */
json FieldValue(std::string_view value);
/**
 * Split one log line of the form `<source> heartbeat: key=value ...`.
 * A token without `=`, or with a key that is not made of letters, digits and `_`, is skipped.
 * @return The parsed line, or nullopt when the line is no heartbeat.
 */
std::optional<Heartbeat> ParseHeartbeat(std::string_view line);
/**
 * @return The newest heartbeat in `log` as {source, fields, line, lines_since} (lines_since: the
 * lines logged after it), or null when the log holds none.
 */
json LatestHeartbeat(const ScriptLogTypes::LogData &log);
/** openttd://ai/status: every AI company with its latest heartbeat. Main thread. */
json Resource();

} // namespace PrismaticMCP::ai_status

#endif /* OTTD_PRISMATIC_MCP_SERVER */

#endif /* PRISMATIC_MCP_AI_STATUS_H */
