/*
 * Fleet management, every heartbeat: capacity follows demand instead of a count fixed at build time.
 *
 * - A route is the set of stations in a vehicle's orders. For each route, the cargo waiting at its
 *   stations (AIStation.GetCargoWaiting) is compared with one vehicle's capacity: when more than 1.5
 *   loads wait and money is above the reserve, one more vehicle is cloned with shared orders, up to
 *   a cap per type (road 8, air 4, ships ships_cap). Trains are not added here: a rail line's trains and wagons are
 *   the Rhai steward's targets, which lines.nut ReconcileLines carries out up to the line's
 *   trains_max, the trains its two-platform termini hold without a meeting on the single track.
 * - Too many vehicles: on a two-stop route the game's link graph gives the planned monthly flow
 *   each way (GetCargoPlannedVia); needed = max(1, ceil(flow / one vehicle's monthly capacity)), and
 *   the newest vehicle above that is sold. Without link data yet, a route with more than one vehicle
 *   where under a quarter of a load waits for 6 heartbeats in a row sells its newest.
 * - A vehicle older than two years that lost money last year and this year is sent to its depot
 *   and sold once it stands there.
 * Every action is logged with its station tile, so the camera can show it.
 */

function PrismaticAI::RouteKey(v) {
	local ids = [];
	for (local i = 0; i < AIOrder.GetOrderCount(v); i++) {
		local st = AIStation.GetStationID(AIOrder.GetOrderDestination(v, i));
		if (AIStation.IsValidStation(st)) ids.push(st);
	}
	ids.sort();
	local k = "";
	foreach (s in ids) k += s + ",";
	return k;
}

function PrismaticAI::DepotFor(v) {
	local type = AIVehicle.GetVehicleType(v);
	local first = AIOrder.GetOrderDestination(v, 0);
	if (type == AIVehicle.VT_AIR) return AIAirport.GetHangarOfAirport(first);
	if (type == AIVehicle.VT_WATER) return PrismaticAI.ShipDepotFor(v); /* ships.nut */
	if (type != AIVehicle.VT_ROAD) return -1;
	local depots = AIDepotList(AITile.TRANSPORT_ROAD);
	depots.Valuate(AIMap.DistanceManhattan, first);
	depots.KeepBelowValue(16);
	depots.Sort(AIList.SORT_BY_VALUE, AIList.SORT_ASCENDING);
	return depots.IsEmpty() ? -1 : depots.Begin();
}

/* Whether the route's vehicles that are old enough to judge (a year) earned money last year. Little
 * waiting cargo on such a route means it is served well, not over-served: aircraft pick up every
 * passenger, and the "nothing waits" rule sold a pair of planes earning about 67k a year each, until
 * the air routes were gone (LAN AI battle, research/ai-battle/README.md). */
function PrismaticAI::RouteEarns(k) {
	local profit = 0, judged = 0;
	local all = AIVehicleList();
	for (local u = all.Begin(); !all.IsEnd(); u = all.Next()) {
		if (AIVehicle.GetAge(u) < 365 || PrismaticAI.RouteKey(u) != k) continue;
		profit += AIVehicle.GetProfitLastYear(u);
		judged++;
	}
	return judged == 0 || profit > 0;
}

/* The setting air_add_full: how full, in percent, the planes calling at an airport of ours must fly
 * before one more plane calls there (0: off). Read here, not in Behaviour (main.nut), because the
 * static fleet and pair-search helpers that use it have no `b`.
 *
 * Why (the owner's co-play game, 2026-11-22, seed 20261122, 1979, read from openttd://vehicles):
 * PrismaticAI's 50 planes flew 35 routes between 9 airports, a full mesh with 7-8 routes and 7-13
 * planes at each airport; they flew 56% full over 171 tiles on average, 9 of them stood broken
 * down, and they earned 22.5k a plane. AAAHogEx's 42 planes flew from 22 airports, one route at
 * each, 89% full over 217 tiles, none broken down, and earned 85k a plane, on the same engine
 * (Bakewell Luckett LB-9). The bot added planes by what a route's planes earned on average
 * (InvestSurplus) and by a fixed 20 passengers waiting (air_add_waiting), and new routes from an
 * airport it had (air_reuse, on once rich); each shared the same airport's passengers. A plane
 * where the planes already fly half empty carries nobody new and pays its running cost. In a
 * headless match (OpenTTD seed 31337, 1960-1970, against AAAHogEx and AdmiralAI) the default
 * bot's air profit was 389k a year with 10 planes in 1963 and 250k with 49 planes on 19 routes
 * in 1968. Waiting passengers were tried first as the measure (a full plane load at every
 * airport of the route): it held that bot at 4 planes, because a station's rating keeps fewer
 * passengers waiting than one 200-seat plane takes, and cost 40% of company value. */
