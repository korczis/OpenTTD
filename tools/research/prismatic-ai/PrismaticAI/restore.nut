/*
 * Restart without amnesia (RESEARCH-ONLY): Save, Load and Restore.
 *
 * Squirrel code only changes when the AI restarts, and the only restart that keeps the company is a
 * server save and load, which restarts every AI. Everything the bot knows lives in memory: its rail
 * lines (RAIL_LINES), the towns, pairs and planner lines it tried, its counters. A restarted bot
 * that knew none of it would build the planner's freight lines a second time beside the ones it runs,
 * and the steward would not know its own lines.
 *
 * - Save() hands the game a table of primitives (OpenTTD keeps integers, strings of at most 255
 *   characters, bools, null, arrays and tables, nested at most 25 deep, within MAX_SL_OPS). The line
 *   records go without their track tile lists (path, path2): a double-track line keeps the mark.
 * - Load() only keeps that table (it runs before Start, in MAX_SL_OPS, and must not touch the map).
 * - Restore(b), called once from Start after the first Sleep: from the save when there is one of this
 *   layout, else from the map when the company already has vehicles or stations (a bot deployed into
 *   a running game, or one whose save came from a version without Save()).
 *
 * From the map, a rail line is a group of trains sharing orders: its stations are its first two
 * orders, its cargo the one its train carries most of, its depot the rail depot nearest to it that
 * no other line took. It has no track list and no station records, so closing it sells its trains
 * and leaves its track, stations and depot standing.
 */

/* The layout of the table Save returns; Restore ignores any other. */
const RESTORE_SAVE_VERSION = 1;
/* An industry this close to a line's station (tiles, to its location tile) is the one it serves. */
const RESTORE_INDUSTRY_REACH = 16;
/* A depot this close to either station may be a line's own (a depot behind a platform or beside
 * the line within its first tiles, rail.nut). */
const RESTORE_DEPOT_REACH = 24;
/* Longer than any platform the game allows (station.station_spread is at most 64). */
const RESTORE_WALK_MAX = 64;

/* Whether Restore ran: until then Save hands back the loaded table, not the empty state. */
RESTORE_DONE <- false;

/* The bot's state as a save table (see the head of this file). */
function PrismaticAI::Save() {
	/* Saved before Restore ran (during Start's first Sleep): what was loaded, else nothing, so the
	 * next load rebuilds from the map rather than from an empty save. */
	if (!RESTORE_DONE) return (this.loaded != null && typeof this.loaded.data == "table") ? this.loaded.data : {};
	local lines = [];
	foreach (line in RAIL_LINES) {
		local l = {};
		foreach (k, v in line) if (k != "path" && k != "path2") l[k] <- v;
		if ("path2" in line) l.dbl_track <- true;
		if (("label" in l) && typeof l.label == "string" && l.label.len() > 200) l.label = l.label.slice(0, 200);
		lines.push(l);
	}
	local rich = false;
	try { rich = this.rich; } catch (e) {}
	return {
		v = RESTORE_SAVE_VERSION,
		lines = lines,
		used_towns = this.used_towns,
		routes = this.routes,
		air_done = this.air_done,
		rail_done = this.rail_done,
		/* main.nut PairDue: the date the passenger pair first was due, and that pair, so a load
		 * keeps its wait. */
		pair_due_since = this.pair_due_since,
		pair_due_key = this.pair_due_key,
		freight_tried = this.freight_tried,
		own_built = this.own_built,
		own_failed = this.own_failed,
		own_failed_year = this.own_failed_year,
		rescued_at = this.rescued_at,
		selling = this.selling,
		resizing = this.resizing,
		lost = this.lost,
		orphans = this.orphans,
		self_targets = this.self_targets,
		auto_air = this.auto_air,
		air_failed = this.air_failed,
		air_year = this.air_year,
		ringed = this.ringed,
		twins = this.twins,
		air_towns = this.air_towns,
		air_tried = this.air_tried,
		rich = rich,
		air_ports = AIR_PORTS,
		air_bad_towns = AIR_BAD_TOWNS,
		failed_pairs = FAILED_PAIRS,
		modernize_set = MODERNIZE_SET,
		/* rail.nut ClaimFresh: when each foreign rail station tile near a producer was first seen. */
		claims_seen = CLAIM_SEEN,
	};
}

