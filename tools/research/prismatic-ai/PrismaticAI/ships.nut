/*
 * Ships (RESEARCH-ONLY; setting ships_max, default 0: the bot builds no ship at all).
 *
 * Why: the bot had road, rail and air and nothing on water. In the owner's co-play game (1979)
 * AAAHogEx earned 8,091-14,671 a year per ship, and PrismaticAI ran none. An oil rig is the plain
 * case: it stands in the sea, has no land for a rail station (rail.nut NextOwnFreightLine skips
 * industries built on water since 2026-11-21) and hands its oil only to its own neutral station
 * (AIIndustry.GetDockLocation), so a ship or a helicopter is the only way to move it.
 *
 * How, planned like the bot's own freight lines (rail.nut NextOwnFreightLine), one route a
 * heartbeat while fewer than ships_max exist and money plus loan headroom reach line_budget:
 * - Candidates, scored on the map's own numbers and nothing built yet:
 *   - freight: a producing industry with water within a dock's reach (an oil rig always), a freight
 *     cargo it made at least SHIP_MIN_PRODUCTION of last month and under SHIP_SERVED_BELOW percent
 *     of which was transported (scored on the part left), and an industry accepting that cargo with
 *     water within reach, SHIP_MIN_TILES to ships_max_tiles away;
 *   - passengers: two towns of at least SHIP_MIN_POPULATION with water near their centre, the same
 *     distance apart.
 *   A route's score is its estimated yearly profit: the best ship for the cargo (PickShip:
 *   capacity x speed, refitted if needed), its round trip at its speed (the fleet's estimate, 0.036
 *   tiles a day per km/h, fleet.nut), the game's payment for the cargo over that distance and time
 *   (AICargo.GetCargoIncome), the ships the supply fills (at most ships_cap) and their running
 *   costs. A route whose ship would not pay back its price within SHIP_PAYBACK_YEARS is dropped.
 * - The best SHIP_TRIES untried candidates are then checked on the map, in score order, still
 *   building nothing: a dock site at each end (an oil rig's own station; a dock of ours already
 *   there; else a sloped tile whose dock the game accepts in AITestMode, with a two-tile water front
 *   and, at a cargo's destination, acceptance of it; failing that, a shore tile that becomes such a
 *   slope once its side towards the water is lowered, ShoreLevel, after AAAHogEx), and water
 *   joining the two fronts (WaterPath: A* over tiles AIMarine.AreWaterTilesConnected says a ship
 *   crosses). The game's own ship
 *   pathfinder (YAPF) routes the ships later; this only proves that a way exists.
 * - The first candidate that passes is built on borrowed money, like every route: the docks, a
 *   ship depot on open water near a land dock (or one of ours within SHIP_DEPOT_REACH), and as many
 *   ships as the supply fills, at most SHIP_START_MAX at once, each only while half a year of the
 *   route's running costs stays at hand. Freight loads full at the source; passengers alternate
 *   their first stop. A route with no ship takes its new docks and depot down again.
 * - From then on the fleet runs it like any road or air route (fleet.nut): ManageFleet adds a ship
 *   where cargo waits (up to ships_cap) and sells one the link graph does not need or that lost
 *   money two years running, InvestSurplus puts idle money into the route that earns most per
 *   vehicle (ships_cap), events.nut deals with lost and unprofitable ships, and ShipDepotFor gives
 *   both a ship's depot.
 * Every candidate checked or built logs one line, "PrismaticAI: ships: ...".
 *
 * Restart-safe without a save of its own: routes are counted from the ships the company runs
 * (CountShipRoutes), and a producer or town our ships already serve is never a new candidate.
 */

/* Shortest ship route, in tiles. */
const SHIP_MIN_TILES = 20;
/* Least production last month of a freight cargo worth a ship route. */
const SHIP_MIN_PRODUCTION = 30;
/* A producer others serve is a candidate while under this share of its output was transported. */
const SHIP_SERVED_BELOW = 60;
/* Least population of a town at either end of a passenger ship route. */
const SHIP_MIN_POPULATION = 800;
/* A route is dropped when one ship would not pay back its price within this many years. */
const SHIP_PAYBACK_YEARS = 6;
/* Candidates checked on the map (dock sites, water path) per heartbeat at most. */
const SHIP_TRIES = 4;
/* Ships bought when a route is built at most; the fleet adds more where cargo waits. */
const SHIP_START_MAX = 3;
/* Tiles A* over water visits at most for one route. */
const SHIP_PATH_BUDGET = 30000;
/* An existing ship depot of ours this close to a dock serves the new route too. */
const SHIP_DEPOT_REACH = 12;
/* Days a ship loses at each end of a trip (docking, loading). */
const SHIP_STOP_DAYS = 6;