function PrismaticAI::AirAddFull() {
	return max(0, AIController.GetSetting("air_add_full"));
}

/* How full, in percent of their passenger capacity, the planes among `vehicles` (vehicle ids as
 * list keys) fly now; -1 when none of them is flying. Only a running plane counts (not one
 * loading at a station, in a hangar or broken down): it holds the load of the leg it flies. */
function PrismaticAI::AirLoadPercent(vehicles) {
	local pax = PrismaticAI.PassengerCargo();
	if (pax == -1) return -1;
	local load = 0, cap = 0;
	foreach (v, _ in vehicles) {
		if (AIVehicle.GetVehicleType(v) != AIVehicle.VT_AIR || AIVehicle.GetState(v) != AIVehicle.VS_RUNNING) continue;
		local c = AIVehicle.GetCapacity(v, pax);
		if (c <= 0) continue;
		cap += c;
		load += AIVehicle.GetCargoLoad(v, pax);
	}
	return cap > 0 ? load * 100 / cap : -1;
}

/* Whether air route `v` (a plane on it) fills one plane more: its flying planes are at least
 * air_add_full percent full (AirAddFull; 0, or no plane of the route flying now: yes). */
function PrismaticAI::AirRouteHasDemand(v) {
	local full = PrismaticAI.AirAddFull();
	if (full == 0) return true;
	local k = PrismaticAI.RouteKey(v);
	local planes = AIList();
	foreach (u, _ in AIVehicleList_Station(AIStation.GetStationID(AIOrder.GetOrderDestination(v, 0)))) {
		if (PrismaticAI.RouteKey(u) == k) planes.AddItem(u, 0);
	}
	local pct = PrismaticAI.AirLoadPercent(planes);
	return pct < 0 || pct >= full;
}

/*
 * Yield gates (settings load_add_min, road_yield, road_probes; info.nut "Yield" block).
 *
 * Why (headless matches of 2026-11-22, the live game's settings, 1950-1956, seeds 15805, 2927,
 * 31337 and 4242 against AAAHogEx, LuDiAI AfterFix, CivilAI and AdmiralAI, referee REF Q lines):
 * - InvestSurplus judged a route by what its year-old vehicles earned on average, so a route
 *   that earned anything above price / invest_ppv_div grew to its cap whether one more vehicle
 *   carried anybody or not. Seed 15805: the first air route earned 109302 a year with 5 planes
 *   (21860 each); InvestSurplus took it to 10 planes, and the 10 earned 118569 (11856 each) while
 *   flying 28-33% full. Seed 2927: 138599 with 6 planes, 100370 with 10. On the road it added
 *   buses to routes earning 892 to 1684 a vehicle, up to 8 on one route.
 * - Bus routes came first: the heartbeat built one a heartbeat up to the autopilot's 30 routes,
 *   18 by 1951 on seed 15805, while the company's 23 to 42 road vehicles earned 7.3k to 13.5k
 *   a year between them, a few hundred each, and the second air route waited until 1954-55 for
 *   money and loan headroom.
 *
 * RouteLoad measures how full a route's vehicles run: every heartbeat it samples the cargo
 * aboard each running vehicle against its capacity, as a decaying sum per route, so one empty
 * leg does not decide. RoadPays decides whether another bus route is worth building from what
 * the company's year-old road vehicles earned last year against their price.
 */

/* Route key -> {load, cap, unit}: the decaying sums RouteLoad reads (SampleRouteLoads). */
ROUTE_LOAD <- {};
/* Each heartbeat the old sums count 7/8, so about the last 8 heartbeats weigh most. */
const ROUTE_LOAD_KEEP = 7;
/* A route's load is judged once its decaying capacity sum reaches this many halves of the
 * biggest capacity sampled on it in one heartbeat (about three heartbeats of it running). */
