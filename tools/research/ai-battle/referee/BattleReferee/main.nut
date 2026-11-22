/*
 * RESEARCH-ONLY referee Game Script -- see info.nut.
 *
 * Line format (one per company, all fields space separated key=value, name last because
 * it may contain spaces):
 *   REF Q date=<y>-<m> cid=<n> value=<v> money=<m> income=<i> perf=<p> vehicles=<n>
 *       stations=<n> rail=<n> road=<n> air=<n> water=<n> prail=<p> proad=<p> pair=<p>
 *       pwater=<p> cargo=<delivered> expenses=<e> name=<company name>
 * rail..water count vehicles by transport mode; prail..pwater are their summed profit last year;
 * cargo is cargo delivered and expenses the expenses, both of the last completed quarter.
 * value/income/perf are for the last *completed* quarter (GSCompany quarter index 1);
 * money, vehicles and stations are the current values.
 *
 * Shared producers, one line per producing industry whose output rail stations of two or more
 * companies pick up (a station with a rating for one of its cargoes), after the company lines
 * of each quarter:
 *   REF S date=<y>-<m> ind=<industry id> owners=<cid>,<cid>[,...] name=<industry name>
 * Two lines at one producer split its output by station rating; the orchestrator fails the fitness
 * check when two of them are PrismaticAI's (research/incident-2026-11-22-sibling-industries.md).
 */
class BattleReferee extends GSController {
	function Start();
	function Report();
	function ReportShared(d);
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
		/* Only a station that picks up what the industry produces: one with a rating for one of
		 * its cargoes. A station that delivers to a sawmill or a factory also catches it, and two
		 * companies delivering to one acceptor take nothing from each other. */
		local made = GSCargoList_IndustryProducing(i);
		local owners = {};
		foreach (t, _ in tiles) {
			local st = GSStation.GetStationID(t);
			foreach (c, _ in made) {
				if (GSStation.HasCargoRating(st, c)) { owners[GSTile.GetOwner(t)] <- true; break; }
			}
		}
		if (owners.len() < 2) continue;
		local ids = [];
		foreach (o, _ in owners) ids.push(o);
		ids.sort();
		local list = "";
		foreach (k, o in ids) list += (k > 0 ? "," : "") + o;
		GSLog.Error("REF S date=" + GSDate.GetYear(d) + "-" + GSDate.GetMonth(d) + " ind=" + i
			+ " owners=" + list + " name=" + GSIndustry.GetName(i));
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
