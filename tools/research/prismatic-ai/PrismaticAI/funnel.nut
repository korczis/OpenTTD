/*
 * The growth funnel (RESEARCH-ONLY observability: it counts and logs, never acts).
 *
 * Growth runs through four stages: candidate discovery, affordability, construction, and
 * operation that pays. The evidence of where it stops was scattered through the log: producers
 * skipped as claimed, "0 pairs tried", path searches that ran out of nodes, single-track
 * fallbacks, a locomotive refused for money (a line idle 99 days in live game 10487), 117
 * identical bus failures, and 165M idle in the bank of a company that ran 40 trains. Widening the
 * search without counting each stage may only add failures (research/growth-funnel.md).
 *
 * Funnel(mode, stage) counts one event of a stage, at the place the bot already decides it.
 * FunnelCensus keeps the last own-freight-line search's skip counts (rail.nut NextOwnFreightLine).
 * FunnelTick, once a heartbeat, counts the heartbeats short of money for a line and those with
 * money idle. The first count or heartbeat of a new game year logs one line per mode for the year
 * before (FunnelRoll), then starts again from zero:
 *
 *   FUNNEL year=<y> mode=rail hb=<n> short_hb=<n> idle_hb=<n> searches=<n> found=<n>
 *     producers=<n> ours=<n> claimed=<n> small=<n> served=<n> no_acceptor=<n> pairs_tried=<n>
 *     tried=<n> stations=<n> path_fail=<n> laid=<n> built=<n> single_track=<n> trainless=<n>
 *     loco_refused_cash=<n> trained=<n> lines=<n> paying=<n> trains=<n> trains_added=<n>
 *   FUNNEL year=<y> mode=road tried=<n> stop_fail=<n> network_fail=<n> depot_fail=<n>
 *     vehicle_fail=<n> built=<n> invested=<n> vehicles=<n> paying=<n>
 *   FUNNEL year=<y> mode=air short=<n> no_pair=<n> tried=<n> built=<n> invested=<n> vehicles=<n> paying=<n>
 *   FUNNEL year=<y> mode=ship invested=<n> vehicles=<n> paying=<n>
 *
 * (one log line each; a mode with nothing counted and no vehicle is left out; ` partial=1` ends the
 * line of a year the count joined late). Logged with
 * AILog.Error, like the referee's REF lines, because a headless match runs at -d script=1, which
 * shows only that level; battle.py reads them (parse_funnel). The counters live in globals and
 * are not saved: after a load the first line covers only the part of the year since the load, and
 * says so (partial=1). An event counts in the game year it is counted in, even inside a heartbeat
 * that builds for game months.
 *
 * What the counts mean: the census (producers to pairs_tried) is the last search's, not a sum,
 * since each search walks every producer again; searches, found and the build stages sum the
 * year's events. A double track retried single track counts tried and stations twice (it builds
 * new stations and searches again). lines, paying and trains are what runs at the report;
 * paying is a rail line whose trains together earned money last year, or a road, air or water
 * vehicle a year old that did.
 */

/* Each mode's stages in the order its line prints them, the census's keys before the events'. */
FUNNEL_ORDER <- {
	rail = ["hb", "short_hb", "idle_hb", "searches", "found", "producers", "ours", "claimed", "released",
		"small", "served", "no_acceptor", "pairs_tried", "tried", "stations", "path_fail", "laid", "built",
		"single_track", "trainless", "loco_refused_cash", "trained"],
	road = ["tried", "stop_fail", "network_fail", "depot_fail", "vehicle_fail", "built", "invested"],
	air = ["short", "no_pair", "tried", "built", "invested"],
	ship = ["invested"]
};
/* The counts of the year in progress, by mode and stage. */
FUNNEL <- {};
/* The game year FUNNEL counts for (-1: not started). */
FUNNEL_YEAR <- -1;
/* The trains the company ran when the year's count started. */
FUNNEL_TRAINS <- 0;
/* Whether the year's count started after its first month (a load, a late start). */
FUNNEL_PARTIAL <- false;

/* One more event of `stage` in `mode` this year. */
function PrismaticAI::Funnel(mode, stage) {
	PrismaticAI.FunnelRoll();
	if (!(mode in FUNNEL)) FUNNEL[mode] <- {};
	local m = FUNNEL[mode];
	if (stage in m) m[stage]++;
	else m[stage] <- 1;
}

/* The funnel mode of an AIVehicle type: the investor's road, air and water vehicles. */
function PrismaticAI::FunnelMode(type) {
	if (type == AIVehicle.VT_AIR) return "air";
	if (type == AIVehicle.VT_WATER) return "ship";
	if (type == AIVehicle.VT_RAIL) return "rail";
	return "road";
}