const ROUTE_LOAD_MIN_LOADS = 6;

/* Samples the cargo aboard every running road vehicle, plane and ship of ours into ROUTE_LOAD.
 * Called once a heartbeat (ManageFleet). */
function PrismaticAI::SampleRouteLoads() {
	local seen = {};
	foreach (v, _ in AIVehicleList()) {
		local type = AIVehicle.GetVehicleType(v);
		if (type != AIVehicle.VT_ROAD && type != AIVehicle.VT_AIR && type != AIVehicle.VT_WATER) continue;
		if (AIVehicle.GetState(v) != AIVehicle.VS_RUNNING) continue;
		local cargo = AIEngine.GetCargoType(AIVehicle.GetEngineType(v));
		local cap = AIVehicle.GetCapacity(v, cargo);
		if (cap <= 0) continue;
		local k = PrismaticAI.RouteKey(v);
		if (k == "") continue;
		if (!seen.rawin(k)) seen[k] <- [0, 0];
		seen[k][0] += AIVehicle.GetCargoLoad(v, cargo);
		seen[k][1] += cap;
	}
	foreach (k, s in seen) {
		if (!ROUTE_LOAD.rawin(k)) ROUTE_LOAD[k] <- {load = 0, cap = 0, unit = 0};
		local r = ROUTE_LOAD[k];
		r.load = r.load * ROUTE_LOAD_KEEP / 8 + s[0];
		r.cap = r.cap * ROUTE_LOAD_KEEP / 8 + s[1];
		r.unit = max(r.unit, s[1]);
	}
}

/* How full, in percent, route `k` (RouteKey) ran over the recent heartbeats; -1 while too little
 * was sampled to say. */
function PrismaticAI::RouteLoad(k) {
	if (!ROUTE_LOAD.rawin(k)) return -1;
	local r = ROUTE_LOAD[k];
	if (r.cap <= 0 || 2 * r.cap < ROUTE_LOAD_MIN_LOADS * r.unit) return -1;
	return r.load * 100 / r.cap;
}

/* Whether route `k` takes one vehicle more by its load: it ran at least load_add_min percent full,
 * or it was not sampled enough to say (load_add_min 0, the default: always). At 50 the gate kept
 * planes off routes that still paid: 24 replayed 20-year arena matches on 11411cdc4d gave
 * load_add_min=0 +29.6% geometric company value (22 of 24 better, 0.88 to 2.03) and 332 more planes
 * (2026-11-23). The 1950-1956 single matches it was tuned on had towns too small to fill a plane. */
function PrismaticAI::RouteFillsMore(k) {
	local min_pct = max(0, AIController.GetSetting("load_add_min"));
	if (min_pct == 0) return true;
	local pct = PrismaticAI.RouteLoad(k);
	return pct < 0 || pct >= min_pct;
}

/* Whether another bus route is worth building (setting road_yield, percent; 0: always). Once any
 * road vehicle of ours is a year old: those vehicles earned, last year, at least road_yield percent
 * of their price between them. Before that: fewer than road_probes road routes run, so the first
 * ones show what the road earns on this map before more money goes into it. */
function PrismaticAI::RoadPays() {
	local pct = max(0, AIController.GetSetting("road_yield"));
	if (pct == 0) return true;
	local profit = 0, price = 0, judged = 0, routes = {};
	foreach (v, _ in AIVehicleList()) {
		if (AIVehicle.GetVehicleType(v) != AIVehicle.VT_ROAD) continue;
		routes[PrismaticAI.RouteKey(v)] <- true;
		if (AIVehicle.GetAge(v) < 365) continue;
		profit += AIVehicle.GetProfitLastYear(v);
		price += AIEngine.GetPrice(AIVehicle.GetEngineType(v));
		judged++;
	}
	if (judged == 0) return routes.len() < max(0, AIController.GetSetting("road_probes"));
	return profit * 100 >= pct * price;
}

/* The game year RoadReport last logged. */
ROAD_REPORT_YEAR <- -1;