/* The buildable ship with the best capacity x speed that carries `cargo` or refits to it and may
 * sail `dist` tiles (AIEngine.GetMaximumOrderDistance, 0: no limit), or -1. */
function PrismaticAI::PickShip(cargo, dist = 0) {
	local best = -1, best_value = -1;
	foreach (e, _ in AIEngineList(AIVehicle.VT_WATER)) {
		if (!AIEngine.IsBuildable(e)) continue;
		if (AIEngine.GetCargoType(e) != cargo && !AIEngine.CanRefitCargo(e, cargo)) continue;
		local range = AIEngine.GetMaximumOrderDistance(e);
		if (range > 0 && dist * dist > range) continue;
		local value = AIEngine.GetCapacity(e) * AIEngine.GetMaxSpeed(e);
		if (value > best_value) { best_value = value; best = e; }
	}
	return best;
}

/* Days one way over `tiles` for `engine` (the fleet's estimate: 0.036 tiles a day per km/h). */
function PrismaticAI::ShipDays(engine, tiles) {
	return tiles * 1000 / (max(1, AIEngine.GetMaxSpeed(engine)) * 36) + 1;
}

/* Whether `tile` is the dock of ours or the neutral dock (an oil rig) a ship of ours can call at. */
function PrismaticAI::IsUsableDock(tile) {
	if (!AIMarine.IsDockTile(tile) && !AITile.IsStationTile(tile)) return false;
	return AIStation.IsValidStation(AIStation.GetStationID(tile));
}

/* Our ships whose orders call at the station of `tile`. */
function PrismaticAI::ShipsAt(tile) {
	local st = AIStation.GetStationID(tile);
	if (!AIStation.IsValidStation(st)) return 0;
	local n = 0;
	foreach (v, _ in AIVehicleList_Station(st)) if (AIVehicle.GetVehicleType(v) == AIVehicle.VT_WATER) n++;
	return n;
}

/* Water tiles (not coast) within `r` tiles of `tile`, as an array. */
function PrismaticAI::WaterNear(tile, r) {
	local tiles = AITileList();
	local x = AIMap.GetTileX(tile), y = AIMap.GetTileY(tile);
	tiles.AddRectangle(AIMap.GetTileIndex(max(1, x - r), max(1, y - r)),
		AIMap.GetTileIndex(min(AIMap.GetMapSizeX() - 2, x + r), min(AIMap.GetMapSizeY() - 2, y + r)));
	tiles.Valuate(AITile.IsWaterTile);
	tiles.KeepValue(1);
	local out = [];
	foreach (t, _ in tiles) out.push(t);
	return out;
}

/* Whether the sea (or a lake) or its shore lies within a dock's reach of `list` (a tile list around
 * an industry or town). A river does not count: it is one tile wide, and a dock needs two tiles of
 * water in front of it (DockFront); on OpenTTD seed 178496673 every town and industry the bot
 * looked at stood by a river, with no shore tile within reach. AAAHogEx digs canals there. */
function PrismaticAI::ListTouchesWater(list) {
	local l = AIList();
	l.AddList(list);
	l.Valuate(AITile.IsCoastTile);
	l.KeepValue(1);
	if (!l.IsEmpty()) return true;
	l.AddList(list);
	l.Valuate(AITile.IsSeaTile);
	l.KeepValue(1);
	return !l.IsEmpty();
}

/* The water tile two steps in front of a dock on sloped tile `t`: the dock's own water part is the
 * first step, and the game wants the second to be water too (station_cmd.cpp CmdBuildDock). -1
 * when no side of `t` has that. */
function PrismaticAI::DockFront(t) {
	local w = AIMap.GetMapSizeX();
	foreach (d in [1, -1, w, -w]) {
		local a = t + d;
		local f = t + 2 * d;
		if (!AIMap.IsValidTile(f)) continue;
		if (AITile.IsWaterTile(a) && AITile.GetSlope(a) == AITile.SLOPE_FLAT && AITile.IsWaterTile(f) && AITile.GetSlope(f) == AITile.SLOPE_FLAT) return f;
	}
	return -1;
}

