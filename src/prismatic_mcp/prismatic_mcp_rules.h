/*
 * This file is part of korczis's PRIVATE RESEARCH FORK of OpenTTD and is NOT
 * part of upstream OpenTTD. See research/prismatic-mcp/ and AGENTS.md.
 *
 * RESEARCH-ONLY, UNSTABLE. Compiled only with OPTION_PRISMATIC_MCP_SERVER.
 *
 * The owner's standing approval rules (ADR 0041): "always allow <tool-glob> [<json matcher>]" and
 * "always deny ...". Pure helpers, no game state: the glob, the JSON matcher with its paths, which
 * rule decides an approval, the console spelling of a rule, and a rule as JSON. The rule list and
 * its application to the queue live in prismatic_mcp_server.cpp, next to the queue; the tests are in
 * src/tests/test_prismatic_mcp.cpp.
 */

/** @file prismatic_mcp_rules.h Standing approval rules: tool globs and JSON matchers (research-only). */

#ifndef PRISMATIC_MCP_RULES_H
#define PRISMATIC_MCP_RULES_H

#include <algorithm>
#include <cstdint>
#include <optional>
#include <span>
#include <string>
#include <string_view>
#include <vector>

#include "../3rdparty/nlohmann/json.hpp"
#include "../core/format.hpp"

