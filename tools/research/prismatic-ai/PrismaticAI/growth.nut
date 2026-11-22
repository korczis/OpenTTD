/*
 * Growing stations (RESEARCH-ONLY): a busy rail line's platforms get longer, so its trains get longer.
 *
 * PlatformPlan (rail.nut) sizes a terminus by the supply when the line is built, usually 2 to 3 tiles
 * and 5 wagons. Served industries grow, and AAAHogEx runs long trains on long platforms. In the battle,
 * 4-tile platforms from the start beat the derived length in 9 of 9 replayed matches (+13.5%
 * geometric), while 7 tiles from the start lost 8%: length pays where the cargo is, not everywhere.
 * So a line grows its platforms one tile at a time, only while it earns, its trains are already as
 * long as the platforms take, and a full trainload still waits.
 *
 * Growth adds one station tile behind the dead end of every platform of the line, at both ends,
 * joined to the same station (AIRail.BuildRailStation with the station's id), so a platform stays
 * one platform. A depot standing behind a platform moves one tile back when no vehicle is inside it.
 * The new tiles are tried in test mode at both ends first, so a line never ends up with platforms of
 * two lengths. Only stations that load and unload grow: an idle station costs its town rating.
 *
 * Setting rail_grow_max: the longest platform growth makes (0: off), never beyond the game's
 * vehicle.max_train_length and station.station_spread.
 */

/* The new tiles behind the dead ends of a station record's platforms (rail.nut TerminusGeometry
 * `backs`: [tile behind, platform end]), with the tile beyond each one for a moved depot. */
function PrismaticAI::GrowthTiles(st) {
	local out = [];
	if (!("backs" in st)) return out;
	foreach (b in st.backs) out.push({tile = b[0], beyond = b[0] + (b[0] - b[1]), end = b[1]});
	return out;
}

/* The smallest index among `tiles` (a rectangle's corner for AIRail.BuildRailStation). */
function PrismaticAI::MinTile(tiles) {
	local m = tiles[0].tile;
	foreach (t in tiles) if (t.tile < m) m = t.tile;
	return m;
}

/* Whether no vehicle stands in `depot`. */
function PrismaticAI::DepotEmpty(depot) {
	local list = AIVehicleList();
	list.Valuate(AIVehicle.GetLocation);
	list.KeepValue(depot);
	foreach (v, _ in list) if (AIVehicle.IsInDepot(v)) return false;
	return true;
}

/* The longest platform the game allows growth to: rail_grow_max, the train length limit and the
 * station spread. */
function PrismaticAI::GrowCap(b) {
	local cap = b.rail_grow_max;
	if (AIGameSettings.IsValid("vehicle.max_train_length")) cap = min(cap, AIGameSettings.GetValue("vehicle.max_train_length"));
	if (AIGameSettings.IsValid("station.station_spread")) cap = min(cap, AIGameSettings.GetValue("station.station_spread"));
	return cap;
}