/* Keep the saved table for Restore; nothing else (Load runs before Start). */
function PrismaticAI::Load(version, data) {
	this.loaded = {version = version, data = data};
}

/* `data[key]` when it is there and of the type of `fallback`, else `fallback`. */
function PrismaticAI::RestoreField(data, key, fallback) {
	if (!(key in data) || data[key] == null) return fallback;
	return (typeof data[key] == typeof fallback) ? data[key] : fallback;
}

/* Replace the contents of the table `target` (a global the other files hold) with `source`. */
function PrismaticAI::RestoreFill(target, source) {
	target.clear();
	if (typeof source != "table") return;
	foreach (k, v in source) target[k] <- v;
}

/* Once, from Start: the state from the save, else from the map, else nothing (a new company). */
function PrismaticAI::Restore(b) {
	local saved = this.loaded;
	local how = null;
	if (saved != null && typeof saved.data == "table" && ("v" in saved.data) && saved.data.v == RESTORE_SAVE_VERSION) {
		this.RestoreFromSave(saved.data);
		how = "from the save";
	} else if (AIVehicleList().Count() > 0 || AIStationList(AIStation.STATION_ANY).Count() > 0) {
		this.RestoreFromMap(b);
		how = "from the map";
	}
	/* Only now: a save while Start was suspended in here still hands back the loaded table. */
	this.loaded = null;
	RESTORE_DONE = true;
	if (how == null) return;
	local old_claims = PrismaticAI.ClaimsAfterRestart();
	local freight = 0;
	foreach (line in RAIL_LINES) if ("from" in line) freight++;
	/* The planner's passenger pair (main.nut PairDue), when there is one. */
	local pair = "";
	if (b.rail_a >= 0 && b.rail_b >= 0) {
		if (this.rail_done == b.rail_a + "-" + b.rail_b) pair = "; passenger pair done";
		else if (this.pair_due_since >= 0) pair = "; passenger pair due since " + PrismaticAI.RestoreDate(this.pair_due_since);
		else pair = "; passenger pair not due yet";
	}
	AILog.Info("PrismaticAI: restored after a restart: " + RAIL_LINES.len() + " rail lines (" + freight + " freight), " +
		this.auto_air + " autopilot air routes, " + this.routes + " routes, " + this.freight_tried.len() + " freight lines tried, " +
		this.own_built + " own, " + this.twins + " twins, " + this.used_towns.len() + " road towns, " + AIR_PORTS.len() +
		" airports, " + old_claims + " foreign rail station tiles of unknown age taken as old (" + how + ")" + pair);
}

/* `date` (AIDate) as YYYY-MM-DD. */
function PrismaticAI::RestoreDate(date) {
	local m = AIDate.GetMonth(date), d = AIDate.GetDayOfMonth(date);
	return AIDate.GetYear(date) + "-" + (m < 10 ? "0" : "") + m + "-" + (d < 10 ? "0" : "") + d;
}

/* The own freight lines that stand: RAIL_LINES not closed whose producer, acceptor and cargo the
 * planner did not hand over in fr_* .. fr16_*, each pair once (a twin is its pair's). */
function PrismaticAI::OwnLinesOpen() {
	local planned = {};
	foreach (fl in PrismaticAI.FreightLines()) planned[fl.from + "-" + fl.to + "-" + fl.cargo] <- true;
	local own = {};
	foreach (l in RAIL_LINES) {
		if (l.closed || !("from" in l) || !("to" in l)) continue;
		local key = l.from + "-" + l.to + "-" + l.cargo;
		if (!planned.rawin(key)) own[key] <- true;
	}
	return own.len();
}

/* own_built after a load: the own freight lines that stand (OwnLinesOpen), never the saved count.
 * A closed line gives its slot back (lines.nut CloseLine), so a load does not count it either.
 * Before main.nut stopped counting a failed own line (PR #234), the saved count held every line
 * tried: the owner's live game (seed 15805) was saved with "4 own" and one rail line, so after its
 * load with the fix it still planned no own line. `saved` is the count the save held, logged when
 * it differs. */
function PrismaticAI::RestoreOwnBuilt(saved) {
	local own = PrismaticAI.OwnLinesOpen();
	if (own != saved) AILog.Info("PrismaticAI: own lines counted from the restored lines: " + own + " (the save said " + saved + ")");
	return own;
}