namespace PrismaticMCP::Rules {

/** What a rule does with an approval it matches. */
enum class Effect : uint8_t {
	Allow, ///< Approve it: it executes at once, recorded as approved via the rule.
	Deny,  ///< Deny it: it is withdrawn, nothing is enacted.
};

/** One standing rule of the owner. */
struct Rule {
	uint64_t id = 0;           ///< Its number, for 'prismatic_mcp rule remove <id>'.
	Effect effect = Effect::Deny;
	std::string tool = "*";    ///< A glob over the tool name: the MCP tool or the action type.
	nlohmann::json match;      ///< A JSON matcher over the approval's arguments; null matches any.
	std::string created_via;   ///< "console", "window" or "panel": who made it.
	std::string created_at;    ///< Wall-clock time it was made, ISO 8601 UTC.
	uint64_t created_tick = 0; ///< The game tick it was made at.
	uint64_t hits = 0;         ///< How many approvals it decided in this process.
	bool persisted = false;    ///< Kept in the rules file, so a restart loads it again.
};

/** The channels whose rule or bulk approval may decide an owner-only tool: the game's console and the owner's click in the game. */
inline bool OwnersOwnChannel(std::string_view via)
{
	return via == "console" || via == "window";
}

/**
 * Whether an action is the owner's alone (ADR 0038): a Battlebrain live decision applies through
 * ai.set_settings, one approval for a whole plan, so the panel never decides it.
 */
inline bool OwnerOnly(std::string_view action)
{
	return action == "ai.set_settings" || action == "openttd.ai.set_settings";
}

/** The length in bytes of the UTF-8 character starting at \p i of \p s (1 for a stray byte). */
inline size_t Utf8Length(std::string_view s, size_t i)
{
	const unsigned char c = static_cast<unsigned char>(s[i]);
	size_t n = c < 0x80 ? 1 : (c >> 5) == 0x6 ? 2 : (c >> 4) == 0xE ? 3 : (c >> 3) == 0x1E ? 4 : 1;
	return std::min(n, s.size() - i);
}

/**
 * A glob: `*` matches any run of characters (none included), `?` exactly one character (one UTF-8
 * character, not one byte), `\*`, `\?` and `\\` the character itself, anything else itself.
 * @param pattern The glob.
 * @param text What it is matched against, whole.
 * @return Whether the whole text matches.
 */
inline bool Glob(std::string_view pattern, std::string_view text)
{
	size_t p = 0, t = 0;
	size_t star = std::string_view::npos, mark = 0;
	while (t < text.size()) {
		if (p < pattern.size() && pattern[p] == '*') {
			star = p++;
			mark = t;
			continue;
		}
		if (p < pattern.size() && pattern[p] == '?') {
			p++;
			t += Utf8Length(text, t);
			continue;
		}
		if (p < pattern.size()) {
			const size_t lit = (pattern[p] == '\\' && p + 1 < pattern.size()) ? p + 1 : p;
			if (pattern[lit] == text[t]) {
				p = lit + 1;
				t++;
				continue;
			}
		}
		if (star == std::string_view::npos) return false;
		p = star + 1;
		mark += Utf8Length(text, mark);
		t = mark;
	}
	while (p < pattern.size() && pattern[p] == '*') p++;
	return p == pattern.size();
}

/** Whether a glob has a wildcard (`*` or `?` not escaped): it then names more than one tool. */
inline bool HasWildcard(std::string_view pattern)
{
	for (size_t i = 0; i < pattern.size(); i++) {
		if (pattern[i] == '\\') { i++; continue; }
		if (pattern[i] == '*' || pattern[i] == '?') return true;
	}
	return false;
}

/** One step of a JSON path. */
struct PathStep {
	enum class Kind : uint8_t {
		Key,     ///< `.name` or `['name']`: one member.
		Index,   ///< `[n]`: one element.
		Every,   ///< `[*]` or `.*`: every element or member, each of which must match.
		Descend, ///< `..name`: that member at any depth, each one found must match.
	};
	Kind kind = Kind::Key;
	std::string key;  ///< Key and Descend: the member's name.
	size_t index = 0; ///< Index: the element.
};

/**
 * Parse a JSON path: `$` followed by `.name`, `['name']`, `[n]`, `[*]`, `.*` and `..name` steps.
 * @param path The path, starting with `$`.
 * @return The steps; nullopt when it does not parse.
 */
inline std::optional<std::vector<PathStep>> ParsePath(std::string_view path)
{
	if (path.empty() || path[0] != '$') return std::nullopt;
	std::vector<PathStep> steps;
	size_t i = 1;
	auto name_end = [&path](size_t from) {
		size_t e = from;
		while (e < path.size() && path[e] != '.' && path[e] != '[') e++;
		return e;
	};
	while (i < path.size()) {
		if (path.compare(i, 2, "..") == 0) {
			size_t e = name_end(i + 2);
			if (e == i + 2) return std::nullopt;
			steps.push_back({PathStep::Kind::Descend, std::string(path.substr(i + 2, e - i - 2)), 0});
			i = e;
		} else if (path[i] == '.') {
			size_t e = name_end(i + 1);
			if (e == i + 1) return std::nullopt;
			std::string_view name = path.substr(i + 1, e - i - 1);
			steps.push_back(name == "*" ? PathStep{PathStep::Kind::Every, {}, 0} : PathStep{PathStep::Kind::Key, std::string(name), 0});
			i = e;
		} else if (path[i] == '[') {
			size_t close = path.find(']', i);
			if (close == std::string_view::npos) return std::nullopt;
			std::string_view inner = path.substr(i + 1, close - i - 1);
			if (inner == "*") {
				steps.push_back({PathStep::Kind::Every, {}, 0});
			} else if (inner.size() >= 2 && (inner.front() == '\'' || inner.front() == '"') && inner.back() == inner.front()) {
				steps.push_back({PathStep::Kind::Key, std::string(inner.substr(1, inner.size() - 2)), 0});
			} else {
				if (inner.empty() || inner.find_first_not_of("0123456789") != std::string_view::npos || inner.size() > 9) return std::nullopt;
				PathStep s{PathStep::Kind::Index, {}, 0};
				for (char c : inner) s.index = s.index * 10 + static_cast<size_t>(c - '0');
				steps.push_back(s);
			}
			i = close + 1;
		} else {
			return std::nullopt;
		}
	}
	return steps;
}

/** Every value under \p node that has member \p key, at any depth, in document order. */
inline void Descend(const nlohmann::json &node, const std::string &key, std::vector<const nlohmann::json *> &out)
{
	if (node.is_object()) {
		for (auto it = node.begin(); it != node.end(); ++it) {
			if (it.key() == key) out.push_back(&it.value());
			Descend(it.value(), key, out);
		}
	} else if (node.is_array()) {
		for (const auto &e : node) Descend(e, key, out);
	}
}

/**
 * The values a path selects in \p root.
 * @param root The value the path starts at (`$`).
 * @param steps The parsed path.
 * @return Each value selected; empty when the path leads nowhere.
 */
inline std::vector<const nlohmann::json *> Select(const nlohmann::json &root, const std::vector<PathStep> &steps)
{
	std::vector<const nlohmann::json *> at = {&root};
	for (const PathStep &s : steps) {
		std::vector<const nlohmann::json *> next;
		for (const nlohmann::json *n : at) {
			switch (s.kind) {
				case PathStep::Kind::Key:
					if (n->is_object() && n->contains(s.key)) next.push_back(&(*n)[s.key]);
					break;
				case PathStep::Kind::Index:
					if (n->is_array() && s.index < n->size()) next.push_back(&(*n)[s.index]);
					break;
				case PathStep::Kind::Every:
					if (n->is_array() || n->is_object()) for (const auto &e : *n) next.push_back(&e);
					break;
				case PathStep::Kind::Descend:
					Descend(*n, s.key, next);
					break;
			}
		}
		at = std::move(next);
	}
	return at;
}

/**
 * Whether \p value matches the JSON matcher \p pattern.
 * - An object: every key must match. A plain key is a member the value must have, whose value
 *   matches recursively (a subset match: members not named are free). A key starting with `$` is a
 *   path from this value; it must select at least one value, and every value it selects must match
 *   (`[*]` and `..name` select several).
 *   A path that selects nothing (a missing key, `[*]` of an empty array) fails the key: an allow
 *   rule never matches vacuously, and neither does a deny rule, so a deny rule that must hold for
 *   an empty list names the list with a plain key instead.
 * - `$lt`, `$lte`, `$gt`, `$gte` (numbers), `$in` (an array of patterns, any may match) and `$ne` (a
 *   pattern that must not match) as keys compare the value itself: {"value": {"$gte": 0, "$lt": 100}}.
 * - A string: `"*"` matches any value present; any other string is a glob against a string value,
 *   or against the JSON text of any other value (`"1?"` matches 12, `"[1,*"` an array starting 1).
 * - A number, a boolean or null: JSON equality, or a string holding its JSON text ("5" for 5).
 * - An array: an array of the same length whose elements match in order.
 */
inline bool Match(const nlohmann::json &pattern, const nlohmann::json &value);

/** The comparison operators a matcher object may hold instead of a key or a path. */
inline bool IsOperator(std::string_view key)
{
	return key == "$lt" || key == "$lte" || key == "$gt" || key == "$gte" || key == "$in" || key == "$ne";
}

/** Whether \p value satisfies the operator \p op with operand \p operand (see Match). */
inline bool Satisfies(std::string_view op, const nlohmann::json &operand, const nlohmann::json &value)
{
	if (op == "$in") {
		if (!operand.is_array()) return false;
		for (const auto &e : operand) if (Match(e, value)) return true;
		return false;
	}
	if (op == "$ne") return !Match(operand, value);
	if (!operand.is_number() || !value.is_number()) return false;
	const double a = value.get<double>(), b = operand.get<double>();
	if (op == "$lt") return a < b;
	if (op == "$lte") return a <= b;
	if (op == "$gt") return a > b;
	return a >= b; // $gte
}

inline bool Match(const nlohmann::json &pattern, const nlohmann::json &value)
{
	if (pattern.is_object()) {
		for (auto it = pattern.begin(); it != pattern.end(); ++it) {
			const std::string &k = it.key();
			if (IsOperator(k)) {
				if (!Satisfies(k, it.value(), value)) return false;
			} else if (!k.empty() && k[0] == '$') {
				auto steps = ParsePath(k);
				if (!steps.has_value()) return false;
				auto found = Select(value, *steps);
				if (found.empty()) return false;
				for (const nlohmann::json *f : found) if (!Match(it.value(), *f)) return false;
			} else if (!value.is_object() || !value.contains(k) || !Match(it.value(), value[k])) {
				return false;
			}
		}
		return true;
	}
	if (pattern.is_string()) {
		const std::string &p = pattern.get_ref<const std::string &>();
		if (p == "*") return true;
		return Glob(p, value.is_string() ? value.get_ref<const std::string &>() : value.dump());
	}
	if (pattern.is_array()) {
		if (!value.is_array() || value.size() != pattern.size()) return false;
		for (size_t i = 0; i < pattern.size(); i++) if (!Match(pattern[i], value[i])) return false;
		return true;
	}
	/* A number, a boolean or null against a string: the string holding its JSON text ("5" for 5). */
	if (value.is_string()) return pattern.dump() == value.get_ref<const std::string &>();
	return pattern == value;
}

/**
 * Why a matcher cannot be a rule's, or nullopt when it can: it is null (any arguments) or an
 * object, and every `$` key in it, at any depth, parses as a path.
 */
inline std::optional<std::string> MatcherError(const nlohmann::json &m)
{
	if (m.is_null()) return std::nullopt;
	if (!m.is_object()) return "a matcher is a JSON object, e.g. {\"company\": 1, \"name\": \"max_*\"}";
	std::vector<const nlohmann::json *> todo = {&m};
	while (!todo.empty()) {
		const nlohmann::json *n = todo.back();
		todo.pop_back();
		if (n->is_object()) {
			for (auto it = n->begin(); it != n->end(); ++it) {
				const std::string &k = it.key();
				if (k == "$in" && !it.value().is_array()) return "$in takes an array of patterns";
				if (IsOperator(k) && k != "$in" && k != "$ne" && !it.value().is_number()) return fmt::format("{} takes a number", k);
				if (!IsOperator(k) && !k.empty() && k[0] == '$' && !ParsePath(k).has_value()) {
					return fmt::format("'{}' is not a path: $ then .name, ['name'], [n], [*], .* or ..name", k);
				}
				todo.push_back(&it.value());
			}
		} else if (n->is_array()) {
			for (const auto &e : *n) todo.push_back(&e);
		}
	}
	return std::nullopt;
}

/** What a rule is matched against: one approval waiting. */
struct Subject {
	std::string_view mcp_tool;  ///< The MCP tool as it was called (openttd.ai.set_setting); may be empty.
	std::string_view action;    ///< The action type (ai.set_setting, command.BuildRail, game.set_pause).
	const nlohmann::json &args; ///< The canonical arguments it binds to.
};

/** Whether \p r applies to \p s: its glob matches the MCP tool or the action type, and its matcher the arguments. */
inline bool Applies(const Rule &r, const Subject &s)
{
	bool tool = Glob(r.tool, s.action) || (!s.mcp_tool.empty() && Glob(r.tool, s.mcp_tool));
	return tool && (r.match.is_null() || Match(r.match, s.args));
}

/** Whether a rule made through \p via may decide an approval of \p action: an allow rule from the panel never decides an owner-only tool. */
inline bool MayDecide(Effect effect, std::string_view via, std::string_view action)
{
	return effect == Effect::Deny || !OwnerOnly(action) || OwnersOwnChannel(via);
}

/** Which rule decides an approval, and which applies without deciding it. */
struct Verdict {
	const Rule *decides = nullptr; ///< The deny rule, else the allow rule, that decides it; null when none.
	const Rule *blocked = nullptr; ///< An allow rule that applies but may not decide it (an owner-only tool, a panel rule).
};

/**
 * The verdict of \p rules on \p s: a deny rule that applies wins over every allow rule; else the
 * first allow rule that applies and may decide it. Rules are taken in id order.
 */
inline Verdict Decide(const std::vector<Rule> &rules, const Subject &s)
{
	Verdict v;
	for (const Rule &r : rules) {
		if (r.effect != Effect::Deny || !Applies(r, s)) continue;
		v.decides = &r;
		return v;
	}
	for (const Rule &r : rules) {
		if (r.effect != Effect::Allow || !Applies(r, s)) continue;
		if (MayDecide(r.effect, r.created_via, s.action)) {
			v.decides = &r;
			return v;
		}
		if (v.blocked == nullptr) v.blocked = &r;
	}
	return v;
}

/** "allow" or "deny". */
inline std::string_view EffectName(Effect e)
{
	return e == Effect::Allow ? "allow" : "deny";
}

/** A rule in one line, as a decision records it: `rule #3 allow openttd.ai.set_setting {"name":"max_*"}`. */
inline std::string Describe(const Rule &r)
{
	return fmt::format("rule #{} {} {}{}", r.id, EffectName(r.effect), r.tool, r.match.is_null() ? "" : " " + r.match.dump());
}

/** A rule as JSON, as openttd://approvals, openttd.ui.control and the rules file carry it. */
inline nlohmann::json ToJson(const Rule &r)
{
	return {{"id", r.id}, {"effect", EffectName(r.effect)}, {"tool", r.tool}, {"match", r.match},
		{"created_via", r.created_via}, {"created_at", r.created_at}, {"created_tick", r.created_tick},
		{"hits", r.hits}, {"persisted", r.persisted}, {"text", Describe(r)}};
}

/** A rule read back from JSON (the rules file); nullopt when it is not one. */
inline std::optional<Rule> FromJson(const nlohmann::json &j)
{
	if (!j.is_object()) return std::nullopt;
	Rule r;
	const std::string effect = j.value("effect", "");
	if (effect != "allow" && effect != "deny") return std::nullopt;
	r.effect = effect == "allow" ? Effect::Allow : Effect::Deny;
	if (!j.contains("tool") || !j["tool"].is_string() || j["tool"].get<std::string>().empty()) return std::nullopt;
	r.tool = j["tool"].get<std::string>();
	r.match = j.contains("match") ? j["match"] : nlohmann::json();
	if (MatcherError(r.match).has_value()) return std::nullopt;
	r.created_via = j.value("created_via", "console");
	if (r.created_via.empty()) r.created_via = "console";
	r.created_at = j.value("created_at", "");
	r.created_tick = j.contains("created_tick") && j["created_tick"].is_number_unsigned() ? j["created_tick"].get<uint64_t>() : 0;
	r.persisted = true;
	return r;
}

/**
 * Rewrite single-quoted strings as JSON ones: 'it''s' style is not JSON, so `'O\\'Brien'` (an escaped
 * quote) becomes "O'Brien", a `"` inside a single-quoted string is escaped, and a `'` inside a
 * double-quoted string ("O'Brien") is left alone.
 */
inline std::string SingleToDoubleQuotes(std::string_view text)
{
	std::string out;
	char in = 0; // the quote of the string being read, or 0 outside one
	for (size_t i = 0; i < text.size(); i++) {
		const char c = text[i];
		if (in == 0) {
			if (c == '\'' || c == '"') in = c;
			out += c == '\'' ? '"' : c;
		} else if (c == '\\' && i + 1 < text.size()) {
			if (in == '\'' && text[i + 1] == '\'') {
				out += '\'';
			} else {
				out += c;
				out += text[i + 1];
			}
			i++;
		} else if (c == in) {
			out += '"';
			in = 0;
		} else if (c == '"') {
			out += "\\\"";
		} else {
			out += c;
		}
	}
	return out;
}

/**
 * Read a matcher typed at the console: JSON as it is, or, when that does not parse, with its
 * single-quoted strings read as JSON ones ({'name': 'max_*'}), since a console line quotes with ".
 * @return The matcher (null for empty text); nullopt when neither parses.
 */
inline std::optional<nlohmann::json> ParseMatcherText(std::string_view text)
{
	size_t b = text.find_first_not_of(' ');
	if (b == std::string_view::npos) return nlohmann::json();
	nlohmann::json j = nlohmann::json::parse(text, nullptr, false);
	if (!j.is_discarded()) return j;
	j = nlohmann::json::parse(SingleToDoubleQuotes(text), nullptr, false);
	if (!j.is_discarded()) return j;
	return std::nullopt;
}

/** A rule, an approve all or a deny all as typed at the console, after its verb. */
struct Spec {
	std::string tool = "*";    ///< The tool glob ("*" when none is given).
	nlohmann::json match;      ///< The matcher; null when none is given.
	bool persist = false;      ///< `--persist`: keep it in the rules file.
	bool force = false;        ///< `--force`: an allow rule for a whole glob without a matcher (the console only).
	std::string via = "console"; ///< `--via window|panel|auto`: the channel that asks, recorded with it.
	std::string nonce;         ///< `--nonce <n>`: the caller's token for its machine-readable result line.
	std::string error;         ///< Why the tokens are not a spec; empty when they are.
};

/** The channels a console line may name with --via; without it the line is the console's own. */
inline bool KnownVia(std::string_view via)
{
	return via == "window" || via == "panel" || via == "auto";
}

/**
 * Read `[<tool-glob> [<json matcher> ...]] [--persist] [--force] [--via window|panel|auto] [--nonce <n>]`
 * from console tokens.
 * The tokens after the glob that are not flags are joined with spaces and read as one matcher, so
 * an unquoted {'a': 1, 'b': 'x'} works as well as one quoted token ("{\"a\":1}").
 */
inline Spec ParseSpec(std::span<const std::string_view> tokens)
{
	Spec s;
	std::string matcher;
	bool have_tool = false;
	for (size_t i = 0; i < tokens.size(); i++) {
		std::string_view t = tokens[i];
		if (t == "--persist") { s.persist = true; continue; }
		if (t == "--force") { s.force = true; continue; }
		if (t == "--nonce") {
			if (i + 1 >= tokens.size()) {
				s.error = "--nonce takes a value";
				return s;
			}
			s.nonce = std::string(tokens[++i]);
			continue;
		}
		if (t == "--via") {
			if (i + 1 >= tokens.size() || !KnownVia(tokens[i + 1])) {
				s.error = "--via takes 'window', 'panel' or 'auto'";
				return s;
			}
			s.via = std::string(tokens[++i]);
			continue;
		}
		if (!have_tool) {
			if (t.empty() || t.find_first_of(" \t") != std::string_view::npos || t.front() == '{') {
				s.error = fmt::format("'{}' is not a tool glob (e.g. openttd.ai.set_setting, openttd.cmd.*, *)", t);
				return s;
			}
			s.tool = std::string(t);
			have_tool = true;
			continue;
		}
		if (!matcher.empty()) matcher += ' ';
		matcher += t;
	}
	auto m = ParseMatcherText(matcher);
	if (!m.has_value()) {
		s.error = fmt::format("the matcher is not JSON: {} (quote it as one token with \\\" inside, or write it with single quotes)", matcher);
		return s;
	}
	if (auto err = MatcherError(*m); err.has_value()) {
		s.error = *err;
		return s;
	}
	s.match = std::move(*m);
	return s;
}

/**
 * Why an allow rule may not be made as \p spec asks, or nullopt: a glob over several tools with no
 * matcher approves whatever those tools ask, so only the game's console makes one, and only with
 * --force; the window and the panel name one tool or give a matcher.
 */
inline std::optional<std::string> BroadAllowRefusal(Effect effect, const Spec &spec)
{
	if (effect != Effect::Allow || !spec.match.is_null() || !HasWildcard(spec.tool)) return std::nullopt;
	if (spec.force && spec.via == "console") return std::nullopt;
	return fmt::format("an allow rule for every tool of '{}' with any arguments approves whatever they ask: give a matcher, name one tool, "
		"or add --force at the game's console", spec.tool);
}

/** One console token holding \p text exactly: in double quotes, with every " written as \" (the console's own escape). */
inline std::string ConsoleQuote(std::string_view text)
{
	std::string out = "\"";
	for (char c : text) {
		if (c == '"') out += '\\';
		out += c;
	}
	out += '"';
	return out;
}

/** The console line that makes a rule: `prismatic_mcp rule allow <tool> "<matcher>" [--persist] --via <via>`. */
inline std::string RuleCommand(Effect effect, std::string_view tool, const nlohmann::json &match, bool persist, std::string_view via)
{
	std::string line = fmt::format("prismatic_mcp rule {} {}", EffectName(effect), tool);
	if (!match.is_null()) line += " " + ConsoleQuote(match.dump());
	if (persist) line += " --persist";
	if (via != "console") line += fmt::format(" --via {}", via);
	return line;
}

/**
 * The matcher the window offers for "Always ... with these arguments": the approval's arguments as
 * they are, without a stale date (expires_tick), which a later request of the same plan would not repeat.
 */
inline nlohmann::json MatcherFromArgs(const nlohmann::json &args)
{
	if (!args.is_object()) return nlohmann::json::object();
	nlohmann::json m = args;
	m.erase("expires_tick");
	return m;
}

} // namespace PrismaticMCP::Rules

#endif /* PRISMATIC_MCP_RULES_H */