/* Once a game year, the road fleet's yield in the log, like AirReport: per road route its
 * vehicles, the distance between its first two stops, whether both are in one town, how full it
 * ran (RouteLoad) and last year's profit per vehicle a year old; then the company's total and
 * RoadPays. RESEARCH-ONLY observability: it reads and logs, never acts. */
function PrismaticAI::RoadReport() {
	local year = AIDate.GetYear(AIDate.GetCurrentDate());
	if (year == ROAD_REPORT_YEAR) return;
	ROAD_REPORT_YEAR = year;
	local routes = {}, n = 0, judged = 0, profit = 0, price = 0;
	foreach (v, _ in AIVehicleList()) {
		if (AIVehicle.GetVehicleType(v) != AIVehicle.VT_ROAD) continue;
		local k = PrismaticAI.RouteKey(v);
		if (!routes.rawin(k)) routes[k] <- {sample = v, n = 0, judged = 0, profit = 0};
		local r = routes[k];
		r.n++;
		n++;
		if (AIVehicle.GetAge(v) < 365) continue;
		local p = AIVehicle.GetProfitLastYear(v);
		r.judged++;
		r.profit += p;
		judged++;
		profit += p;
		price += AIEngine.GetPrice(AIVehicle.GetEngineType(v));
	}
	if (n == 0) return;
	foreach (k, r in routes) {
		local v = r.sample, tiles = [];
		for (local i = 0; i < AIOrder.GetOrderCount(v) && tiles.len() < 2; i++) {
			local st = AIStation.GetStationID(AIOrder.GetOrderDestination(v, i));
			if (AIStation.IsValidStation(st)) tiles.push(AIStation.GetLocation(st));
		}
		local dist = tiles.len() >= 2 ? AIMap.DistanceManhattan(tiles[0], tiles[1]) : 0;
		local kind = (tiles.len() >= 2 && AITile.GetClosestTown(tiles[0]) == AITile.GetClosestTown(tiles[1])) ? "town" : "intercity";
		AILog.Info("PrismaticAI: road report " + year + ": route " + k + " " + kind + " " + dist + " tiles, " + r.n + " vehicle(s) running " +
			PrismaticAI.RouteLoad(k) + "% full, " + r.judged + " a year old, " + (r.judged > 0 ? r.profit / r.judged : 0) + " a vehicle last year (price " +
			AIEngine.GetPrice(AIVehicle.GetEngineType(v)) + ")");
	}
	AILog.Info("PrismaticAI: road report " + year + ": " + n + " vehicle(s) on " + routes.len() + " route(s), " + judged + " a year old earned " + profit +
		" last year, " + (judged > 0 ? profit / judged : 0) + " a vehicle, " + (price > 0 ? profit * 100 / price : 0) + "% of their price; another route: " +
		(PrismaticAI.RoadPays() ? "yes" : "no"));
}

/* Surplus to put to work (steward on): money above this many reserves, and above the setting
 * invest_min, funds more vehicles. */
const INVEST_SURPLUS_RESERVES = 20;

/* Put idle money to work: one more vehicle on the route whose judged vehicles (a year old) earned
 * the most per vehicle last year, while that is above a third of the vehicle's price and the route
 * is under its cap. The winner of the LAN AI battle grew its fleet tenfold while PrismaticAI sat on
 * millions with ~30 vehicles; waiting cargo alone never called for more. Road, air and water: rail
 * lines grow through the steward's targets (lines.nut, steward.nut). */