/* The own-freight-line search just ran: `why` holds its skip counts (rail.nut NextOwnFreightLine),
 * kept as the year's census (the last search's, not a sum: each search walks every producer
 * again); `found` whether it returned a line. */
function PrismaticAI::FunnelCensus(why, found) {
	PrismaticAI.Funnel("rail", "searches");
	if (found) PrismaticAI.Funnel("rail", "found");
	local m = FUNNEL["rail"];
	foreach (k, v in why) m[(k == "tried") ? "pairs_tried" : k] <- v;
}

/* The year's lines and a fresh count when the game year changed since the count started. */
function PrismaticAI::FunnelRoll() {
	local now = AIDate.GetCurrentDate();
	local year = AIDate.GetYear(now);
	if (FUNNEL_YEAR == -1) {
		FUNNEL_YEAR = year;
		FUNNEL_TRAINS = PrismaticAI.FunnelTrains();
		FUNNEL_PARTIAL = AIDate.GetMonth(now) > 1;
	}
	if (year == FUNNEL_YEAR) return;
	PrismaticAI.FunnelReport(FUNNEL_YEAR);
	FUNNEL = {};
	FUNNEL_YEAR = year;
	FUNNEL_PARTIAL = false;
}

/* Once a heartbeat, from the main loop: the heartbeat counts, and the year's lines when the game
 * year changed. Short: the bank plus the loan headroom are under line_budget, the price a new own
 * line needs. Idle: the bank alone holds the investor's threshold (fleet.nut InvestSurplus), money
 * that buys no train, since InvestSurplus adds road, air and water vehicles only. */
function PrismaticAI::FunnelTick(b) {
	local money = AICompany.GetBankBalance(AICompany.COMPANY_SELF);
	PrismaticAI.Funnel("rail", "hb");
	if (money + AICompany.GetMaxLoanAmount() - AICompany.GetLoanAmount() < b.line_budget) PrismaticAI.Funnel("rail", "short_hb");
	if (money >= max(b.invest_min, INVEST_SURPLUS_RESERVES * b.cash_reserve)) PrismaticAI.Funnel("rail", "idle_hb");
}

/* The company's trains. */
function PrismaticAI::FunnelTrains() {
	local trains = AIVehicleList();
	trains.Valuate(AIVehicle.GetVehicleType);
	trains.KeepValue(AIVehicle.VT_RAIL);
	return trains.Count();
}

/* The year's FUNNEL lines, with what runs now: rail lines open, those whose trains together earned
 * money last year (paying), the trains and how many more than at the year's start; per other mode
 * the vehicles and those that earned money last year among the ones a year old. */
function PrismaticAI::FunnelReport(year) {
	local trains = 0, lines = 0, paying = 0;
	foreach (line in RAIL_LINES) {
		if (line.closed) continue;
		lines++;
		local profit = 0, own = PrismaticAI.TrainsOf(line);
		foreach (v in own) profit += AIVehicle.GetProfitLastYear(v);
		if (own.len() > 0 && profit > 0) paying++;
	}
	local vehicles = {}, earning = {};
	foreach (v, _ in AIVehicleList()) {
		local mode = PrismaticAI.FunnelMode(AIVehicle.GetVehicleType(v));
		if (mode == "rail") { trains++; continue; }
		vehicles[mode] <- ((mode in vehicles) ? vehicles[mode] : 0) + 1;
		if (AIVehicle.GetAge(v) >= 365 && AIVehicle.GetProfitLastYear(v) > 0) earning[mode] <- ((mode in earning) ? earning[mode] : 0) + 1;
	}
	foreach (mode in ["rail", "road", "air", "ship"]) {
		local m = (mode in FUNNEL) ? FUNNEL[mode] : {};
		local n = (mode in vehicles) ? vehicles[mode] : 0;
		if (mode != "rail" && m.len() == 0 && n == 0) continue;
		local s = "FUNNEL year=" + year + " mode=" + mode;
		foreach (k in FUNNEL_ORDER[mode]) {
			/* `released` only where the search counts it (a producer another station only catches). */
			if (k == "released" && !(k in m)) continue;
			s += " " + k + "=" + ((k in m) ? m[k] : 0);
		}
		if (mode == "rail") {
			s += " lines=" + lines + " paying=" + paying + " trains=" + trains + " trains_added=" + (trains - FUNNEL_TRAINS);
		} else {
			s += " vehicles=" + n + " paying=" + ((mode in earning) ? earning[mode] : 0);
		}
		if (FUNNEL_PARTIAL) s += " partial=1";
		AILog.Error(s);
	}
	FUNNEL_TRAINS = trains;
}
