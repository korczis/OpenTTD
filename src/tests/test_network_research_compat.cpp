/*
 * This file is part of korczis's PRIVATE RESEARCH FORK of OpenTTD and is NOT part of upstream.
 * RESEARCH-ONLY. Compiled only when OTTD_RESEARCH_INSTRUMENTATION is defined.
 */

/** @file test_network_research_compat.cpp The research override of the network revision check. */

#include "../stdafx.h"

#ifdef OTTD_RESEARCH_INSTRUMENTATION

#include "../3rdparty/catch2/catch.hpp"
#include "../network/core/network_game_info.h"

#include "../safeguards.h"

/*
 * The policy only ever widens which peers are accepted; it never narrows the genuine self-match
 * and never changes the revision this build reports (that function has no setter). "other" below
 * is a revision this build certainly is not.
 */
static constexpr std::string_view OTHER = "a-revision-this-build-certainly-is-not";

TEST_CASE("research_net_compat policy only widens which peer revisions are accepted", "[research]")
{
	const ResearchNetCompat saved = _research_net_compat;
	const std::string_view self = GetNetworkRevisionString();

	SECTION("strict is upstream behaviour: a genuine match passes, a mismatch does not") {
		_research_net_compat = {};
		CHECK(IsNetworkCompatibleVersion(self));
		CHECK_FALSE(IsNetworkCompatibleVersion(OTHER));
	}

	SECTION("open accepts any peer") {
		_research_net_compat = {ResearchRevisionPolicy::Open, {}, {}};
		CHECK(IsNetworkCompatibleVersion(OTHER));
		CHECK(IsNetworkCompatibleVersion(""));
		CHECK(IsNetworkCompatibleVersion(self));
	}

	SECTION("whitelist accepts a genuine match plus listed revisions, and nothing else") {
		_research_net_compat = {ResearchRevisionPolicy::Whitelist, {std::string(OTHER)}, {}};
		CHECK(IsNetworkCompatibleVersion(OTHER));                 // listed
		CHECK(IsNetworkCompatibleVersion(self));                  // genuine match still passes
		CHECK_FALSE(IsNetworkCompatibleVersion("something-else-entirely"));
	}

	SECTION("blocklist accepts everything except listed revisions") {
		_research_net_compat = {ResearchRevisionPolicy::Blocklist, {}, {std::string(OTHER)}};
		CHECK_FALSE(IsNetworkCompatibleVersion(OTHER));           // blocked
		CHECK(IsNetworkCompatibleVersion("any-other-revision"));  // not blocked
	}

	_research_net_compat = saved;
}

TEST_CASE("research_net_compat policy names and aliases parse both ways", "[research]")
{
	CHECK(ResearchNetRevisionPolicyName(ResearchRevisionPolicy::Strict) == "strict");
	CHECK(ResearchNetRevisionPolicyName(ResearchRevisionPolicy::Open) == "open");
	CHECK(ParseResearchNetRevisionPolicy("whitelist") == ResearchRevisionPolicy::Whitelist);
	CHECK(ParseResearchNetRevisionPolicy("full") == ResearchRevisionPolicy::Open);   // alias
	CHECK(ParseResearchNetRevisionPolicy("on") == ResearchRevisionPolicy::Open);     // boolean alias
	CHECK(ParseResearchNetRevisionPolicy("off") == ResearchRevisionPolicy::Strict);  // boolean alias
	CHECK_FALSE(ParseResearchNetRevisionPolicy("nonsense").has_value());
}

TEST_CASE("A client rejoins the company it played after the server restarts its game", "[research]")
{
	/* Upstream reconnects every client as a spectator; the owner co-playing company 1 asks for it again. */
	CHECK(ResearchRejoinCompany(CompanyID{0}) == CompanyID{0});
	CHECK(ResearchRejoinCompany(CompanyID{MAX_COMPANIES - 1}) == CompanyID{MAX_COMPANIES - 1});
	/* A spectator, or a client that was still choosing a company, rejoins as upstream does. */
	CHECK(ResearchRejoinCompany(COMPANY_SPECTATOR) == COMPANY_SPECTATOR);
	CHECK(ResearchRejoinCompany(COMPANY_NEW_COMPANY) == COMPANY_SPECTATOR);
	CHECK(ResearchRejoinCompany(CompanyID::Invalid()) == COMPANY_SPECTATOR);
	/* Until a server restart is seen, the reconnect company is the upstream one. */
	CHECK(_research_rejoin_company == COMPANY_SPECTATOR);
}

#endif /* OTTD_RESEARCH_INSTRUMENTATION */
