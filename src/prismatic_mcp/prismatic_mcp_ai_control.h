/*
 * Part of korczis's PRIVATE RESEARCH FORK of OpenTTD. RESEARCH-ONLY / UNSTABLE.
 * Compiled only when OTTD_PRISMATIC_MCP_SERVER is defined.
 */

/**
 * @file prismatic_mcp_ai_control.h Which control edits an AI setting, derived once for every client.
 *
 * openttd://ai gives every setting a `control`: how a form edits it and where the form puts it.
 * The web panel (tools/research/mcp-panel/static/bots.js) and the game's own settings window draw
 * what it says; the panel derives the same itself only for a setting read from an info.nut, with no
 * game to ask. One fixture, src/tests/prismatic_mcp_ai_controls.json, pins both derivations.
 *
 * Pure: it reads the settings as openttd://ai describes them (name, description, min, max, step,
 * flags, labels) and needs no game.
 */

#ifndef PRISMATIC_MCP_AI_CONTROL_H
#define PRISMATIC_MCP_AI_CONTROL_H

#ifdef OTTD_PRISMATIC_MCP_SERVER

#include "../3rdparty/nlohmann/json.hpp"

#include <algorithm>
#include <charconv>
#include <cstdint>
#include <map>
#include <optional>
#include <string>
#include <string_view>
#include <utility>
#include <vector>