/* Levelling a shore tile into a dock site (learned from AAAHogEx's WaterStation.Build, which levels
 * the edge of a coast tile towards the water to the water's height). Most shore tiles are not the
 * one inclined slope a dock needs: on OpenTTD seed 178496673 every town and industry the bot
 * looked at by the sea had none within reach, while AAAHogEx ran 18 ships there by 1970. For coast
 * tile `t` and a side whose next two tiles are flat water (DockFront): the two corners on that
 * side, at most one step above the water, are lowered to it, and the two corners away from the
 * water must already stand one step above it, so the tile becomes inclined towards the water.
 * Only checked here (each lowering in AITestMode, the caller's mode); BuildShipEnd lowers.
 * Returns {front, corners (the AITile slope bits to lower, one LowerTile each)} or null. */
function PrismaticAI::ShoreLevel(t) {
	if (!AITile.IsCoastTile(t)) return null;
	local w = AIMap.GetMapSizeX();
	/* Per side: [step, the corners on it, the corners away from it] (corner N at (x, y), W at
	 * (x + 1, y), E at (x, y + 1), S at (x + 1, y + 1)). */
	local sides = [
		[1, [AITile.CORNER_W, AITile.CORNER_S], [AITile.CORNER_N, AITile.CORNER_E]],
		[-1, [AITile.CORNER_N, AITile.CORNER_E], [AITile.CORNER_W, AITile.CORNER_S]],
		[w, [AITile.CORNER_E, AITile.CORNER_S], [AITile.CORNER_N, AITile.CORNER_W]],
		[-w, [AITile.CORNER_N, AITile.CORNER_W], [AITile.CORNER_E, AITile.CORNER_S]],
	];
	local bit = {};
	bit[AITile.CORNER_N] <- AITile.SLOPE_N;
	bit[AITile.CORNER_E] <- AITile.SLOPE_E;
	bit[AITile.CORNER_S] <- AITile.SLOPE_S;
	bit[AITile.CORNER_W] <- AITile.SLOPE_W;
	foreach (side in sides) {
		local d = side[0];
		local a = t + d;
		local f = t + 2 * d;
		if (!AIMap.IsValidTile(f) || !AITile.IsWaterTile(a) || !AITile.IsWaterTile(f)) continue;
		if (AITile.GetSlope(a) != AITile.SLOPE_FLAT || AITile.GetSlope(f) != AITile.SLOPE_FLAT) continue;
		local h0 = AITile.GetMinHeight(a);
		local ok = true;
		foreach (c in side[2]) if (AITile.GetCornerHeight(t, c) != h0 + 1) ok = false;
		local lower = [];
		foreach (c in side[1]) {
			local h = AITile.GetCornerHeight(t, c);
			if (h == h0 + 1) lower.push(bit[c]);
			else if (h != h0) ok = false;
		}
		if (!ok || lower.len() == 0) continue;
		foreach (b in lower) if (!AITile.LowerTile(t, b)) ok = false;
		if (ok) return {front = f, corners = lower};
	}
	return null;
}

/* An existing dock station of ours with a dock tile in `list`, as that tile, or -1. */
function PrismaticAI::OurDockIn(list) {
	foreach (t, _ in list) {
		if (AIMarine.IsDockTile(t) && AICompany.IsMine(AITile.GetOwner(t))) return t;
	}
	return -1;
}

/* Where a ship route ends at a producer or acceptor: {tile (the order's destination), fronts (water
 * tiles where the path check starts or ends), site (a tile to build a dock on, or -1), label}, or
 * null when no dock can serve it. `list` holds the tiles a dock there may stand on (its catchment
 * covers the industry or town), `toward` the other end, `cargo` and `accept` what the dock must
 * take (an acceptance of 8 in its catchment) and `produce` whether it must also collect it. */