function PrismaticAI::InvestSurplus(b) {
	local money = AICompany.GetBankBalance(AICompany.COMPANY_SELF);
	/* With invest_borrow the unused loan counts too (AAAHogEx plans against balance + unused loan). */
	local usable = money + (b.invest_borrow == 1 ? AICompany.GetMaxLoanAmount() - AICompany.GetLoanAmount() : 0);
	if (usable < max(b.invest_min, INVEST_SURPLUS_RESERVES * b.cash_reserve)) return;
	local routes = {};
	local all = AIVehicleList();
	for (local v = all.Begin(); !all.IsEnd(); v = all.Next()) {
		if (this.selling.rawin(v)) continue;
		local type = AIVehicle.GetVehicleType(v);
		if (type != AIVehicle.VT_ROAD && type != AIVehicle.VT_AIR && type != AIVehicle.VT_WATER) continue;
		local k = PrismaticAI.RouteKey(v);
		if (k == "") continue;
		if (!routes.rawin(k)) routes[k] <- {sample = v, count = 0, profit = 0, judged = 0, type = type};
		local r = routes[k];
		r.count++;
		if (AIVehicle.GetAge(v) >= 365) { r.profit += AIVehicle.GetProfitLastYear(v); r.judged++; }
	}
	local best = null, best_ppv = 0;
	foreach (k, r in routes) {
		local cap = (r.type == AIVehicle.VT_AIR) ? b.invest_air_cap : ((r.type == AIVehicle.VT_WATER) ? b.ships_cap : b.invest_road_cap);
		if (r.judged == 0 || r.count >= cap) continue;
		/* A rich cap (rich.nut) above what the airports hold would only jam them. */
		if (r.type == AIVehicle.VT_AIR && !PrismaticAI.RouteHasAirRoom(r.sample)) continue;
		/* Nor where its planes already fly part empty (air_add_full, AirRouteHasDemand). */
		if (r.type == AIVehicle.VT_AIR && !PrismaticAI.AirRouteHasDemand(r.sample)) continue;
		/* Nor where its vehicles run part empty over the last heartbeats (load_add_min, RouteLoad):
		 * a vehicle more there shares the same passengers and adds only its running cost. */
		if (!PrismaticAI.RouteFillsMore(k)) continue;
		/* At most as many unjudged vehicles (under a year old) as judged ones: the profit per vehicle
		 * counts only the judged, so it does not fall as vehicles are added, and without this GA
		 * g276.brainSafe1 (OpenTTD seed 1878134858, slot 0) put 50 buses on one route at 1810 a year
		 * each and sold 96 vehicles over the game as each new one lost money. */
		if (r.count - r.judged >= r.judged) continue;
		local ppv = r.profit / r.judged;
		if (ppv * b.invest_ppv_div < AIEngine.GetPrice(AIVehicle.GetEngineType(r.sample))) continue;
		if (ppv > best_ppv) { best_ppv = ppv; best = r; }
	}
	if (best == null) return;
	local depot = PrismaticAI.DepotFor(best.sample);
	if (depot == -1) return;
	local price = AIEngine.GetPrice(AIVehicle.GetEngineType(best.sample));
	if (money < price + b.cash_reserve) {
		if (b.invest_borrow != 1) return;
		local step = AICompany.GetLoanInterval();
		local want = AICompany.GetLoanAmount() + ((price + b.cash_reserve - money) / step + 1) * step;
		if (want > AICompany.GetMaxLoanAmount() || !AICompany.SetLoanAmount(want)) return;
	}
	local nv = AIVehicle.CloneVehicle(depot, best.sample, true);
	if (AIVehicle.IsValidVehicle(nv)) {
		AIVehicle.StartStopVehicle(nv);
		PrismaticAI.Funnel(PrismaticAI.FunnelMode(best.type), "invested");
		AILog.Info("PrismaticAI: invest: " + best_ppv + " a year per vehicle on route " + PrismaticAI.RouteKey(best.sample) +
			"; added vehicle " + nv + " (" + (best.count + 1) + " on the route, money " + money + ")");
	}
}

/* The game year AirReport last logged. */
AIR_REPORT_YEAR <- -1;

/* Once a game year, the air fleet's yield in the log: per air route its planes, the distance
 * between its airports, the airport types, the engines and last year's profit per plane a year
 * old, then the company's total. RESEARCH-ONLY observability: it reads and logs, never acts, so
 * the battle's logs (battle.py ablate --log-dir) measure air income per aircraft, the gap the
 * owner's co-play game showed (2026-11-22, seed 20261122, 1979: 17,393 a plane for PrismaticAI's
 * 52 aircraft, 90,050 for AAAHogEx's). */
