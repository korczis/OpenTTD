/*
 * Part of korczis's PRIVATE RESEARCH FORK of OpenTTD. RESEARCH-ONLY.
 * Compiled only when OTTD_PRISMATIC_MCP_SERVER is defined.
 */

/** @file test_prismatic_mcp.cpp Unit tests for the pure MCP security/logic helpers. */

#include "../stdafx.h"

#ifdef OTTD_PRISMATIC_MCP_SERVER

#include "../3rdparty/catch2/catch.hpp"
#include "../core/format.hpp"
#include "../prismatic_mcp/prismatic_mcp_detail.h"
#include "../prismatic_mcp/prismatic_mcp_menu.h"
#include "../prismatic_mcp/prismatic_mcp.h"
#include "../prismatic_mcp/prismatic_mcp_panel.h"
#include "../prismatic_mcp/prismatic_mcp_ui.h"
#include "../prismatic_mcp/prismatic_mcp_console.h"
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

#include <set>
#include "../prismatic_mcp/prismatic_mcp_params.hpp"
#include "../news_type.h"
#include "../order_base.h"
#include "../tile_type.h"

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
			case M::Action::Browse: CHECK(arg.rfind("/", 0) == 0); break;
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
	CHECK(kinds == std::vector<K>{K::Heading, K::Text, K::Heading, K::Approval, K::Approval, K::Heading, K::Text,
		K::Heading, K::Text, K::Text, K::Heading, K::Switch});
	CHECK(rows[1].text == "listening on 127.0.0.1:9000");
	CHECK(rows[2].text == "Waiting for your approval (2)");
	/* An approval row carries its id for Approve and Deny, and whether it expired. */
	CHECK((rows[3].id == 7 && !rows[3].flag && rows[3].text == "#7  openttd.game.set_pause  {\"pause\":true}"));
	CHECK((rows[4].id == 9 && rows[4].flag));
	CHECK(rows[6].text.find("VERIFIED_SUCCESS") != std::string::npos);
	CHECK(rows[8].text == "#2 ask: nearest forest - Forest at 12x40");
	CHECK(rows[9].text == "#3 watch - waiting for an answer"); // an unanswered request with a null answer
	CHECK((rows[11].id == 6 && rows[11].flag && rows[11].text == "Human player: paused"));

	/* Nothing waits, nothing ran, nothing asked, no switch: each section says so, no switch heading. */
	auto quiet = P::Rows({{"status", "stopped"}, {"approvals", json::array()}, {"decisions", json::array()},
		{"requests", json::array()}, {"switches", json::array()}});
	REQUIRE(quiet.size() == 8);
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
	CHECK(partial[5].text == "42");
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
	CHECK(P::ConsentCommand(true, 7) == "prismatic_mcp approve 7");
	CHECK(P::ConsentCommand(false, 12) == "prismatic_mcp deny 12");
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

#endif /* OTTD_PRISMATIC_MCP_SERVER */