function PrismaticAI::ShipEnd(industry, list, toward, cargo, accept, produce, label) {
	if (industry != -1 && AIIndustry.HasDock(industry)) {
		local rig = AIIndustry.GetDockLocation(industry);
		if (rig == AIMap.TILE_INVALID || !PrismaticAI.IsUsableDock(rig)) return null;
		return {tile = rig, fronts = PrismaticAI.WaterNear(rig, 3), site = -1, level = null, label = label};
	}
	local ours = PrismaticAI.OurDockIn(list);
	if (ours != -1) return {tile = ours, fronts = PrismaticAI.WaterNear(ours, 3), site = -1, level = null, label = label};
	local radius = AIStation.GetCoverageRadius(AIStation.STATION_DOCK);
	local sites = AIList();
	sites.AddList(list);
	sites.Valuate(AITile.GetSlope);
	local inclined = {};
	foreach (s in [AITile.SLOPE_NE, AITile.SLOPE_SE, AITile.SLOPE_SW, AITile.SLOPE_NW]) inclined[s] <- true;
	local keep = AIList();
	foreach (t, slope in sites) {
		if (inclined.rawin(slope) || AITile.IsCoastTile(t)) keep.AddItem(t, AIMap.DistanceManhattan(t, toward));
	}
	keep.Sort(AIList.SORT_BY_VALUE, AIList.SORT_ASCENDING);
	local test = AITestMode();
	/* Sites the game takes as they are first; a shore tile that needs levelling only after. */
	local shore = null;
	foreach (t, _ in keep) {
		if (accept && AITile.GetCargoAcceptance(t, cargo, 1, 1, radius) < 8) continue;
		if (produce && AITile.GetCargoProduction(t, cargo, 1, 1, radius) <= 0) continue;
		local front = PrismaticAI.DockFront(t);
		if (front != -1 && inclined.rawin(AITile.GetSlope(t)) && AIMarine.BuildDock(t, AIStation.STATION_NEW)) {
			return {tile = t, fronts = [front], site = t, level = null, label = label};
		}
		if (shore == null) {
			local lv = PrismaticAI.ShoreLevel(t);
			if (lv != null) shore = {tile = t, fronts = [lv.front], site = t, level = lv, label = label};
		}
	}
	return shore;
}

/* Whether water joins one tile of `starts` to one of `goals` (arrays): A* over the four neighbours a
 * ship may cross (AIMarine.AreWaterTilesConnected), at most SHIP_PATH_BUDGET tiles. Returns the
 * number of steps, or -1. */
function PrismaticAI::WaterPath(starts, goals, budget = SHIP_PATH_BUDGET) {
	if (starts.len() == 0 || goals.len() == 0) return -1;
	local goal = {};
	foreach (g in goals) goal[g] <- true;
	local aim = goals[0];
	local w = AIMap.GetMapSizeX();
	local open = AIPriorityQueue();
	local cost = {};
	foreach (s in starts) {
		if (goal.rawin(s)) return 0;
		cost[s] <- 0;
		open.Insert(s, AIMap.DistanceManhattan(s, aim));
	}
	while (!open.IsEmpty() && budget-- > 0) {
		local cur = open.Pop();
		if (goal.rawin(cur)) return cost[cur];
		foreach (next in [cur - 1, cur + 1, cur - w, cur + w]) {
			if (!AIMap.IsValidTile(next) || !AIMarine.AreWaterTilesConnected(cur, next)) continue;
			local c = cost[cur] + 1;
			if (!cost.rawin(next) || c < cost[next]) {
				cost[next] <- c;
				open.Insert(next, c + AIMap.DistanceManhattan(next, aim));
			}
		}
	}
	return -1;
}

/* Our ship depot for `v`: the nearest one within SHIP_DEPOT_REACH + 8 tiles of any of its order
 * destinations (a route from an oil rig has its depot at the land end), or -1. fleet.nut DepotFor
 * asks this for ships, so ManageFleet, InvestSurplus and events.nut OrdersFor serve them too. */
function PrismaticAI::ShipDepotFor(v) {
	local depots = AIDepotList(AITile.TRANSPORT_WATER);
	if (depots.IsEmpty()) return -1;
	local best = -1, best_d = SHIP_DEPOT_REACH + 9;
	for (local i = 0; i < AIOrder.GetOrderCount(v); i++) {
		local dest = AIOrder.GetOrderDestination(v, i);
		if (!AIMap.IsValidTile(dest)) continue;
		foreach (d, _ in depots) {
			local dist = AIMap.DistanceManhattan(d, dest);
			if (dist < best_d) { best_d = dist; best = d; }
		}
	}
	return best;
}

/* A ship depot near `end` (a ShipEnd) whose ships reach `front`: one of ours within
 * SHIP_DEPOT_REACH, else a new one on open water 3 to 8 tiles from the dock (both its tiles and
 * the tiles past each end water, so it blocks no channel). Returns {tile, built} or null. */
