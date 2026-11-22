/*
 * Part of korczis's PRIVATE RESEARCH FORK of OpenTTD. RESEARCH-ONLY.
 * Compiled only when OTTD_PRISMATIC_MCP_SERVER is defined.
 */

/** @file test_prismatic_mcp_ai_status.cpp Unit tests for openttd://ai/status: the heartbeat parser and the resource shape. */

#include "../stdafx.h"

#ifdef OTTD_PRISMATIC_MCP_SERVER

#include "../3rdparty/catch2/catch.hpp"
#include "../ai/ai_instance.hpp"
#include "../company_base.h"
#include "../prismatic_mcp/prismatic_mcp_ai_status.h"

#include "../safeguards.h"

using namespace PrismaticMCP::ai_status;
using json = nlohmann::json;

/* A line PrismaticAI logged in an arena match (ttd-battle on lundra), and one in the format main.nut logs now. */
static const std::string ARENA_LINE = "PrismaticAI heartbeat: date=712939 money=14509 vehicles=3 routes=3/4 aggressiveness=50 max_routes=4 cash_reserve=20000 intercity=1";
static const std::string MASTER_LINE = "PrismaticAI heartbeat: date=740123 money=-25000 vehicles=12 routes=5/8 own_cap=8 air_cap=10 aggressiveness=50 max_routes=8 cash_reserve=20000 intercity=1 self_steward=1 own_lines=8 autopilot=growth-on-plan rich=1";

static ScriptLogTypes::LogLine Line(const std::string &text)
{
	return ScriptLogTypes::LogLine{text, ScriptLogTypes::LOG_INFO, 0};
}

TEST_CASE("FieldValue is an integer only when the whole value is one", "[prismatic_mcp][ai_status]")
{
	CHECK(FieldValue("42") == json(42));
	CHECK(FieldValue("-25000") == json(-25000));
	CHECK(FieldValue("3/8") == json("3/8"));
	CHECK(FieldValue("growth-on-plan") == json("growth-on-plan"));
	CHECK(FieldValue("12a") == json("12a"));
	CHECK(FieldValue("") == json(""));
	CHECK(FieldValue("99999999999999999999") == json("99999999999999999999")); // beyond 64 bits stays text
}

TEST_CASE("ParseHeartbeat splits a heartbeat line into its fields, naming none", "[prismatic_mcp][ai_status]")
{
	auto arena = ParseHeartbeat(ARENA_LINE);
	REQUIRE(arena.has_value());
	CHECK(arena->source == "PrismaticAI");
	CHECK(arena->fields == json::parse(R"({"date":712939,"money":14509,"vehicles":3,"routes":"3/4","aggressiveness":50,"max_routes":4,"cash_reserve":20000,"intercity":1})"));

	auto master = ParseHeartbeat(MASTER_LINE);
	REQUIRE(master.has_value());
	CHECK(master->fields.size() == 14);
	CHECK(master->fields["money"] == json(-25000));
	CHECK(master->fields["own_cap"] == json(8));
	CHECK(master->fields["autopilot"] == json("growth-on-plan"));
	CHECK(master->fields["rich"] == json(1));

	/* Any script's heartbeat, and tokens that are no key=value skipped. */
	auto other = ParseHeartbeat("OtherAI heartbeat: a=1  noeq =5 bad-key=2 b_2=x=y ");
	REQUIRE(other.has_value());
	CHECK(other->source == "OtherAI");
	CHECK(other->fields == json::parse(R"({"a":1,"b_2":"x=y"})"));
	auto empty = ParseHeartbeat("OtherAI heartbeat: ");
	REQUIRE(empty.has_value());
	CHECK(empty->fields == json::object());

	/* Not heartbeats. */
	CHECK_FALSE(ParseHeartbeat("PrismaticAI behaviour: aggressiveness=50 max_routes=8").has_value());
	CHECK_FALSE(ParseHeartbeat("heartbeat: a=1").has_value());
	CHECK_FALSE(ParseHeartbeat(" heartbeat: a=1").has_value());
	CHECK_FALSE(ParseHeartbeat("two words heartbeat: a=1").has_value());
	CHECK_FALSE(ParseHeartbeat("").has_value());
}

TEST_CASE("LatestHeartbeat takes the newest heartbeat and counts the lines after it", "[prismatic_mcp][ai_status]")
{
	ScriptLogTypes::LogData log;
	CHECK(LatestHeartbeat(log) == json::parse(R"({"source":null,"fields":null,"line":null,"lines_since":null})"));

	log.push_back(Line(ARENA_LINE));
	log.push_back(Line("PrismaticAI behaviour: aggressiveness=50"));
	log.push_back(Line(MASTER_LINE));
	log.push_back(Line("Built a road route"));
	log.push_back(Line("Bought a bus"));
	json hb = LatestHeartbeat(log);
	CHECK(hb["source"] == "PrismaticAI");
	CHECK(hb["line"] == MASTER_LINE);
	CHECK(hb["lines_since"] == 2);
	CHECK(hb["fields"]["routes"] == "5/8");
}

TEST_CASE("openttd://ai/status lists every AI company with its latest heartbeat", "[prismatic_mcp][ai_status]")
{
	/* A human company (not listed), a running AI with a log, and an AI company with no script running. */
	Company::CreateAtIndex(CompanyID{0}, StringID{}, false);
	Company *running = Company::CreateAtIndex(CompanyID{1}, StringID{}, true);
	Company::CreateAtIndex(CompanyID{2}, StringID{}, true);
	running->ai_instance = std::make_unique<AIInstance>();
	running->ai_instance->GetLogData().push_back(Line(MASTER_LINE));
	running->ai_instance->GetLogData().push_back(Line("Bought a bus"));

	json data = Resource();
	_company_pool.CleanPool();

	CHECK(data["network_client"] == false);
	REQUIRE(data["companies"].size() == 2);
	const json &a = data["companies"][0];
	CHECK(a["company"] == 1);
	CHECK(a["running"] == true);
	CHECK(a["ai"].is_null()); // no AIConfig script in a unit test
	CHECK(a["source"] == "PrismaticAI");
	CHECK(a["line"] == MASTER_LINE);
	CHECK(a["lines_since"] == 1);
	CHECK(a["fields"]["vehicles"] == 12);
	const json &b = data["companies"][1];
	CHECK(b["company"] == 2);
	CHECK(b["running"] == false);
	CHECK(b["fields"].is_null());
	CHECK(b["line"].is_null());
}

#endif /* OTTD_PRISMATIC_MCP_SERVER */