function PrismaticAI::AirReport() {
	local year = AIDate.GetYear(AIDate.GetCurrentDate());
	if (year == AIR_REPORT_YEAR) return;
	AIR_REPORT_YEAR = year;
	local routes = {}, planes = 0, judged = 0, profit = 0, losing = 0, all = AIList();
	foreach (v, _ in AIVehicleList()) {
		if (AIVehicle.GetVehicleType(v) != AIVehicle.VT_AIR) continue;
		local k = PrismaticAI.RouteKey(v);
		if (!routes.rawin(k)) routes[k] <- {sample = v, n = 0, judged = 0, profit = 0, engines = {}, planes = AIList()};
		local r = routes[k];
		r.n++;
		r.planes.AddItem(v, 0);
		all.AddItem(v, 0);
		planes++;
		r.engines[AIEngine.GetName(AIVehicle.GetEngineType(v))] <- true;
		if (AIVehicle.GetAge(v) < 365) continue;
		local p = AIVehicle.GetProfitLastYear(v);
		r.judged++;
		r.profit += p;
		judged++;
		profit += p;
		if (p < 0) losing++;
	}
	if (planes == 0) return;
	foreach (k, r in routes) {
		local v = r.sample, ports = "", tiles = [];
		for (local i = 0; i < AIOrder.GetOrderCount(v); i++) {
			/* An order's destination tile need not be an airport tile: take one of the station's. */
			local st = AIStation.GetStationID(AIOrder.GetOrderDestination(v, i));
			if (!AIStation.IsValidStation(st)) continue;
			local at = AITileList_StationType(st, AIStation.STATION_AIRPORT);
			if (at.IsEmpty()) continue;
			tiles.push(AIStation.GetLocation(st));
			ports += (ports == "" ? "" : "/") + AIAirport.GetAirportType(at.Begin()) + "@" + PrismaticAI.AirRoutesAt(st);
		}
		local dist = tiles.len() >= 2 ? AIMap.DistanceManhattan(tiles[0], tiles[1]) : 0;
		local engines = "";
		foreach (e, _ in r.engines) engines += (engines == "" ? "" : ",") + e;
		AILog.Info("PrismaticAI: air report " + year + ": route " + k + " " + dist + " tiles, airports (type@routes) " + ports +
			", " + r.n + " plane(s) flying " + PrismaticAI.AirLoadPercent(r.planes) + "% full, " + r.judged + " a year old, " + (r.judged > 0 ? r.profit / r.judged : 0) +
			" a plane last year (" + engines + ")");
	}
	AILog.Info("PrismaticAI: air report " + year + ": " + planes + " plane(s) on " + routes.len() + " route(s) flying " + PrismaticAI.AirLoadPercent(all) + "% full, " + judged +
		" a year old earned " + profit + " last year, " + (judged > 0 ? profit / judged : 0) + " a plane, " + losing + " lost money");
}