/* The state Save wrote. */
function PrismaticAI::RestoreFromSave(data) {
	RAIL_LINES.clear();
	foreach (l in PrismaticAI.RestoreField(data, "lines", [])) {
		if (typeof l != "table") continue;
		if (!("path" in l)) l.path <- [];
		if ("dbl_track" in l) { delete l.dbl_track; l.path2 <- []; }
		if (!("stations" in l)) l.stations <- [];
		if (!("closed" in l)) l.closed <- false;
		if (!("restored" in l)) l.restored <- 0;
		RAIL_LINES.push(l);
	}
	this.used_towns = PrismaticAI.RestoreField(data, "used_towns", {});
	this.routes = PrismaticAI.RestoreField(data, "routes", 0);
	this.air_done = PrismaticAI.RestoreField(data, "air_done", "");
	this.rail_done = PrismaticAI.RestoreField(data, "rail_done", "");
	/* A save from before it was saved has none: the wait starts again, as it did then. */
	this.pair_due_since = PrismaticAI.RestoreField(data, "pair_due_since", -1);
	this.pair_due_key = PrismaticAI.RestoreField(data, "pair_due_key", "");
	this.freight_tried = PrismaticAI.RestoreField(data, "freight_tried", {});
	this.own_built = PrismaticAI.RestoreOwnBuilt(PrismaticAI.RestoreField(data, "own_built", 0));
	this.own_failed = PrismaticAI.RestoreField(data, "own_failed", 0);
	this.own_failed_year = PrismaticAI.RestoreField(data, "own_failed_year", 0);
	this.rescued_at = PrismaticAI.RestoreField(data, "rescued_at", -1);
	this.selling = PrismaticAI.RestoreField(data, "selling", {});
	this.resizing = PrismaticAI.RestoreField(data, "resizing", {});
	this.lost = PrismaticAI.RestoreField(data, "lost", {});
	this.orphans = PrismaticAI.RestoreField(data, "orphans", {});
	this.self_targets = PrismaticAI.RestoreField(data, "self_targets", {});
	this.auto_air = PrismaticAI.RestoreField(data, "auto_air", 0);
	this.air_failed = PrismaticAI.RestoreField(data, "air_failed", 0);
	this.air_year = PrismaticAI.RestoreField(data, "air_year", 0);
	this.ringed = PrismaticAI.RestoreField(data, "ringed", {});
	this.twins = PrismaticAI.RestoreField(data, "twins", 0);
	this.air_towns = PrismaticAI.RestoreField(data, "air_towns", {});
	this.air_tried = PrismaticAI.RestoreField(data, "air_tried", {});
	try { this.rich = PrismaticAI.RestoreField(data, "rich", false); } catch (e) {}
	PrismaticAI.RestoreFill(AIR_PORTS, PrismaticAI.RestoreField(data, "air_ports", {}));
	PrismaticAI.RestoreFill(AIR_BAD_TOWNS, PrismaticAI.RestoreField(data, "air_bad_towns", {}));
	PrismaticAI.RestoreFill(FAILED_PAIRS, PrismaticAI.RestoreField(data, "failed_pairs", {}));
	PrismaticAI.RestoreFill(MODERNIZE_SET, PrismaticAI.RestoreField(data, "modernize_set", {}));
	PrismaticAI.RestoreFill(CLAIM_SEEN, PrismaticAI.RestoreField(data, "claims_seen", {}));
	PrismaticAI.RestoreTwinMarks();
}

/* After a load, the twins as the lines stand (steward.nut TwinLines, HasOpenTwin). A `twinned` mark
 * with no open line between the same industries is a failed attempt's (before TwinLines dated
 * failed attempts, it marked the line before building and kept the mark when the build failed) or
 * a closed twin's: the line becomes one whose attempt is due now, so a game saved before the fix
 * gets its twins tried, one a game month. this.twins, which rail_twin_max caps, is counted again
 * from the open twins: the saved count kept closed ones. */