function PrismaticAI::ShipDepotNear(end, front) {
	foreach (d, _ in AIDepotList(AITile.TRANSPORT_WATER)) {
		if (AIMap.DistanceManhattan(d, end.tile) > SHIP_DEPOT_REACH) continue;
		if (PrismaticAI.WaterPath(PrismaticAI.WaterNear(d, 1), [front], 4000) >= 0) return {tile = d, built = false};
	}
	local w = AIMap.GetMapSizeX();
	local spots = AIList();
	foreach (t in PrismaticAI.WaterNear(end.tile, 8)) {
		local dist = AIMap.DistanceManhattan(t, end.tile);
		if (dist >= 3) spots.AddItem(t, dist);
	}
	spots.Sort(AIList.SORT_BY_VALUE, AIList.SORT_ASCENDING);
	foreach (t, _ in spots) {
		foreach (d in [1, w]) {
			local ok = true;
			foreach (u in [t - d, t, t + d, t + 2 * d]) {
				if (!AITile.IsWaterTile(u) || AITile.GetSlope(u) != AITile.SLOPE_FLAT) { ok = false; break; }
			}
			if (!ok || !AIMarine.BuildWaterDepot(t, t - d)) continue;
			if (PrismaticAI.WaterPath([t - d, t + 2 * d], [front], 4000) >= 0) return {tile = t, built = true};
			AIMarine.RemoveWaterDepot(t);
		}
	}
	return null;
}

/* Distinct routes our ships run (fleet.nut RouteKey), counted from the map. */
function PrismaticAI::CountShipRoutes() {
	local keys = {};
	foreach (v, _ in AIVehicleList()) {
		if (AIVehicle.GetVehicleType(v) != AIVehicle.VT_WATER) continue;
		local k = PrismaticAI.RouteKey(v);
		if (k != "") keys[k] <- true;
	}
	return keys.len();
}

/* The yearly profit of a route `dist` tiles long carrying `supply` a month each way it loads
 * (`ways`: 1 freight, 2 passengers) with ships of `engine`: {ships, profit, per_ship, days}. */
function PrismaticAI::ShipEstimate(engine, cargo, dist, supply, ways, cap_max) {
	local days = PrismaticAI.ShipDays(engine, dist);
	local round = 2 * days + 2 * SHIP_STOP_DAYS;
	local cap = max(1, AIEngine.GetCapacity(engine));
	local monthly = max(1, cap * 30 / round); /* one ship, one way */
	local ships = max(1, min(cap_max, (supply + monthly - 1) / monthly));
	local carried = min(supply * 12, ships * cap * 365 / round) * ways;
	local profit = carried * AICargo.GetCargoIncome(cargo, dist, days) - ships * AIEngine.GetRunningCost(engine);
	return {ships = ships, profit = profit, per_ship = profit / ships, days = days};
}

