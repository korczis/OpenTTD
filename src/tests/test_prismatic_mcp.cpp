/*
 * Part of korczis's PRIVATE RESEARCH FORK of OpenTTD. RESEARCH-ONLY.
 * Compiled only when OTTD_PRISMATIC_MCP_SERVER is defined.
 */

/** @file test_prismatic_mcp.cpp Unit tests for the pure MCP security/logic helpers. */

#include "../stdafx.h"

#ifdef OTTD_PRISMATIC_MCP_SERVER

#include "../3rdparty/catch2/catch.hpp"
#include "../core/format.hpp"
#include "../prismatic_mcp/prismatic_mcp.h"
#include "../prismatic_mcp/prismatic_mcp_detail.h"
#include "../prismatic_mcp/prismatic_mcp_menu.h"
#include "../prismatic_mcp/prismatic_mcp.h"
#include "../prismatic_mcp/prismatic_mcp_panel.h"
#include "../prismatic_mcp/prismatic_mcp_rules.h"
#include "../prismatic_mcp/prismatic_mcp_ui.h"
#include "../prismatic_mcp/prismatic_mcp_console.h"
#include "../console_func.h"
#include "../console_internal.h"
#include "../prismatic_mcp/prismatic_mcp_view.h"
#include "../prismatic_mcp/prismatic_mcp_text.h"
#include "../network/network.h"
#include "../prismatic_mcp/prismatic_mcp_world.h"
#include "../prismatic_mcp/prismatic_mcp_entities.h"
#include "../prismatic_mcp/prismatic_mcp_graph.h"
#include "../economy_type.h"
#include "../map_func.h"
#include "../tile_map.h"

#include <chrono>
#include <filesystem>
#include <fstream>
#include <set>
#include "../prismatic_mcp/prismatic_mcp_params.hpp"
#include "../news_type.h"
#include "../order_base.h"
#include "../settings_type.h"
#include "../tile_type.h"
#include "../timer/timer_game_tick.h"

#include "../safeguards.h"

using namespace PrismaticMCP::detail;
using json = nlohmann::json;

TEST_CASE("ArgDigest is deterministic and argument-binding", "[prismatic_mcp]")
{
	/* Identical input -> identical digest (approval can match a re-submitted call). */
	CHECK(ArgDigestHex("game.set_pause", "{\"pause\":true}") == ArgDigestHex("game.set_pause", "{\"pause\":true}"));

	/* Different arguments -> different digest: this is what defeats the
	 * "change arguments after approval" abuse case. */
	CHECK(ArgDigestHex("game.set_pause", "{\"pause\":true}") != ArgDigestHex("game.set_pause", "{\"pause\":false}"));

	/* Different tool -> different digest. */
	CHECK(ArgDigestHex("game.set_pause", "{}") != ArgDigestHex("game.save", "{}"));

	/* 16 lowercase hex chars. */
	std::string d = ArgDigestHex("x", "y");
	CHECK(d.size() == 16);
	CHECK(d.find_first_not_of("0123456789abcdef") == std::string::npos);
}

TEST_CASE("IsKnownActionType gates the typed catalogue", "[prismatic_mcp]")
{
	CHECK(IsKnownActionType("game.set_pause"));
	CHECK_FALSE(IsKnownActionType("game.delete_everything"));
	CHECK_FALSE(IsKnownActionType(""));
	CHECK_FALSE(IsKnownActionType("rcon"));
	for (std::string_view t : ACTION_TYPES) CHECK(IsKnownActionType(t)); // the table is the catalogue
	/* A game command by its Cmd* name; existence is checked against the command table later. */
	CHECK(IsKnownActionType("command.StartStopVehicle"));
	CHECK_FALSE(IsKnownActionType("command."));
	CHECK_FALSE(IsKnownActionType("command.Start Stop"));
	CHECK_FALSE(IsKnownActionType("command.../x"));
}

namespace {
enum class TestEnum : uint8_t { A = 0, B = 7 };
struct NotConvertible { int a; int b; NotConvertible() = delete; };
}

TEST_CASE("Command parameters convert from JSON by their declared type", "[prismatic_mcp]")
{
	using namespace PrismaticMCP::params;
	std::string err;

	bool b = false;
	CHECK(FromJson(json(true), b, err));
	CHECK(b);
	CHECK_FALSE(FromJson(json(1), b, err));

	uint8_t u8 = 0;
	CHECK(FromJson(json(255), u8, err));
	CHECK(u8 == 255);
	CHECK_FALSE(FromJson(json(256), u8, err));
	CHECK_FALSE(FromJson(json(-1), u8, err));
	CHECK_FALSE(FromJson(json("3"), u8, err));

	int16_t i16 = 0;
	CHECK(FromJson(json(-32768), i16, err));
	CHECK_FALSE(FromJson(json(40000), i16, err));

	TestEnum e = TestEnum::A;
	CHECK(FromJson(json(7), e, err));
	CHECK(e == TestEnum::B);

	std::string str;
	CHECK(FromJson(json("depot"), str, err));
	CHECK(str == "depot");

	/* A strong type is carried as its base integer. */
	TileIndex tile{};
	CHECK(FromJson(json(1234), tile, err));
	CHECK(tile.base() == 1234);
	CHECK(Schema<TileIndex>()["type"] == "integer");

	/* An unsupported type is reported, never guessed at. */
	CHECK_FALSE(IsSupported<NotConvertible>());
	CHECK(IsSupported<TileIndex>());
	CHECK(IsSupported<TestEnum>());
}

TEST_CASE("An order is its wire fields, and any choice of types is null or {index, value}", "[prismatic_mcp]")
{
	using namespace PrismaticMCP::params;
	std::string err;

	/* An order: type, flags and destination required; the timetable fields optional. */
	CHECK(IsSupported<Order>());
	CHECK(Schema<Order>()["required"] == json::array({"type", "flags", "dest"}));
	Order order;
	CHECK(FromJson(json{{"type", static_cast<int>(OT_GOTO_STATION)}, {"flags", 0}, {"dest", 5}, {"max_speed", 80}}, order, err));
	CHECK(order.IsType(OT_GOTO_STATION));
	CHECK(order.GetDestination().base() == 5);
	CHECK(order.GetMaxSpeed() == 80);
	CHECK_FALSE(FromJson(json{{"type", 1}, {"flags", 0}}, order, err));                       // no destination
	CHECK_FALSE(FromJson(json{{"type", 1}, {"flags", 0}, {"dest", 70000}}, order, err));       // past uint16
	CHECK_FALSE(FromJson(json{{"type", 1}, {"flags", 0}, {"dest", 1}, {"speed", 3}}, order, err)); // unknown field

	/* A choice of types (here the news reference): null is the empty alternative, else {index, value}. */
	CHECK(IsSupported<NewsReference>());
	CHECK(Schema<NewsReference>()["oneOf"].size() == std::variant_size_v<NewsReference>);
	NewsReference ref = TileIndex{3};
	CHECK(FromJson(json(nullptr), ref, err));
	CHECK(std::holds_alternative<std::monostate>(ref));
	CHECK(FromJson(json{{"index", 1}, {"value", 1234}}, ref, err));
	REQUIRE(std::holds_alternative<TileIndex>(ref));
	CHECK(std::get<TileIndex>(ref).base() == 1234);
	CHECK(FromJson(json{{"index", 5}, {"value", 7}}, ref, err));
	CHECK(std::holds_alternative<TownID>(ref));
	CHECK_FALSE(FromJson(json{{"index", 0}, {"value", 1}}, ref, err));  // the empty one is null
	CHECK_FALSE(FromJson(json{{"index", 9}, {"value", 1}}, ref, err));  // no such alternative
	CHECK_FALSE(FromJson(json{{"index", 1}, {"value", "x"}}, ref, err)); // the value must fit its type
	CHECK_FALSE(FromJson(json(4), ref, err));
}

TEST_CASE("A text, a vehicle list and a cargo source convert from their JSON spelling", "[prismatic_mcp]")
{
	using namespace PrismaticMCP::params;
	std::string err;

	/* A text is a plain string, carried as the game's raw-string message, bounded. */
	CHECK(IsSupported<EncodedString>());
	CHECK(Schema<EncodedString>()["type"] == "string");
	EncodedString text;
	CHECK(FromJson(json("Deliver 500 t of coal"), text, err));
	CHECK_FALSE(text.empty());
	CHECK_FALSE(FromJson(json(5), text, err));
	CHECK_FALSE(FromJson(json(std::string(MAX_TEXT_BYTES + 1, 'x')), text, err));

	/* A vehicle list: every field, each in its range, nothing else. */
	CHECK(IsSupported<VehicleListIdentifier>());
	CHECK(Schema<VehicleListIdentifier>()["required"].size() == 4);
	VehicleListIdentifier vli;
	CHECK(FromJson(json{{"type", 4}, {"vtype", 1}, {"company", 0}, {"index", 7}}, vli, err));
	CHECK(vli.type == VehicleListType::Group);
	CHECK(vli.vtype == VehicleType::Road);
	CHECK(vli.index == 7);
	CHECK_FALSE(FromJson(json{{"type", 5}, {"vtype", 1}, {"company", 0}, {"index", 7}}, vli, err)); // past End
	CHECK_FALSE(FromJson(json{{"type", 0}, {"vtype", 4}, {"company", 0}, {"index", 0}}, vli, err)); // an effect vehicle
	CHECK_FALSE(FromJson(json{{"type", 0}, {"vtype", 0}, {"company", 0}}, vli, err));             // no index
	CHECK_FALSE(FromJson(json{{"type", 0}, {"vtype", 0}, {"company", 0}, {"index", 0}, {"x", 1}}, vli, err));
	CHECK_FALSE(FromJson(json(3), vli, err));

	/* A cargo source: an id and its kind. */
	CHECK(IsSupported<Source>());
	Source src;
	CHECK(FromJson(json{{"id", 12}, {"type", 1}}, src, err));
	CHECK(src.id == 12);
	CHECK(src.type == SourceType::Town);
	CHECK_FALSE(FromJson(json{{"id", 12}, {"type", 3}}, src, err));
	CHECK_FALSE(FromJson(json{{"id", 70000}, {"type", 0}}, src, err));
	CHECK_FALSE(FromJson(json{{"id", 1}}, src, err));
}

TEST_CASE("A parameter tuple is converted positionally and checked for arity", "[prismatic_mcp]")
{
	using namespace PrismaticMCP::params;
	std::string err;
	std::tuple<TileIndex, bool, uint8_t> t{};
	CHECK(TupleFromJson(json::array({10, true, 3}), t, err));
	CHECK(std::get<0>(t).base() == 10);
	CHECK(std::get<1>(t));
	CHECK(std::get<2>(t) == 3);
	CHECK_FALSE(TupleFromJson(json::array({10, true}), t, err));
	CHECK(err.find("expected 3 params") != std::string::npos);
	CHECK_FALSE(TupleFromJson(json::array({10, 1, 3}), t, err));
	CHECK(err.find("params[1]") != std::string::npos);
	CHECK(TupleSchemas<decltype(t)>().size() == 3);
	CHECK((TupleSupported<std::tuple<TileIndex, bool>>()));
}

TEST_CASE("A reset drops approvals always and waiting decisions only with the world", "[prismatic_mcp]")
{
	/* Every reset drops every approval: consent binds one session of one world. */
	for (auto r : {ResetReason::ServerStopped, ResetReason::TokenRotated, ResetReason::GameModeChanged, ResetReason::WorldReplaced}) {
		CHECK(EffectOf(r).clear_approvals);
		CHECK_FALSE(ReasonText(r).empty());
	}
	/* Posted commands still run in the same world after a stop or a token rotation: keep waiting. */
	CHECK_FALSE(EffectOf(ResetReason::ServerStopped).invalidate_awaiting);
	CHECK_FALSE(EffectOf(ResetReason::TokenRotated).invalidate_awaiting);
	/* The world is gone: their results will never come. */
	CHECK(EffectOf(ResetReason::GameModeChanged).invalidate_awaiting);
	CHECK(EffectOf(ResetReason::WorldReplaced).invalidate_awaiting);
	/* Only a decision still waiting is closed; a verdict already taken stands. */
	CHECK(ClosedByReset("PENDING_OBSERVATION"));
	CHECK_FALSE(ClosedByReset("VERIFIED_SUCCESS"));
	CHECK_FALSE(ClosedByReset("VERIFIED_FAILURE"));
	CHECK_FALSE(ClosedByReset("INVALIDATED"));
}

TEST_CASE("ParseContentLength is case-insensitive and bounded", "[prismatic_mcp]")
{
	CHECK(ParseContentLength("POST /mcp HTTP/1.1\r\nContent-Length: 42\r\n\r\n") == 42u);
	CHECK(ParseContentLength("content-length:7\r\n") == 7u);          // lowercase, no space
	CHECK(ParseContentLength("CONTENT-LENGTH:  128\r\n") == 128u);    // uppercase, extra spaces
	CHECK_FALSE(ParseContentLength("Host: localhost\r\n").has_value()); // absent
	CHECK_FALSE(ParseContentLength("Content-Length: \r\n").has_value()); // present but empty
	CHECK(ParseContentLength("Content-Length: 0\r\n") == 0u);
}

TEST_CASE("ExtractBearer only accepts the Bearer scheme", "[prismatic_mcp]")
{
	CHECK(ExtractBearer("Bearer abc123") == "abc123");
	CHECK(ExtractBearer("bearer abc123") == "abc123");   // case-insensitive scheme
	CHECK(ExtractBearer("Bearer   spaced  ") == "spaced"); // OWS trimmed
	CHECK(ExtractBearer("Basic abc123").empty());         // wrong scheme
	CHECK(ExtractBearer("abc123").empty());               // no scheme
	CHECK(ExtractBearer("").empty());
}

TEST_CASE("ConstantTimeEquals matches semantics of ==", "[prismatic_mcp]")
{
	CHECK(ConstantTimeEquals("deadbeef", "deadbeef"));
	CHECK_FALSE(ConstantTimeEquals("deadbeef", "deadbee0"));
	CHECK_FALSE(ConstantTimeEquals("short", "longer"));    // length mismatch
	CHECK(ConstantTimeEquals("", ""));
}

TEST_CASE("A token fingerprint names the token without containing it", "[prismatic_mcp]")
{
	/* BLAKE2b-256("abc") = bddd813c63423972..., the RFC 7693 test vector, truncated to 8 bytes. */
	CHECK(TokenFingerprint("abc") == "blake2b:bddd813c63423972");
	const std::string token = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
	std::string fp = TokenFingerprint(token);
	CHECK(fp == "blake2b:3083e1ce982abd7f");
	CHECK(fp == TokenFingerprint(token));                       // deterministic
	CHECK(fp.find(token.substr(0, 8)) == std::string::npos);    // no piece of the secret
	CHECK(TokenFingerprint(token.substr(0, 63) + "0") != fp);   // one digit changes it
	CHECK(TokenFingerprint("").empty());
}

TEST_CASE("The default token file is one per port", "[prismatic_mcp]")
{
	CHECK(DefaultTokenFileName(8731) == "prismatic_mcp_token_8731.json");
	CHECK(DefaultTokenFileName(18731) != DefaultTokenFileName(8731));
}

TEST_CASE("Only a tick counter that went back means another world", "[prismatic_mcp]")
{
	using PrismaticMCP::detail::WorldChanged;
	CHECK_FALSE(WorldChanged(0, 0));
	CHECK_FALSE(WorldChanged(100, 100));
	CHECK_FALSE(WorldChanged(100, 250));
	CHECK(WorldChanged(250, 3));   // a new game or a load restarted the counter
}

TEST_CASE("The toolbar menu is one table, described item for item", "[prismatic_mcp]")
{
	namespace M = PrismaticMCP::Menu;
	auto d = M::Describe();
	REQUIRE(d["items"].size() == M::Items().size());
	for (size_t i = 0; i < M::Items().size(); i++) {
		CHECK(d["items"][i]["index"] == i);
		CHECK(d["items"][i]["label"] == std::string{M::Items()[i].label});
		CHECK(!M::Items()[i].label.empty());
		if (M::Items()[i].action == M::Action::Toggle) CHECK(!M::Items()[i].key.empty());
		if (M::Items()[i].action == M::Action::Request) CHECK(!M::Items()[i].arg.empty());
	}
	CHECK(d["hub"].get<std::string>().rfind("http", 0) == 0);
}

TEST_CASE("Submenus derive from the items' groups: one row per group at the top, Back and the group's items inside", "[prismatic_mcp]")
{
	namespace M = PrismaticMCP::Menu;
	auto groups = M::Groups();
	REQUIRE_FALSE(groups.empty());
	/* Item results never collide with the rows that are not items. */
	REQUIRE(M::Items().size() < static_cast<size_t>(M::MENU_BACK_ROW));
	std::set<std::string> labels;
	size_t top_items = 0;
	for (const auto &item : M::Items()) {
		CHECK(labels.insert(std::string{item.label}).second); // a label names one item
		if (item.group.empty()) top_items++;
	}
	auto top = M::Rows();
	CHECK(top.size() == top_items + groups.size());
	std::set<int> group_rows;
	for (const auto &row : top) {
		if (row.result >= M::MENU_GROUP_ROW) group_rows.insert(row.result);
		else CHECK(M::Items()[row.result].group.empty());
	}
	CHECK(group_rows.size() == groups.size());
	for (size_t g = 0; g < groups.size(); g++) {
		CHECK(M::GroupOfRow(M::MENU_GROUP_ROW + static_cast<int>(g)) == groups[g]);
		auto sub = M::Rows(groups[g]);
		REQUIRE(sub.size() >= 2); // Back and at least one item: no empty submenu
		CHECK(sub.front().result == M::MENU_BACK_ROW);
		for (size_t r = 1; r < sub.size(); r++) CHECK(M::Items()[sub[r].result].group == groups[g]);
	}
	CHECK_FALSE(M::GroupOfRow(M::MENU_BACK_ROW).has_value());
	CHECK_FALSE(M::GroupOfRow(M::MENU_GROUP_ROW + static_cast<int>(groups.size())).has_value());
	/* openttd://ui/menu says the same: each item's group and the groups in order. */
	auto d = M::Describe();
	REQUIRE(d["groups"].size() == groups.size());
	for (size_t g = 0; g < groups.size(); g++) CHECK(d["groups"][g] == std::string{groups[g]});
	for (size_t i = 0; i < M::Items().size(); i++) {
		CHECK(d["items"][i]["group"] == (M::Items()[i].group.empty() ? nlohmann::json(nullptr) : nlohmann::json(std::string{M::Items()[i].group})));
	}
}

TEST_CASE("Every menu item's target exists: its tool, console command, window or hub path", "[prismatic_mcp]")
{
	namespace M = PrismaticMCP::Menu;
	std::set<std::string> tools;
	for (const auto &t : PrismaticMCP::View::Tools()) tools.insert(t["name"].get<std::string>());
	for (const auto &t : PrismaticMCP::UI::Tools()) tools.insert(t["name"].get<std::string>());
	for (const auto &t : M::Tools()) tools.insert(t["name"].get<std::string>());
	IConsoleStdLibRegister(); // the console's commands, as the game registers them at start
	for (const auto &item : M::Items()) {
		INFO(std::string{item.label});
		std::string arg{item.arg};
		switch (item.action) {
			case M::Action::Toggle: CHECK(tools.contains(arg)); break;
			case M::Action::Console: CHECK(IConsole::CmdGet(arg.substr(0, arg.find(' '))) != nullptr); break;
			case M::Action::Window: CHECK(arg == "control"); break; // Panel::Show, the one fork window
			case M::Action::Browse: CHECK((arg.rfind("/", 0) == 0 || arg.rfind("panel:/", 0) == 0)); break;
			case M::Action::Request: case M::Action::Ask: CHECK_FALSE(arg.empty()); break;
		}
	}
}

