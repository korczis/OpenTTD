/*
 * RESEARCH-ONLY referee Game Script -- see info.nut.
 *
 * Line format (one per company, all fields space separated key=value, name last because
 * it may contain spaces):
 *   REF Q date=<y>-<m> cid=<n> value=<v> money=<m> loan=<l> income=<i> perf=<p> vehicles=<n>
 *       stations=<n> rail=<n> road=<n> air=<n> water=<n> prail=<p> proad=<p> pair=<p>
 *       pwater=<p> cargo=<delivered> expenses=<e> name=<company name>
 * rail..water count vehicles by transport mode; prail..pwater are their summed profit last year;
 * cargo is cargo delivered and expenses the expenses, both of the last completed quarter.
 * value/income/perf are for the last *completed* quarter (GSCompany quarter index 1);
 * money (the bank balance, borrowed money included), loan, vehicles and stations are the current
 * values: a company that just drew its whole loan to build shows the loan as money, so the
 * orchestrator judges hoarding on money minus loan (battle.py hoarding).
 *
 * Shared producers, one line per producing industry and cargo that rail stations of two or more
 * companies pick up (stations with a rating for that cargo), after the company lines of each
 * quarter:
 *   REF S date=<y>-<m> ind=<industry id> cargo=<cargo id> owners=<cid>,<cid>[,...] name=<industry name>
 * Two lines taking one cargo from a producer split it by station rating; the orchestrator fails the
 * fitness check when two of them are PrismaticAI's (research/incident-2026-11-22-sibling-industries.md).
 * Two companies taking different cargoes of one farm split nothing, so they are no shared producer.
 *
 * Trainless rail stations, one line per company with a rail station, after the shared producers:
 *   REF T date=<y>-<m> cid=<n> rail_stations=<n> trainless=<n> ids=<station id>,<station id>[,...]
 * trainless counts the company's rail stations that no train of its own has orders to: the termini
 * of lines that never got a train or were given up half laid; ids names them (empty when none). The
 * orchestrator fails the fitness check when a PrismaticAI company keeps the same ones for a year, so
 * a line still being laid does not count (research/incident-2026-11-23-trainless-lines.md).
 */
class BattleReferee extends GSController {
	function Start();
	function Report();
	function ReportShared(d);
	function ReportTrainless(d);
	function LoanOf(c);
}

/* The loan of company `c`: GSCompany.GetLoanAmount answers for the company of the current mode
 * only, so it is read in that company's mode, which ends when this function returns. */
function BattleReferee::LoanOf(c)
{
	local mode = GSCompanyMode(c);
	return GSCompany.GetLoanAmount();
}

function BattleReferee::ReportTrainless(d)
{
	local rail = {}, bare = {}, ids = {};
	foreach (st, _ in GSStationList(GSStation.STATION_TRAIN)) {
		local o = GSStation.GetOwner(st);
		rail[o] <- (rail.rawin(o) ? rail[o] : 0) + 1;
		/* The trains of the station's own company that have orders to it (deity mode sees all). */
		local trains = GSVehicleList_Station(st);
		trains.Valuate(GSVehicle.GetVehicleType);
		trains.KeepValue(GSVehicle.VT_RAIL);
		trains.Valuate(GSVehicle.GetOwner);
		trains.KeepValue(o);
		if (!trains.IsEmpty()) continue;
		bare[o] <- (bare.rawin(o) ? bare[o] : 0) + 1;
		ids[o] <- (ids.rawin(o) ? ids[o] + "," : "") + st;
	}
	foreach (o, n in rail) {
		GSLog.Error("REF T date=" + GSDate.GetYear(d) + "-" + GSDate.GetMonth(d) + " cid=" + o
			+ " rail_stations=" + n + " trainless=" + (bare.rawin(o) ? bare[o] : 0) + " ids=" + (ids.rawin(o) ? ids[o] : ""));
	}
}