/* Every untried ship route this map offers, best first (see the head of this file). */
function PrismaticAI::ShipCandidates(b) {
	local out = [];
	local radius = AIStation.GetCoverageRadius(AIStation.STATION_DOCK);
	local far = b.ships_max_tiles;
	local wet = {}; /* acceptor -> {list: the tiles a dock there may use (null: its own dock), ok: water within reach} */
	local engines = {};
	foreach (p, _ in AIIndustryList()) {
		local rig = AIIndustry.HasDock(p);
		local plist = null;
		if (!rig) {
			plist = AITileList_IndustryProducing(p, radius);
			if (!PrismaticAI.ListTouchesWater(plist)) continue;
			local ours = PrismaticAI.OurDockIn(plist);
			if (ours != -1 && PrismaticAI.ShipsAt(ours) > 0) continue;
		} else if (PrismaticAI.ShipsAt(AIIndustry.GetDockLocation(p)) > 0) continue;
		foreach (c, _ in AICargoList_IndustryProducing(p)) {
			if (!AICargo.IsFreight(c)) continue;
			local prod = AIIndustry.GetLastMonthProduction(p, c);
			if (prod < SHIP_MIN_PRODUCTION) continue;
			local served = AIIndustry.GetLastMonthTransportedPercentage(p, c);
			if (served >= SHIP_SERVED_BELOW) continue;
			prod = prod * (100 - served) / 100;
			if (!engines.rawin(c)) engines[c] <- PrismaticAI.PickShip(c);
			local engine = engines[c];
			if (engine == -1) continue;
			local accs = AIIndustryList_CargoAccepting(c);
			accs.Valuate(AIIndustry.GetDistanceManhattanToTile, AIIndustry.GetLocation(p));
			accs.KeepBetweenValue(SHIP_MIN_TILES - 1, far + 1);
			foreach (q, dist in accs) {
				local key = p + "-" + q + "-" + c;
				if (this.ship_tried.rawin(key) || q == p) continue;
				if (!wet.rawin(q)) {
					local ql = AIIndustry.HasDock(q) ? null : AITileList_IndustryAccepting(q, radius);
					wet[q] <- {list = ql, ok = ql == null || PrismaticAI.ListTouchesWater(ql)};
				}
				if (!wet[q].ok) continue;
				local est = PrismaticAI.ShipEstimate(engine, c, dist, prod * 70 / 100, 1, b.ships_cap);
				if (est.per_ship * SHIP_PAYBACK_YEARS < AIEngine.GetPrice(engine)) continue;
				out.push({key = key, kind = "freight", cargo = c, from = p, to = q, plist = plist, qlist = wet[q].list,
					dist = dist, engine = engine, est = est, score = est.profit,
					label = AICargo.GetCargoLabel(c) + " " + AIIndustry.GetName(p) + " - " + AIIndustry.GetName(q)});
			}
		}
	}
	local pax = PrismaticAI.PassengerCargo();
	local pax_engine = (pax == -1) ? -1 : PrismaticAI.PickShip(pax);
	if (pax_engine != -1) {
		local towns = [];
		foreach (t, _ in AITownList()) {
			if (AITown.GetPopulation(t) < SHIP_MIN_POPULATION) continue;
			local tl = PrismaticAI.TownDockTiles(t);
			if (!PrismaticAI.ListTouchesWater(tl)) continue;
			local ours = PrismaticAI.OurDockIn(tl);
			if (ours != -1 && PrismaticAI.ShipsAt(ours) > 0) continue;
			towns.push({id = t, tiles = tl, supply = AITown.GetLastMonthProduction(t, pax) / 4});
		}
		foreach (i, ta in towns) {
			foreach (j, tb in towns) {
				if (j <= i) continue;
				local dist = AIMap.DistanceManhattan(AITown.GetLocation(ta.id), AITown.GetLocation(tb.id));
				if (dist < SHIP_MIN_TILES || dist > far) continue;
				local key = "town-" + ta.id + "-" + tb.id;
				if (this.ship_tried.rawin(key)) continue;
				local est = PrismaticAI.ShipEstimate(pax_engine, pax, dist, min(ta.supply, tb.supply), 2, b.ships_cap);
				if (est.per_ship * SHIP_PAYBACK_YEARS < AIEngine.GetPrice(pax_engine)) continue;
				out.push({key = key, kind = "passengers", cargo = pax, from = ta.id, to = tb.id, plist = ta.tiles, qlist = tb.tiles,
					dist = dist, engine = pax_engine, est = est, score = est.profit,
					label = "PASS " + AITown.GetName(ta.id) + " - " + AITown.GetName(tb.id)});
			}
		}
	}
	out.sort(function(x, y) { return (y.score > x.score) ? 1 : ((y.score < x.score) ? -1 : 0); });
	return out;
}

/* Tiles within 8 of town `t`'s centre where a dock collects and delivers its passengers. */
function PrismaticAI::TownDockTiles(t) {
	local c = AITown.GetLocation(t), r = 8;
	local tiles = AITileList();
	tiles.AddRectangle(AIMap.GetTileIndex(max(1, AIMap.GetTileX(c) - r), max(1, AIMap.GetTileY(c) - r)),
		AIMap.GetTileIndex(min(AIMap.GetMapSizeX() - 2, AIMap.GetTileX(c) + r), min(AIMap.GetMapSizeY() - 2, AIMap.GetTileY(c) + r)));
	return tiles;
}

/* Both ends of candidate `c` on the map, still building nothing: {a, b, steps} or why not. */
function PrismaticAI::CheckShipRoute(c) {
	local freight = c.kind == "freight";
	local la = freight ? AIIndustry.GetLocation(c.from) : AITown.GetLocation(c.from);
	local lb = freight ? AIIndustry.GetLocation(c.to) : AITown.GetLocation(c.to);
	local a = PrismaticAI.ShipEnd(freight ? c.from : -1, c.plist, lb, c.cargo, !freight, true,
		freight ? AIIndustry.GetName(c.from) : AITown.GetName(c.from));
	if (a == null) return "no dock site at " + (freight ? AIIndustry.GetName(c.from) : AITown.GetName(c.from));
	local b = PrismaticAI.ShipEnd(freight ? c.to : -1, c.qlist, la, c.cargo, true, !freight,
		freight ? AIIndustry.GetName(c.to) : AITown.GetName(c.to));
	if (b == null) return "no dock site at " + (freight ? AIIndustry.GetName(c.to) : AITown.GetName(c.to));
	if (a.tile == b.tile) return "both ends share the dock at tile " + a.tile;
	local steps = PrismaticAI.WaterPath(a.fronts, b.fronts);
	if (steps < 0) return "no water joins tile " + a.tile + " and tile " + b.tile + " within " + SHIP_PATH_BUDGET + " tiles";
	return {a = a, b = b, steps = steps};
}

