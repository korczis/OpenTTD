/*
 * Part of korczis's PRIVATE RESEARCH FORK of OpenTTD. RESEARCH-ONLY / UNSTABLE.
 * Compiled only when OTTD_PRISMATIC_MCP_SERVER is defined.
 */

/** @file prismatic_mcp_ai_status.cpp openttd://ai/status: each AI company's latest heartbeat line as fields. */

#include "../stdafx.h"

#ifdef OTTD_PRISMATIC_MCP_SERVER

#include "prismatic_mcp_ai_status.h"

#include "../ai/ai_config.hpp"
#include "../ai/ai_instance.hpp"
#include "../company_base.h"
#include "../network/network.h"

#include <charconv>
#include <cstdint>

#include "../safeguards.h"

namespace PrismaticMCP::ai_status {

json FieldValue(std::string_view value)
{
	int64_t n = 0;
	const char *end = value.data() + value.size();
	auto [ptr, ec] = std::from_chars(value.data(), end, n);
	if (!value.empty() && ec == std::errc{} && ptr == end) return n;
	return std::string(value);
}

std::optional<Heartbeat> ParseHeartbeat(std::string_view line)
{
	size_t at = line.find(HEARTBEAT_MARKER);
	if (at == std::string_view::npos || at == 0) return std::nullopt;
	std::string_view source = line.substr(0, at);
	if (source.find(' ') != std::string_view::npos) return std::nullopt;

	Heartbeat hb{std::string(source), json::object()};
	std::string_view rest = line.substr(at + HEARTBEAT_MARKER.size());
	while (!rest.empty()) {
		size_t space = rest.find(' ');
		std::string_view token = rest.substr(0, space);
		rest = space == std::string_view::npos ? std::string_view{} : rest.substr(space + 1);
		size_t eq = token.find('=');
		if (eq == std::string_view::npos || eq == 0) continue;
		std::string_view key = token.substr(0, eq);
		if (key.find_first_not_of("abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789_") != std::string_view::npos) continue;
		hb.fields[std::string(key)] = FieldValue(token.substr(eq + 1));
	}
	return hb;
}

json LatestHeartbeat(const ScriptLogTypes::LogData &log)
{
	size_t since = 0;
	for (auto it = log.rbegin(); it != log.rend(); ++it, ++since) {
		std::optional<Heartbeat> hb = ParseHeartbeat(it->text);
		if (hb.has_value()) return json{{"source", hb->source}, {"fields", hb->fields}, {"line", it->text}, {"lines_since", since}};
	}
	return json{{"source", nullptr}, {"fields", nullptr}, {"line", nullptr}, {"lines_since", nullptr}};
}

json Resource()
{
	static const ScriptLogTypes::LogData NO_LOG;
	json companies = json::array();
	for (const Company *c : Company::Iterate()) {
		if (!c->is_ai) continue;
		const AIConfig *config = AIConfig::GetConfig(c->index);
		bool running = c->ai_instance != nullptr;
		json row = {
			{"company", c->index.base()}, {"running", running},
			{"ai", config->HasScript() ? json(config->GetName()) : json(nullptr)},
		};
		row.update(LatestHeartbeat(running ? c->ai_instance->GetLogData() : NO_LOG));
		companies.push_back(row);
	}
	return json{{"companies", companies}, {"network_client", _networking && !_network_server}};
}

} // namespace PrismaticMCP::ai_status

#endif /* OTTD_PRISMATIC_MCP_SERVER */