namespace PrismaticMCP::detail {

/** A range of at most this many steps slides; a longer one is typed as a number. */
inline constexpr double AI_CONTROL_MAX_RANGE_STEPS = 2000;

/** @return The string field @p key of @p st, or "" when it is absent or not a string. */
inline std::string AiSettingString(const nlohmann::json &st, const char *key)
{
	auto it = st.find(key);
	return it != st.end() && it->is_string() ? it->get<std::string>() : std::string();
}

/**
 * The world list a setting's value is an id of, as its description names it: "(town id",
 * "(industry id" or "(cargo id", the earliest when it names several.
 * @return "town", "industry", "cargo", or "" when the description names none.
 */
inline std::string AiSettingPicker(std::string_view description)
{
	std::string found;
	size_t at = std::string_view::npos;
	for (const char *list : {"town", "industry", "cargo"}) {
		size_t p = description.find(std::string("(") + list + " id");
		if (p < at) { at = p; found = list; }
	}
	return found;
}

/**
 * The control one setting gets by itself: `{kind, options?, picker?}`.
 * A boolean flag is a "toggle"; labels that name integer values make a "select" whose options are
 * those values in ascending order; a range of more than one value with at most
 * AI_CONTROL_MAX_RANGE_STEPS steps (a step of 0 counts as 1) is a "range"; anything else is a
 * "number". A range or number whose description names a world list says so in `picker`, and a
 * client that has that list lets the owner pick from it.
 */
inline nlohmann::json AiSettingControl(const nlohmann::json &st)
{
	auto flags = st.find("flags");
	if (flags != st.end() && flags->is_array() && std::ranges::find(*flags, nlohmann::json("boolean")) != flags->end()) {
		return {{"kind", "toggle"}};
	}
	std::vector<std::pair<int64_t, nlohmann::json>> labels;
	auto declared = st.find("labels");
	if (declared != st.end() && declared->is_object()) {
		for (const auto &[key, label] : declared->items()) {
			int64_t value = 0;
			auto [end, ec] = std::from_chars(key.data(), key.data() + key.size(), value);
			if (ec == std::errc{} && end == key.data() + key.size()) labels.emplace_back(value, label);
		}
	}
	if (!labels.empty()) {
		std::ranges::stable_sort(labels, {}, &std::pair<int64_t, nlohmann::json>::first);
		nlohmann::json options = nlohmann::json::array();
		for (const auto &[value, label] : labels) options.push_back({{"value", value}, {"label", label}});
		return {{"kind", "select"}, {"options", options}};
	}
	auto number = [&](const char *key) {
		auto it = st.find(key);
		return it != st.end() && it->is_number() ? std::optional<double>(it->get<double>()) : std::nullopt;
	};
	std::optional<double> min = number("min"), max = number("max"), step = number("step");
	double by = step.value_or(0) != 0 ? *step : 1;
	bool slides = min && max && *max - *min > 0 && (*max - *min) / by <= AI_CONTROL_MAX_RANGE_STEPS;
	nlohmann::json control = {{"kind", slides ? "range" : "number"}};
	std::string picker = AiSettingPicker(AiSettingString(st, "description"));
	if (!picker.empty()) control["picker"] = picker;
	return control;
}

/** A setting name of the shape `<base><N>_<field>`: one field of line N of a table. */
struct AiSettingLine {
	std::string base;  ///< Lowercase letters, the table's name.
	uint64_t line;     ///< The digits after the base; a bare base is line 1.
	std::string field; ///< Lowercase letters after the underscore, the table's column.
};

/** @return The line a setting name is a field of, or nothing when the name is not `<base><N>_<field>`. */
inline std::optional<AiSettingLine> AiSettingLineOf(std::string_view name)
{
	auto lower = [](char ch) { return ch >= 'a' && ch <= 'z'; };
	size_t letters = 0;
	while (letters < name.size() && lower(name[letters])) letters++;
	size_t digits = letters;
	while (digits < name.size() && name[digits] >= '0' && name[digits] <= '9') digits++;
	if (letters == 0 || digits == name.size() || name[digits] != '_') return std::nullopt;
	std::string_view field = name.substr(digits + 1);
	if (field.empty() || !std::ranges::all_of(field, lower)) return std::nullopt;
	uint64_t line = 1;
	if (digits > letters && std::from_chars(name.data() + letters, name.data() + digits, line).ec != std::errc{}) return std::nullopt;
	return AiSettingLine{std::string(name.substr(0, letters)), line, std::string(field)};
}

/**
 * Where a form puts each setting, by name. A base with at least two lines is one table, and each of
 * its settings is `{table: {base, line, field}}`. The others group by their first word (air_a,
 * air_b -> "air") when another setting shares it, and form `{group: "general"}` when none does.
 * A name declared twice is placed once, by its first declaration.
 */
inline std::map<std::string, nlohmann::json> AiSettingPlaces(const nlohmann::json &settings)
{
	std::vector<std::string> names;
	for (const auto &st : settings) {
		std::string name = AiSettingString(st, "name");
		if (std::ranges::find(names, name) == names.end()) names.push_back(std::move(name));
	}
	std::map<std::string, std::map<uint64_t, std::map<std::string, std::string>>> bases;
	for (const auto &name : names) {
		if (auto l = AiSettingLineOf(name)) bases[l->base][l->line][l->field] = name;
	}
	std::map<std::string, nlohmann::json> places;
	for (const auto &[base, lines] : bases) {
		if (lines.size() < 2) continue;
		for (const auto &[line, fields] : lines) {
			for (const auto &[field, name] : fields) {
				places[name] = {{"table", {{"base", base}, {"line", line}, {"field", field}}}};
			}
		}
	}
	auto word = [](std::string_view name) { return std::string(name.substr(0, name.find('_'))); };
	std::map<std::string, int> sharing;
	for (const auto &name : names) if (!places.contains(name)) sharing[word(name)]++;
	for (const auto &name : names) {
		if (!places.contains(name)) places[name] = {{"group", sharing[word(name)] < 2 ? "general" : word(name)}};
	}
	return places;
}

/** Give every setting of one AI its `control`: AiSettingControl and its place from AiSettingPlaces. */
inline void AddAiSettingControls(nlohmann::json &settings)
{
	const std::map<std::string, nlohmann::json> places = AiSettingPlaces(settings);
	for (auto &st : settings) {
		nlohmann::json control = AiSettingControl(st);
		control.update(places.at(AiSettingString(st, "name")));
		st["control"] = std::move(control);
	}
}

} // namespace PrismaticMCP::detail

#endif /* OTTD_PRISMATIC_MCP_SERVER */
#endif /* PRISMATIC_MCP_AI_CONTROL_H */