/* Build a dock for ShipEnd `e` when it needs one; true when the end can take ships afterwards. */
function PrismaticAI::BuildShipEnd(e) {
	if (e.site == -1) return true;
	if (e.level != null) {
		foreach (b in e.level.corners) {
			if (!AITile.LowerTile(e.site, b)) {
				AILog.Warning("PrismaticAI: ships: levelling the shore at " + e.label + ", tile " + e.site + " refused: " + AIError.GetLastErrorString());
				return false;
			}
		}
		AILog.Info("PrismaticAI: ships: levelled the shore at " + e.label + ", tile " + e.site + " for a dock");
	}
	if (AIMarine.BuildDock(e.site, AIStation.STATION_NEW)) {
		AILog.Info("PrismaticAI: ships: built a dock at " + e.label + ", tile " + e.site);
		AIController.Sleep(PACE);
		return true;
	}
	AILog.Warning("PrismaticAI: ships: dock at " + e.label + ", tile " + e.site + " refused: " + AIError.GetLastErrorString());
	return false;
}

/* Take down what a route that got no ship built (docks with no ship calling, a depot). */
function PrismaticAI::UndoShipRoute(ends, depot) {
	foreach (e in ends) {
		if (e.site != -1 && AIMarine.IsDockTile(e.site) && PrismaticAI.ShipsAt(e.site) == 0 && AIMarine.RemoveDock(e.site)) {
			AILog.Info("PrismaticAI: ships: removed the unused dock at tile " + e.site);
		}
	}
	if (depot != null && depot.built && AIMarine.RemoveWaterDepot(depot.tile)) {
		AILog.Info("PrismaticAI: ships: removed the unused ship depot at tile " + depot.tile);
	}
}

/* Build checked route `c` (CheckShipRoute's `r`): docks, depot, ships. Returns the ships running. */
function PrismaticAI::BuildShipRoute(c, r, b) {
	if (!PrismaticAI.BuildShipEnd(r.a)) return 0;
	if (!PrismaticAI.BuildShipEnd(r.b)) { PrismaticAI.UndoShipRoute([r.a], null); return 0; }
	/* The depot by a land dock (an oil rig has no shore). */
	local land = (r.a.site != -1 || !(c.kind == "freight" && AIIndustry.HasDock(c.from))) ? r.a : r.b;
	local depot = PrismaticAI.ShipDepotNear(land, land.fronts[0]);
	if (depot == null && land != r.b) depot = PrismaticAI.ShipDepotNear(r.b, r.b.fronts[0]);
	if (depot == null) {
		AILog.Warning("PrismaticAI: ships: no ship depot site near " + land.label + ": " + AIError.GetLastErrorString());
		PrismaticAI.UndoShipRoute([r.a, r.b], null);
		return 0;
	}
	if (depot.built) AILog.Info("PrismaticAI: ships: built a ship depot at tile " + depot.tile);
	local engine = PrismaticAI.PickShip(c.cargo, c.dist);
	if (engine == -1) engine = c.engine;
	local want = min(SHIP_START_MAX, c.est.ships);
	local running = 0;
	for (local i = 0; i < want; i++) {
		/* Past the first ship, only while half a year of the route's running costs stays at hand. */
		local funds = AICompany.GetBankBalance(AICompany.COMPANY_SELF) + AICompany.GetMaxLoanAmount() - AICompany.GetLoanAmount();
		if (i > 0 && funds < AIEngine.GetPrice(engine) + (i + 1) * AIEngine.GetRunningCost(engine) / 2) {
			AILog.Info("PrismaticAI: ships: stopped at " + running + " ship(s): " + funds + " left would not run more");
			break;
		}
		local v = (AIEngine.GetCargoType(engine) == c.cargo) ? AIVehicle.BuildVehicle(depot.tile, engine)
			: AIVehicle.BuildVehicleWithRefit(depot.tile, engine, c.cargo);
		if (!AIVehicle.IsValidVehicle(v)) {
			AILog.Warning("PrismaticAI: ships: ship " + i + " (" + PrismaticAI.EngineLabel(engine) + ") not bought: " + AIError.GetLastErrorString());
			break;
		}
		local ok = true;
		if (c.kind == "freight") {
			ok = AIOrder.AppendOrder(v, r.a.tile, AIOrder.OF_FULL_LOAD_ANY) && AIOrder.AppendOrder(v, r.b.tile, AIOrder.OF_NONE);
		} else {
			local first = (i % 2 == 0) ? r.a.tile : r.b.tile, second = (i % 2 == 0) ? r.b.tile : r.a.tile;
			ok = AIOrder.AppendOrder(v, first, AIOrder.OF_NONE) && AIOrder.AppendOrder(v, second, AIOrder.OF_NONE);
		}
		if (!ok) {
			AILog.Warning("PrismaticAI: ships: orders for ship " + v + " refused: " + AIError.GetLastErrorString() + "; sold");
			AIVehicle.SellVehicle(v);
			break;
		}
		AIVehicle.StartStopVehicle(v);
		running++;
		AILog.Info("PrismaticAI: ships: ship " + running + " (" + PrismaticAI.EngineLabel(engine) + ", " +
			AIVehicle.GetCapacity(v, c.cargo) + " " + AICargo.GetCargoLabel(c.cargo) + ") sails " + c.label);
		AIController.Sleep(PACE);
	}
	if (running == 0) PrismaticAI.UndoShipRoute([r.a, r.b], depot);
	return running;
}