TEST_CASE("The control window is a snapshot of what the console and the menu already read", "[prismatic_mcp]")
{
	namespace M = PrismaticMCP::Menu;
	namespace P = PrismaticMCP::Panel;
	/* The window opens from the menu's first row; a process without an interface opens nothing. */
	REQUIRE(M::Items()[0].action == M::Action::Window);
	CHECK(M::Describe()["items"][0]["action"] == "window");
	CHECK(M::Call("openttd.ui.menu.run", {{"index", 0}})->value("ok", true) == false);
	CHECK_FALSE(P::Show());

	json s = P::Snapshot();
	for (const char *key : {"status", "approvals", "decisions", "requests", "switches", "links"}) CHECK(s.contains(key));
	CHECK(s["status"].is_string());
	CHECK(s["approvals"].size() == PrismaticMCP::PendingApprovals().size());
	/* Switches and links come from the menu's own table, never from a list of the window's. */
	size_t toggles = 0, browses = 0;
	for (const auto &item : M::Items()) {
		toggles += item.action == M::Action::Toggle;
		browses += item.action == M::Action::Browse;
	}
	CHECK(s["switches"].size() == toggles);
	CHECK(s["links"].size() == browses);
	/* A request typed in the game is in the snapshot at once. */
	uint64_t id = M::Queue("watch", "");
	CHECK(P::Snapshot()["requests"].back()["id"] == id);

	/* openttd.ui.control answers the same snapshot, read-only. */
	auto tool = M::Call("openttd.ui.control", json::object());
	REQUIRE(tool.has_value());
	CHECK(tool->value("ok", false));
	CHECK((*tool)["approvals"] == P::Snapshot()["approvals"]);
	CHECK(P::Tool()["annotations"]["readOnlyHint"] == true);
}

TEST_CASE("The control window lists every line from its snapshot alone", "[prismatic_mcp]")
{
	namespace P = PrismaticMCP::Panel;
	using K = P::Row::Kind;
	const json s = {
		{"status", "listening on 127.0.0.1:9000"},
		{"approvals", json::array({{{"id", 7}, {"tool", "openttd.game.set_pause"}, {"args", "{\"pause\":true}"}, {"expired", false}},
			{{"id", 9}, {"tool", "openttd.ai.stop"}, {"args", "{}"}, {"expired", true}}})},
		{"decisions", json::array({"decision #1 approval #3 pause result=succeeded verdict=VERIFIED_SUCCESS"})},
		{"requests", json::array({{{"id", 2}, {"kind", "ask"}, {"text", "nearest forest"}, {"answered", true}, {"answer", "Forest at 12x40"}},
			{{"id", 3}, {"kind", "watch"}, {"text", ""}, {"answered", false}, {"answer", nullptr}}})},
		{"switches", json::array({{{"index", 6}, {"label", "Human player: paused"}, {"on", true}}})},
	};
	auto rows = P::Rows(s);
	std::vector<K> kinds;
	for (const auto &r : rows) kinds.push_back(r.kind);
	CHECK(kinds == std::vector<K>{K::Heading, K::Text, K::Heading, K::Approval, K::Approval, K::Heading, K::Text, K::Heading, K::Text,
		K::Heading, K::Text, K::Text, K::Heading, K::Switch});
	CHECK(rows[1].text == "listening on 127.0.0.1:9000");
	CHECK(rows[2].text == "Waiting for your approval (2)");
	/* An approval row carries its id for Approve and Deny, and whether it expired. */
	CHECK((rows[3].id == 7 && !rows[3].flag && rows[3].text == "#7  openttd.game.set_pause  {\"pause\":true}"));
	CHECK((rows[4].id == 9 && rows[4].flag));
	CHECK(rows[5].text == "Standing rules (0)");
	CHECK(rows[8].text.find("VERIFIED_SUCCESS") != std::string::npos);
	CHECK(rows[10].text == "#2 ask: nearest forest - Forest at 12x40");
	CHECK(rows[11].text == "#3 watch - waiting for an answer"); // an unanswered request with a null answer
	CHECK((rows[13].id == 6 && rows[13].flag && rows[13].text == "Human player: paused"));

	/* Nothing waits, nothing ran, nothing asked, no switch: each section says so, no switch heading. */
	auto quiet = P::Rows({{"status", "stopped"}, {"approvals", json::array()}, {"decisions", json::array()},
		{"requests", json::array()}, {"switches", json::array()}});
	REQUIRE(quiet.size() == 10);
	CHECK(quiet[2].text == "Waiting for your approval (0)");
	CHECK(quiet[3].kind == K::Text);
	CHECK(quiet.back().kind == K::Text);

	/* A partial or malformed snapshot (a client's cached copy of a server's queue) never throws. */
	CHECK_NOTHROW(P::Rows(json::object()));
	CHECK_NOTHROW(P::Rows(json(nullptr)));
	CHECK_NOTHROW(P::Rows({{"approvals", "not a list"}, {"decisions", json::array({42})},
		{"requests", json::array({{{"id", -1}, {"answered", true}, {"answer", nullptr}}})}}));
	auto partial = P::Rows({{"approvals", json::array({{{"id", "x"}}})}, {"decisions", json::array({42})}});
	CHECK(partial[1].text == "status unknown");
	CHECK((partial[3].kind == K::Approval && partial[3].id == 0)); // an id that is not a number selects nothing real
	CHECK(partial[7].text == "42");
	/* The snapshot the server builds lists the same way. */
	CHECK_FALSE(P::Rows(P::Snapshot()).empty());
}

TEST_CASE("Only the owner's own click consents in the control window", "[prismatic_mcp]")
{
	namespace P = PrismaticMCP::Panel;
	namespace U = PrismaticMCP::UI;
	CHECK_FALSE(U::InjectingInput());
	CHECK_FALSE(P::ConsentRefusal(true).has_value());
	CHECK_FALSE(P::ConsentRefusal(false).has_value());
	{
		U::InjectionScope script; // as an openttd.input.* tool does around the input it feeds
		CHECK(U::InjectingInput());
		CHECK(P::ConsentRefusal(true).has_value());   // a script never approves
		CHECK_FALSE(P::ConsentRefusal(false).has_value()); // withdrawing a request is anyone's
		{
			U::InjectionScope nested(false);
			CHECK(U::InjectingInput()); // an inner scope never clears an outer one
		}
	}
	CHECK_FALSE(U::InjectingInput());
	/* Consent goes through the console's own command, the path the owner types. */
	CHECK(P::ConsentCommand(true, 7) == "prismatic_mcp approve 7 --via window");
	CHECK(P::ConsentCommand(false, 12) == "prismatic_mcp deny 12 --via window");
	CHECK(P::ConsentCommand(true, std::vector<uint64_t>{3, 4, 5}) == "prismatic_mcp approve 3 4 5 --via window");
}