/* Lengthens `line`'s platforms by one tile at both ends. Returns null when it did, else why not. */
function PrismaticAI::GrowLine(line) {
	local plan = [];
	foreach (st in line.stations) {
		local tiles = PrismaticAI.GrowthTiles(st);
		if (tiles.len() == 0) return "no dead ends recorded";
		local id = AIStation.GetStationID(st.tile);
		if (!AIStation.IsValidStation(id)) return "station gone";
		local axis = (st.step == 1) ? AIRail.RAILTRACK_NE_SW : AIRail.RAILTRACK_NW_SE;
		plan.push({st = st, tiles = tiles, id = id, axis = axis});
	}
	/* A depot in the way moves; one with a vehicle inside stops the growth. */
	local depot_moves = [];
	foreach (p in plan) {
		foreach (t in p.tiles) {
			if (t.tile == line.depot) {
				if (!PrismaticAI.DepotEmpty(line.depot)) return "a vehicle is in the depot behind a platform";
				depot_moves.push(t);
			} else if (!AITile.IsBuildable(t.tile)) {
				return "tile " + t.tile + " behind a platform is not buildable" + (AIRail.IsRailDepotTile(t.tile) ? " (another depot)" : (AIRail.IsRailTile(t.tile) ? " (track)" : ""));
			}
		}
	}
	{
		local test = AITestMode();
		foreach (p in plan) {
			local rows = p.tiles.len();
			/* The depot tile is not buildable in test mode; test the other rows only then. */
			local free = [];
			foreach (t in p.tiles) if (t.tile != line.depot) free.push(t);
			if (free.len() == 0) continue;
			if (free.len() == rows && !AIRail.BuildRailStation(PrismaticAI.MinTile(free), p.axis, rows, 1, p.id)) return "test refused: " + AIError.GetLastErrorString();
		}
	}
	foreach (t in depot_moves) AITile.DemolishTile(t.tile);
	local done = [];
	foreach (p in plan) {
		if (!AIRail.BuildRailStation(PrismaticAI.MinTile(p.tiles), p.axis, p.tiles.len(), 1, p.id)) {
			AILog.Warning("PrismaticAI: growing the platforms at tile " + p.st.tile + " refused: " + AIError.GetLastErrorString());
			foreach (q in done) AIRail.RemoveRailStationTileRectangle(PrismaticAI.MinTile(q.tiles), PrismaticAI.MinTile(q.tiles) + (q.tiles.len() - 1) * (q.st.step == 1 ? AIMap.GetMapSizeX() : 1), false);
			foreach (t in depot_moves) AIRail.BuildRailDepot(t.tile, t.end);
			return "refused: " + AIError.GetLastErrorString();
		}
		done.push(p);
	}
	/* The records follow: new dead ends, a longer rectangle, the depot behind its new end. */
	foreach (p in plan) {
		local backs = [];
		foreach (t in p.tiles) backs.push([t.beyond, t.tile]);
		p.st.backs = backs;
		local r = p.st.platform_rect;
		local lo = min(min(r[0], r[1]), PrismaticAI.MinTile(p.tiles));
		local hi = r[0], far = r[1];
		foreach (t in p.tiles) if (t.tile > far) far = t.tile;
		hi = max(max(r[0], r[1]), far);
		p.st.platform_rect = [lo, hi];
		p.st.len <- (("len" in p.st) ? p.st.len : line.len) + 1;
	}
	foreach (t in depot_moves) {
		if (AIRail.BuildRailDepot(t.beyond, t.tile)) {
			line.depot = t.beyond;
		} else {
			local d = null;
			foreach (st in line.stations) { d = PrismaticAI.BuildRailDepotBehind(st); if (d != null) break; }
			if (d != null) line.depot = d;
			else AILog.Warning("PrismaticAI: line " + line.label + " lost its depot while its platforms grew");
		}
	}
	line.len++;
	if ("wagons_fit" in line) delete line.wagons_fit;
	return null;
}

/* Once a game month, at most one line grows: the one with the most cargo waiting per trainload among
 * those that earn, run trains as long as their platforms and still leave a full trainload waiting. */
function PrismaticAI::GrowPlatforms(b) {
	if (!("rail_grow_max" in b) || b.rail_grow_max <= 0) return;
	local cap = PrismaticAI.GrowCap(b);
	local money = AICompany.GetBankBalance(AICompany.COMPANY_SELF);
	if (money < b.cash_reserve + 20000) return;
	local best = null, best_loads = 1.0;
	foreach (i, line in RAIL_LINES) {
		if (line.closed || line.len >= cap || !("stations" in line) || line.stations.len() != 2) continue;
		if (("len_cap" in line) && line.len >= line.len_cap) continue;
		local s = this.LineStats(line);
		if (s.trains == 0 || s.capacity <= 0 || s.profit_last <= 0) continue;
		local fit = ("wagons_fit" in line) ? line.wagons_fit : s.wagons;
		if (s.wagons < fit) continue;
		local loads = PrismaticAI.LineWaiting(line).tofloat() * s.trains / s.capacity.tofloat();
		if (loads >= best_loads) { best_loads = loads; best = [i, line]; }
	}
	if (best == null) return;
	local line = best[1];
	local slot = best[0] + 1;
	local why = PrismaticAI.GrowLine(line);
	if (why == null) {
		AILog.Info("PrismaticAI: grew the platforms of rail line " + slot + " (" + line.label + ") to " + line.len +
			" tiles: " + best_loads + " trainloads waiting, trains at full length");
		AILog.Info(PrismaticAI.LineRecord(slot, line));
		/* The point of the longer platform: every train of the line takes the wagons it now holds at
		 * once (lines.nut resizes them in the depot), rather than waiting for the steward's lengthen
		 * rule, which a second train added meanwhile kept from firing. */
		local cur = this.self_targets.rawin(slot) ? this.self_targets[slot] : { trains = -1, wagons = -1 };
		this.self_targets[slot] <- { trains = cur.trains, wagons = line.wagons_fit };
		AILog.Info("PrismaticAI: self-steward line " + slot + " (" + line.label + "): lengthen to " + line.wagons_fit + " wagons after the platforms grew");
	} else {
		AILog.Info("PrismaticAI: rail line " + slot + " (" + line.label + ") cannot grow its platforms at " + line.len + " tiles: " + why);
		line.len_cap <- line.len;
	}
}