function PrismaticAI::RestoreTwinMarks() {
	local now = AIDate.GetCurrentDate(), open = {};
	foreach (i, l in RAIL_LINES) {
		if (!("from" in l) || !("to" in l)) continue;
		if (!l.closed) {
			local key = l.from + "-" + l.to + "-" + l.cargo;
			open[key] <- (open.rawin(key) ? open[key] : 0) + 1;
		}
		if (!("twinned" in l) || PrismaticAI.HasOpenTwin(l)) continue;
		delete l.twinned;
		l.twin_fails <- 1;
		l.twin_failed <- now - TWIN_RETRY_DAYS;
		if (!l.closed) AILog.Info("PrismaticAI: rail line " + (i + 1) + " (" + l.label + ") was marked twinned without a twin; its twin is due again");
	}
	local twins = 0;
	foreach (_, n in open) twins += n - 1;
	if (twins != this.twins) AILog.Info("PrismaticAI: twins counted from the open lines: " + twins + " (the save said " + this.twins + ")");
	this.twins = twins;
}

/* How many tiles of station `st`'s platforms of direction `dir` follow `tile` by the step `d`. */
function PrismaticAI::RestoreRun(tile, d, st, dir) {
	local n = 0;
	for (local t = tile + d; n < RESTORE_WALK_MAX && AIMap.IsValidTile(t) && AIRail.IsRailStationTile(t) &&
			AIStation.GetStationID(t) == st && AIRail.GetRailStationDirection(t) == dir; t += d) n++;
	return n;
}

/* The platform through the station tile `tile`: {len, rows}, its length and the platforms side by
 * side with it (a station with lines joined beside each other counts all of theirs). */
function PrismaticAI::RestorePlatform(tile) {
	if (!AIRail.IsRailStationTile(tile)) return {len = 1, rows = 1};
	local st = AIStation.GetStationID(tile), dir = AIRail.GetRailStationDirection(tile);
	local map_w = AIMap.GetMapSizeX();
	local step = (dir == AIRail.RAILTRACK_NE_SW) ? 1 : map_w, side = (step == 1) ? map_w : 1;
	return {len = 1 + PrismaticAI.RestoreRun(tile, step, st, dir) + PrismaticAI.RestoreRun(tile, -step, st, dir),
		rows = 1 + PrismaticAI.RestoreRun(tile, side, st, dir) + PrismaticAI.RestoreRun(tile, -side, st, dir)};
}

/* The cargo `train` carries most of, or -1. */
function PrismaticAI::RestoreCargo(train) {
	local best = -1, best_cap = 0;
	foreach (c, _ in AICargoList()) {
		local cap = AIVehicle.GetCapacity(train, c);
		if (cap > best_cap) { best_cap = cap; best = c; }
	}
	return best;
}

/* The industry nearest `tile` (within RESTORE_INDUSTRY_REACH) in `list`, or -1. */
function PrismaticAI::RestoreIndustryNear(list, tile) {
	list.Valuate(AIIndustry.GetDistanceManhattanToTile, tile);
	list.KeepBelowValue(RESTORE_INDUSTRY_REACH + 1);
	list.Sort(AIList.SORT_BY_VALUE, AIList.SORT_ASCENDING);
	return list.IsEmpty() ? -1 : list.Begin();
}

/* The line's depot: a rail depot whose front is a platform of one of its stations first (rail.nut
 * BuildRailDepotBehind), else the nearest within RESTORE_DEPOT_REACH, one that no line restored
 * before took (`taken`) when there is one. -1 when none is near. */
function PrismaticAI::RestoreDepot(a, b, taken) {
	local st_a = AIStation.GetStationID(a), st_b = AIStation.GetStationID(b);
	local best = -1, best_rank = 1 << 30, best_free = false;
	foreach (d, _ in AIDepotList(AITile.TRANSPORT_RAIL)) {
		local dist = min(AIMap.DistanceManhattan(d, a), AIMap.DistanceManhattan(d, b));
		if (dist > RESTORE_DEPOT_REACH) continue;
		local front = AIRail.GetRailDepotFrontTile(d);
		local behind = AIRail.IsRailStationTile(front) &&
			(AIStation.GetStationID(front) == st_a || AIStation.GetStationID(front) == st_b);
		local rank = (behind ? 0 : 100) + dist;
		local free = !taken.rawin(d);
		if ((free && !best_free) || (free == best_free && rank < best_rank)) {
			best = d; best_rank = rank; best_free = free;
		}
	}
	return best;
}

