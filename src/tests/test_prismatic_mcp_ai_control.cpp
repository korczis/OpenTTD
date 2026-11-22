/*
 * Part of korczis's PRIVATE RESEARCH FORK of OpenTTD. RESEARCH-ONLY.
 * Compiled only when OTTD_PRISMATIC_MCP_SERVER is defined.
 */

/**
 * @file test_prismatic_mcp_ai_control.cpp The control openttd://ai derives for each AI setting,
 * against the fixture the web panel's test (tools/research/mcp-panel/test/bots.test.mjs) reads too.
 */

#include "../stdafx.h"

#ifdef OTTD_PRISMATIC_MCP_SERVER

#include "../3rdparty/catch2/catch.hpp"
#include "../prismatic_mcp/prismatic_mcp_ai_control.h"

#include <filesystem>
#include <fstream>

#include "../safeguards.h"

using namespace PrismaticMCP::detail;
using json = nlohmann::json;

/** The shared fixture, next to this source file. */
static json Fixture()
{
	std::ifstream in(std::filesystem::path(__FILE__).parent_path() / "prismatic_mcp_ai_controls.json");
	REQUIRE(in.good());
	return json::parse(in);
}

TEST_CASE("openttd://ai gives every setting the control the shared fixture names", "[prismatic_mcp]")
{
	const json fixture = Fixture();
	REQUIRE(fixture["cases"].size() >= 2);
	for (const json &c : fixture["cases"]) {
		INFO(c["name"].get<std::string>());
		json settings = c["settings"];
		AddAiSettingControls(settings);
		REQUIRE(settings.size() == c["controls"].size());
		for (size_t i = 0; i < settings.size(); i++) {
			INFO(settings[i]["name"].get<std::string>());
			CHECK(settings[i]["control"] == c["controls"][i]);
			/* Additive: every field the setting had is still there, unchanged. */
			settings[i].erase("control");
			CHECK(settings[i] == c["settings"][i]);
		}
	}
}

TEST_CASE("an AI setting's picker and line are read from its description and name", "[prismatic_mcp]")
{
	CHECK(AiSettingPicker("") == "");
	CHECK(AiSettingPicker("Cargo (cargo id) to a (town id)") == "cargo");
	CHECK_FALSE(AiSettingLineOf("_a").has_value());
	CHECK_FALSE(AiSettingLineOf("fr2").has_value());
	CHECK_FALSE(AiSettingLineOf("fr2_").has_value());
	auto l = AiSettingLineOf("fr12_cargo");
	REQUIRE(l.has_value());
	CHECK(l->base == "fr");
	CHECK(l->line == 12);
	CHECK(l->field == "cargo");
}

#endif /* OTTD_PRISMATIC_MCP_SERVER */