function PrismaticAI::ManageFleet(reserve) {
	PrismaticAI.AirReport();
	PrismaticAI.RoadReport();
	PrismaticAI.SampleRouteLoads();
	local routes = {};
	local list = AIVehicleList();
	/* Planes per air route that are not on their way to be sold. With air_keep_last on (AIR_KEEP_LAST,
	 * main.nut) the last one is never sold: on
	 * OpenTTD seed 725554694 (company 2) every plane lost money in 1964-66, the seller took the last
	 * of each route, and with no plane left to clone the company never flew again and ended among
	 * the weakest AIs. The default, which kept one plane on its first route, grew it back to 12
	 * planes at 23k a year each. */
	local flying = {};
	for (local v = list.Begin(); !list.IsEnd(); v = list.Next()) {
		if (AIVehicle.GetVehicleType(v) != AIVehicle.VT_AIR || this.selling.rawin(v)) continue;
		local k = PrismaticAI.RouteKey(v);
		flying[k] <- (flying.rawin(k) ? flying[k] : 0) + 1;
	}
	for (local v = list.Begin(); !list.IsEnd(); v = list.Next()) {
		/* Selling: two years old, losing money last year and this year. */
		if (this.selling.rawin(v)) {
			if (AIVehicle.IsStoppedInDepot(v)) {
				AILog.Info("PrismaticAI: fleet: sold unprofitable vehicle " + v + " at tile " + AIVehicle.GetLocation(v));
				AIVehicle.SellVehicle(v);
				delete this.selling[v];
			}
			continue;
		}
		if (AIVehicle.GetAge(v) > 730 && AIVehicle.GetProfitLastYear(v) < 0 && AIVehicle.GetProfitThisYear(v) < 0) {
			local air = AIVehicle.GetVehicleType(v) == AIVehicle.VT_AIR;
			if (air && AIR_KEEP_LAST == 1 && flying[PrismaticAI.RouteKey(v)] <= 1) continue; /* the route's last plane */
			/* The fleet's one selling path (events.nut SendToSell). */
			if (this.SendToSell(v, "PrismaticAI: fleet: vehicle " + v + " lost money two years running; sending it to be sold")) {
				if (air) flying[PrismaticAI.RouteKey(v)]--;
			}
			continue;
		}
		local k = PrismaticAI.RouteKey(v);
		if (k == "") continue;
		if (!routes.rawin(k)) routes[k] <- {sample = v, count = 0, newest = v};
		routes[k].count++;
		if (AIVehicle.GetAge(v) < AIVehicle.GetAge(routes[k].newest)) routes[k].newest = v;
	}
	foreach (k, r in routes) {
		local v = r.sample;
		local type = AIVehicle.GetVehicleType(v);
		if (type == AIVehicle.VT_RAIL) continue;
		/* Planes per air route: 4, or 8 with the steward on (autopilot): in the battle the winner ran
		 * 10-14 planes on its first routes by 1962 and its air profit funded everything else. */
		/* Ships: ships_cap (ships.nut). */
		local cap_limit = (type == AIVehicle.VT_AIR) ? (this.steward_on ? this.air_cap : 4) : ((type == AIVehicle.VT_WATER) ? this.ship_cap : 8);
		if (r.count >= cap_limit) continue;
		local cargo = AIEngine.GetCargoType(AIVehicle.GetEngineType(v));
		local cap = AIVehicle.GetCapacity(v, cargo);
		if (cap <= 0) continue;
		/* What waits for this route: at each stop, the cargo bound for the route's next stop
		 * (GetCargoWaitingVia). Under cargodist much of a station's cargo waits for other next hops
		 * and would never board here (openttd-a0's link audit caught the over-adding). */
		local waiting = 0, where = -1;
		local n_orders = AIOrder.GetOrderCount(v);
		for (local i = 0; i < n_orders; i++) {
			local tile = AIOrder.GetOrderDestination(v, i);
			local st = AIStation.GetStationID(tile);
			local next = AIStation.GetStationID(AIOrder.GetOrderDestination(v, (i + 1) % n_orders));
			if (!AIStation.IsValidStation(st) || !AIStation.IsValidStation(next)) continue;
			local w = AIStation.GetCargoWaitingVia(st, next, cargo);
			if (w > waiting) { waiting = w; where = tile; }
		}
		/* Too many by the game's own link graph: on a two-stop route the planned monthly flow each way
		 * (AIStation.GetCargoPlannedVia) needs max(1, ceil(flow / monthly capacity of one vehicle))
		 * vehicles; the monthly capacity is the vehicle's capacity times its round trips a month
		 * (0.036 tiles a day per km/h, .ai knowledge cargo-payment-and-vehicle-speed). Judged once the
		 * newest vehicle is 120 days old, so the link graph has measured the route. */
		local stops = [];
		for (local i = 0; i < AIOrder.GetOrderCount(v); i++) {
			local st = AIStation.GetStationID(AIOrder.GetOrderDestination(v, i));
			if (!AIStation.IsValidStation(st)) continue;
			local seen = false;
			foreach (x in stops) if (x == st) seen = true; /* no Array.find in this Squirrel */
			if (!seen) stops.push(st);
		}
		if (stops.len() == 2 && r.count > 1 && type != AIVehicle.VT_RAIL && AIVehicle.GetAge(r.newest) > 120) {
			local flow = max(AIStation.GetCargoPlannedVia(stops[0], stops[1], cargo), AIStation.GetCargoPlannedVia(stops[1], stops[0], cargo));
			local tiles = AIMap.DistanceManhattan(AIStation.GetLocation(stops[0]), AIStation.GetLocation(stops[1]));
			local kmh = max(1, AIEngine.GetMaxSpeed(AIVehicle.GetEngineType(v)));
			local round_days = 2 * tiles * 1000 / (kmh * 36) + 4;
			local monthly = max(1, cap * 30 / max(1, round_days));
			local needed = max(1, (flow + monthly - 1) / monthly);
			if (flow > 0 && r.count > needed) {
				this.SendToSell(r.newest, "PrismaticAI: fleet: link flow " + flow + "/month needs " + needed + " vehicle(s) of " + monthly +
					"/month, route has " + r.count + "; selling vehicle " + r.newest);
				continue;
			}
		}
		/* Too many by waiting cargo, where the link graph has no answer yet: almost nothing waits at any
		 * stop for 6 heartbeats in a row and the route has more than one vehicle. */
		if (waiting * 4 < cap && r.count > 1 && !PrismaticAI.RouteEarns(k)) {
			local n = this.idle.rawin(k) ? this.idle[k] + 1 : 1;
			this.idle[k] <- n;
			if (n >= 6) {
				this.idle[k] <- 0;
				local newest = -1, newest_age = 1 << 30;
				local all = AIVehicleList();
				for (local u = all.Begin(); !all.IsEnd(); u = all.Next()) {
					if (PrismaticAI.RouteKey(u) == k && AIVehicle.GetAge(u) < newest_age && !this.selling.rawin(u)) { newest = u; newest_age = AIVehicle.GetAge(u); }
				}
				if (newest != -1) this.SendToSell(newest, "PrismaticAI: fleet: " + r.count + " vehicles on a route where " + waiting +
					" wait (capacity " + cap + "); selling vehicle " + newest);
			}
			continue;
		}
		this.idle[k] <- 0;
		/* Air, learned from AAAHogEx: a plane more while both airports have passengers waiting, not
		 * only when one end piles up 1.5 loads (it ran twice our planes on the same routes). */
		local both_ok = false;
		if (type == AIVehicle.VT_AIR && this.air_add_waiting > 0 && stops.len() == 2) {
			local w0 = AIStation.GetCargoWaiting(stops[0], cargo), w1 = AIStation.GetCargoWaiting(stops[1], cargo);
			both_ok = min(w0, w1) > this.air_add_waiting;
			if (both_ok) { waiting = min(w0, w1); where = AIStation.GetLocation(stops[0]); }
		}
		if (!both_ok && waiting * 2 <= cap * 3) continue;
		/* And never above what the link's planned flow needs plus one, once the link graph knows it. */
		if (stops.len() == 2 && !both_ok) {
			local flow = max(AIStation.GetCargoPlannedVia(stops[0], stops[1], cargo), AIStation.GetCargoPlannedVia(stops[1], stops[0], cargo));
			local tiles = AIMap.DistanceManhattan(AIStation.GetLocation(stops[0]), AIStation.GetLocation(stops[1]));
			local kmh = max(1, AIEngine.GetMaxSpeed(AIVehicle.GetEngineType(v)));
			local monthly = max(1, cap * 30 / max(1, 2 * tiles * 1000 / (kmh * 36) + 4));
			if (flow > 0 && r.count >= (flow + monthly - 1) / monthly + 1) continue;
		}
		if (type == AIVehicle.VT_AIR && !PrismaticAI.RouteHasAirRoom(v)) continue;
		if (type == AIVehicle.VT_AIR && !PrismaticAI.AirRouteHasDemand(v)) continue;
		if (!PrismaticAI.RouteFillsMore(k)) continue; /* load_add_min, as in InvestSurplus */
		/* Never a vehicle more on a route whose year-old vehicles lost money last year, whatever the
		 * waiting cargo says. With air_full_load and a low air_add_waiting, GA g276.brainSafe1 (OpenTTD
		 * seed 1878134858, slot 0) put 17 planes on 3 routes that lost 42.7k a year between them, and
		 * ended last. */
		if (!PrismaticAI.RouteEarns(k)) continue;
		if (AICompany.GetBankBalance(AICompany.COMPANY_SELF) < reserve) return;
		local depot = PrismaticAI.DepotFor(v);
		if (depot == -1) continue;
		local nv = AIVehicle.CloneVehicle(depot, v, true);
		if (AIVehicle.IsValidVehicle(nv)) {
			AIVehicle.StartStopVehicle(nv);
			AILog.Info("PrismaticAI: fleet: " + waiting + " " + AICargo.GetCargoLabel(cargo) + " waiting at tile " + where +
				" for a capacity of " + cap + "; added vehicle " + nv + " (" + (r.count + 1) + " on the route)");
		}
	}
}