/* The state from the map, for a bot that restarted with no save of its own (see the head). */
function PrismaticAI::RestoreFromMap(b) {
	/* Rail lines: one per group of trains sharing orders, ordered by their lowest vehicle id. */
	RAIL_LINES.clear();
	local trains = AIVehicleList();
	trains.Valuate(AIVehicle.GetVehicleType);
	trains.KeepValue(AIVehicle.VT_RAIL);
	trains.Sort(AIList.SORT_BY_ITEM, AIList.SORT_ASCENDING);
	local seen = {}, depots = {}, keys = {}, pair_line = false;
	local cap_platforms = max(1, b.rail_platforms);
	if (b.rail_double == 1) cap_platforms = max(cap_platforms, 2);
	foreach (v, _ in trains) {
		if (seen.rawin(v)) continue;
		local group = AIVehicleList_SharedOrders(v);
		foreach (u, w in group) seen[u] <- true;
		if (AIOrder.GetOrderCount(v) < 2 || !AIOrder.IsGotoStationOrder(v, 0) || !AIOrder.IsGotoStationOrder(v, 1)) continue;
		local a = AIOrder.GetOrderDestination(v, 0), bt = AIOrder.GetOrderDestination(v, 1);
		if (!AIRail.IsRailStationTile(a) || !AIRail.IsRailStationTile(bt)) continue;
		local cargo = PrismaticAI.RestoreCargo(v);
		if (cargo == -1) continue;
		local pa = PrismaticAI.RestorePlatform(a), pb = PrismaticAI.RestorePlatform(bt);
		/* The train fits its platform: never a platform shorter than the train is. */
		local len = max(min(pa.len, pb.len), (AIVehicle.GetLength(v) + 15) / 16);
		local platforms = min(min(pa.rows, pb.rows), cap_platforms);
		local depot = PrismaticAI.RestoreDepot(a, bt, depots);
		if (depot != -1) depots[depot] <- true;
		local line = {a = a, b = bt, depot = depot, cargo = cargo, wagons = max(1, AIVehicle.GetNumWagons(v) - 1),
			flags = AIOrder.GetOrderFlags(v, 0), label = "", railtype = AIRail.GetRailType(a), restored = 0, len = len,
			/* A double-track line ran more trains than platforms: the trains it runs now are safe. */
			trains_max = max(platforms, group.Count()), path = [], stations = [], closed = false,
			platforms = platforms, first = v};
		if (AICargo.HasCargoClass(cargo, AICargo.CC_PASSENGERS)) {
			local ta = AIStation.GetNearestTown(AIStation.GetStationID(a));
			local tb = AIStation.GetNearestTown(AIStation.GetStationID(bt));
			line.label = AITown.GetName(ta) + " - " + AITown.GetName(tb);
			if ((ta == b.rail_a && tb == b.rail_b) || (ta == b.rail_b && tb == b.rail_a)) pair_line = true;
		} else {
			local from = PrismaticAI.RestoreIndustryNear(AIIndustryList_CargoProducing(cargo), a);
			local to = PrismaticAI.RestoreIndustryNear(AIIndustryList_CargoAccepting(cargo), bt);
			if (from != -1 && to != -1 && from != to) {
				line.from <- from;
				line.to <- to;
				line.label = AICargo.GetCargoLabel(cargo) + " " + AIIndustry.GetName(from) + " - " + AIIndustry.GetName(to);
			} else {
				line.label = AICargo.GetCargoLabel(cargo) + " " + AIStation.GetName(AIStation.GetStationID(a)) + " - " +
					AIStation.GetName(AIStation.GetStationID(bt));
			}
		}
		RAIL_LINES.push(line);
		if (!("from" in line)) continue;
		/* Two lines between the same industries: the second is the first's twin (steward.nut). */
		local key = line.from + "-" + line.to + "-" + line.cargo;
		if (keys.rawin(key)) {
			keys[key].twinned <- true;
			line.twinned <- true;
			this.twins++;
		} else {
			keys[key] <- line;
		}
	}

	/* Freight tried: every restored line's pair, and each planner line whose producer has a station
	 * of ours; own lines are the restored pairs the planner did not hand over. */
	this.freight_tried = {};
	local planned = {};
	foreach (fl in b.freight) {
		local key = fl.from + "-" + fl.to + "-" + fl.cargo;
		planned[key] <- true;
		if (keys.rawin(key) || (AIIndustry.IsValidIndustry(fl.from) &&
				PrismaticAI.OwnRailStationNear(AIIndustry.GetLocation(fl.from), 8) != null)) this.freight_tried[key] <- true;
	}
	this.own_built = 0;
	foreach (key, _ in keys) {
		this.freight_tried[key] <- true;
		if (!planned.rawin(key)) this.own_built++;
	}
	/* The planner's passenger pair is done only when a passenger line between its two towns stands:
	 * any rail line marked it done before, so a bot with freight lines and no pair line never built
	 * the pair after such a restart. Whether it was tried and failed the map cannot tell; it is tried
	 * again. Its wait (PairDue) starts again too: only a save holds it. */
	if (pair_line) this.rail_done = b.rail_a + "-" + b.rail_b;

	/* Air: routes are the distinct station sets of our aircraft (fleet.nut RouteKey); airports by
	 * their town (air.nut AIR_PORTS, autopilot air_towns). */
	local air_routes = {}, road_routes = {};
	foreach (v, _ in AIVehicleList()) {
		local type = AIVehicle.GetVehicleType(v);
		if (type != AIVehicle.VT_AIR && type != AIVehicle.VT_ROAD) continue;
		local k = PrismaticAI.RouteKey(v);
		if (k == "") continue;
		if (type == AIVehicle.VT_ROAD) { road_routes[k] <- true; continue; }
		if (air_routes.rawin(k)) continue;
		air_routes[k] <- true;
		local towns = [];
		for (local i = 0; i < AIOrder.GetOrderCount(v) && towns.len() < 2; i++) {
			local st = AIStation.GetStationID(AIOrder.GetOrderDestination(v, i));
			if (AIStation.IsValidStation(st)) towns.push(AIStation.GetNearestTown(st));
		}
		if (towns.len() == 2) {
			this.air_tried[towns[0] + "-" + towns[1]] <- true;
			this.air_tried[towns[1] + "-" + towns[0]] <- true;
		}
	}
	AIR_PORTS.clear();
	foreach (st, _ in AIStationList(AIStation.STATION_AIRPORT)) {
		local tiles = AITileList_StationType(st, AIStation.STATION_AIRPORT);
		if (tiles.IsEmpty()) continue;
		local town = AIStation.GetNearestTown(st);
		AIR_PORTS[town] <- tiles.Begin();
		this.air_towns[town] <- true;
	}
	local planner_air = b.air_a >= 0 && b.air_b >= 0 && air_routes.len() > 0;
	if (planner_air) this.air_done = b.air_a + "-" + b.air_b;
	this.auto_air = max(0, air_routes.len() - (planner_air ? 1 : 0));

	/* Road: the towns of our bus and truck stops, those of an airport's catchment ring left out. */
	foreach (kind in [AIStation.STATION_BUS_STOP, AIStation.STATION_TRUCK_STOP]) {
		foreach (st, _ in AIStationList(kind)) {
			if (AIStation.HasStationType(st, AIStation.STATION_AIRPORT)) continue;
			this.used_towns[AIStation.GetNearestTown(st)] <- true;
		}
	}
	this.routes = RAIL_LINES.len() + air_routes.len() + road_routes.len();

	/* A vehicle the old instance sent to its depot (to be sold or resized) would stand there for
	 * good: road vehicles and aircraft are sold as it meant, trains run again and the steward
	 * decides anew. */
	foreach (v, _ in AIVehicleList()) {
		local rail = AIVehicle.GetVehicleType(v) == AIVehicle.VT_RAIL;
		if (AIVehicle.IsStoppedInDepot(v)) {
			/* A train with no orders (a save while BuildTrain bought its wagons) stays: events.nut
			 * LookAfterVehicles gives it a line's orders or sells it. */
			if (rail && AIOrder.GetOrderCount(v) > 0) AIVehicle.StartStopVehicle(v);
			else if (rail) continue;
			else this.selling[v] <- true;
		} else if (AIOrder.IsGotoDepotOrder(v, AIOrder.ORDER_CURRENT) &&
				(AIOrder.GetOrderFlags(v, AIOrder.ORDER_CURRENT) & AIOrder.OF_STOP_IN_DEPOT) != 0) {
			/* Sending it again cancels the trip (lines.nut ReconcileLines). */
			if (rail) AIVehicle.SendVehicleToDepot(v);
			else this.selling[v] <- true;
		}
	}
}