function BattleReferee::ReportShared(d)
{
	local radius = GSStation.GetCoverageRadius(GSStation.STATION_TRAIN);
	foreach (i, _ in GSIndustryList()) {
		if (GSIndustry.IsBuiltOnWater(i)) continue;
		local tiles = GSTileList_IndustryProducing(i, radius);
		if (tiles.IsEmpty()) continue;
		tiles.Valuate(GSRail.IsRailStationTile);
		tiles.KeepValue(1);
		/* Only a station that picks up what the industry produces: one with a rating for the cargo.
		 * A station that delivers to a sawmill or a factory also catches it, and two companies
		 * delivering to one acceptor take nothing from each other; nor do two taking different
		 * cargoes of one farm. */
		foreach (c, _ in GSCargoList_IndustryProducing(i)) {
			local owners = {};
			foreach (t, _ in tiles) {
				if (GSStation.HasCargoRating(GSStation.GetStationID(t), c)) owners[GSTile.GetOwner(t)] <- true;
			}
			if (owners.len() < 2) continue;
			local ids = [];
			foreach (o, _ in owners) ids.push(o);
			ids.sort();
			local list = "";
			foreach (k, o in ids) list += (k > 0 ? "," : "") + o;
			GSLog.Error("REF S date=" + GSDate.GetYear(d) + "-" + GSDate.GetMonth(d) + " ind=" + i + " cargo=" + c
				+ " owners=" + list + " name=" + GSIndustry.GetName(i));
		}
	}
}

function BattleReferee::Report()
{
	local d = GSDate.GetCurrentDate();
	/* One pass over every vehicle: count and last year's profit per owner and transport mode,
	 * so the orchestrator can say where a company earns and where it loses. */
	local kinds = [GSVehicle.VT_RAIL, GSVehicle.VT_ROAD, GSVehicle.VT_AIR, GSVehicle.VT_WATER];
	local agg = {};
	local all = GSVehicleList();
	foreach (v, _ in all) {
		local o = GSVehicle.GetOwner(v);
		if (!agg.rawin(o)) agg[o] <- { n = [0, 0, 0, 0], p = [0, 0, 0, 0] };
		local t = GSVehicle.GetVehicleType(v);
		foreach (i, k in kinds) {
			if (t != k) continue;
			agg[o].n[i]++;
			agg[o].p[i] += GSVehicle.GetProfitLastYear(v);
		}
	}
	for (local c = GSCompany.COMPANY_FIRST; c <= GSCompany.COMPANY_LAST; c++) {
		if (GSCompany.ResolveCompanyID(c) == GSCompany.COMPANY_INVALID) continue;
		local a = agg.rawin(c) ? agg[c] : { n = [0, 0, 0, 0], p = [0, 0, 0, 0] };
		local st = GSStationList(GSStation.STATION_ANY);
		st.Valuate(GSStation.GetOwner);
		st.KeepValue(c);
		/* GSLog.Error so the line shows at `-d script=1` without the AIs' info chatter. */
		GSLog.Error("REF Q date=" + GSDate.GetYear(d) + "-" + GSDate.GetMonth(d)
			+ " cid=" + c
			+ " value=" + GSCompany.GetQuarterlyCompanyValue(c, 1)
			+ " money=" + GSCompany.GetBankBalance(c)
			+ " loan=" + this.LoanOf(c)
			+ " income=" + GSCompany.GetQuarterlyIncome(c, 1)
			+ " perf=" + GSCompany.GetQuarterlyPerformanceRating(c, 1)
			+ " vehicles=" + (a.n[0] + a.n[1] + a.n[2] + a.n[3])
			+ " stations=" + st.Count()
			+ " rail=" + a.n[0] + " road=" + a.n[1] + " air=" + a.n[2] + " water=" + a.n[3]
			+ " prail=" + a.p[0] + " proad=" + a.p[1] + " pair=" + a.p[2] + " pwater=" + a.p[3]
			+ " cargo=" + GSCompany.GetQuarterlyCargoDelivered(c, 1)
			+ " expenses=" + GSCompany.GetQuarterlyExpenses(c, 1)
			+ " name=" + GSCompany.GetName(c));
	}
	this.ReportShared(d);
	this.ReportTrainless(d);
}

function BattleReferee::Start()
{
	local last_q = -1;
	while (true) {
		local d = GSDate.GetCurrentDate();
		local q = GSDate.GetYear(d) * 4 + (GSDate.GetMonth(d) - 1) / 3;
		/* A few days into each quarter, so the previous quarter's statistics are settled. */
		if (q != last_q && GSDate.GetDayOfMonth(d) >= 3) {
			last_q = q;
			this.Report();
		}
		this.Sleep(74);
	}
}
