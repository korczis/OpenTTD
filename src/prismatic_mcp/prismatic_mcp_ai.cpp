/*
 * Part of korczis's PRIVATE RESEARCH FORK of OpenTTD. RESEARCH-ONLY / UNSTABLE.
 * Compiled only when OTTD_PRISMATIC_MCP_SERVER is defined.
 */

/** @file prismatic_mcp_ai.cpp The company AIs over MCP: settings derived from AIInfo, changed live. */

#include "../stdafx.h"

#ifdef OTTD_PRISMATIC_MCP_SERVER

#include "prismatic_mcp_ai.h"
#include "prismatic_mcp_ai_control.h"
#include "prismatic_mcp_commands.h"
#include "prismatic_mcp_detail.h"

#include "../ai/ai.hpp"
#include "../ai/ai_config.hpp"
#include "../ai/ai_info.hpp"
#include "../command_func.h"
#include "../company_base.h"
#include "../company_cmd.h"
#include "../company_func.h"
#include "../network/network.h"
#include "../openttd.h"
#include "../settings_type.h"
#include "../timer/timer_game_tick.h"

#include <algorithm>
#include <optional>

#include "../safeguards.h"

namespace PrismaticMCP::ai {

namespace {

json FlagNames(ScriptConfigFlags f)
{
	json arr = json::array();
	if (f.Test(ScriptConfigFlag::Boolean)) arr.push_back("boolean");
	if (f.Test(ScriptConfigFlag::InGame)) arr.push_back("in_game");
	if (f.Test(ScriptConfigFlag::Developer)) arr.push_back("developer");
	return arr;
}

/** One setting as its script declared it, with the value the script reads now. */
json DescribeSetting(const ScriptConfigItem &item, int current, bool running)
{
	json labels = json::object();
	for (const auto &[value, label] : item.labels) labels[fmt::format("{}", value)] = label;
	return json{
		{"name", item.name}, {"description", item.description},
		{"min", item.min_value}, {"max", item.max_value}, {"step", item.step_size},
		{"default", item.default_value}, {"flags", FlagNames(item.flags)},
		{"labels", labels}, {"value", current},
		{"editable", item.flags.Test(ScriptConfigFlag::InGame) || !running},
	};
}

json DescribeInfo(const ScriptInfo *info)
{
	if (info == nullptr) return nullptr;
	return json{{"name", info->GetName()}, {"short_name", info->GetShortName()}, {"version", info->GetVersion()},
		{"author", info->GetAuthor()}, {"description", info->GetDescription()}};
}

bool IsNetworkClient() { return _networking && !_network_server; }

/** The company an action names, checked to be one an AI runs. */
std::optional<CompanyID> AiCompany(const json &args, json &errors)
{
	if (!args.contains("company") || !args["company"].is_number_integer()) {
		errors.push_back("company (integer) required");
		return std::nullopt;
	}
	int64_t v = args["company"].get<int64_t>();
	if (v < 0 || v >= MAX_COMPANIES || !Company::IsValidAiID(CompanyID(static_cast<uint8_t>(v)))) {
		errors.push_back(fmt::format("company {} is not run by an AI", v));
		return std::nullopt;
	}
	CompanyID c(static_cast<uint8_t>(v));
	if (c == _local_company) {
		errors.push_back("that is the local company, not an AI");
		return std::nullopt;
	}
	return c;
}

const ScriptConfigItem *FindItem(const ScriptConfigItemList *list, std::string_view name)
{
	if (list == nullptr) return nullptr;
	for (const auto &item : *list) if (item.name == name) return &item;
	return nullptr;
}

/** Check a value against a declared setting. */
bool ValueFits(const ScriptConfigItem &item, const json &value, std::string &err)
{
	if (!value.is_number_integer()) { err = fmt::format("{}: value must be an integer", item.name); return false; }
	int64_t v = value.get<int64_t>();
	if (v < item.min_value || v > item.max_value) {
		err = fmt::format("{}: {} is outside {}..{}", item.name, v, item.min_value, item.max_value);
		return false;
	}
	if (item.flags.Test(ScriptConfigFlag::Boolean) && v != 0 && v != 1) {
		err = fmt::format("{}: a boolean setting takes 0 or 1", item.name);
		return false;
	}
	return true;
}

/**
 * Check one change of a setting of company `c`'s AI: the setting exists, may change now (while the AI
 * runs only with the in_game flag) and `value` fits its range. ai.set_setting and ai.set_settings
 * both check through here, so one setting is refused for the same reasons either way.
 * @param value The new value, or nullptr when none was given.
 * @param[out] errors Every reason the change is refused.
 * @return The setting as its script declared it, or null when the AI has no such setting.
 */
json CheckSetting(CompanyID c, const std::string &name, const json *value, json &errors)
{
	AIConfig *config = AIConfig::GetConfig(c);
	const ScriptConfigItem *item = FindItem(config->HasScript() ? config->GetConfigList() : nullptr, name);
	if (item == nullptr) { errors.push_back(fmt::format("the AI of company {} has no setting {}", c.base(), name)); return nullptr; }
	bool running = Company::Get(c)->ai_instance != nullptr;
	json described = DescribeSetting(*item, config->GetSetting(name), running);
	if (running && !item->flags.Test(ScriptConfigFlag::InGame)) {
		errors.push_back(fmt::format("{} cannot be changed while the AI runs (no in_game flag)", name));
	}
	std::string err;
	if (value == nullptr || !ValueFits(*item, *value, err)) errors.push_back(err.empty() ? "value (integer) required" : err);
	return described;
}

/** The first company slot no company holds: where start_ai puts a new AI. */
CompanyID FreeSlot()
{
	uint8_t n = 0;
	for (const Company *c : Company::Iterate()) {
		if (c->index != n) break;
		n++;
	}
	return CompanyID(n);
}

/** Lifecycle checks shared by start, stop and reload. */
void CheckCanManage(json &errors)
{
	if (_game_mode != GameMode::Normal) errors.push_back("AIs can only be managed in a game");
	if (IsNetworkClient()) errors.push_back("only the server runs AIs; this process is a network client");
}

bool PostCompanyCtrl(CompanyCtrlAction action, CompanyID company, CompanyRemoveReason reason)
{
	return Command<Commands::CompanyControl>::Post(StringID{}, &CcPrismaticMcp, action, company, reason, ClientID::Invalid);
}

} // namespace

bool IsAiAction(std::string_view type)
{
	return type == "ai.set_setting" || type == "ai.set_settings" || type == "ai.start" || type == "ai.stop" || type == "ai.reload";
}

json Resource()
{
	json companies = json::array();
	for (const Company *c : Company::Iterate()) {
		if (!c->is_ai) continue;
		bool running = c->ai_instance != nullptr;
		AIConfig *config = AIConfig::GetConfig(c->index);
		json settings = json::array();
		if (config->HasScript()) {
			for (const auto &item : *config->GetConfigList()) {
				settings.push_back(DescribeSetting(item, config->GetSetting(item.name), running));
			}
		}
		detail::AddAiSettingControls(settings);
		companies.push_back({
			{"company", c->index.base()}, {"running", running},
			{"ai", config->HasScript() ? json(config->GetName()) : json(nullptr)},
			{"version", config->HasScript() ? json(config->GetVersion()) : json(nullptr)},
			{"info", DescribeInfo(c->ai_info)}, {"settings", settings},
		});
	}
	json available = json::array();
	if (const ScriptInfoList *list = AI::GetUniqueInfoList(); list != nullptr) {
		for (const auto &[key, info] : *list) available.push_back(DescribeInfo(info));
	}
	return json{
		{"companies", companies}, {"available", available},
		{"can_start_new", _game_mode == GameMode::Normal && !IsNetworkClient() && AI::CanStartNew()},
		{"ai_in_multiplayer", (bool)_settings_game.ai.ai_in_multiplayer},
		{"network_client", IsNetworkClient()},
	};
}

json Validate(std::string_view type, const json &args, json &errors)
{
	json details = json::object();
	if (type == "ai.set_setting") {
		if (IsNetworkClient()) { errors.push_back("only the server runs AIs; this process is a network client"); return details; }
		auto c = AiCompany(args, errors);
		if (!c) return details;
		if (!args.contains("name") || !args["name"].is_string()) { errors.push_back("name (string) required"); return details; }
		if (args.contains("plan") && !detail::ValidPlanId(args["plan"])) {
			errors.push_back(fmt::format("plan must be 1-{} letters, digits or ._:- (the re-plan these settings belong to)", detail::PLAN_ID_MAX));
			return details;
		}
		json described = CheckSetting(*c, args["name"].get<std::string>(), args.contains("value") ? &args["value"] : nullptr, errors);
		if (!described.is_null()) details["setting"] = described;
	} else if (type == "ai.set_settings") {
		/* Every setting is checked, and every refusal named: the decision runs whole or not at all. */
		if (IsNetworkClient()) { errors.push_back("only the server runs AIs; this process is a network client"); return details; }
		detail::SetSettingsPlan plan = detail::PlanSetSettings(args);
		for (const std::string &e : plan.errors) errors.push_back(e);
		auto c = AiCompany(args, errors);
		if (!c || !plan.errors.empty()) return details;
		if (plan.expires_tick.has_value()) {
			uint64_t now = (uint64_t)TimerGameTick::counter;
			details["expires_tick"] = *plan.expires_tick;
			if (!detail::StillFresh(now, plan.expires_tick)) {
				errors.push_back(fmt::format("the decision expired at tick {}; it is now tick {}", *plan.expires_tick, now));
			}
		}
		json described = json::array();
		for (const auto &[name, value] : plan.settings) {
			json d = CheckSetting(*c, name, &args["settings"][name], errors);
			if (!d.is_null()) described.push_back(std::move(d));
		}
		details["settings"] = std::move(described);
	} else if (type == "ai.start") {
		CheckCanManage(errors);
		if (Company::GetNumItems() == CompanyPool::MAX_SIZE) errors.push_back("no free company slot");
		if (_networking && !_settings_game.ai.ai_in_multiplayer) errors.push_back("AIs are not allowed in multiplayer by configuration (ai.ai_in_multiplayer)");
		if (!AI::CanStartNew()) errors.push_back("a new AI cannot be started now");
		const AIInfo *info = nullptr;
		if (args.contains("name")) {
			if (!args["name"].is_string()) { errors.push_back("name must be a string"); return details; }
			int version = args.contains("version") && args["version"].is_number_integer() ? args["version"].get<int>() : -1;
			info = AI::FindInfo(args["name"].get<std::string>(), version, version != -1);
			if (info == nullptr) errors.push_back(fmt::format("no AI {} is installed", args["name"].get<std::string>()));
		}
		details["company"] = FreeSlot().base();
		if (info != nullptr) details["ai"] = DescribeInfo(info);
		if (args.contains("settings")) {
			if (!args["settings"].is_object()) { errors.push_back("settings must be an object {name: value}"); return details; }
			if (info == nullptr) { errors.push_back("settings need the AI named, to check them against its info"); return details; }
			for (const auto &[k, v] : args["settings"].items()) {
				const ScriptConfigItem *item = FindItem(info->GetConfigList(), k);
				std::string err;
				if (item == nullptr) errors.push_back(fmt::format("{} declares no setting {}", info->GetName(), k));
				else if (!ValueFits(*item, v, err)) errors.push_back(err);
			}
		}
	} else if (type == "ai.stop" || type == "ai.reload") {
		CheckCanManage(errors);
		if (auto c = AiCompany(args, errors)) details["company"] = c->base();
	} else {
		errors.push_back("unknown AI action");
	}
	return details;
}

Posted Execute(std::string_view type, const json &args, const std::function<void()> &expect_callback)
{
	Posted p;
	json errors = json::array();
	Validate(type, args, errors);
	if (!errors.empty()) {
		if (type != "ai.set_settings") { p.error = errors[0].get<std::string>(); return p; }
		/* All or nothing: nothing was applied, and the answer names every setting that stopped it. */
		p.error = "nothing applied:";
		for (const auto &e : errors) p.error += fmt::format(" {};", e.get<std::string>());
		p.error.pop_back();
		return p;
	}

	if (type == "ai.set_setting") {
		CompanyID c(static_cast<uint8_t>(args["company"].get<int64_t>()));
		std::string name = args["name"].get<std::string>();
		int value = args["value"].get<int>();
		AIConfig::GetConfig(c)->SetSetting(name, value);
		/* Read back through the same configuration AIController.GetSetting reads. */
		p.observe = [c, name, value]() { return Company::IsValidAiID(c) && AIConfig::GetConfig(c)->GetSetting(name) == value; };
		p.ok = true;
		return p;
	}
	if (type == "ai.set_settings") {
		/* Validated whole just above, on this main-thread call: every setting is applied here, in the
		 * same call, so the script never reads half of the decision. */
		CompanyID c(static_cast<uint8_t>(args["company"].get<int64_t>()));
		AIConfig *config = AIConfig::GetConfig(c);
		std::vector<std::pair<std::string, int>> applied;
		for (const auto &[name, value] : args["settings"].items()) {
			config->SetSetting(name, value.get<int>());
			applied.emplace_back(name, value.get<int>());
		}
		p.observe = [c, applied]() {
			if (!Company::IsValidAiID(c)) return false;
			const AIConfig *cfg = AIConfig::GetConfig(c);
			return std::ranges::all_of(applied, [cfg](const auto &s) { return cfg->GetSetting(s.first) == s.second; });
		};
		p.ok = true;
		return p;
	}
	if (type == "ai.start") {
		CompanyID slot = FreeSlot();
		AIConfig *config = AIConfig::GetConfig(slot);
		if (args.contains("name")) {
			int version = args.contains("version") && args["version"].is_number_integer() ? args["version"].get<int>() : -1;
			config->Change(args["name"].get<std::string>(), version, version != -1);
			if (!config->HasScript()) { p.error = "the AI could not be loaded"; return p; }
		}
		if (args.contains("settings")) {
			for (const auto &[k, v] : args["settings"].items()) config->SetSetting(k, v.get<int>());
		}
		expect_callback();
		p.ok = PostCompanyCtrl(CompanyCtrlAction::NewAI, CompanyID::Invalid(), CompanyRemoveReason::None);
		p.callbacks = p.ok ? 1 : 0;
		p.observe = [slot]() { return Company::IsValidAiID(slot); };
		if (!p.ok) p.error = "the company could not be created";
		return p;
	}
	CompanyID c(static_cast<uint8_t>(args["company"].get<int64_t>()));
	if (type == "ai.stop") {
		expect_callback();
		p.ok = PostCompanyCtrl(CompanyCtrlAction::Delete, c, CompanyRemoveReason::Manual);
		p.callbacks = p.ok ? 1 : 0;
		p.observe = [c]() { return !Company::IsValidID(c); };
		if (!p.ok) p.error = "the company could not be removed";
		return p;
	}
	/* ai.reload: remove the company, then start the configured AI again in the same slot,
	 * exactly as reload_ai does. */
	expect_callback();
	if (!PostCompanyCtrl(CompanyCtrlAction::Delete, c, CompanyRemoveReason::Manual)) {
		p.error = "the company could not be removed";
		return p;
	}
	p.callbacks = 1;
	expect_callback();
	p.ok = PostCompanyCtrl(CompanyCtrlAction::NewAI, c, CompanyRemoveReason::None);
	if (p.ok) p.callbacks = 2;
	else p.error = "the AI could not be started again";
	p.observe = [c]() { return Company::IsValidAiID(c); };
	return p;
}

json Tools()
{
	json company = {{"type", "integer"}, {"minimum", 0}, {"maximum", MAX_COMPANIES - 1}};
	json arr = json::array();
	arr.push_back({{"name", "openttd.ai.list"},
		{"description", "Every company's AI with its settings as its info.nut declares them (range, step, flags, labels), the value the running script reads and the control a form edits each with (control: kind, options, picker, table or group); plus the AIs that can be started"},
		{"inputSchema", {{"type", "object"}}}});
	arr.push_back({{"name", "openttd.ai.set_setting"},
		{"description", "Approval-gated: change one setting of a company's AI; while it runs only settings flagged in_game. The script reads it on its next GetSetting; the verdict reads it back. plan (optional) names the re-plan the setting belongs to: every pending setting of one plan is approved, denied or settled at its deadline together, a newer plan of the company withdraws the older one, and a change without a plan to a setting a pending plan holds is refused"},
		{"inputSchema", {{"type", "object"}, {"properties", {{"company", company}, {"name", {{"type", "string"}}}, {"value", {{"type", "integer"}}},
			{"plan", {{"type", "string"}, {"minLength", 1}, {"maxLength", detail::PLAN_ID_MAX}, {"pattern", "^[A-Za-z0-9._:-]+$"}}}}},
			{"required", json::array({"company", "name", "value"})}}}});
	arr.push_back({{"name", "openttd.ai.set_settings"},
		{"description", fmt::format("Approval-gated: change several settings of one company's AI under ONE approval, all applied in the same "
			"main-thread call or none (an unknown name, a value out of range or a setting that cannot change while the AI runs "
			"refuses the whole call and is named). At most {} settings. expires_tick (optional, a game tick) is bound with the "
			"arguments: the approval expires at the earlier of it and the usual TTL, and a call after it is refused even when "
			"approved. The verdict reads every value back", detail::SET_SETTINGS_MAX)},
		{"inputSchema", {{"type", "object"}, {"properties", {{"company", company},
			{"settings", {{"type", "object"}, {"minProperties", 1}, {"maxProperties", detail::SET_SETTINGS_MAX}, {"additionalProperties", {{"type", "integer"}}}}},
			{"expires_tick", {{"type", "integer"}, {"minimum", 0}}}}},
			{"required", json::array({"company", "settings"})}}},
		{"annotations", {{"readOnlyHint", false}, {"destructiveHint", true}, {"idempotentHint", false}, {"openWorldHint", false}}}});
	arr.push_back({{"name", "openttd.ai.start"},
		{"description", "Approval-gated: start an AI in the next free company slot (start_ai), optionally a named AI and version with initial settings {name: value}"},
		{"inputSchema", {{"type", "object"}, {"properties", {{"name", {{"type", "string"}}}, {"version", {{"type", "integer"}}},
			{"settings", {{"type", "object"}, {"additionalProperties", {{"type", "integer"}}}}}}}}}});
	arr.push_back({{"name", "openttd.ai.stop"},
		{"description", "Approval-gated: stop a company's AI and remove its company (stop_ai)"},
		{"inputSchema", {{"type", "object"}, {"properties", {{"company", company}}}, {"required", json::array({"company"})}}}});
	arr.push_back({{"name", "openttd.ai.reload"},
		{"description", "Approval-gated: restart a company's AI in the same slot with its current configuration (reload_ai)"},
		{"inputSchema", {{"type", "object"}, {"properties", {{"company", company}}}, {"required", json::array({"company"})}}}});
	return arr;
}

} // namespace PrismaticMCP::ai

#endif /* OTTD_PRISMATIC_MCP_SERVER */