TEST_CASE("A network client reads the server's approval queue from its '[approvals]' replies", "[prismatic_mcp]")
{
	ServerApprovalsReader r;
	CHECK_FALSE(r.Feed("hello"));                      // ordinary chat is not the queue
	CHECK_FALSE(r.Feed("[approvals] not json"));       // nor is a line that does not parse
	CHECK_FALSE(r.Feed("[approvals] [1, 2]"));         // nor JSON that is not an object
	CHECK(r.Feed(R"([approvals] {"id": 22, "tool": "ai.set_setting", "args": "{\"company\":0}", "chat_decidable": true})"));
	CHECK(r.Feed(R"([approvals] {"id": 23, "tool": "ai.start", "chat_decidable": false, "args_truncated": true})"));
	CHECK(r.rows.empty());                             // a reply counts once it is whole
	CHECK(r.replies == 0);
	CHECK(r.Feed(R"([approvals] {"count": 12, "page": 1, "pages": 1})"));
	REQUIRE(r.rows.size() == 2);
	CHECK(r.rows[0]["id"] == 22);
	CHECK(r.rows[1]["tool"] == "ai.start");
	CHECK(r.count == 12);                              // the count the server says, whatever the rows
	CHECK(r.replies == 1);
	/* The next whole reply replaces the queue; an empty one empties it. */
	CHECK(r.Feed(R"([approvals] {"count": 0, "page": 1, "pages": 1})"));
	CHECK(r.rows.empty());
	CHECK(r.count == 0);
	CHECK(r.replies == 2);
	CHECK_FALSE(r.Feed(R"([approvals] {"page": 1})")); // neither a row nor the count

	/* Not a network client here: the window keeps its own process's queue, and every reply line is
	 * printed (none is swallowed), since this process asked for none. */
	namespace P = PrismaticMCP::Panel;
	CHECK_FALSE(P::OnServerChat("hello"));
	CHECK_FALSE(P::OnServerChat(R"([approvals] {"id": 5, "tool": "ai.set_setting"})"));
	CHECK_FALSE(P::OnServerChat(R"([approvals] {"count": 1, "page": 1, "pages": 1})"));

	/* A server's approval that a chat approver may not decide says who decides it. */
	json snap = {{"approvals", json::array({{{"id", 9}, {"tool", "ai.start"}, {"args", "{}"}, {"on_server", true}, {"chat_decidable", false}},
		{{"id", 10}, {"tool", "ai.set_setting"}, {"args", "{}"}, {"on_server", true}, {"chat_decidable", true}}})}};
	std::vector<P::Row> rows = P::Rows(snap);
	auto approval = [&rows](uint64_t id) { return std::ranges::find_if(rows, [id](const P::Row &r) { return r.kind == P::Row::Kind::Approval && r.id == id; }); };
	REQUIRE(approval(9) != rows.end());
	CHECK(approval(9)->text.ends_with("(the server's operator decides)"));
	REQUIRE(approval(10) != rows.end());
	CHECK_FALSE(approval(10)->text.ends_with("(the server's operator decides)"));
}

TEST_CASE("A request is queued, listed with its kind and answered once", "[prismatic_mcp]")
{
	namespace M = PrismaticMCP::Menu;
	size_t before = M::Requests()["requests"].size();
	uint64_t id = M::Queue("serve_here", "");
	auto list = M::Requests()["requests"];
	REQUIRE(list.size() == before + 1);
	CHECK(list.back()["id"] == id);
	CHECK(list.back()["kind"] == "serve_here");
	CHECK(list.back()["answered"] == false);
	CHECK(list.back()["tile"].is_null()); // no main window in a unit test
	CHECK(M::Call("openttd.ui.requests.answer", {{"id", id}})->value("ok", true) == false); // no answer text
	CHECK(M::Call("openttd.ui.requests.answer", {{"id", id + 1000}, {"answer", "x"}})->value("ok", true) == false);
	CHECK(M::Call("openttd.ui.menu.run", {{"index", -1}})->value("ok", true) == false);
	CHECK_FALSE(M::Call("openttd.ui.something.else", json::object()).has_value());

	/* Ask() queues the same way under the kind "ask", and that string is a cross-process
	 * interface: rhai/ask_relay.rhai skips every queued request whose kind is not "ask", so a
	 * rename here is otherwise caught only by tools/research/mcp-ask-check and a whole stack. */
	size_t before_ask = M::Requests()["requests"].size();
	uint64_t ask_id = M::Ask("build the most lucrative line");
	CHECK(ask_id != 0);
	auto asked = M::Requests()["requests"];
	REQUIRE(asked.size() == before_ask + 1);
	CHECK(asked.back()["id"] == ask_id);
	CHECK(asked.back()["kind"] == "ask");
	CHECK(asked.back()["text"] == "build the most lucrative line");
	CHECK(M::Ask("") == 0); // nothing to ask: no request is queued
	CHECK(M::Requests()["requests"].size() == before_ask + 1);

	/* A coverage probe must not run the menu's tools, which act; only the control snapshot reads. */
	for (const auto &t : M::Tools()) CHECK(t["annotations"]["readOnlyHint"] == (t["name"] == "openttd.ui.control"));
}

/* ---- the catalogue as console commands (prismatic_mcp_console.cpp) ---------------------------- */

namespace {
using njson = nlohmann::json;
namespace mc = PrismaticMCP::Console::detail;

const std::vector<std::string> NAMES = {"openttd.view.zoom", "openttd.view.state", "openttd.ui.windows",
	"openttd.input.pointer", "openttd.decision.submit", "openttd.game.set_pause"};

njson ZoomSchema()
{
	return {{"type", "object"}, {"properties", {{"level", {{"type", "integer"}}}}}, {"required", {"level"}}};
}

njson Args(const njson &schema, std::vector<std::string_view> tokens, std::vector<std::string> &errors)
{
	return mc::ParseArgs(schema, tokens, errors);
}
} // namespace

TEST_CASE("Console names resolve exactly, without the prefix or by a unique suffix", "[prismatic_mcp]")
{
	CHECK(mc::Resolve(NAMES, "openttd.view.zoom") == std::vector<std::string>{"openttd.view.zoom"});
	CHECK(mc::Resolve(NAMES, "view.zoom") == std::vector<std::string>{"openttd.view.zoom"});
	CHECK(mc::Resolve(NAMES, "zoom") == std::vector<std::string>{"openttd.view.zoom"});
	CHECK(mc::Resolve(NAMES, "set_pause") == std::vector<std::string>{"openttd.game.set_pause"});
	CHECK(mc::Resolve({"openttd://company/finances", "openttd://map"}, "map") == std::vector<std::string>{"openttd://map"});
	/* Two suffix matches: candidates, never a guess. */
	CHECK(mc::Resolve({"openttd.a.state", "openttd.b.state"}, "state").size() == 2);
	/* A lone substring match is offered, not run: the second entry is the empty marker. */
	auto sub = mc::Resolve(NAMES, "windo");
	REQUIRE(sub.size() == 2);
	CHECK(sub[0] == "openttd.ui.windows");
	CHECK(sub[1].empty());
	CHECK(mc::Resolve(NAMES, "nothing").empty());
	CHECK(mc::Resolve(NAMES, "").empty());
}

TEST_CASE("Console values are typed by the schema property", "[prismatic_mcp]")
{
	std::string e;
	njson b = {{"type", "boolean"}};
	for (auto t : {"true", "1", "yes", "on"}) { CHECK(mc::ParseValue(b, t, e) == true); CHECK(e.empty()); }
	for (auto f : {"false", "0", "no", "off"}) CHECK(mc::ParseValue(b, f, e) == false);
	mc::ParseValue(b, "maybe", e);
	CHECK_FALSE(e.empty());

	njson i = {{"type", "integer"}};
	CHECK(mc::ParseValue(i, "-42", e) == -42);
	mc::ParseValue(i, "1.5", e);
	CHECK_FALSE(e.empty());
	mc::ParseValue(i, "12abc", e);
	CHECK_FALSE(e.empty());

	njson nullable = {{"type", {"integer", "null"}}};
	CHECK(mc::ParseValue(nullable, "null", e).is_null());
	CHECK(e.empty());
	CHECK(mc::ParseValue(nullable, "7", e) == 7);

	CHECK(mc::ParseValue({{"type", "number"}}, "2.5", e) == 2.5);
	CHECK(mc::ParseValue({{"type", "string"}}, "a b", e) == "a b");
	CHECK(mc::ParseValue({{"type", "array"}, {"items", {{"type", "integer"}}}}, "1,2,3", e) == njson::array({1, 2, 3}));
	mc::ParseValue({{"type", "array"}, {"items", {{"type", "integer"}}}}, "1,x", e);
	CHECK_FALSE(e.empty());
	CHECK(mc::ParseValue({{"type", "array"}}, "[1,\"a\"]", e) == njson::array({1, "a"}));
	CHECK(mc::ParseValue({{"type", "object"}}, "{\"k\":1}", e) == njson{{"k", 1}});

	njson en = {{"type", "string"}, {"enum", {"rail", "road"}}};
	CHECK(mc::ParseValue(en, "rail", e) == "rail");
	mc::ParseValue(en, "air", e);
	CHECK_FALSE(e.empty());

	/* No type: JSON when it parses, else the text. */
	CHECK(mc::ParseValue(njson::object(), "3", e) == 3);
	CHECK(mc::ParseValue(njson::object(), "hello", e) == "hello");
}

TEST_CASE("Console arguments: key=value, bare values for required keys, refusals", "[prismatic_mcp]")
{
	std::vector<std::string> errors;
	CHECK(Args(ZoomSchema(), {"level=2"}, errors) == njson{{"level", 2}});
	CHECK(errors.empty());
	CHECK(Args(ZoomSchema(), {"3"}, errors) == njson{{"level", 3}});
	CHECK(errors.empty());

	Args(ZoomSchema(), {}, errors);
	REQUIRE(errors.size() == 1);
	CHECK(errors[0].find("missing required key 'level'") != std::string::npos);

	errors.clear();
	Args(ZoomSchema(), {"level=2", "speed=9"}, errors);
	REQUIRE(errors.size() == 1);
	CHECK(errors[0].find("unknown key 'speed'") != std::string::npos);

	errors.clear();
	Args(ZoomSchema(), {"level=high"}, errors);
	REQUIRE(errors.size() == 2);   // mistyped, and therefore still missing
	CHECK(errors[0].find("level:") == 0);

	errors.clear();
	Args(ZoomSchema(), {"2", "3"}, errors);
	CHECK(errors.size() == 1);   // no required key left for the second bare value

	/* A JSON object token is merged; an open schema accepts any key, typed by JSON. */
	errors.clear();
	njson open = {{"type", "object"}};
	CHECK(Args(open, {"{\"action\":{\"type\":\"game.set_pause\"}}", "note=x"}, errors) ==
		njson{{"action", {{"type", "game.set_pause"}}}, {"note", "x"}});
	CHECK(errors.empty());

	/* anyOf: one complete set of required keys is enough. */
	njson either = {{"type", "object"}, {"properties", {{"tile", {{"type", "integer"}}}, {"x", {{"type", "integer"}}}, {"y", {{"type", "integer"}}}}},
		{"anyOf", njson::array({njson{{"required", {"tile"}}}, njson{{"required", {"x", "y"}}}})}};
	Args(either, {"x=1", "y=2"}, errors);
	CHECK(errors.empty());
	Args(either, {"tile=5"}, errors);
	CHECK(errors.empty());
	Args(either, {"x=1"}, errors);
	REQUIRE(errors.size() == 1);
	CHECK(errors[0] == "needs tile or x+y");
}

TEST_CASE("Console usage lines and result lines are derived, not written", "[prismatic_mcp]")
{
	CHECK(mc::Usage("openttd.view.zoom", ZoomSchema()) == "mcp openttd.view.zoom level=<integer>");
	njson s = {{"type", "object"}, {"properties", {{"mode", {{"type", "string"}, {"enum", {"a", "b"}}}}, {"pause", {{"type", "boolean"}}}}}, {"required", {"pause"}}};
	CHECK(mc::Usage("t", s) == "mcp t pause=<boolean> [mode=a|b]");
	CHECK(mc::Usage("t", njson{{"type", "object"}}) == "mcp t [{json}]");

	njson result = {{"content", njson::array({{{"type", "text"}, {"text", "{\"ok\":true,\"n\":1}"}}})}};
	CHECK(mc::ResultLines(result, true) == std::vector<std::string>{"{\"n\":1,\"ok\":true}"});
	CHECK(mc::ResultLines(result, false).size() == 4);   // { "n": 1, "ok": true }
	njson plain = {{"contents", njson::array({{{"text", "a\nb"}}})}};
	CHECK(mc::ResultLines(plain, false) == std::vector<std::string>{"a", "b"});
}

TEST_CASE("Console completion is derived from the catalogue, with or without a leading slash", "[prismatic_mcp]")
{
	const json tools = json::array({
		{{"name", "openttd.view.zoom"}, {"inputSchema", {{"type", "object"}, {"properties", {{"level", {{"type", "integer"}}}}}}}},
		{{"name", "openttd.cmd.Pause"}, {"inputSchema", {{"type", "object"}, {"properties", {{"company", json::object()}, {"params", json::object()}}}}}},
	});
	const json resources = json::array({{{"uri", "openttd://game/state"}}, {{"uri", "openttd://commands"}}});
	auto complete = [&](std::vector<std::string_view> words, std::string_view query) { return mc::Complete(tools, resources, words, query); };

	/* After mcp: its subcommands, then every tool without the openttd. prefix. */
	auto all = complete({"mcp"}, "");
	CHECK(std::find(all.begin(), all.end(), "read") != all.end());
	CHECK(std::find(all.begin(), all.end(), "cmd.Pause") != all.end());
	CHECK(complete({"mcp"}, "vi") == std::vector<std::string>{"view.zoom"});
	CHECK(complete({"/mcp"}, "vi") == std::vector<std::string>{"view.zoom"}); // the slash changes nothing
	/* After mcp read, the resources; after mcp help, the tools. */
	CHECK(complete({"mcp", "read"}, "openttd://c") == std::vector<std::string>{"openttd://commands"});
	CHECK(complete({"mcp", "help"}, "cmd") == std::vector<std::string>{"cmd.Pause"});
	/* After a tool, its parameters not given yet, as key=. */
	CHECK(complete({"mcp", "cmd.Pause"}, "") == std::vector<std::string>{"company=", "params="});
	CHECK(complete({"mcp", "cmd.Pause", "company=server"}, "") == std::vector<std::string>{"params="});
	CHECK(complete({"mcp", "zoom"}, "l") == std::vector<std::string>{"level="});
	CHECK(complete({"mcp", "no_such_tool"}, "").empty());
	/* bot and its alias ai: the subcommands. */
	CHECK(complete({"bot"}, "s") == std::vector<std::string>{"status", "set", "start", "stop"});
	CHECK(complete({"/ai"}, "re") == std::vector<std::string>{"reload"});
	/* Anything else is not the fork's to complete. */
	CHECK(complete({"echo"}, "").empty());
	CHECK(complete({}, "mc").empty());
}

TEST_CASE("A dedicated server has no toolbar: of the menu only the control snapshot", "[prismatic_mcp]")
{
	namespace M = PrismaticMCP::Menu;
	const bool was = _network_dedicated;
	_network_dedicated = true;
	/* What the control window shows stays readable: the approvals live on a dedicated server. */
	REQUIRE(M::Tools().size() == 1);
	CHECK(M::Tools()[0]["name"] == "openttd.ui.control");
	CHECK(M::Call("openttd.ui.control", json::object())->value("ok", false));
	CHECK(M::Call("openttd.ui.menu.run", {{"index", 0}})->value("ok", true) == false);
	CHECK(M::Call("openttd.ui.requests.answer", {{"id", 1}, {"answer", "x"}})->value("ok", true) == false);
	CHECK(M::Describe()["items"].empty());
	CHECK(M::Describe()["dedicated"] == true);
	CHECK(M::Requests()["requests"].empty());
	_network_dedicated = was;
	CHECK_FALSE(M::Tools().empty());
}

TEST_CASE("Every price is named once, in the game's order", "[prismatic_mcp]")
{
	auto p = PrismaticMCP::World::Prices();
	REQUIRE(p["order"].size() == to_underlying(Price::End));
	std::set<std::string> seen;
	for (const auto &n : p["order"]) {
		std::string name = n.get<std::string>();
		CHECK(seen.insert(name).second);
		CHECK(name.find_first_not_of("abcdefghijklmnopqrstuvwxyz_") == std::string::npos);
		CHECK(p["prices"].contains(name));
		CHECK(p["base"].contains(name));
	}
	CHECK(p["order"][to_underlying(Price::BuildRail)] == "build_rail");
	CHECK(p["order"][to_underlying(Price::BuildBridge)] == "build_bridge");
	CHECK(p["order"][to_underlying(Price::BuildTunnel)] == "build_tunnel");
	CHECK(p["order"][to_underlying(Price::ClearTrees)] == "clear_trees");
}

TEST_CASE("The terrain shows a raised corner in the two rows it touches, and nowhere else", "[prismatic_mcp]")
{
	Map::Allocate(64, 64);
	auto flat = PrismaticMCP::World::Terrain();
	REQUIRE(flat["slopes"].size() == 64);
	CHECK(flat["slopes"][10].get<std::string>() == std::string(128, '0'));
	CHECK(flat["ground"][10].get<std::string>() == std::string(64, 'b')); // a zeroed clear tile: grass of density 0
	CHECK(flat["buildable"][10].get<std::string>() == std::string(64, '.'));
	CHECK(flat["vehicles"][10].get<std::string>() == std::string(64, '.'));
	CHECK(flat["gradients"][10].get<std::string>() == std::string(64, '0'));

	SetTileHeight(TileXY(10, 10), 1);
	auto raised = PrismaticMCP::World::Terrain();
	/* The height of tile (10,10) is the north corner of (10,10), the west of (9,10)... : rows 9 and 10 change. */
	for (uint y = 0; y < 64; y++) {
		bool changed = raised["row_digests"][y] != flat["row_digests"][y];
		CHECK(changed == (y == 9 || y == 10));
	}
	CHECK(raised["slopes"][10].get<std::string>().substr(20, 2) != "00");
	CHECK(raised["gradients"][10].get<std::string>()[10] == '1');
	CHECK(raised["gradients"][10].get<std::string>()[11] == '1');
	CHECK(raised["gradients"][10].get<std::string>()[12] == '0');
	CHECK(raised["gradients"][9].get<std::string>()[10] == '1');
}


TEST_CASE("The owner switch auto-answers the bot's question with no dialog", "[prismatic_mcp]")
{
	namespace V = PrismaticMCP::View;
	// Off by default: turning it on with "yes" reports the state.
	auto on = V::Call("openttd.view.ask.auto", {{"answer", "yes"}});
	REQUIRE(on.has_value());
	CHECK((*on)["auto"] == true);
	CHECK((*on)["answer"] == "yes");

	// With it on, a question is answered at once and shows no dialog (no ShowQuery, so safe in a test).
	auto asked = V::Call("openttd.view.ask", {{"question", "build here?"}});
	REQUIRE(asked.has_value());
	CHECK((*asked)["ok"] == true);
	CHECK((*asked)["auto_answered"] == true);
	CHECK((*asked)["answer"] == "yes");

	// Spellings normalise: "off" means the answer "no".
	V::Call("openttd.view.ask.auto", {{"answer", "off"}});
	CHECK((*V::Call("openttd.view.ask", {{"question", "again?"}}))["answer"] == "no");

	// Turning it off clears the switch (a later openttd.view.ask would show the dialog again).
	auto off = V::Call("openttd.view.ask.auto", json::object());
	REQUIRE(off.has_value());
	CHECK((*off)["auto"] == false);
	CHECK((*off)["answer"].is_null());

	// The tool is listed, so it reaches the `mcp` console, Swagger and .ai by derivation.
	bool listed = false;
	for (const auto &t : V::Tools()) if (t["name"] == "openttd.view.ask.auto") listed = true;
	CHECK(listed);
}

TEST_CASE("The owner switch with a countdown confirms after the countdown, and its input is checked", "[prismatic_mcp]")
{
	namespace V = PrismaticMCP::View;
	// A countdown alone means "yes after N seconds".
	auto on = V::Call("openttd.view.ask.auto", {{"countdown", 10}});
	REQUIRE(on.has_value());
	CHECK((*on)["ok"] == true);
	CHECK((*on)["auto"] == true);
	CHECK((*on)["answer"] == "yes");
	CHECK((*on)["countdown"] == 10);

	// With an answer it counts down to that answer.
	auto no = V::Call("openttd.view.ask.auto", {{"answer", "no"}, {"countdown", 5}});
	CHECK((*no)["answer"] == "no");
	CHECK((*no)["countdown"] == 5);

	// A process with no window to count down in (this test, a dedicated server) answers at once.
	auto asked = V::Call("openttd.view.ask", {{"question", "build here?"}});
	REQUIRE(asked.has_value());
	CHECK((*asked)["auto_answered"] == true);
	CHECK((*asked)["answer"] == "no");

	// Out of range or mistyped countdowns are refused, and leave the switch as it was.
	CHECK((*V::Call("openttd.view.ask.auto", {{"countdown", 601}}))["ok"] == false);
	CHECK((*V::Call("openttd.view.ask.auto", {{"countdown", -1}}))["ok"] == false);
	CHECK((*V::Call("openttd.view.ask.auto", {{"countdown", "ten"}}))["ok"] == false);
	CHECK((*V::Call("openttd.view.ask.auto", {{"answer", 3}}))["ok"] == false);
	CHECK((*V::Call("openttd.view.ask", {{"question", "still no?"}}))["answer"] == "no");

	// The schema declares the countdown, so the console and Swagger offer it.
	for (const auto &t : V::Tools()) {
		if (t["name"] != "openttd.view.ask.auto") continue;
		CHECK(t["inputSchema"]["properties"]["countdown"]["type"] == "integer");
		CHECK(t["inputSchema"]["properties"]["countdown"]["maximum"] == 600);
	}

	// Off: no answer and no countdown.
	auto off = V::Call("openttd.view.ask.auto", json::object());
	CHECK((*off)["auto"] == false);
	CHECK((*off)["countdown"] == 0);
}

TEST_CASE("Every map resource is one row per map row, sized to the map", "[prismatic_mcp]")
{
	Map::Allocate(64, 128); // the smallest width, a taller height: rows follow y
	auto m = PrismaticMCP::World::Map();
	CHECK(m["width"] == 64);
	CHECK(m["height"] == 128);
	REQUIRE(m["types"].size() == 128);
	CHECK(m["types"][5].get<std::string>() == std::string(64, 'C')); // a zeroed tile is clear land
	CHECK(m["heights"][5].get<std::string>() == std::string(128, '0')); // two hex digits per tile
	CHECK(m["owners"][5].get<std::string>().size() == 64);
	for (const char *layer : {"types", "heights", "owners"}) CHECK(m["encoding"].contains(layer));

	auto w = PrismaticMCP::Graph::Ways();
	CHECK(w["schema"] == "map-ways/v1");
	REQUIRE(w["roads"].size() == 128);
	CHECK(w["roads"][0].get<std::string>() == std::string(64, '0'));     // no road anywhere
	CHECK(w["rails"][0].get<std::string>() == std::string(128, '0'));    // no track anywhere
	CHECK(w["foundations"][0].get<std::string>().size() == 64);
	CHECK(w["crossings"].empty());
	CHECK(w["occupancy"].empty());

	auto t = PrismaticMCP::World::Terrain();
	for (const char *layer : {"ground", "gradients", "buildable", "vehicles"}) CHECK(t[layer][127].get<std::string>().size() == 64);
	CHECK(t["slopes"][127].get<std::string>().size() == 128);
	CHECK(t["row_digests"].size() == 128);
}

TEST_CASE("The building and vehicle lists are empty when nothing is built", "[prismatic_mcp]")
{
	Map::Allocate(64, 64);
	CHECK(PrismaticMCP::World::Stations()["stations"].empty());
	auto d = PrismaticMCP::World::Depots();
	CHECK(d["depots"].empty());
	CHECK(d["service_reach_tiles"].is_number());
	CHECK(PrismaticMCP::Entities::Vehicles()["vehicles"].empty());
}

TEST_CASE("Every client tool says whether it changes anything", "[prismatic_mcp]")
{
	/* Clients and the coverage probe decide by readOnlyHint: a tool without it is never probed. */
	std::vector<nlohmann::json> tools;
	for (const auto &t : PrismaticMCP::View::Tools()) tools.push_back(t);
	for (const auto &t : PrismaticMCP::UI::Tools()) tools.push_back(t);
	for (const auto &t : PrismaticMCP::Menu::Tools()) tools.push_back(t);
	tools.push_back(ReadManyTool());
	REQUIRE_FALSE(tools.empty());
	for (const auto &t : tools) {
		INFO(t["name"].get<std::string>());
		REQUIRE(t.contains("annotations"));
		CHECK(t["annotations"].contains("readOnlyHint"));
		CHECK(t["annotations"]["readOnlyHint"].is_boolean());
	}
}

TEST_CASE("Status capabilities are derived from the tool registry and the process's arming", "[prismatic_mcp]")
{
	const json reader = {{"readOnlyHint", true}};
	const json mutator = {{"readOnlyHint", false}};
	auto field = [](std::string_view f) {
		for (size_t i = 0; i < MUTATING_CAPABILITIES.size(); i++) if (MUTATING_CAPABILITIES[i].field == f) return i;
		FAIL("no capability " << f);
		return size_t{0};
	};

	/* A registry with a tool of every mutating kind: not read-only, and each kind is named. */
	json mutating = json::array({
		{{"name", "openttd.cmd.Pause"}}, // no annotations: counts as mutating, as the server's own pass would annotate it
		{{"name", "openttd.api.ScriptCompany.SetLoanAmount"}, {"annotations", mutator}},
		{{"name", "openttd.ai.list"}, {"annotations", reader}}, // a reader under a mutating prefix makes nothing
		{{"name", "openttd.ai.stop"}, {"annotations", mutator}},
		{{"name", "openttd.game.set_pause"}, {"annotations", mutator}},
		{{"name", "openttd.ui.menu.run"}, {"annotations", mutator}}, // the toolbar menu, not interface input
		{{"name", "openttd.input.pointer"}, {"annotations", mutator}},
		{{"name", "openttd.view.zoom"}, {"annotations", mutator}}, // the local viewport: no capability
	});
	Capabilities all = CapabilitiesOf(mutating, 0, true);
	CHECK(all.reads);
	CHECK_FALSE(all.ReadOnly());
	for (const auto &r : MUTATING_CAPABILITIES) {
		INFO(r.field);
		CHECK(all.mutating[field(r.field)]);
	}
	json j = CapabilitiesJson(all);
	CHECK(j.size() == MUTATING_CAPABILITIES.size() + 1);
	CHECK(j["reads"] == true);
	CHECK(j["approval_gated_commands"] == true);
	CHECK(j["interface_input"] == true);
	CHECK(CapabilitiesLine(all) == "read_only=false capabilities=reads,approval_gated_commands,script_api,ai_tools,pause,toolbar_menu,interface_input input_armed=true");

	/* The same registry in an unarmed process: the input tools are there but cannot run. */
	Capabilities unarmed_all = CapabilitiesOf(mutating, 0, false);
	CHECK_FALSE(unarmed_all.mutating[field("interface_input")]);
	CHECK(unarmed_all.mutating[field("approval_gated_commands")]);
	CHECK_FALSE(unarmed_all.ReadOnly());

	/* A registry with no mutating tool is read-only; reads come from a reader tool or a resource. */
	json readers = json::array({
		{{"name", "openttd.decision.validate"}, {"annotations", reader}},
		{{"name", "openttd.ui.windows"}, {"annotations", reader}},
	});
	Capabilities ro = CapabilitiesOf(readers, 0, false);
	CHECK(ro.ReadOnly());
	CHECK(ro.reads);
	CHECK(CapabilitiesJson(ro)["approval_gated_commands"] == false);
	CHECK(CapabilitiesLine(ro) == "read_only=true capabilities=reads input_armed=false");
	CHECK(CapabilitiesOf(json::array(), 3, false).reads);
	CHECK(CapabilitiesLine(CapabilitiesOf(json::array(), 0, false)) == "read_only=true capabilities=none input_armed=false");

	/* The real interface tools in an unarmed process (this test binary is not started with
	 * PRISMATIC_MCP_INPUT=1): no interface input, so read-only; armed, they are interface input. */
	REQUIRE_FALSE(PrismaticMCP::UI::InputArmed());
	json ui = json::array();
	for (const auto &t : PrismaticMCP::View::Tools()) ui.push_back(t);
	for (const auto &t : PrismaticMCP::UI::Tools()) ui.push_back(t);
	REQUIRE_FALSE(ui.empty());
	Capabilities unarmed = CapabilitiesOf(ui, 0, PrismaticMCP::UI::InputArmed());
	CHECK(unarmed.ReadOnly());
	CHECK_FALSE(unarmed.input_armed);
	CHECK_FALSE(unarmed.mutating[field("interface_input")]);
	Capabilities armed = CapabilitiesOf(ui, 0, true);
	CHECK(armed.mutating[field("interface_input")]);
	CHECK_FALSE(armed.ReadOnly());
}

TEST_CASE("openttd://mcp/status and the status line report the capabilities the server registers", "[prismatic_mcp]")
{
	json status = PrismaticMCP::Dispatch({{"jsonrpc", "2.0"}, {"id", 1}, {"method", "resources/read"}, {"params", {{"uri", "openttd://mcp/status"}}}});
	REQUIRE(status.contains("result"));
	json d = json::parse(status["result"]["contents"][0]["text"].get<std::string>())["data"];
	json tools = PrismaticMCP::Dispatch({{"jsonrpc", "2.0"}, {"id", 2}, {"method", "tools/list"}})["result"]["tools"];
	json resources = PrismaticMCP::Dispatch({{"jsonrpc", "2.0"}, {"id", 3}, {"method", "resources/list"}})["result"]["resources"];
	Capabilities expected = CapabilitiesOf(tools, resources.size(), PrismaticMCP::UI::InputArmed());

	/* The literal is gone: this build registers approval-gated command tools, so it is not read-only. */
	CHECK(d["capabilities"] == CapabilitiesJson(expected));
	CHECK(d["capabilities"]["reads"] == true);
	CHECK(d["capabilities"]["approval_gated_commands"] == true);
	CHECK(d["capabilities"]["pause"] == true);
	CHECK(d["capabilities"]["interface_input"] == false);
	CHECK(d["read_only"] == false);
	CHECK(d["read_only"] == expected.ReadOnly());
	CHECK(d["input_armed"] == false);

	/* The console prints the same derived values, whether or not the server runs. */
	REQUIRE_FALSE(PrismaticMCP::IsRunning());
	CHECK(PrismaticMCP::GetStatusLine() == "prismatic_mcp stopped " + CapabilitiesLine(expected));
}

TEST_CASE("Several resources at one tick: the call is bounded, and a bad uri fails only its own entry", "[prismatic_mcp]")
{
	auto known = [](const std::string &u) { return u == "openttd://towns" || u == "openttd://map"; };

	/* The tool reads only, and its schema says the cap and the scheme the plan enforces. */
	json tool = ReadManyTool();
	CHECK(tool["name"] == "openttd.resources.read_many");
	CHECK(tool["annotations"]["readOnlyHint"] == true);
	CHECK(tool["inputSchema"]["required"] == json::array({"uris"}));
	CHECK(tool["inputSchema"]["properties"]["uris"]["maxItems"] == READ_MANY_MAX_URIS);
	CHECK(tool["inputSchema"]["properties"]["uris"]["minItems"] == 1);

	/* Malformed arguments refuse the whole call, before anything is read. */
	for (const json &bad : {json::object(), json::array(), json{{"uris", "openttd://towns"}}, json{{"uris", json::array()}}}) {
		INFO(bad.dump());
		ReadManyPlan p = PlanReadMany(bad, known);
		CHECK_FALSE(p.error.empty());
		CHECK(p.entries.empty());
	}

	/* The cap: exactly READ_MANY_MAX_URIS is planned, one more is refused as a whole. */
	json at_cap = {{"uris", json::array()}};
	for (size_t i = 0; i < READ_MANY_MAX_URIS; i++) at_cap["uris"].push_back(fmt::format("openttd://x{}", i));
	ReadManyPlan full = PlanReadMany(at_cap, known);
	CHECK(full.error.empty());
	CHECK(full.entries.size() == READ_MANY_MAX_URIS);
	at_cap["uris"].push_back("openttd://towns");
	ReadManyPlan over = PlanReadMany(at_cap, known);
	CHECK(over.error.find("at most") != std::string::npos);
	CHECK(over.entries.empty());

	/* Per entry, in the order asked: known ones are read; the rest say why, and the call still goes on. */
	json args = {{"uris", json::array({"openttd://towns", "openttd://nope", 7, "https://example.com/", "openttd://map", "openttd://towns"})}};
	ReadManyPlan p = PlanReadMany(args, known);
	REQUIRE(p.error.empty());
	REQUIRE(p.entries.size() == 6);
	CHECK(p.entries[0].uri == "openttd://towns");
	CHECK(p.entries[0].error.empty());
	CHECK(p.entries[1].error == "unknown resource uri");
	CHECK(p.entries[2].uri == "7");
	CHECK(p.entries[2].error == "not a string");
	CHECK(p.entries[3].error == "only openttd:// resources");
	CHECK(p.entries[4].error.empty());
	CHECK(p.entries[5].error.starts_with("duplicate"));
}

TEST_CASE("The approval state machine binds to the tool and canonical args, executes once, refuses replay, changed args and another tool", "[prismatic_mcp]")
{
	/* ClassifySubmit is exactly what the submit handler runs to decide pending/reuse/execute. An
	 * approval binds to the submission's action (tool) AND its EXACT canonical arguments, not the
	 * 64-bit FNV-1a digest (which could collide). States mirror ApprovalState: 0 Pending,
	 * 1 Approved, 3 Consumed. */
	const std::string pause = "game.set_pause";
	const std::string pause5 = R"({"ticks":5})";
	const std::string pause6 = R"({"ticks":6})";
	size_t idx = 999;

	SECTION("submit with nothing live asks for approval; that pending approval is then reused") {
		std::vector<ApprovalView> a;
		CHECK(ClassifySubmit(a, pause, pause5, 10, idx) == SubmitDecision::CreatePending);
		a.push_back({0, pause, pause5, 100});
		idx = 999;
		CHECK(ClassifySubmit(a, pause, pause5, 10, idx) == SubmitDecision::ReusePending);
		CHECK(idx == 0);
	}
	SECTION("an approved, unexpired approval executes once for the exact tool and args") {
		std::vector<ApprovalView> a{{1, pause, pause5, 100}};
		CHECK(ClassifySubmit(a, pause, pause5, 10, idx) == SubmitDecision::Execute);
		CHECK(idx == 0);
	}
	SECTION("changed arguments after approval are refused -- a fresh approval is required") {
		std::vector<ApprovalView> a{{1, pause, pause5, 100}};
		CHECK(ClassifySubmit(a, pause, pause6, 10, idx) == SubmitDecision::CreatePending);
	}
	SECTION("a replay of an already-consumed approval is refused") {
		std::vector<ApprovalView> a{{3, pause, pause5, 100}};
		CHECK(ClassifySubmit(a, pause, pause5, 10, idx) == SubmitDecision::CreatePending);
	}
	SECTION("an expired approval does not execute") {
		std::vector<ApprovalView> a{{1, pause, pause5, 5}};
		CHECK(ClassifySubmit(a, pause, pause5, 10, idx) == SubmitDecision::CreatePending);
	}
	SECTION("the binding is the exact args, so two different argument sets never share one approval") {
		std::vector<ApprovalView> a{{1, pause, pause5, 100}};
		CHECK(ClassifySubmit(a, pause, pause5, 10, idx) == SubmitDecision::Execute);
		CHECK(ClassifySubmit(a, pause, pause6, 10, idx) == SubmitDecision::CreatePending);
	}
	SECTION("an approval for one tool is never spent by another tool with the same arguments") {
		/* Audit finding C1: ai.reload and ai.stop both take {company:N}; approving the reload must
		 * not let a stop (which deletes the company) run on that approval. */
		const std::string company3 = R"({"company":3})";
		std::vector<ApprovalView> a{{1, "ai.reload", company3, 100}};
		CHECK(ClassifySubmit(a, "ai.stop", company3, 10, idx) == SubmitDecision::CreatePending);
		std::vector<ApprovalView> p{{0, "ai.reload", company3, 100}};
		CHECK(ClassifySubmit(p, "ai.stop", company3, 10, idx) == SubmitDecision::CreatePending);
		idx = 999;
		CHECK(ClassifySubmit(a, "ai.reload", company3, 10, idx) == SubmitDecision::Execute);
		CHECK(idx == 0);
	}
}

TEST_CASE("Several AI settings are one decision: the shape is checked whole, the expiry clamps the approval, and every map is its own approval", "[prismatic_mcp]")
{
	CHECK(IsKnownActionType("ai.set_settings"));

	SECTION("a well-formed call plans every setting in canonical order, with its expiry") {
		SetSettingsPlan p = PlanSetSettings(json{{"company", 1}, {"settings", {{"fr_to", 7}, {"fr_from", 3}}}, {"expires_tick", 500}});
		CHECK(p.errors.empty());
		REQUIRE(p.settings.size() == 2);
		CHECK(p.settings[0] == std::pair<std::string, int64_t>{"fr_from", 3});
		CHECK(p.settings[1] == std::pair<std::string, int64_t>{"fr_to", 7});
		CHECK(p.expires_tick == std::optional<uint64_t>{500});
		CHECK_FALSE(PlanSetSettings(json{{"company", 1}, {"settings", {{"a", 1}}}}).expires_tick.has_value());
	}
	SECTION("no map, a map that is not an object, and an empty map are refused") {
		for (const json &bad : {json{{"company", 1}}, json{{"company", 1}, {"settings", json::array({1})}}, json{{"company", 1}, {"settings", json::object()}}, json::array()}) {
			INFO(bad.dump());
			SetSettingsPlan p = PlanSetSettings(bad);
			CHECK(p.errors.size() == 1);
			CHECK(p.settings.empty());
		}
		CHECK(PlanSetSettings(json{{"settings", json::object()}}).errors[0].find("empty") != std::string::npos);
	}
	SECTION("the cap: SET_SETTINGS_MAX settings are planned, one more refuses the call") {
		json args = {{"company", 1}, {"settings", json::object()}};
		for (size_t i = 0; i < SET_SETTINGS_MAX; i++) args["settings"][fmt::format("s{:02}", i)] = static_cast<int>(i);
		SetSettingsPlan full = PlanSetSettings(args);
		CHECK(full.errors.empty());
		CHECK(full.settings.size() == SET_SETTINGS_MAX);
		args["settings"]["one_more"] = 1;
		SetSettingsPlan over = PlanSetSettings(args);
		REQUIRE(over.errors.size() == 1);
		CHECK(over.errors[0].find(fmt::format("at most {}", SET_SETTINGS_MAX)) != std::string::npos);
		CHECK(over.settings.empty());
	}
	SECTION("every value that is not an integer is named, and a bad expires_tick is refused") {
		SetSettingsPlan p = PlanSetSettings(json{{"settings", {{"a", 1}, {"b", "2"}, {"c", 1.5}, {"d", true}}}});
		CHECK(p.errors == std::vector<std::string>{"b: value must be an integer", "c: value must be an integer", "d: value must be an integer"});
		CHECK(p.settings.size() == 1);
		for (const json &e : {json(-1), json("100"), json(2.5)}) {
			INFO(e.dump());
			CHECK(PlanSetSettings(json{{"settings", {{"a", 1}}}, {"expires_tick", e}}).errors.size() == 1);
		}
		CHECK(PlanSetSettings(json::parse(R"({"settings":{"a":1},"expires_tick":0})")).errors.empty());
	}
	SECTION("the approval expires at the earlier of the TTL and expires_tick; execution re-checks the date") {
		CHECK(ApprovalExpiry(1000, 100000, std::nullopt) == 101000);
		CHECK(ApprovalExpiry(1000, 100000, 5000) == 5000);
		CHECK(ApprovalExpiry(1000, 100000, 900000) == 101000);
		CHECK(BoundExpiresTick("ai.set_settings", json{{"settings", {{"a", 1}}}, {"expires_tick", 5000}}) == std::optional<uint64_t>{5000});
		CHECK_FALSE(BoundExpiresTick("ai.set_setting", json{{"name", "a"}, {"value", 1}, {"expires_tick", 5000}}).has_value());
		CHECK(StillFresh(5000, 5000));
		CHECK_FALSE(StillFresh(5001, 5000));
		CHECK(StillFresh(1u << 30, std::nullopt));
		/* Approved, but past its own date: the clamped approval no longer executes. */
		const std::string args = json{{"company", 1}, {"settings", {{"a", 1}}}, {"expires_tick", 5000}}.dump();
		std::vector<ApprovalView> a{{1, "ai.set_settings", args, ApprovalExpiry(1000, 100000, 5000)}};
		size_t idx = 999;
		CHECK(ClassifySubmit(a, "ai.set_settings", args, 5000, idx) == SubmitDecision::Execute);
		CHECK(ClassifySubmit(a, "ai.set_settings", args, 5001, idx) == SubmitDecision::CreatePending);
	}
	SECTION("a different map or a different expiry is a different approval; the same map in another key order is the same") {
		const std::string base = json{{"company", 1}, {"settings", {{"fr_from", 3}, {"fr_to", 7}}}, {"expires_tick", 5000}}.dump();
		const std::string reordered = json::parse(R"({"settings":{"fr_to":7,"fr_from":3},"expires_tick":5000,"company":1})").dump();
		const std::string other_value = json{{"company", 1}, {"settings", {{"fr_from", 3}, {"fr_to", 8}}}, {"expires_tick", 5000}}.dump();
		const std::string subset = json{{"company", 1}, {"settings", {{"fr_from", 3}}}, {"expires_tick", 5000}}.dump();
		const std::string later = json{{"company", 1}, {"settings", {{"fr_from", 3}, {"fr_to", 7}}}, {"expires_tick", 9000}}.dump();
		const std::string no_expiry = json{{"company", 1}, {"settings", {{"fr_from", 3}, {"fr_to", 7}}}}.dump();
		std::vector<ApprovalView> a{{1, "ai.set_settings", base, 5000}};
		size_t idx = 999;
		CHECK(ClassifySubmit(a, "ai.set_settings", reordered, 10, idx) == SubmitDecision::Execute);
		for (const std::string &other : {other_value, subset, later, no_expiry}) {
			INFO(other);
			CHECK(ClassifySubmit(a, "ai.set_settings", other, 10, idx) == SubmitDecision::CreatePending);
		}
		/* Nor does one of its settings, sent alone to ai.set_setting, spend it. */
		CHECK(ClassifySubmit(a, "ai.set_setting", base, 10, idx) == SubmitDecision::CreatePending);
	}
}

TEST_CASE("PrismaticMCP: game text reaches MCP clients as plain text")
{
	using PrismaticMCP::PlainText;
	/* A news headline as GetDecodedString returns it: SCC codes (U+E054, U+E006, U+E052, U+E055)
	 * around the words and the game's own blank line. */
	const std::string headline = "\xEE\x81\x94\xEE\x80\x86New coal mine\xEE\x81\x92\n\nunder construction near Brno\xEE\x81\x95";
	CHECK(PlainText(headline) == "New coal mine under construction near Brno");
	CHECK(PlainText("  plain   text\t") == "plain text");
	CHECK(PlainText("") == "");
	CHECK(PlainText("\xC5\xBD" "abov" "\xC5\x99" "esky") == "\xC5\xBD" "abov" "\xC5\x99" "esky"); /* "Zabovresky" with its UTF-8 diacritics */
}

TEST_CASE("The in-game approval channel parses one id per line and refuses lists and junk", "[prismatic_mcp]")
{
	using V = ChatRequest::Verb;
	CHECK(ParseChatApproval("hello there").verb == V::None);
	CHECK(ParseChatApproval("").verb == V::None);
	CHECK(ParseChatApproval("!approvalsx").verb == V::None);
	auto list = ParseChatApproval("!approvals");
	CHECK(list.verb == V::List);
	CHECK(list.page == 1);
	auto page = ParseChatApproval("  !approvals   3 ");
	CHECK(page.verb == V::List);
	CHECK(page.page == 3);
	CHECK(ParseChatApproval("!approvals 0").verb == V::Bad);
	CHECK(ParseChatApproval("!approvals x").verb == V::Bad);
	CHECK(ParseChatApproval("!approvals 1 2").verb == V::Bad);
	CHECK(ParseChatApproval("!approvals 100001").verb == V::Bad);
	auto ok = ParseChatApproval("!approve 42");
	CHECK(ok.verb == V::Approve);
	CHECK(ok.id == 42);
	auto deny = ParseChatApproval("!deny 7");
	CHECK(deny.verb == V::Deny);
	CHECK(deny.id == 7);
	/* One id per line: a list is refused, and so is a line with no id. */
	auto bulk = ParseChatApproval("!approve 1 2 3");
	CHECK(bulk.verb == V::Bad);
	CHECK(bulk.error.find("one id per line") != std::string::npos);
	CHECK(ParseChatApproval("!approve").verb == V::Bad);
	CHECK(ParseChatApproval("!deny -3").verb == V::Bad);
	CHECK(ParseChatApproval("!approve 12a").verb == V::Bad);
	CHECK(ParseChatApproval("!approve 1234567890123456789").verb == V::Bad);
	CHECK(ParseChatApproval("!approve 9x").error == "not an approval id: '9x'");
	CHECK(ParseChatApproval("   !approve   5 ").verb == V::Approve);  /* read as the server reads it, spaces and all */
	CHECK(ParseChatApproval(" !deny 5").id == 5);
	CHECK(ParseChatApproval("!deny").error.find("usage: !deny") == 0);
	CHECK(ParseChatApproval("!deny 1 2").verb == V::Bad);
	CHECK(ParseChatApproval("!approve 123456789012345678").id == 123456789012345678ULL);  /* 18 digits: the most it reads */
	CHECK(ParseChatApproval("!Approve 5").verb == V::None);  /* the verbs are exact: anything else is chat */
}

TEST_CASE("A chat decision names the condition that failed, and only an approver's own input approves", "[prismatic_mcp]")
{
	ChatFacts all;
	all.exists = all.pending = all.chat_decidable = all.valid_company = all.same_company = all.key_registered = true;
	CHECK(DecideChat(all, true) == ChatVerdict::Allow);
	CHECK(DecideChat(all, false) == ChatVerdict::Allow);

	auto with = [&](auto change) { ChatFacts f = all; change(f); return f; };
	CHECK(DecideChat(with([](ChatFacts &f) { f.exists = false; }), true) == ChatVerdict::NoSuchApproval);
	CHECK(DecideChat(with([](ChatFacts &f) { f.pending = false; }), true) == ChatVerdict::NotPending);
	CHECK(DecideChat(with([](ChatFacts &f) { f.expired = true; }), true) == ChatVerdict::Expired);
	CHECK(DecideChat(with([](ChatFacts &f) { f.chat_decidable = false; }), true) == ChatVerdict::ServerOnly);
	CHECK(DecideChat(with([](ChatFacts &f) { f.chat_decidable = false; }), false) == ChatVerdict::ServerOnly);
	CHECK(DecideChat(with([](ChatFacts &f) { f.valid_company = false; }), true) == ChatVerdict::Spectator);
	CHECK(DecideChat(with([](ChatFacts &f) { f.same_company = false; }), false) == ChatVerdict::NotYourCompany);
	/* Approving needs a registered approver; denying withdraws and needs none. */
	ChatFacts nobody = with([](ChatFacts &f) { f.key_registered = false; f.key_in_allow_list = false; });
	CHECK(DecideChat(nobody, true) == ChatVerdict::NotAnApprover);
	CHECK(DecideChat(nobody, false) == ChatVerdict::Allow);
	/* A key in the company's allow list is not an approver: an agent's own client playing the
	 * owner's company is in it, and must never approve (the owner's decision, 2026-11-22). */
	ChatFacts member = with([](ChatFacts &f) { f.key_registered = false; f.key_in_allow_list = true; });
	CHECK(DecideChat(member, true) == ChatVerdict::NotAnApprover);
	CHECK(DecideChat(member, false) == ChatVerdict::Allow);
	CHECK(DecideChat(with([](ChatFacts &f) { f.key_registered = true; f.key_in_allow_list = false; }), true) == ChatVerdict::Allow);
	/* Injected input never approves (a script driving the owner's client), but it may withdraw. */
	CHECK(DecideChat(with([](ChatFacts &f) { f.injecting = true; }), true) == ChatVerdict::Injected);
	CHECK(DecideChat(with([](ChatFacts &f) { f.injecting = true; }), false) == ChatVerdict::Allow);

	for (ChatVerdict v : {ChatVerdict::Allow, ChatVerdict::NoSuchApproval, ChatVerdict::NotPending, ChatVerdict::Expired, ChatVerdict::ServerOnly,
			ChatVerdict::Spectator, ChatVerdict::NotYourCompany, ChatVerdict::NotAnApprover, ChatVerdict::Injected, ChatVerdict::SharedKey}) {
		CHECK_FALSE(ChatVerdictText(v).empty());
	}
	CHECK(ChatVerdictText(ChatVerdict::NotAnApprover).find("approvers add") != std::string_view::npos);
	CHECK(ChatVerdictText(ChatVerdict::NotAnApprover).find("allow list, is not enough") != std::string_view::npos);
	CHECK(ChatVerdictText(static_cast<ChatVerdict>(200)) == "refused");
}

TEST_CASE("A chat line is capped at a UTF-8 boundary, and a newer AI settings plan supersedes the older one", "[prismatic_mcp]")
{
	CHECK(CapLine("short", 10) == "short");
	CHECK(CapLine("0123456789abc", 10) == "0123456...");
	CHECK(CapLine("ab\xC5\xBD" "cdefgh", 6) == "ab...");  /* never cuts the two bytes of a letter apart */
	CHECK(CapLine("abcdef", 2) == "ab");                    /* no room for the ellipsis: never over the limit */
	CHECK(CapLine("\xC5\xBD\xC5\xBD", 3) == "...");
	CHECK(CapLine("\xC5\xBD\xC5\xBD", 2) == "\xC5\xBD"); /* cut below 3 still at a letter's boundary */
	CHECK(CapLine("\xC5\xBD\xC5\xBD", 1).empty());
	CHECK(CapLine("\xE2\x82\xAC\xE2\x82\xAC\xE2\x82\xAC", 7) == "\xE2\x82\xAC...");  /* a three-byte letter kept whole */

	json a = {{"company", 0}, {"settings", {{"x", 1}}}};
	json b = {{"company", 0}, {"settings", {{"y", 2}}}};
	json other = {{"company", 1}, {"settings", {{"x", 1}}}};
	json x2 = {{"company", 0}, {"settings", {{"x", 2}, {"z", 3}}}};
	CHECK(Supersedes("ai.set_settings", x2, "ai.set_settings", a));          /* both change x */
	CHECK_FALSE(Supersedes("ai.set_settings", b, "ai.set_settings", a));      /* y and x: nothing shared */
	CHECK_FALSE(Supersedes("ai.set_settings", other, "ai.set_settings", a));  /* another company */
	/* One setting and a plan of several supersede each other when they share a setting. */
	CHECK(Supersedes("ai.set_setting", json{{"company", 0}, {"name", "x"}, {"value", 5}}, "ai.set_settings", a));
	CHECK(Supersedes("ai.set_settings", a, "ai.set_setting", json{{"company", 0}, {"name", "x"}, {"value", 5}}));
	CHECK_FALSE(Supersedes("ai.set_setting", json{{"company", 0}, {"name", "q"}, {"value", 5}}, "ai.set_settings", a));
	CHECK(SettingKeys("ai.set_settings", x2) == std::vector<std::string>{"x", "z"});
	CHECK(SettingKeys("game.set_pause", json{{"pause", true}}).empty());
	CHECK(Supersedes("ai.set_setting", json{{"company", 0}, {"name", "n"}, {"value", 2}}, "ai.set_setting", json{{"company", 0}, {"name", "n"}, {"value", 1}}));
	CHECK_FALSE(Supersedes("ai.set_setting", json{{"company", 0}, {"name", "m"}, {"value", 2}}, "ai.set_setting", json{{"company", 0}, {"name", "n"}, {"value", 1}}));
	CHECK_FALSE(Supersedes("game.set_pause", json{{"pause", true}}, "game.set_pause", json{{"pause", true}}));
	CHECK_FALSE(Supersedes("ai.set_settings", json{{"settings", json::object()}}, "ai.set_settings", a));
	CHECK_FALSE(Supersedes("ai.set_settings", json::array(), "ai.set_settings", a));
	CHECK_FALSE(Supersedes("ai.set_settings", a, "ai.set_settings", json::array()));            /* an old approval that does not parse */
	CHECK_FALSE(Supersedes("ai.set_settings", a, "ai.set_settings", json{{"settings", {{"x", 1}}}}));  /* an old one naming no company */
	CHECK(SettingKeys("ai.set_setting", json{{"name", "n"}, {"value", 1}}) == std::vector<std::string>{"n"});
	CHECK(SettingKeys("ai.set_setting", json{{"name", 3}}).empty());
	CHECK(SettingKeys("ai.set_settings", json{{"settings", json::array()}}).empty());
	CHECK(SettingKeys("ai.set_settings", json::array()).empty());
}

TEST_CASE("An unanswered approval is settled at its deadline with the operator's standing verdict, keep by default", "[prismatic_mcp]")
{
	DeadlinePolicy keep;  /* the policy a server starts with */
	CHECK(keep.ticks == 30 * DEADLINE_DAY_TICKS);
	CHECK(keep.verdict == DeadlineVerdict::Keep);
	CHECK(DeadlineTick(1000, keep.ticks) == std::optional<uint64_t>(1000 + 30 * 74));
	CHECK_FALSE(DeadlineTick(1000, 0).has_value());

	const uint64_t due = 1000 + keep.ticks;
	CHECK(DeadlineDue(true, 1000, due - 1, keep, true, true) == DeadlineAction::None);
	CHECK(DeadlineDue(true, 1000, due, keep, true, true) == DeadlineAction::Retire);
	CHECK(DeadlineDue(true, 1000, due + 5000, keep, false, false) == DeadlineAction::Retire);
	/* Decided already: the deadline has nothing to settle. */
	CHECK(DeadlineDue(false, 1000, due + 1, keep, true, true) == DeadlineAction::None);

	DeadlinePolicy approve{keep.ticks, DeadlineVerdict::Approve};
	CHECK(DeadlineDue(true, 1000, due, approve, true, true) == DeadlineAction::Execute);
	/* Wider than a chat approver may decide (a server setting, starting an AI): retired, never executed. */
	CHECK(DeadlineDue(true, 1000, due, approve, false, true) == DeadlineAction::Retire);

	DeadlinePolicy off{0, DeadlineVerdict::Approve};
	CHECK(DeadlineDue(true, 1000, UINT64_MAX, off, true, true) == DeadlineAction::None);

	CHECK(DeadlineVerdictName(DeadlineVerdict::Keep) == "keep");
	CHECK(DeadlineVerdictName(DeadlineVerdict::Approve) == "approve");
}

TEST_CASE("The deadline policy is parsed from the console's game days and verdict, and a malformed line changes nothing", "[prismatic_mcp]")
{
	auto p = ParseDeadlinePolicy("7", "");
	REQUIRE(p.has_value());
	CHECK(p->ticks == 7 * DEADLINE_DAY_TICKS);
	CHECK(p->verdict == DeadlineVerdict::Keep);
	CHECK(ParseDeadlinePolicy("30", "keep")->verdict == DeadlineVerdict::Keep);
	CHECK(ParseDeadlinePolicy("30", "approve")->verdict == DeadlineVerdict::Approve);
	CHECK(ParseDeadlinePolicy("0", "keep")->ticks == 0);
	CHECK(ParseDeadlinePolicy("3650", "")->ticks == 3650 * DEADLINE_DAY_TICKS);
	CHECK_FALSE(ParseDeadlinePolicy("3651", "").has_value());
	CHECK_FALSE(ParseDeadlinePolicy("", "").has_value());
	CHECK_FALSE(ParseDeadlinePolicy("-1", "").has_value());
	CHECK_FALSE(ParseDeadlinePolicy("7d", "").has_value());
	CHECK_FALSE(ParseDeadlinePolicy("99999", "").has_value());
	CHECK_FALSE(ParseDeadlinePolicy("7", "yes").has_value());

	/* The start-up form, PRISMATIC_MCP_APPROVAL_DEADLINE="<days>[:verdict]". */
	CHECK(ParseDeadlinePolicyText("30:approve")->verdict == DeadlineVerdict::Approve);
	CHECK(ParseDeadlinePolicyText("30:approve")->ticks == 30 * DEADLINE_DAY_TICKS);
	CHECK(ParseDeadlinePolicyText("14")->verdict == DeadlineVerdict::Keep);
	CHECK(ParseDeadlinePolicyText("0:keep")->ticks == 0);
	CHECK_FALSE(ParseDeadlinePolicyText("approve").has_value());
	CHECK(ParseDeadlinePolicyText("30:")->verdict == DeadlineVerdict::Keep); // an empty verdict is keep
	CHECK_FALSE(ParseDeadlinePolicyText("30:yes").has_value());
	CHECK_FALSE(ParseDeadlinePolicyText(":approve").has_value());
}

TEST_CASE("Only an action on one company is decided in the game's chat, classified by the command's own flags and category", "[prismatic_mcp]")
{
	json none = json::array();
	CHECK(ChatScoped("ai.set_setting", none, ""));
	CHECK(ChatScoped("ai.set_settings", none, ""));
	/* Starting, stopping or reloading an AI creates, removes or replaces a company: the operator's. */
	CHECK_FALSE(ChatScoped("ai.stop", none, ""));
	CHECK_FALSE(ChatScoped("ai.start", none, ""));
	CHECK_FALSE(ChatScoped("game.set_pause", none, ""));
	CHECK_FALSE(ChatScoped("api.ScriptCompany.SetName", none, ""));
	CHECK(ChatScoped("command.RenamePresident", json::array({"str_ctrl"}), "company_setting"));
	CHECK(ChatScoped("command.BuildRailroadTrack", json::array({"auto", "no_water"}), "landscape_construction"));
	CHECK_FALSE(ChatScoped("command.Pause", json::array({"server"}), "server_setting"));
	CHECK_FALSE(ChatScoped("command.ChangeSetting", json::array({"server"}), "server_setting"));
	CHECK_FALSE(ChatScoped("command.Money", json::array({"offline"}), "cheat"));
	CHECK_FALSE(ChatScoped("command.FutureCheat", json::array(), "cheat"));  /* the cheat category, whatever its flags */
	CHECK_FALSE(ChatScoped("command.CreateGoal", json::array({"deity", "str_ctrl"}), "other_management"));
	/* CompanyControl has no Server flag; its category is what makes it the operator's. */
	CHECK_FALSE(ChatScoped("command.CompanyControl", json::array({"spectator", "client_id", "no_estimate"}), "server_setting"));
	CHECK_FALSE(ChatScoped("command.Unknown", json::object(), "company_setting"));
	CHECK_FALSE(ChatScoped("command.Unknown", none, ""));
	CHECK(KeyFingerprint("abcdefghijklmnopqrstuvwxyz") == "abcdefghijkl");
	CHECK(KeyFingerprint("short") == "short");
}

/* ---- the owner's bulk consent and standing rules (ADR 0041) ------------------------------- */

TEST_CASE("A rule's tool glob takes * for any run and ? for one character, over the whole name", "[prismatic_mcp]")
{
	namespace R = PrismaticMCP::Rules;
	CHECK(R::Glob("openttd.ai.set_setting", "openttd.ai.set_setting"));
	CHECK_FALSE(R::Glob("openttd.ai.set_setting", "openttd.ai.set_settings")); // whole name, never a prefix
	CHECK(R::Glob("openttd.ai.*", "openttd.ai.set_settings"));
	CHECK(R::Glob("*", ""));
	CHECK(R::Glob("*", "anything"));
	CHECK(R::Glob("*setting?", "ai.set_settings"));
	CHECK_FALSE(R::Glob("*setting?", "ai.set_setting"));
	CHECK(R::Glob("a*b*c", "aXbYbZc"));
	CHECK_FALSE(R::Glob("a*b*c", "aXbYbZ"));
	CHECK(R::Glob("command.Build*", "command.BuildRailroadTrack"));
	CHECK(R::Glob("*", "*a")); // a literal star in the text is matched by the glob's star
	CHECK(R::Glob("?", "x"));
	CHECK_FALSE(R::Glob("?", ""));
	CHECK_FALSE(R::Glob("", "x"));
	CHECK(R::Glob("", ""));
	CHECK(R::Glob("a**", "a"));
}

TEST_CASE("A rule's JSON matcher: plain keys match a subset, $ paths select, strings are globs, the rest is equality", "[prismatic_mcp]")
{
	namespace R = PrismaticMCP::Rules;
	const json args = {{"company", 1}, {"name", "max_loan"}, {"value", 250000}, {"enabled", true}, {"note", nullptr},
		{"params", json::array({3, 4, 5})}, {"plan", {{"steps", json::array({{{"name", "rail"}, {"cost", 10}}, {{"name", "road"}, {"cost", 20}}})}}}};
	/* Plain keys: the args must have each, and its value must match; keys not named are free. */
	CHECK(R::Match(json::object(), args));
	CHECK(R::Match({{"company", 1}}, args));
	CHECK_FALSE(R::Match({{"company", 2}}, args));
	CHECK_FALSE(R::Match({{"missing", "*"}}, args));
	CHECK(R::Match({{"company", 1}, {"name", "max_*"}}, args));
	CHECK_FALSE(R::Match({{"company", 1}, {"name", "min_*"}}, args));
	CHECK(R::Match({{"note", "*"}}, args)); // "*" matches any value present, null included
	CHECK(R::Match({{"note", nullptr}}, args));
	CHECK(R::Match({{"enabled", true}}, args));
	CHECK_FALSE(R::Match({{"enabled", false}}, args));
	CHECK(R::Match({{"value", 250000.0}}, args)); // JSON numeric equality
	/* A string pattern against a non-string value is a glob over its JSON text. */
	CHECK(R::Match({{"value", "25????"}}, args));
	CHECK_FALSE(R::Match({{"value", "1*"}}, args));
	CHECK(R::Match({{"params", "[3,*"}}, args));
	CHECK(R::Match({{"enabled", "true"}}, args));
	/* An array pattern: the same length, element by element. */
	CHECK(R::Match({{"params", json::array({3, "*", 5})}}, args));
	CHECK_FALSE(R::Match({{"params", json::array({3, 4})}}, args));
	CHECK_FALSE(R::Match({{"company", json::array({1})}}, args));
	/* A nested object pattern recurses; a plain key against a value that is no object does not match. */
	CHECK(R::Match({{"plan", {{"steps", "*"}}}}, args));
	CHECK_FALSE(R::Match({{"company", {{"id", 1}}}}, args));
	/* $ paths: one value, an index, every element, every member, at any depth. */
	CHECK(R::Match({{"$.company", 1}}, args));
	CHECK(R::Match({{"$['name']", "max_loan"}}, args));
	CHECK(R::Match({{"$.params[0]", 3}}, args));
	CHECK_FALSE(R::Match({{"$.params[7]", "*"}}, args)); // a path that leads nowhere does not match
	CHECK(R::Match({{"$.params[*]", "?"}}, args));       // every element must match
	CHECK_FALSE(R::Match({{"$.params[*]", 3}}, args));
	CHECK(R::Match({{"$.plan.steps[*].cost", "*"}}, args));
	CHECK(R::Match({{"$.plan.steps[1].name", "ro*"}}, args));
	CHECK(R::Match({{"$..cost", "?0"}}, args));         // every "cost" at any depth
	CHECK_FALSE(R::Match({{"$..cost", 10}}, args));
	CHECK_FALSE(R::Match({{"$..absent", "*"}}, args));
	CHECK(R::Match({{"$.plan.*", "*"}}, args));
	CHECK(R::Match({{"$", {{"company", 1}}}}, args));   // $ alone is the arguments themselves
	CHECK(R::Match({{"plan", {{"$.steps[0].name", "rail"}}}}, args)); // a path in a nested pattern starts there
	CHECK_FALSE(R::Match({{"$.params[x]", "*"}}, args)); // a path that does not parse never matches
	/* The paths parse, or say why not. */
	CHECK(R::ParsePath("$.a.b[2][*]..c.*['d e']").has_value());
	CHECK(R::ParsePath("$").has_value());
	for (const char *bad : {"", "a.b", "$.", "$..", "$[", "$[1", "$[-1]", "$[x]", "$a", "$[1234567890]"}) {
		INFO(bad);
		CHECK_FALSE(R::ParsePath(bad).has_value());
	}
	CHECK_FALSE(R::MatcherError(json()).has_value());
	CHECK_FALSE(R::MatcherError({{"a", {{"$.b", json::array({{{"$.c", 1}}})}}}}).has_value());
	CHECK(R::MatcherError(json::array()).has_value());
	CHECK(R::MatcherError("x").has_value());
	CHECK(R::MatcherError({{"a", json::array({json{{"$[", 1}}})}}).has_value()); // a bad path at any depth
}

TEST_CASE("A deny rule wins over an allow rule, and an allow rule from the panel never decides an owner-only tool", "[prismatic_mcp]")
{
	namespace R = PrismaticMCP::Rules;
	auto rule = [](uint64_t id, R::Effect e, std::string tool, json match, std::string via) {
		R::Rule r;
		r.id = id;
		r.effect = e;
		r.tool = std::move(tool);
		r.match = std::move(match);
		r.created_via = std::move(via);
		return r;
	};
	const json args = {{"company", 1}, {"name", "aggressiveness"}, {"value", 80}};
	R::Subject set{"openttd.ai.set_setting", "ai.set_setting", args};
	/* The glob matches the MCP tool name or the action type. */
	CHECK(R::Applies(rule(1, R::Effect::Allow, "openttd.ai.set_setting", json(), "console"), set));
	CHECK(R::Applies(rule(1, R::Effect::Allow, "ai.set_setting", json(), "console"), set));
	CHECK(R::Applies(rule(1, R::Effect::Allow, "openttd.ai.*", {{"name", "aggr*"}}, "console"), set));
	CHECK_FALSE(R::Applies(rule(1, R::Effect::Allow, "openttd.ai.*", {{"company", 2}}, "console"), set));
	CHECK(R::Applies(rule(1, R::Effect::Allow, "ai.set_setting", json(), "console"), R::Subject{"", "ai.set_setting", args}));
	CHECK_FALSE(R::Applies(rule(1, R::Effect::Allow, "openttd.ai.set_setting", json(), "console"), R::Subject{"", "ai.set_setting", args}));

	std::vector<R::Rule> rules = {rule(1, R::Effect::Allow, "openttd.ai.*", json(), "console"),
		rule(2, R::Effect::Deny, "*", {{"value", "8?"}}, "panel"), rule(3, R::Effect::Allow, "*", json(), "window")};
	R::Verdict v = R::Decide(rules, set);
	REQUIRE(v.decides != nullptr);
	CHECK(v.decides->id == 2); // deny wins although an allow rule came first
	const json low = {{"company", 1}, {"name", "aggressiveness"}, {"value", 20}};
	v = R::Decide(rules, R::Subject{"openttd.ai.set_setting", "ai.set_setting", low});
	REQUIRE(v.decides != nullptr);
	CHECK(v.decides->id == 1); // else the first allow rule in id order
	CHECK(R::Decide({}, set).decides == nullptr);

	/* An owner-only tool (a Battlebrain plan, ai.set_settings): an allow rule from the panel applies but is held. */
	const json plan = {{"company", 3}, {"settings", {{"a", 1}}}, {"expires_tick", 900}};
	R::Subject owners{"openttd.ai.set_settings", "ai.set_settings", plan};
	std::vector<R::Rule> panel = {rule(4, R::Effect::Allow, "*", json(), "panel"), rule(5, R::Effect::Allow, "openttd.ai.set_settings", json(), "panel")};
	v = R::Decide(panel, owners);
	CHECK(v.decides == nullptr);
	REQUIRE(v.blocked != nullptr);
	CHECK(v.blocked->id == 4);
	/* The same rule made at the console, or by the owner's click in the window, decides it. */
	for (const char *via : {"console", "window"}) {
		INFO(via);
		std::vector<R::Rule> own = {rule(6, R::Effect::Allow, "*", json(), via)};
		v = R::Decide(own, owners);
		REQUIRE(v.decides != nullptr);
		CHECK(v.blocked == nullptr);
	}
	/* A deny rule from the panel withdraws an owner-only request: withdrawing enacts nothing. */
	panel.push_back(rule(7, R::Effect::Deny, "*set_settings", json(), "panel"));
	v = R::Decide(panel, owners);
	REQUIRE(v.decides != nullptr);
	CHECK(v.decides->id == 7);
	/* Approve all from the panel, and per id from the panel, skip an owner-only tool the same way. */
	CHECK_FALSE(R::MayDecide(R::Effect::Allow, "panel", "ai.set_settings"));
	CHECK_FALSE(R::MayDecide(R::Effect::Allow, "panel", "openttd.ai.set_settings"));
	CHECK(R::MayDecide(R::Effect::Allow, "panel", "ai.set_setting"));
	CHECK(R::MayDecide(R::Effect::Allow, "console", "ai.set_settings"));
	CHECK(R::MayDecide(R::Effect::Allow, "window", "ai.set_settings"));
	CHECK(R::MayDecide(R::Effect::Deny, "panel", "ai.set_settings"));
	CHECK_FALSE(R::OwnersOwnChannel("panel"));
	CHECK_FALSE(R::OwnersOwnChannel(""));
}

TEST_CASE("A rule, an approve all and a deny all are read from console tokens, and the window writes them back", "[prismatic_mcp]")
{
	namespace R = PrismaticMCP::Rules;
	auto parse = [](std::vector<std::string_view> t) { return R::ParseSpec(t); };
	R::Spec s = parse({});
	CHECK((s.error.empty() && s.tool == "*" && s.match.is_null() && !s.persist && s.via == "console"));
	s = parse({"openttd.ai.set_setting"});
	CHECK((s.error.empty() && s.tool == "openttd.ai.set_setting" && s.match.is_null()));
	/* One quoted token, as the console's tokenizer leaves "{\"name\":\"max_*\"}". */
	s = parse({"openttd.ai.*", R"({"name":"max_*"})", "--persist"});
	CHECK((s.error.empty() && s.match == json{{"name", "max_*"}} && s.persist));
	/* Unquoted with single quotes: the tokens after the glob are joined and read with ' as ". */
	s = parse({"--via", "panel", "ai.set_setting", "{'company':", "1,", "'name':", "'x y'}"});
	CHECK((s.error.empty() && s.via == "panel" && s.match == json{{"company", 1}, {"name", "x y"}}));
	s = parse({"*", "{\"a\":1}", "--via", "window"});
	CHECK((s.error.empty() && s.via == "window"));
	CHECK_FALSE(parse({"*", "--via"}).error.empty());
	CHECK_FALSE(parse({"*", "--via", "mcp"}).error.empty());  // only the window and the panel name themselves
	CHECK_FALSE(parse({"{\"a\":1}"}).error.empty());           // a matcher where the glob belongs
	CHECK_FALSE(parse({"*", "{not json"}).error.empty());
	CHECK_FALSE(parse({"*", "[1,2]"}).error.empty());          // a matcher is an object
	CHECK_FALSE(parse({"*", R"({"$.a[":1})"}).error.empty());  // with paths that parse
	CHECK_FALSE(parse({""}).error.empty());

	CHECK(R::ParseMatcherText("  ")->is_null());
	CHECK_FALSE(R::ParseMatcherText("{'a': }").has_value());
	CHECK(R::ConsoleQuote(R"({"a":"b"})") == R"("{\"a\":\"b\"}")");
	CHECK(R::RuleCommand(R::Effect::Allow, "openttd.ai.set_setting", {{"name", "max_*"}}, true, "window")
		== R"(prismatic_mcp rule allow openttd.ai.set_setting "{\"name\":\"max_*\"}" --persist --via window)");
	CHECK(R::RuleCommand(R::Effect::Deny, "*", json(), false, "console") == "prismatic_mcp rule deny *");
	/* The matcher "Always ... matching" offers: the arguments without a stale date. */
	CHECK(R::MatcherFromArgs({{"company", 1}, {"expires_tick", 9}}) == json{{"company", 1}});
	CHECK(R::MatcherFromArgs("x") == json::object());

	/* A rule as JSON and back, as the rules file keeps it. */
	R::Rule r;
	r.id = 9;
	r.effect = R::Effect::Allow;
	r.tool = "openttd.cmd.*";
	r.match = {{"$.params[*]", "*"}};
	r.created_via = "window";
	r.created_at = "2026-11-23T10:00:00Z";
	r.created_tick = 42;
	r.hits = 3;
	json j = R::ToJson(r);
	CHECK(j["text"] == R"(rule #9 allow openttd.cmd.* {"$.params[*]":"*"})");
	auto back = R::FromJson(j);
	REQUIRE(back.has_value());
	CHECK((back->effect == R::Effect::Allow && back->tool == r.tool && back->match == r.match && back->created_via == "window"
		&& back->created_tick == 42 && back->persisted));
	CHECK(R::FromJson({{"effect", "deny"}, {"tool", "*"}, {"created_via", ""}})->created_via == "console");
	CHECK_FALSE(R::FromJson("x").has_value());
	CHECK_FALSE(R::FromJson({{"effect", "maybe"}, {"tool", "*"}}).has_value());
	CHECK_FALSE(R::FromJson({{"effect", "deny"}, {"tool", ""}}).has_value());
	CHECK_FALSE(R::FromJson({{"effect", "deny"}, {"tool", "*"}, {"match", 5}}).has_value());
	CHECK(R::EffectName(R::Effect::Deny) == "deny");
}

namespace {

/** Ask for a pause or a resume through the MCP dispatch, as a client would; the answer's JSON text. */
json AskPause(bool pause)
{
	json reply = PrismaticMCP::Dispatch({{"jsonrpc", "2.0"}, {"id", 7}, {"method", "tools/call"},
		{"params", {{"name", "openttd.game.set_pause"}, {"arguments", {{"pause", pause}}}}}});
	return json::parse(reply["result"]["content"][0]["text"].get<std::string>(), nullptr, false);
}

/** Every pending approval's id. */
std::vector<uint64_t> PendingIds()
{
	std::vector<uint64_t> ids;
	for (const auto &a : PrismaticMCP::PendingApprovals()) ids.push_back(a.id);
	return ids;
}

/** Remove every standing rule and withdraw every pending approval: the process is shared by the cases. */
void ClearRulesAndQueue()
{
	std::string m;
	for (const auto &r : PrismaticMCP::ApprovalRules()) PrismaticMCP::RemoveApprovalRule(r.id, m);
	for (uint64_t id : PendingIds()) PrismaticMCP::DenyById(id, m);
}

/**
 * Run \p f as a network client would: an approved pause is then refused before it is posted
 * (ClientMayPost), so the execution path runs to the command without a video driver, which the
 * pause command needs and a unit test has none of. Every approval decided inside answers "rejected".
 */
template <typename F>
auto AsNetworkClient(F f)
{
	_networking = true;
	auto out = f();
	_networking = false;
	return out;
}

/** The settled_by of approval \p id in openttd://approvals. */
std::string SettledBy(uint64_t id)
{
	json r = PrismaticMCP::Dispatch({{"jsonrpc", "2.0"}, {"id", 8}, {"method", "resources/read"}, {"params", {{"uri", "openttd://approvals"}}}});
	json data = json::parse(r["result"]["contents"][0]["text"].get<std::string>())["data"];
	for (const json &a : data["approvals"]) if (a["id"] == id) return a.value("settled_by", "");
	return "absent";
}

} // namespace

TEST_CASE("A standing rule decides every waiting approval it matches when it is added, and every new one as it is created", "[prismatic_mcp]")
{
	namespace R = PrismaticMCP::Rules;
	ClearRulesAndQueue();
	json first = AskPause(true);
	REQUIRE(first["status"] == "pending_approval");
	const uint64_t pause_id = first["approval_id"].get<uint64_t>();
	json second = AskPause(false);
	REQUIRE(second["status"] == "pending_approval");
	const uint64_t resume_id = second["approval_id"].get<uint64_t>();
	auto pending = PrismaticMCP::PendingApprovals();
	REQUIRE(pending.size() == 2);
	CHECK(pending[0].mcp_tool == "openttd.game.set_pause");
	CHECK(pending[0].matched_rule == 0);

	/* A deny rule with a matcher withdraws only what it matches, at once, recorded with the rule. */
	R::Spec spec;
	spec.tool = "openttd.game.*";
	spec.match = {{"pause", true}};
	auto added = PrismaticMCP::AddApprovalRule(R::Effect::Deny, spec);
	REQUIRE(added.ok);
	CHECK(added.decided == 1);
	CHECK(added.lines.front().find("added rule #") == 0);
	CHECK(PendingIds() == std::vector<uint64_t>{resume_id});
	CHECK(SettledBy(pause_id).find("rule #") == 0);
	auto rules = PrismaticMCP::ApprovalRules();
	REQUIRE(rules.size() == 1);
	CHECK((rules[0].hits == 1 && rules[0].created_via == "console" && !rules[0].persisted && !rules[0].created_at.empty()));

	/* A new request it matches is denied as it is created; the requester is told by which rule. */
	json again = AskPause(true);
	CHECK(again["status"] == "denied");
	CHECK(again["rule"] == rules[0].id);
	CHECK(PrismaticMCP::ApprovalRules()[0].hits == 2);

	/* The snapshot and openttd://approvals carry the rules; the console lists them. */
	json snap = PrismaticMCP::Panel::Snapshot();
	REQUIRE(snap["rules"].size() == 1);
	CHECK(snap["rules"][0]["effect"] == "deny");
	CHECK(snap["rules"][0]["hits"] == 2);
	IConsoleStdLibRegister();
	IConsoleCmdExec("prismatic_mcp rules");

	/* An allow rule from the panel decides the resume (a set_pause is no owner-only tool): it is
	 * approved and goes to the one execution path, recorded with the rule. */
	R::Spec everything;
	everything.via = "panel";
	everything.match = {{"pause", false}}; // {"pause": "*"} over every tool would be broad: refused without --force
	auto allow = AsNetworkClient([&] { return PrismaticMCP::AddApprovalRule(R::Effect::Allow, everything); });
	REQUIRE(allow.ok);
	CHECK(allow.refused == 1); // posted as a client would be: refused before the command, see AsNetworkClient
	REQUIRE(allow.lines.size() == 2);
	CHECK(allow.lines[1].find(fmt::format("#{}: rejected -- ", resume_id)) == 0);
	CHECK(PendingIds().empty());
	json via = PrismaticMCP::Dispatch({{"jsonrpc", "2.0"}, {"id", 9}, {"method", "resources/read"}, {"params", {{"uri", "openttd://decisions"}}}});
	json decisions = json::parse(via["result"]["contents"][0]["text"].get<std::string>())["data"]["decisions"];
	REQUIRE_FALSE(decisions.empty());
	CHECK(decisions.back()["approval_id"] == resume_id);
	CHECK(decisions.back()["approved_via"].get<std::string>().find("rule #") == 0);
	CHECK(PrismaticMCP::DecisionTrace(1).find(" via=rule #") != std::string::npos);
	/* Deny still wins over it for a pause. */
	CHECK(AskPause(true)["status"] == "denied");

	/* Removing a rule ends it; a rule that is not there is said to be absent. */
	std::string m;
	for (const auto &r : PrismaticMCP::ApprovalRules()) CHECK(PrismaticMCP::RemoveApprovalRule(r.id, m));
	CHECK(m.find("removed rule #") == 0);
	CHECK_FALSE(PrismaticMCP::RemoveApprovalRule(999999, m));
	CHECK(m.find("no rule #999999") == 0);
	CHECK(AskPause(true)["status"] == "pending_approval");
	/* A malformed rule is refused whole. */
	R::Spec bad;
	bad.error = "nope";
	CHECK_FALSE(PrismaticMCP::AddApprovalRule(R::Effect::Allow, bad).ok);
	R::Spec bad_match;
	bad_match.match = json::array();
	CHECK_FALSE(PrismaticMCP::AddApprovalRule(R::Effect::Allow, bad_match).ok);
	CHECK(PrismaticMCP::ApprovalRules().empty());
	ClearRulesAndQueue();
}

TEST_CASE("Approve all and deny all act on what their filter selects, and say what each id did", "[prismatic_mcp]")
{
	namespace R = PrismaticMCP::Rules;
	ClearRulesAndQueue();
	const uint64_t pause_id = AskPause(true)["approval_id"].get<uint64_t>();
	const uint64_t resume_id = AskPause(false)["approval_id"].get<uint64_t>();

	R::Spec none;
	none.tool = "openttd.cmd.*";
	auto out = PrismaticMCP::DecideAll(true, none);
	CHECK((out.ok && out.decided == 0 && out.lines.empty()));

	R::Spec resume;
	resume.match = {{"pause", false}};
	out = PrismaticMCP::DecideAll(false, resume);
	CHECK((out.decided == 1 && out.refused == 0));
	REQUIRE(out.lines.size() == 1);
	CHECK(out.lines[0].find(fmt::format("#{}: denied #{}", resume_id, resume_id)) == 0);
	CHECK(out.lines[0].find("tool=openttd.game.set_pause") != std::string::npos);
	CHECK(SettledBy(resume_id).find("denied via console (deny all * {\"pause\":false})") == 0);
	CHECK(PendingIds() == std::vector<uint64_t>{pause_id});

	R::Spec all;
	all.tool = "game.set_pause";
	all.via = "panel";
	/* An expired approval is skipped by approve all and by an allow rule, never approved. */
	const uint64_t now = TimerGameTick::counter;
	TimerGameTick::counter = now + 100001;
	out = PrismaticMCP::DecideAll(true, all);
	CHECK((out.skipped == 1 && out.decided == 0));
	CHECK(out.lines[0].find("skipped: expired") != std::string::npos);
	R::Spec any;
	CHECK_FALSE(PrismaticMCP::AddApprovalRule(R::Effect::Allow, any).ok); // every tool, any arguments: only with --force
	any.force = true;
	auto expired_rule = PrismaticMCP::AddApprovalRule(R::Effect::Allow, any);
	CHECK((expired_rule.skipped == 1 && expired_rule.decided == 0));
	CHECK(PrismaticMCP::PendingApprovals()[0].matched_rule == expired_rule.rule_id);
	CHECK_FALSE(PrismaticMCP::PendingApprovals()[0].rule_decides); // an allow rule never approves an expired one
	json held = PrismaticMCP::Dispatch({{"jsonrpc", "2.0"}, {"id", 8}, {"method", "resources/read"}, {"params", {{"uri", "openttd://approvals"}}}});
	json first = json::parse(held["result"]["contents"][0]["text"].get<std::string>())["data"]["approvals"][0];
	CHECK(first["matched_rule"]["decides"] == false);
	std::string m;
	PrismaticMCP::RemoveApprovalRule(expired_rule.rule_id, m);
	TimerGameTick::counter = now;
	/* Approve all from the panel decides the rest, one id at a time through ApproveById. */
	out = AsNetworkClient([&] { return PrismaticMCP::DecideAll(true, all); });
	CHECK(out.refused == 1);
	REQUIRE(out.lines.size() == 1);
	CHECK(out.lines[0].find(fmt::format("#{}: approved #{}", pause_id, pause_id)) == 0);
	CHECK(PendingIds().empty());

	R::Spec bad;
	bad.error = "no";
	CHECK_FALSE(PrismaticMCP::DecideAll(true, bad).ok);
	ClearRulesAndQueue();
}

TEST_CASE("The console spells rules, approve all and deny all, and an injected or armed line approves nothing", "[prismatic_mcp]")
{
	namespace U = PrismaticMCP::UI;
	ClearRulesAndQueue();
	IConsoleStdLibRegister();
	AskPause(true);
	AskPause(false);
	IConsoleCmdExec(R"(prismatic_mcp rule deny openttd.game.set_pause "{\"pause\":true}")");
	auto rules = PrismaticMCP::ApprovalRules();
	REQUIRE(rules.size() == 1);
	CHECK(rules[0].match == json{{"pause", true}});
	CHECK(PrismaticMCP::PendingApprovals().size() == 1);
	IConsoleCmdExec("prismatic_mcp rule");      // lists, like 'rules'
	IConsoleCmdExec("prismatic_mcp rule maybe x"); // usage, nothing made
	IConsoleCmdExec("prismatic_mcp rule allow"); // usage, nothing made
	IConsoleCmdExec("prismatic_mcp rule remove"); // usage
	IConsoleCmdExec("prismatic_mcp rule allow {'pause':false}"); // a matcher where the glob belongs: refused
	CHECK(PrismaticMCP::ApprovalRules().size() == 1);
	{
		/* Injected input: no allow rule, no approve all, no approve; a deny rule and deny all stay open. */
		U::InjectionScope script;
		IConsoleCmdExec("prismatic_mcp rule allow *");
		IConsoleCmdExec("prismatic_mcp approve all");
		CHECK(PrismaticMCP::ApprovalRules().size() == 1);
		CHECK(PrismaticMCP::PendingApprovals().size() == 1);
		IConsoleCmdExec("prismatic_mcp rule deny openttd.cmd.* {'company': 9} --via window");
		CHECK(PrismaticMCP::ApprovalRules().size() == 2);
		CHECK(PrismaticMCP::ApprovalRules().back().created_via == "window");
	}
	IConsoleCmdExec(fmt::format("prismatic_mcp rule remove {}", PrismaticMCP::ApprovalRules().back().id));
	IConsoleCmdExec("prismatic_mcp rule remove 999999");
	CHECK(PrismaticMCP::ApprovalRules().size() == 1);
	IConsoleCmdExec("prismatic_mcp deny all openttd.cmd.*"); // matches nothing: says so
	CHECK(PrismaticMCP::PendingApprovals().size() == 1);
	IConsoleCmdExec("prismatic_mcp approve all * {bad");       // a malformed filter decides nothing
	CHECK(PrismaticMCP::PendingApprovals().size() == 1);
	IConsoleCmdExec("prismatic_mcp approve");                  // usage
	/* Per id, from the panel: a denial records its channel. */
	const uint64_t resume_id = PendingIds().front();
	IConsoleCmdExec(fmt::format("prismatic_mcp deny {} --via panel", resume_id));
	CHECK(PrismaticMCP::PendingApprovals().empty());
	CHECK(SettledBy(resume_id) == "denied via panel");
	/* Deny all with a filter, as the console prints it. */
	AskPause(false);
	IConsoleCmdExec("prismatic_mcp deny all game.set_pause {'pause': false}");
	CHECK(PrismaticMCP::PendingApprovals().empty());
	/* A rule persisted to the rules file, and removed from it again. */
	IConsoleCmdExec("prismatic_mcp rule deny openttd.ai.* --persist");
	auto persisted = PrismaticMCP::ApprovalRules();
	REQUIRE(persisted.size() == 2);
	CHECK(persisted.back().persisted);
	IConsoleCmdExec(fmt::format("prismatic_mcp rule remove {}", persisted.back().id));
	ClearRulesAndQueue();
	std::filesystem::remove(PrismaticMCP::RulesFile());
}

TEST_CASE("Persisted rules are loaded once per rules file, each said aloud, and a bad entry is skipped", "[prismatic_mcp]")
{
	ClearRulesAndQueue();
	const uint16_t port = _settings_client.network.server_port;
	_settings_client.network.server_port = 40416;
	const auto owner_only = std::filesystem::perms::owner_read | std::filesystem::perms::owner_write;
	{
		std::ofstream f(PrismaticMCP::RulesFile());
		f << R"({"rules": [{"effect": "allow", "tool": "*", "created_via": "console"}]})";
	}
	std::filesystem::permissions(PrismaticMCP::RulesFile(), owner_only | std::filesystem::perms::group_read, std::filesystem::perm_options::replace);
	auto said = PrismaticMCP::LoadApprovalRules();
	REQUIRE(said.size() == 1); // a file others may read is no standing consent of the owner's
	CHECK(said[0].find("refused") != std::string::npos);
	CHECK(PrismaticMCP::ApprovalRules().empty());
	std::filesystem::remove(PrismaticMCP::RulesFile());
	_settings_client.network.server_port = 40417;
	{
		std::ofstream f(PrismaticMCP::RulesFile());
		f << R"({"rules": [{"effect": "deny", "tool": "openttd.ai.*", "match": {"company": 2}, "created_via": "window", "created_at": "2026-11-23T10:00:00Z"}, {"effect": "maybe"}]})";
	}
	std::filesystem::permissions(PrismaticMCP::RulesFile(), owner_only, std::filesystem::perm_options::replace);
	said = PrismaticMCP::LoadApprovalRules();
	REQUIRE(said.size() == 2);
	CHECK(said[0].find("standing approval rule #") == 0);
	CHECK(said[0].find("made via window") != std::string::npos);
	CHECK(said[1].find("skipped an entry that is not a rule") != std::string::npos);
	auto rules = PrismaticMCP::ApprovalRules();
	REQUIRE(rules.size() == 1);
	CHECK((rules[0].persisted && rules[0].match == json{{"company", 2}}));
	CHECK(rules[0].created_via == "file"); // a rule from a file never decides an owner-only tool
	CHECK(PrismaticMCP::LoadApprovalRules().empty()); // once per file: a restart of the server does not double it
	std::filesystem::remove(PrismaticMCP::RulesFile());
	/* Another port's file that is not JSON: said, nothing loaded. */
	_settings_client.network.server_port = 40418;
	{
		std::ofstream f(PrismaticMCP::RulesFile());
		f << "not json";
	}
	std::filesystem::permissions(PrismaticMCP::RulesFile(), owner_only, std::filesystem::perm_options::replace);
	said = PrismaticMCP::LoadApprovalRules();
	REQUIRE(said.size() == 1);
	CHECK(said[0].find("is not JSON") != std::string::npos);
	std::filesystem::remove(PrismaticMCP::RulesFile());
	_settings_client.network.server_port = 40419; // no file at all: nothing to say
	CHECK(PrismaticMCP::LoadApprovalRules().empty());
	ClearRulesAndQueue(); // removing the loaded rule rewrites this port's rules file
	std::filesystem::remove(PrismaticMCP::RulesFile());
	_settings_client.network.server_port = port;
}

TEST_CASE("The control window's rows follow its selectors, and its bulk, next-row and hit helpers are pure", "[prismatic_mcp]")
{
	namespace P = PrismaticMCP::Panel;
	namespace R = PrismaticMCP::Rules;
	using K = P::Row::Kind;
	std::string long_args = "{\"x\":\"" + std::string(300, 'a') + "\"}";
	const json s = {
		{"status", "ok"},
		{"approvals", json::array({
			{{"id", 1}, {"tool", "ai.set_setting"}, {"mcp_tool", "openttd.ai.set_setting"}, {"args", R"({"company":1,"name":"max_loan"})"}, {"expired", false}},
			{{"id", 2}, {"tool", "ai.set_setting"}, {"args", R"({"company":2,"name":"x"})"}, {"expired", true}},
			{{"id", 3}, {"tool", "command.BuildRail"}, {"args", long_args}, {"expired", false},
				{"matched_rule", {{"id", 4}, {"effect", "allow"}, {"decides", false}}}},
			{{"id", 4}, {"tool", "ai.start"}, {"args", "{}"}, {"on_server", true}, {"chat_decidable", false}}})},
		{"rules", json::array({{{"id", 4}, {"effect", "allow"}, {"text", "rule #4 allow command.*"}, {"hits", 2}, {"created_via", "panel"}, {"persisted", true}},
			{{"id", 5}, {"effect", "deny"}, {"text", "rule #5 deny *"}, {"hits", 0}, {"created_via", "console"}}})},
	};
	auto rows = P::Rows(s);
	CHECK(P::Shown(rows) == std::vector<uint64_t>{1, 2, 3, 4});
	CHECK(rows[2].text == "Waiting for your approval (4)");
	const P::Row &built = rows[5];
	CHECK(built.text.find("...") != std::string::npos);          // a long row is cut, the details show it whole
	CHECK(built.text.find("(rule #4 allow, held: you decide it)") != std::string::npos);
	CHECK(built.args == long_args);
	CHECK(built.tool == "command.BuildRail");
	CHECK_FALSE(rows[6].decidable);                                // the server's operator decides ai.start
	CHECK(rows[7].text == "Standing rules (2)");
	CHECK((rows[8].kind == K::Rule && rows[8].id == 4 && rows[8].flag && rows[8].text == "rule #4 allow command.*  hits=2 via=panel persisted"));
	CHECK((rows[9].kind == K::Rule && !rows[9].flag));
	CHECK(P::ToolNames(s) == std::vector<std::string>{"ai.set_setting", "ai.start", "command.BuildRail"});

	/* The selectors: one tool, expired hidden, decided rows hidden, checked rows marked. */
	P::View v;
	v.tool = "ai.set_setting";
	rows = P::Rows(s, v);
	CHECK(P::Shown(rows) == std::vector<uint64_t>{1, 2});
	CHECK(rows[2].text == "Waiting for your approval (2 of 4 shown)");
	v.hide_expired = true;
	CHECK(P::Shown(P::Rows(s, v)) == std::vector<uint64_t>{1});
	v.tool = "none.such";
	CHECK(P::Rows(s, v)[3].text == "nothing the selectors show: choose Tool: all, or show expired");
	/* A row sent to a server stays, marked, and no bulk decision takes it again. */
	P::View sent;
	sent.sent = {1};
	rows = P::Rows(s, sent);
	CHECK(rows[3].sent);
	CHECK(rows[3].text.find("(sent, waiting for the server)") != std::string::npos);
	CHECK(P::BulkTargets(rows, false) == std::vector<uint64_t>{2, 3, 4});

	/* Bulk: every row shown, approve leaving out expired and the operator's; checked rows when any. */
	P::View all;
	rows = P::Rows(s, all);
	CHECK_FALSE(P::AnyChecked(rows));
	CHECK(P::BulkTargets(rows, true) == std::vector<uint64_t>{1, 3});
	CHECK(P::BulkTargets(rows, false) == std::vector<uint64_t>{1, 2, 3, 4});
	all.checked = {2, 3};
	rows = P::Rows(s, all);
	CHECK(P::AnyChecked(rows));
	CHECK(rows[4].checked);
	CHECK(P::BulkTargets(rows, true) == std::vector<uint64_t>{3});
	CHECK(P::BulkTargets(rows, false) == std::vector<uint64_t>{2, 3});

	/* A network client's approve all: the ids the glob and matcher select in its copy of the queue. */
	R::Spec spec;
	spec.tool = "openttd.ai.*";
	CHECK(P::Selected(s, spec) == std::vector<uint64_t>{1});
	spec.tool = "ai.*";
	spec.match = {{"company", 2}};
	CHECK(P::Selected(s, spec) == std::vector<uint64_t>{2});

	/* After a decision the next row under the cursor is selected, else the one before, else none. */
	CHECK(P::NextSelection({1, 2, 3}, {2}, {1, 3}) == std::optional<uint64_t>{3});
	CHECK(P::NextSelection({1, 2, 3}, {3}, {1, 2}) == std::optional<uint64_t>{2});
	CHECK(P::NextSelection({1, 2, 3}, {1, 2}, {3}) == std::optional<uint64_t>{3});
	CHECK(P::NextSelection({1, 2, 3}, {2}, {1}) == std::optional<uint64_t>{1}); // 3 went meanwhile
	CHECK_FALSE(P::NextSelection({1}, {1}, {}).has_value());
	CHECK_FALSE(P::NextSelection({1, 2}, {7}, {1, 2}).has_value());

	/* The row's parts at the same x on every row: box, text, Approve, Deny, Always.... */
	CHECK(P::HitPart(0, 499, 5, 20, 60, 80) == P::RowPart::Check);
	CHECK(P::HitPart(0, 499, 20, 20, 60, 80) == P::RowPart::Text);
	CHECK(P::HitPart(0, 499, 299, 20, 60, 80) == P::RowPart::Text);
	CHECK(P::HitPart(0, 499, 300, 20, 60, 80) == P::RowPart::Approve);
	CHECK(P::HitPart(0, 499, 360, 20, 60, 80) == P::RowPart::Deny);
	CHECK(P::HitPart(0, 499, 420, 20, 60, 80) == P::RowPart::Always);
	CHECK(P::HitPart(0, 499, 499, 20, 60, 80) == P::RowPart::Always);

	/* Go to: the tile or the company an approval names. */
	CHECK(P::TileOf({{"location", 77}}, 100) == std::optional<uint32_t>{77});
	CHECK(P::TileOf({{"tile", 5}}, 100) == std::optional<uint32_t>{5});
	CHECK_FALSE(P::TileOf({{"location", 100}}, 100).has_value());
	CHECK_FALSE(P::TileOf({{"location", -1}}, 100).has_value());
	CHECK_FALSE(P::TileOf("x", 100).has_value());
	CHECK(P::CompanyNamed({{"company", 3}}) == std::optional<int>{3});
	CHECK_FALSE(P::CompanyNamed({{"company", 15}}).has_value());
	CHECK_FALSE(P::CompanyNamed({{"company", "server"}}).has_value());
	CHECK_FALSE(P::CompanyNamed(json::array()).has_value());

	/* A local snapshot carries rules and matched_rule from the core. */
	ClearRulesAndQueue();
	AskPause(true);
	R::Spec owner_like;
	owner_like.tool = "nothing.matches";
	REQUIRE(PrismaticMCP::AddApprovalRule(R::Effect::Allow, owner_like).ok);
	json local = P::Snapshot();
	CHECK(local["rules"].size() == 1);
	CHECK_FALSE(local["approvals"][0].contains("matched_rule"));
	CHECK(local["approvals"][0]["mcp_tool"] == "openttd.game.set_pause");
	ClearRulesAndQueue();
}

TEST_CASE("A glob escapes its wildcards and takes one UTF-8 character for ?, and a matcher compares numbers and strings both ways", "[prismatic_mcp]")
{
	namespace R = PrismaticMCP::Rules;
	CHECK(R::Glob("a\\*b", "a*b"));
	CHECK_FALSE(R::Glob("a\\*b", "axb"));
	CHECK(R::Glob("what\\?", "what?"));
	CHECK_FALSE(R::Glob("what\\?", "whatx"));
	CHECK(R::Glob("a\\\\b", "a\\b"));
	CHECK(R::Glob("trail\\", "trail\\"));            // a lone backslash at the end is itself
	CHECK(R::Glob("?", "\xC5\xBE"));                 // one UTF-8 character, two bytes
	CHECK(R::Glob("n??", "n\xC3\xA1\xC5\xBE"));
	CHECK_FALSE(R::Glob("??", "\xC5\xBE"));
	CHECK(R::Glob("*\xC5\xBE", "abc\xC5\xBE"));
	CHECK(R::Glob("*?x", "\xE2\x82\xACx"));          // a star backtracks by whole characters
	CHECK(R::Glob("?", "\xF0\x9F\x9A\x82"));         // four bytes
	CHECK(R::Glob("?", "\xFF"));                     // a stray byte is one character
	CHECK(R::HasWildcard("openttd.ai.*"));
	CHECK(R::HasWildcard("a?"));
	CHECK_FALSE(R::HasWildcard("openttd.ai.set_setting"));
	CHECK_FALSE(R::HasWildcard("a\\*"));

	/* A number pattern against a string holding it, and a string glob against a number. */
	CHECK(R::Match({{"v", 5}}, {{"v", "5"}}));
	CHECK(R::Match({{"v", "5"}}, {{"v", 5}}));
	CHECK_FALSE(R::Match({{"v", 5}}, {{"v", "05"}}));
	CHECK(R::Match({{"v", true}}, {{"v", "true"}}));
	/* Bounds and sets. */
	const json v = {{"value", 250}, {"name", "max_loan"}};
	CHECK(R::Match({{"value", {{"$gte", 100}, {"$lt", 300}}}}, v));
	CHECK_FALSE(R::Match({{"value", {{"$gt", 250}}}}, v));
	CHECK(R::Match({{"value", {{"$lte", 250}}}}, v));
	CHECK_FALSE(R::Match({{"name", {{"$lt", 5}}}}, v));   // a bound needs a number
	CHECK(R::Match({{"name", {{"$in", json::array({"min_*", "max_*"})}}}}, v));
	CHECK_FALSE(R::Match({{"name", {{"$in", json::array({"x"})}}}}, v));
	CHECK_FALSE(R::Match({{"name", {{"$in", "max_loan"}}}}, v));
	CHECK(R::Match({{"name", {{"$ne", "min_*"}}}}, v));
	CHECK_FALSE(R::Match({{"name", {{"$ne", "max_*"}}}}, v));
	CHECK(R::MatcherError({{"value", {{"$lt", "x"}}}}).has_value());
	CHECK(R::MatcherError({{"value", {{"$in", 3}}}}).has_value());
	CHECK_FALSE(R::MatcherError({{"value", {{"$in", json::array({1, 2})}, {"$ne", 3}}}}).has_value());
	/* An empty list selects nothing through [*]: the key does not match, for an allow rule and a deny rule alike. */
	CHECK_FALSE(R::Match({{"$.params[*]", "*"}}, {{"params", json::array()}}));
	CHECK(R::Match({{"params", json::array()}}, {{"params", json::array()}}));
	/* {} matches anything; a long line is matched whole. */
	CHECK(R::Match(json::object(), json::array({1})));
	const std::string long_value(1024, 'x');
	CHECK(R::Match({{"note", "x*x"}}, {{"note", long_value}}));
	CHECK(R::Glob(std::string(1024, '?'), long_value));
	CHECK_FALSE(R::Glob(std::string(1025, '?'), long_value));

	/* Single quotes at the console: an apostrophe inside a double-quoted string stays, an escaped one in a single-quoted string is one. */
	CHECK(*R::ParseMatcherText("{'name': \"O'Brien\"}") == json{{"name", "O'Brien"}});
	CHECK(*R::ParseMatcherText("{'name': 'O\\'Brien'}") == json{{"name", "O'Brien"}});
	CHECK(*R::ParseMatcherText("{'say': 'a \"quote\"'}") == json{{"say", "a \"quote\""}});
	CHECK(*R::ParseMatcherText("{'p': 'a\\\\b'}") == json{{"p", "a\\b"}});
	CHECK(R::SingleToDoubleQuotes("{'a': 1}") == "{\"a\": 1}");
}

TEST_CASE("An allow rule over a whole glob without a matcher needs --force at the console, and the channels are named", "[prismatic_mcp]")
{
	namespace R = PrismaticMCP::Rules;
	auto spec = [](std::vector<std::string_view> t) { return R::ParseSpec(t); };
	CHECK(R::BroadAllowRefusal(R::Effect::Allow, spec({"openttd.cmd.*"})).has_value());
	CHECK(R::BroadAllowRefusal(R::Effect::Allow, spec({"*", "--force", "--via", "window"})).has_value()); // the console only
	CHECK(R::BroadAllowRefusal(R::Effect::Allow, spec({"*", "--force", "--via", "panel"})).has_value());
	CHECK_FALSE(R::BroadAllowRefusal(R::Effect::Allow, spec({"*", "--force"})).has_value());
	CHECK_FALSE(R::BroadAllowRefusal(R::Effect::Allow, spec({"openttd.cmd.*", "{'company': 1}"})).has_value());
	CHECK_FALSE(R::BroadAllowRefusal(R::Effect::Allow, spec({"openttd.ai.set_setting"})).has_value());
	CHECK_FALSE(R::BroadAllowRefusal(R::Effect::Deny, spec({"*"})).has_value());
	R::Spec s = spec({"*", "--via", "auto", "--nonce", "n-7"});
	CHECK((s.error.empty() && s.via == "auto" && s.nonce == "n-7"));
	CHECK_FALSE(spec({"*", "--nonce"}).error.empty());
	CHECK(R::KnownVia("auto"));
	CHECK_FALSE(R::KnownVia("console")); // the console is the line without --via
	CHECK_FALSE(R::OwnersOwnChannel("auto"));
	CHECK_FALSE(R::OwnersOwnChannel("file"));
	CHECK_FALSE(R::MayDecide(R::Effect::Allow, "auto", "ai.set_settings"));
}

TEST_CASE("Removing a deny rule is refused to injected input, the console answers one result line per --nonce, and rules of a game end with it", "[prismatic_mcp]")
{
	namespace U = PrismaticMCP::UI;
	ClearRulesAndQueue();
	IConsoleStdLibRegister();
	IConsoleCmdExec("prismatic_mcp rule deny openttd.game.* --nonce n1");
	IConsoleCmdExec("prismatic_mcp rule allow openttd.game.set_pause --nonce n2");
	auto rules = PrismaticMCP::ApprovalRules();
	REQUIRE(rules.size() == 2);
	{
		U::InjectionScope script;
		IConsoleCmdExec(fmt::format("prismatic_mcp rule remove {} --nonce n3", rules[0].id)); // the deny rule: refused
		CHECK(PrismaticMCP::ApprovalRules().size() == 2);
		IConsoleCmdExec(fmt::format("prismatic_mcp rule remove {}", rules[1].id)); // the allow rule: anyone may end it
		CHECK(PrismaticMCP::ApprovalRules().size() == 1);
	}
	IConsoleCmdExec("prismatic_mcp rule allow * --nonce n4");     // refused: every tool without --force
	IConsoleCmdExec("prismatic_mcp rule maybe --nonce n5");       // usage
	CHECK(PrismaticMCP::ApprovalRules().size() == 1);
	AskPause(true);
	IConsoleCmdExec("prismatic_mcp deny all game.set_pause --nonce n6");
	IConsoleCmdExec("prismatic_mcp deny 999999 --nonce n7");
	IConsoleCmdExec("prismatic_mcp deny nope --nonce n8");
	IConsoleCmdExec(fmt::format("prismatic_mcp rule remove {} --nonce n9", rules[0].id));
	CHECK(PrismaticMCP::ApprovalRules().empty());

	/* A rule of this game ends with it; a persisted one stays. */
	PrismaticMCP::Rules::Spec game_rule;
	game_rule.tool = "openttd.ai.set_setting";
	game_rule.match = {{"company", 1}};
	REQUIRE(PrismaticMCP::AddApprovalRule(PrismaticMCP::Rules::Effect::Deny, game_rule).ok);
	PrismaticMCP::OnGameModeChanged();
	CHECK(PrismaticMCP::ApprovalRules().empty());
	ClearRulesAndQueue();
}

TEST_CASE("Open approvals in the browser leads to the panel, every other browse item to the hub", "[prismatic_mcp]")
{
	namespace M = PrismaticMCP::Menu;
	CHECK(M::BrowseUrl("/viz") == M::HubUrl() + "/viz");
	CHECK(M::BrowseUrl("panel:/#approvals") == M::PanelUrl() + "/#approvals");
	setenv("PRISMATIC_PANEL_URL", "http://lan:9000//", 1);
	CHECK(M::PanelUrl() == "http://lan:9000");
	unsetenv("PRISMATIC_PANEL_URL");
	CHECK(M::PanelUrl() == "http://127.0.0.1:8800");
	bool listed = false;
	for (const auto &item : M::Items()) listed |= std::string_view{item.label} == "Open approvals in the browser";
	CHECK(listed);
}

TEST_CASE("A network client reads every page of the server's queue before it shows it", "[prismatic_mcp]")
{
	ServerApprovalsReader r;
	CHECK(r.Feed(R"([approvals] {"id": 1, "tool": "ai.set_setting"})"));
	CHECK(r.Feed(R"([approvals] {"count": 2, "page": 1, "pages": 2})"));
	CHECK(r.next_page == 2);
	CHECK(r.rows.empty());      // not whole yet
	CHECK(r.replies == 0);
	CHECK(r.answers == 1);
	CHECK(r.Feed(R"([approvals] {"id": 2, "tool": "ai.set_setting"})"));
	CHECK(r.Feed(R"([approvals] {"count": 2, "page": 2, "pages": 2})"));
	CHECK(r.next_page == 0);
	REQUIRE(r.rows.size() == 2);
	CHECK(r.rows[1]["id"] == 2);
	CHECK(r.replies == 1);
	/* A new first page starts the reply again. */
	CHECK(r.Feed(R"([approvals] {"id": 3})"));
	CHECK(r.Feed(R"([approvals] {"count": 1, "page": 1, "pages": 1})"));
	REQUIRE(r.rows.size() == 1);
	CHECK(r.rows[0]["id"] == 3);
}

TEST_CASE("A held key decides once: every repeat within the quiet time restarts it", "[prismatic_mcp]")
{
	namespace P = PrismaticMCP::Panel;
	using namespace std::chrono_literals;
	const auto t0 = std::chrono::steady_clock::now();
	P::KeyHistory h;
	CHECK(P::KeyTaken(h, 'A', t0));              // the press
	CHECK_FALSE(P::KeyTaken(h, 'A', t0 + 500ms)); // macOS's first repeat, after its initial delay
	CHECK_FALSE(P::KeyTaken(h, 'A', t0 + 583ms)); // then every repeat interval
	CHECK_FALSE(P::KeyTaken(h, 'A', t0 + 1100ms));
	CHECK_FALSE(P::KeyTaken(h, 'A', t0 + 1650ms)); // still held: each sighting restarts the quiet time
	CHECK(P::KeyTaken(h, 'A', t0 + 2300ms));       // released and pressed again, after the quiet time
	CHECK(P::KeyTaken(h, 'D', t0 + 2350ms));       // another key is a new press at once
	CHECK(P::KeyTaken(h, 'A', t0 + 2400ms));
	CHECK(P::KEY_QUIET >= 600ms);
}

TEST_CASE("A broad allow is refused whatever matcher spells any arguments, and {} is no matcher", "[prismatic_mcp]")
{
	namespace R = PrismaticMCP::Rules;
	auto spec = [](std::vector<std::string_view> t) { return R::ParseSpec(t); };
	CHECK(spec({"*", "{}"}).match.is_null());
	CHECK(spec({"*", "{ }"}).match.is_null());
	CHECK(R::IsBroadMatcher(json()));
	CHECK(R::IsBroadMatcher({{"$", "*"}}));
	CHECK(R::IsBroadMatcher({{"$ne", nullptr}}));
	CHECK_FALSE(R::IsBroadMatcher({{"company", 1}}));
	CHECK_FALSE(R::IsBroadMatcher({{"company", 0}}));  // a literal narrows
	CHECK_FALSE(R::IsBroadMatcher({{"name", "max_*"}}));
	for (const char *m : {"{}", "{'$': '*'}", "{'$ne': null}"}) {
		INFO(m);
		CHECK(R::BroadAllowRefusal(R::Effect::Allow, spec({"openttd.cmd.*", m})).has_value());
	}
	CHECK_FALSE(R::BroadAllowRefusal(R::Effect::Allow, spec({"openttd.cmd.*", "{'company': 1}"})).has_value());
	CHECK_FALSE(R::BroadAllowRefusal(R::Effect::Allow, spec({"openttd.cmd.*", "{'$': '*'}", "--force"})).has_value());
}

TEST_CASE("The result line names why each id was refused, rejected or skipped, and the console prints exactly that line", "[prismatic_mcp]")
{
	namespace R = PrismaticMCP::Rules;
	PrismaticMCP::BulkOutcome out;
	out.rule_id = 4;
	out.decided_ids = {7};
	out.refused_ids = {{8, "approval #8 is not pending"}};
	out.rejected_ids = {{9, "the pause command was refused"}};
	out.skipped_ids = {{10, "expired"}};
	CHECK(PrismaticMCP::ResultLine("n1", out, "") == R"([approval-core] {"decided":[7],"error":"","nonce":"n1","ok":true,)"
		R"("refused":[{"error":"approval #8 is not pending","id":8}],"rejected":[{"error":"the pause command was refused","id":9}],)"
		R"("rule_id":4,"skipped":[{"error":"expired","id":10}]})");
	CHECK(PrismaticMCP::ResultLine("n2", {}, "usage") == R"([approval-core] {"decided":[],"error":"usage","nonce":"n2","ok":false,"refused":[],"rejected":[],"rule_id":0,"skipped":[]})");

	/* Tally: a refusal of what was pending and has run is a rejected run; of what was not pending, a refusal. */
	ClearRulesAndQueue();
	const uint64_t id = AskPause(true)["approval_id"].get<uint64_t>();
	PrismaticMCP::BulkOutcome t;
	std::string m;
	const bool approved = AsNetworkClient([&] { return PrismaticMCP::ApproveById(id, m, "console"); });
	PrismaticMCP::Tally(t, id, true, approved, m);
	REQUIRE(t.rejected_ids.size() == 1);
	CHECK(t.rejected_ids[0].id == id);
	CHECK(PrismaticMCP::HasRun(id));
	PrismaticMCP::Tally(t, id, false, false, "approval is not pending");
	REQUIRE(t.refused_ids.size() == 1);
	CHECK(t.refused == 2);

	/* What the console prints, captured through its own log ('script <file>'). */
	const std::filesystem::path log = std::filesystem::temp_directory_path() / fmt::format("prismatic-console-{}.log", id);
	std::filesystem::remove(log);
	IConsoleStdLibRegister();
	const uint64_t waiting = AskPause(false)["approval_id"].get<uint64_t>();
	IConsoleCmdExec(fmt::format("script \"{}\"", log.string()));
	IConsoleCmdExec(fmt::format("prismatic_mcp deny {} {} --nonce c1", waiting, id));
	IConsoleCmdExec("prismatic_mcp rule deny openttd.ai.* {'company': 3} --nonce c2");
	IConsoleCmdExec("prismatic_mcp rule allow openttd.cmd.* {} --nonce c3");
	IConsoleCmdExec("prismatic_mcp approve all openttd.cmd.* --nonce c4");
	IConsoleCmdExec("prismatic_mcp approve --nonce c5");
	IConsoleCmdExec(fmt::format("prismatic_mcp rule remove {} --nonce c6", PrismaticMCP::ApprovalRules().back().id));
	IConsoleCmdExec("script");
	std::ifstream in(log);
	std::vector<json> lines;
	for (std::string l; std::getline(in, l);) {
		const size_t at = l.find("[approval-core] ");
		if (at != std::string::npos) lines.push_back(json::parse(l.substr(at + 16)));
	}
	std::filesystem::remove(log);
	REQUIRE(lines.size() == 6); // one line per command, nothing else carries the tag
	CHECK(lines[0]["nonce"] == "c1");
	CHECK(lines[0]["decided"] == json::array({waiting}));
	CHECK(lines[0]["refused"] == json::array({{{"id", id}, {"error", fmt::format("approval #{} is not pending", id)}}}));
	CHECK((lines[1]["ok"] == true && lines[1]["rule_id"].get<uint64_t>() > 0));
	CHECK(lines[2]["ok"] == false);
	CHECK(lines[2]["error"].get<std::string>().find("--force") != std::string::npos);
	CHECK((lines[3]["ok"] == true && lines[3]["decided"].empty() && lines[3]["skipped"].empty()));
	CHECK((lines[4]["ok"] == false && lines[4]["error"] == "usage"));
	CHECK((lines[5]["ok"] == true && lines[5]["rule_id"] == lines[1]["rule_id"]));
	ClearRulesAndQueue();
}

TEST_CASE("Without a hub address the hub's pages lead to the panel, and an allow rule counts no expired approval", "[prismatic_mcp]")
{
	namespace M = PrismaticMCP::Menu;
	namespace P = PrismaticMCP::Panel;
	unsetenv("PRISMATIC_HUB_URL");
	CHECK(M::HubUrl() == M::PanelUrl());
	setenv("PRISMATIC_HUB_URL", "http://hub:8790/", 1);
	CHECK(M::HubUrl() == "http://hub:8790");
	unsetenv("PRISMATIC_HUB_URL");
	const json s = {{"approvals", json::array({{{"id", 1}, {"tool", "ai.set_setting"}, {"args", "{}"}, {"expired", true}},
		{{"id", 2}, {"tool", "ai.set_setting"}, {"args", "{}"}, {"expired", false}}})}};
	PrismaticMCP::Rules::Spec spec;
	spec.tool = "ai.set_setting";
	CHECK(P::Selected(s, spec) == std::vector<uint64_t>{1, 2});
	CHECK(P::Selected(s, spec, true) == std::vector<uint64_t>{2});
}

TEST_CASE("An allow matcher is broad when it constrains no value, whatever keys or paths it names", "[prismatic_mcp]")
{
	namespace R = PrismaticMCP::Rules;
	/* Broad: every entry is a non-empty run of stars, a lone $ne, or an object that is itself broad. */
	for (const json &m : {json::parse(R"({"company":"*"})"), json::parse(R"({"$.*":"*"})"), json::parse(R"({"$..name":"**"})"),
			json::parse(R"({"a":{"b":"*"}})"), json::parse(R"({"x":{"$ne":null}})"), json::parse(R"({"$":"*"})"), json::object(),
			json::parse(R"({"$ne":3})"), json::parse(R"({"$.l[*]":"*","$['a b']":"***"})")}) {
		INFO(m.dump());
		CHECK(R::IsBroadMatcher(m));
		R::Spec spec;
		spec.tool = "openttd.*";
		spec.match = m;
		spec.via = "window";
		CHECK(R::BroadAllowRefusal(R::Effect::Allow, spec).has_value());
		spec.via = "panel";
		CHECK(R::BroadAllowRefusal(R::Effect::Allow, spec).has_value());
		spec.via = "console";
		CHECK(R::BroadAllowRefusal(R::Effect::Allow, spec).has_value());
		spec.force = true; // the console alone, with --force
		CHECK_FALSE(R::BroadAllowRefusal(R::Effect::Allow, spec).has_value());
		spec.force = false;
		spec.tool = "openttd.ai.set_setting"; // one tool, any arguments: "Always allow X"
		CHECK_FALSE(R::BroadAllowRefusal(R::Effect::Allow, spec).has_value());
	}
	/* Narrowing: a literal, a glob with any other character or a '?', the empty string, $in, bounds, an array. */
	for (const json &m : {json::parse(R"({"company":0})"), json::parse(R"({"name":"a*"})"), json::parse(R"({"n":{"$lt":5}})"),
			json::parse(R"({"c":{"$in":[1]}})"), json::parse(R"({"name":""})"), json::parse(R"({"name":"?"})"),
			json::parse(R"({"n":{"$gte":1}})"), json::parse(R"({"l":["*"]})"), json::parse(R"({"a":{"b":"x"}})"),
			json::parse(R"({"x":{"$ne":null,"$lt":3}})"), json::parse(R"({"company":"*","name":"max_*"})")}) {
		INFO(m.dump());
		CHECK_FALSE(R::IsBroadMatcher(m));
		R::Spec spec;
		spec.tool = "openttd.*";
		spec.match = m;
		spec.via = "panel";
		CHECK_FALSE(R::BroadAllowRefusal(R::Effect::Allow, spec).has_value());
	}
	CHECK_FALSE(R::IsBroadMatcher(json::array()));
	CHECK_FALSE(R::IsBroadMatcher("*"));
	CHECK_FALSE(R::IsBroadValue(5));
	/* The console refuses the bypass the probes let through. */
	ClearRulesAndQueue();
	IConsoleStdLibRegister();
	IConsoleCmdExec("prismatic_mcp rule allow openttd.* {'company': '*'}");
	IConsoleCmdExec("prismatic_mcp rule allow openttd.* {'$.*': '*'} --via panel");
	CHECK(PrismaticMCP::ApprovalRules().empty());
	IConsoleCmdExec("prismatic_mcp rule allow openttd.* {'company': '*'} --force");
	CHECK(PrismaticMCP::ApprovalRules().size() == 1);
	ClearRulesAndQueue();
}

TEST_CASE("The window refuses a broad allow and an injected allow rule with the core's words, before it asks to confirm", "[prismatic_mcp]")
{
	namespace P = PrismaticMCP::Panel;
	namespace R = PrismaticMCP::Rules;
	namespace U = PrismaticMCP::UI;
	R::Spec broad;
	broad.tool = "openttd.*";
	broad.match = {{"company", "*"}};
	broad.force = true; // the window never forces: it says "window", and --force is the console's
	auto refused = P::RuleRefusal(R::Effect::Allow, broad);
	REQUIRE(refused.has_value());
	R::Spec as_core = broad;
	as_core.via = "window";
	CHECK(*refused == *R::BroadAllowRefusal(R::Effect::Allow, as_core));
	CHECK_FALSE(P::RuleRefusal(R::Effect::Deny, broad).has_value()); // a broad deny enacts nothing
	R::Spec one_tool;
	one_tool.tool = "openttd.ai.set_setting"; // "Always allow X (any arguments)"
	CHECK_FALSE(P::RuleRefusal(R::Effect::Allow, one_tool).has_value());
	R::Spec narrow = broad;
	narrow.match = {{"company", 1}};
	CHECK_FALSE(P::RuleRefusal(R::Effect::Allow, narrow).has_value());
	{
		U::InjectionScope script;
		CHECK(P::RuleRefusal(R::Effect::Allow, one_tool) == P::ConsentRefusal(true));
		CHECK_FALSE(P::RuleRefusal(R::Effect::Deny, one_tool).has_value());
	}
}

TEST_CASE("Unattended channels approve only for a company a registered approver plays, as #264's deadline does", "[prismatic_mcp]")
{
	namespace R = PrismaticMCP::Rules;
	CHECK(R::Unattended("auto"));
	CHECK(R::Unattended("file"));
	for (const char *via : {"console", "window", "panel"}) {
		INFO(via);
		CHECK_FALSE(R::Unattended(via));
		CHECK(R::UnattendedMayApprove(via, false, -1, false)); // the owner's present act is not held
	}
	for (const char *via : {"auto", "file"}) {
		INFO(via);
		CHECK(R::UnattendedMayApprove(via, true, 2, true));    // one company's action, its approver plays
		CHECK_FALSE(R::UnattendedMayApprove(via, true, 2, false)); // no registered approver plays company 2
		CHECK_FALSE(R::UnattendedMayApprove(via, true, -1, true)); // it names no company
		CHECK_FALSE(R::UnattendedMayApprove(via, false, 2, true)); // wider than one company: the operator's
	}
	CHECK(R::FileRuleNote(7) == "rule #7 (from the rules file) decides only for companies a registered approver plays; remake it at the console to make it unconditional");
	/* The window shows the note beside an allow rule from the file, not beside a deny rule or one made in the game. */
	namespace P = PrismaticMCP::Panel;
	const json s = {{"rules", json::array({{{"id", 7}, {"effect", "allow"}, {"text", "rule #7 allow ai.set_setting"}, {"created_via", "file"}, {"persisted", true}},
		{{"id", 8}, {"effect", "deny"}, {"text", "rule #8 deny *"}, {"created_via", "file"}},
		{{"id", 9}, {"effect", "allow"}, {"text", "rule #9 allow ai.set_setting"}, {"created_via", "console"}}})}};
	std::vector<P::Row> rules;
	for (const auto &r : P::Rows(s)) if (r.kind == P::Row::Kind::Rule) rules.push_back(r);
	REQUIRE(rules.size() == 3);
	CHECK(rules[0].text.find(R::FileRuleNote(7)) != std::string::npos);
	CHECK(rules[1].text.find("from the rules file") == std::string::npos);
	CHECK(rules[2].text.find("from the rules file") == std::string::npos);
}

TEST_CASE("The deadline's approve stands in only for a registered approver of the company it names, when asked and at the deadline", "[prismatic_mcp]")
{
	DeadlinePolicy approve{30 * DEADLINE_DAY_TICKS, DeadlineVerdict::Approve};
	const uint64_t due = 1000 + approve.ticks;
	/* An approver played the company when it was asked and plays it now: executed. */
	CHECK(DeadlineDue(true, 1000, due, approve, true, true) == DeadlineAction::Execute);
	/* No approver plays that company (an agent's request for another company): retired, nothing enacted. */
	CHECK(DeadlineDue(true, 1000, due, approve, true, false) == DeadlineAction::Retire);
	/* Not due yet: nothing, approver or not. */
	CHECK(DeadlineDue(true, 1000, due - 1, approve, true, false) == DeadlineAction::None);

	std::vector<ApproverSeat> seats = {{1, 7, "owner-key-fp"}};
	CHECK(ApproverPlays(seats, 1));
	CHECK_FALSE(ApproverPlays(seats, 2));
	/* An approval that names no company is never the approver's. */
	CHECK_FALSE(ApproverPlays(seats, -1));
	CHECK_FALSE(ApproverPlays({}, 1));
}

TEST_CASE("Approver seats are the companies registered approvers play, one client per key, never a spectator", "[prismatic_mcp]")
{
	const std::string owner = "OWNERKEY0123456789abcdef";
	const std::string agent = "AGENTKEY0123456789abcdef";
	std::vector<ClientSeat> clients = {
		{2, 1, owner},  // the owner plays company 1
		{1, -1, agent}, // a spectator with an unregistered key
		{5, 3, agent},  // the agent's company, not registered
	};
	std::vector<std::string> registered = {owner};
	auto seats = ApproverSeats(clients, registered);
	REQUIRE(seats.size() == 1);
	CHECK(seats[0] == ApproverSeat{1, 2, KeyFingerprint(owner)});
	CHECK(seats[0].key_fp == "OWNERKEY0123");

	/* The registered approver as a spectator plays no company: no seat. */
	CHECK(ApproverSeats({{2, -1, owner}}, registered).empty());
	/* Not connected: no seat. */
	CHECK(ApproverSeats({{5, 3, agent}}, registered).empty());
	/* Nothing registered: no seat, whoever plays. */
	CHECK(ApproverSeats(clients, {}).empty());

	/* Two registered approvers in two companies, sorted by company. */
	auto two = ApproverSeats({{9, 4, agent}, {2, 1, owner}}, {agent, owner});
	REQUIRE(two.size() == 2);
	CHECK(two[0].company == 1);
	CHECK(two[1].company == 4);
}

TEST_CASE("A registered key that a second client connects with approves from neither and has no seat", "[prismatic_mcp]")
{
	const std::string owner = "OWNERKEY0123456789abcdef";
	/* OpenTTD admits two connections with one key: the owner's client and a copy of its key. */
	std::vector<ClientSeat> clients = {{2, 1, owner}, {6, 1, owner}};
	CHECK(ClientsWithKey(clients, owner) == 2);
	CHECK(ClientsWithKey(clients, "") == 0);
	CHECK(ClientsWithKey({{2, 1, owner}}, owner) == 1);
	CHECK(ApproverSeats(clients, {owner}).empty());

	ChatFacts f;
	f.exists = f.pending = f.chat_decidable = f.valid_company = f.same_company = f.key_registered = true;
	CHECK(DecideChat(f, true) == ChatVerdict::Allow);
	f.key_shared = true;
	CHECK(DecideChat(f, true) == ChatVerdict::SharedKey);
	/* A deny executes nothing, so a shared key still withdraws its own company's approval. */
	CHECK(DecideChat(f, false) == ChatVerdict::Allow);
	CHECK(ChatVerdictText(ChatVerdict::SharedKey).find("connected twice") != std::string_view::npos);
}

TEST_CASE("A rule from the rules file and the auto channel hold where #264's deadline would, and say so", "[prismatic_mcp]")
{
	namespace R = PrismaticMCP::Rules;
	ClearRulesAndQueue();
	IConsoleStdLibRegister();
	/* A file-loaded allow rule: said at load, and held for an approval no registered approver's company owns. */
	const uint16_t port = _settings_client.network.server_port;
	_settings_client.network.server_port = 40421;
	{
		std::ofstream f(PrismaticMCP::RulesFile());
		f << R"({"rules": [{"effect": "allow", "tool": "game.set_pause", "match": {"pause": false}, "created_via": "console"}]})";
	}
	std::filesystem::permissions(PrismaticMCP::RulesFile(), std::filesystem::perms::owner_read | std::filesystem::perms::owner_write, std::filesystem::perm_options::replace);
	auto said = PrismaticMCP::LoadApprovalRules();
	REQUIRE(said.size() == 2);
	const uint64_t file_rule = PrismaticMCP::ApprovalRules().front().id;
	CHECK(said[1] == R::FileRuleNote(file_rule));
	CHECK(PrismaticMCP::ApprovalRules().front().created_via == "file");
	json held = AskPause(false);
	CHECK(held["status"] == "pending_approval"); // game.set_pause is wider than one company: no approver can own it
	const uint64_t id = held["approval_id"].get<uint64_t>();
	auto pending = PrismaticMCP::PendingApprovals();
	REQUIRE(pending.size() == 1);
	CHECK((pending[0].matched_rule == file_rule && !pending[0].rule_decides));
	CHECK(PrismaticMCP::ApprovalRules().front().hits == 0);
	CHECK(PrismaticMCP::UnattendedHoldOf(id, "file").find("held: a rule from the rules file approves only one company's own action") == 0);
	CHECK(PrismaticMCP::UnattendedHoldOf(id, "auto").find("held: the auto channel") == 0);
	CHECK(PrismaticMCP::UnattendedHoldOf(id, "panel").empty()); // the owner's route is not held
	CHECK(PrismaticMCP::UnattendedHoldOf(999999, "auto").empty());
	ClearRulesAndQueue();
	std::filesystem::remove(PrismaticMCP::RulesFile());
	_settings_client.network.server_port = port;

	/* The auto channel: a rule made via auto is held as it is added, approve all --via auto skips, per id is refused. */
	const uint64_t resume = AskPause(false)["approval_id"].get<uint64_t>();
	R::Spec by_auto;
	by_auto.tool = "game.set_pause";
	by_auto.match = {{"pause", false}};
	by_auto.via = "auto";
	auto added = PrismaticMCP::AddApprovalRule(R::Effect::Allow, by_auto);
	REQUIRE(added.ok);
	CHECK((added.decided == 0 && added.skipped == 1));
	REQUIRE(added.skipped_ids.size() == 1);
	CHECK(added.skipped_ids[0].error.find("held: the auto channel approves only one company's own action") == 0);
	std::string m;
	PrismaticMCP::RemoveApprovalRule(added.rule_id, m);
	R::Spec all;
	all.tool = "game.set_pause";
	all.via = "auto";
	auto out = PrismaticMCP::DecideAll(true, all);
	CHECK((out.decided == 0 && out.skipped == 1));
	CHECK(out.skipped_ids[0].id == resume);
	IConsoleCmdExec(fmt::format("prismatic_mcp approve {} --via auto", resume));
	CHECK(PrismaticMCP::HoldsPending(resume));
	/* The owner's own channels decide it: the console, the window, the owner-authenticated panel route. */
	all.via = "panel";
	out = AsNetworkClient([&] { return PrismaticMCP::DecideAll(true, all); });
	CHECK(out.skipped == 0);
	CHECK_FALSE(PrismaticMCP::HoldsPending(resume));
	ClearRulesAndQueue();
}

#endif /* OTTD_PRISMATIC_MCP_SERVER */