/* The heartbeat's ship step (main.nut): one new ship route while fewer than ships_max run and money
 * plus loan headroom reach line_budget. Returns true when it built one. */
function PrismaticAI::ShipRoutes(b) {
	if (b.ships_max <= 0) return false;
	if (b.planned_freight_only == 1) return false; /* one authority outside decides every freight line */
	if (this.ship_routes < 0) {
		this.ship_routes = PrismaticAI.CountShipRoutes();
		if (this.ship_routes > 0) AILog.Info("PrismaticAI: ships: " + this.ship_routes + " ship route(s) found on the map");
	}
	if (this.ship_routes >= b.ships_max) return false;
	local year = AIDate.GetYear(AIDate.GetCurrentDate());
	/* A map that offered nothing is looked at again once a game year: industries open and grow. */
	if (this.ship_idle_year == year) return false;
	local money = AICompany.GetBankBalance(AICompany.COMPANY_SELF);
	local headroom = AICompany.GetMaxLoanAmount() - AICompany.GetLoanAmount();
	if (money + headroom < b.line_budget) return false;
	local candidates = this.ShipCandidates(b);
	local checked = 0;
	foreach (c in candidates) {
		if (checked >= SHIP_TRIES) break;
		checked++;
		this.ship_tried[c.key] <- true;
		local r = PrismaticAI.CheckShipRoute(c);
		if (typeof r == "string") {
			AILog.Info("PrismaticAI: ships: " + c.label + " (" + c.dist + " tiles, about " + c.est.profit + " a year with " +
				c.est.ships + " ship(s)) not possible: " + r);
			continue;
		}
		AILog.Info("PrismaticAI: ships: route " + (this.ship_routes + 1) + "/" + b.ships_max + ": " + c.label + ", " + c.dist +
			" tiles (" + r.steps + " by water), about " + c.est.profit + " a year with " + c.est.ships + " ship(s) of " +
			PrismaticAI.EngineLabel(c.engine) + ", " + c.est.days + " days a way");
		local loan_before = AICompany.GetLoanAmount();
		AICompany.SetLoanAmount(AICompany.GetMaxLoanAmount());
		if (PrismaticAI.BuildShipRoute(c, r, b) > 0) {
			this.ship_routes++;
			this.routes++;
			AILog.Info("PrismaticAI: ships: built ship route " + this.ship_routes + ": " + c.label);
			return true;
		}
		AICompany.SetLoanAmount(loan_before);
		AILog.Info("PrismaticAI: ships: " + c.label + " not built; loan back to " + AICompany.GetLoanAmount());
		return false;
	}
	if (checked == 0) {
		this.ship_idle_year = year;
		AILog.Info("PrismaticAI: ships: no ship route left on this map in " + year + " (" + this.ship_routes + "/" + b.ships_max + " built)");
	}
	return false;
}
