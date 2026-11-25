/*
 * Rail lines: between two towns (the planner's pair, settings rail_a, rail_b) or from a producing
 * industry to an accepting one (fr_*, the bot's own plan). One model, every number derived.
 *
 * Geometry a train can always run (RESEARCH-ONLY knowledge rail-lines-a-train-can-turn-on):
 * - Each station's axis points at the other end where a site allows, else the other axis or the
 *   reversed exit (BuildRailStationAt); its site is the buildable rectangle of the platforms plus
 *   their throat that accepts (or produces) the most cargo, flat or, with build_on_slopes, on
 *   foundations, and levelled only when no orientation has one.
 * - A terminus with `rail_platforms` platforms (default 2). With two, a two-tile throat merges them
 *   into the single line: a path signal on each platform's exit, a path signal facing the station
 *   on the line's first tile. Trains wait in a platform, never on the single track, so as many
 *   trains as the smaller end has platforms share one track without a passing loop and without a
 *   head-on collision (trains_max). With one platform there is no signal and one train.
 * - The line is an A* path between the two throats (FindRailPath). It never lays a 90-degree turn
 *   (Turns90): the game forbids one with pf.forbid_90_deg (on in this owner's config) and slows a
 *   train on one where it is allowed. A train meeting a forbidden turn cannot follow its own line,
 *   gets lost and earns nothing. The check runs on every tile, the first beyond each throat and the
 *   last into it included, and across a re-plan.
 * - The laid line is walked piece by piece (VerifyRailLine, AIRail.AreTilesConnected) before a
 *   train is bought: a gap becomes a warning with its tile, never a lost train.
 * - The depot stands behind a platform's dead end (trains turn in it and leave inside the signals),
 *   else beside a straight piece of the line, never on a corner.
 * - Platform length and wagons follow from demand (PlatformPlan): the cargo a month, the round
 *   trip at the locomotive's speed, the trains the platforms allow, capped by the game's
 *   vehicle.max_train_length and station.station_spread.
 * Every step is logged with its tile, so the camera director shows it, and a finished line logs
 * one record ("PrismaticAI: rail line <slot>: ...", lines.nut LineRecord) that
 * tools/research/mcp-panel/rhai/rail_lib.rhai reads.
 */

/* Whether three moves in a row (each +-1 along x or +-map width along y) make a 90-degree turn: a
 * corner on the tile between the first two moves and another between the last two, the train leaving
 * opposite to how it came (d3 == -d1). Alternating corners (d3 == d1) are a 45-degree diagonal and
 * fine. The game's rule is follow_track.hpp: with forbid_90_deg a trackdir that crosses the one the
 * train arrives on (TrackdirCrossesTrackdirs) is cut from what it may take. */
function PrismaticAI::Turns90(d1, d2, d3) {
	return d1 != d2 && d2 != d3 && d3 == -d1;
}

function PrismaticAI::RailStepCost(tile) {
	if (!AIMap.IsValidTile(tile)) return -1;
	/* A road tile is crossed only as a level crossing (CanCross), never built over as land:
	 * AITile.IsBuildable counts a half road tile as buildable. */
	if (AIRoad.IsRoadTile(tile)) return -1;
	if (AITile.IsWaterTile(tile) || !AITile.IsBuildable(tile)) return -1;
	local slope = AITile.GetSlope(tile);
	if (slope == AITile.SLOPE_FLAT) return 3;
	/* Track on a slope stands on a foundation (build_on_slopes, on by default), but not on a steep one. */
	if (AITile.IsSteepSlope(slope)) return -1;
	return 7;
}

/* Whether a station exit at `exit`, left by the step `out` (from the station's end toward the exit),
 * leads on by the same rules as FindRailPath's first step: straight ahead, or to either side where the
 * corner piece on the exit fits its slope (PieceFits), is land the line can take or a road it can
 * cross. Without the slope rule a site passed here and the search then stopped after one node
 * ("every reachable tile tried, 1 nodes", OpenTTD seed 31337, New Prunpool). */
function PrismaticAI::ExitLeadsOn(exit, out, straight = false) {
	local map_w = AIMap.GetMapSizeX();
	local perp = (out == 1 || out == -1) ? map_w : 1;
	local slope = AITile.GetSlope(exit);
	/* A throat's exit leads on straight only (see FindRailPath's from_straight). */
	foreach (d in (straight ? [out] : [out, perp, -perp])) {
		local n = exit + d;
		if (d != out && !PrismaticAI.PieceFits(slope, PrismaticAI.PieceBetween(exit, exit - out, n))) continue;
		if (PrismaticAI.RailStepCost(n) >= 0 || PrismaticAI.CanCross(n, d)) return true;
	}
	return false;
}

/* Whether the line can cross the road on `tile` as a level crossing when it enters the tile by the
 * step `d` (+-1 along x, +-map width along y): a plain road tile (no station, depot or existing
 * rail) on flat land, whose road runs only at right angles to the line. The game builds the
 * crossing when AIRail.BuildRail lays the straight piece across it (rail_cmd.cpp, a straight road
 * crossed perpendicularly). */
function PrismaticAI::CanCross(tile, d) {
	if (!AIMap.IsValidTile(tile) || !AIRoad.IsRoadTile(tile)) return false;
	if (AIRoad.IsRoadStationTile(tile) || AIRoad.IsRoadDepotTile(tile) || AIRail.IsRailTile(tile)) return false;
	if (AIRail.IsLevelCrossingTile(tile)) return false;
	if (AITile.GetSlope(tile) != AITile.SLOPE_FLAT) return false;
	local map_w = AIMap.GetMapSizeX();
	local along = (d == 1 || d == -1) ? 1 : map_w;   /* the line's axis */
	local across = (along == 1) ? map_w : 1;          /* the road must run this way only */
	if (AIRoad.AreRoadTilesConnected(tile, tile + along) || AIRoad.AreRoadTilesConnected(tile, tile - along)) return false;
	return AIRoad.AreRoadTilesConnected(tile, tile + across) || AIRoad.AreRoadTilesConnected(tile, tile - across);
}

/* The track piece on a tile that joins the side toward tile `a` and the side toward tile `b`, as
 * the game's TrackBits (X 1, Y 2, upper 4, lower 8, left 16, right 32). Sides: x-1 NE, y+1 SE,
 * x+1 SW, y-1 NW. */
function PrismaticAI::SideOf(d, map_w) {
	return d == -1 ? 0 : (d == map_w ? 1 : (d == 1 ? 2 : 3));
}

function PrismaticAI::PieceBetween(tile, a, b) {
	local map_w = AIMap.GetMapSizeX();
	local p = PrismaticAI.SideOf(a - tile, map_w), q = PrismaticAI.SideOf(b - tile, map_w);
	local lo = min(p, q), hi = max(p, q);
	if (lo == 0 && hi == 2) return 1;
	if (lo == 1 && hi == 3) return 2;
	if (lo == 0 && hi == 3) return 4;
	if (lo == 1 && hi == 2) return 8;
	if (lo == 2 && hi == 3) return 16;
	return 32;
}

/* Whether a track piece, or the pieces `piece` (track bits) of one tile together, fit on a slope,
 * as the game's GetRailFoundation decides with build_on_slopes on (rail_cmd.cpp): anything on flat
 * land; one straight piece on any slope that is not steep (inclined or levelled foundation);
 * otherwise only pieces that all need no foundation or all fit one levelled foundation
 * (_valid_tracks_without_foundation, _valid_tracks_on_leveled_foundation, indexed by the slope
 * bits W 1, S 2, E 4, N 8). A steep slope takes no piece here. A throat's M carries a straight
 * piece and a corner, and each alone can fit a slope where the two together do not. */
function PrismaticAI::PieceFits(slope, piece) {
	if (slope == AITile.SLOPE_FLAT) return true;
	if (AITile.IsSteepSlope(slope)) return false;
	if (piece == 1 || piece == 2) return true;
	local without = [63, 32, 4, 1, 16, 0, 2, 8, 8, 2, 0, 16, 1, 4, 32];
	local leveled = [0, 16, 8, 26, 32, 63, 41, 63, 4, 21, 63, 63, 38, 63, 63];
	return (without[slope] & piece) == piece || (leveled[slope] & piece) == piece;
}

/* A* from `from` to `to` over the four neighbours. Costs are in thirds of a straight piece, the game's
 * own measure of travel (vehicle_base.h): a straight piece is 16 steps of TILE_AXIAL_DISTANCE 192
 * (3072), a corner piece 8 steps of 2 * TILE_CORNER_DISTANCE 256 (2048), two thirds as long. So a
 * diagonal (a staircase of corner pieces that alternate, the train running at 45 degrees) costs 2 a
 * piece against 3 for straight track: the same pieces as an L-shaped line, a third less travel. Its
 * entry pays the turn once; a turn back on itself within two tiles (a U) is not taken.
 * `from_end` and `to_end` are the station end tiles beyond `from` and `to`: the line enters `from`
 * from its station and leaves `to` into the other one, so the pieces on both exits are checked
 * like any other. `avoid` (a table of tiles) is left out of the search: a re-plan passes the tile a
 * piece failed on. `from_in` is the move that arrived at `from_end` (null: the station's straight
 * platform, from - from_end); a re-plan passes the move into the last laid tile, so the first new
 * piece cannot make a 90-degree turn with it. `from_straight` and `to_straight` keep the line
 * straight through its first and last tile: beyond a two-platform throat the second platform
 * reaches the line by a sideways move (M2 to M), so a corner on the line's first tile toward that
 * side would make a 90-degree turn on the second platform's route, which the main platform's
 * route does not show (found by an independent review, 2026-11-18). Returns the tiles (from
 * first) or null within the node budget. */
/* The estimate a tile left to go, in the same thirds: 7, between a diagonal piece (6) and a straight
 * one (9). At 6 every staircase toward the goal had the same f = g + h, so the search spread over the
 * whole rectangle between the stations and lines the old search found failed (seed 31337, 2026-11-17). */
H_TILE <- 7;

/* The search's node budget scales with the distance it bridges. A search whose goal cannot be
 * reached under the line's rules tries every tile it can reach first: in the fitness match (OpenTTD
 * seed 930967890, planned lineup, 2026-11-22) three such searches took 43747 to 52161 nodes and
 * 202 to 241 game days each, one of them a re-plan between tiles two apart, and a heartbeat that
 * ran two held the money idle for fifteen months; on seeds 1814625749 and 4242 such searches took
 * 252 to 331 days (58007 to 75453 nodes), one between tiles ten apart. The 42 searches that found
 * their line in those three matches used at most 148 nodes a tile of distance (median 66). So a
 * search gets RAIL_NODES_PER_TILE nodes a tile, at least RAIL_NODES_MIN and at most RAIL_NODES_MAX,
 * and the heartbeat moves on to roads, air and the fleet sooner. Each search logs its nodes and
 * game days. */
const RAIL_NODES_PER_TILE = 200;
const RAIL_NODES_MIN = 2000;
const RAIL_NODES_MAX = 150000;
/* Nodes the last search that found its line used (FindRailPath); BuildDoubleRailLineAndTrain sizes
 * the second track's search from the first's. */
RAIL_LAST_NODES <- 0;

function PrismaticAI::FindRailPath(from, to, from_end, to_end, avoid = null, from_in = null, from_straight = false, to_straight = false, max_nodes = 0) {
	local map_w = AIMap.GetMapSizeX();
	local day0 = AIDate.GetCurrentDate();
	local open = AIPriorityQueue();
	local came = {}, cost = {};
	/* A bridge is a jump in the path: its far head `h2` came from the tile before the near head, and
	 * bridge_from[h2] is the near head, bridge_dir[h2] the direction the line keeps beyond it. */
	local bridge_from = {}, bridge_dir = {};
	cost[from] <- 0;
	came[from] <- -1;
	open.Insert(from, H_TILE * AIMap.DistanceManhattan(from, to));
	local budget = (max_nodes > 0) ? max_nodes : RAIL_NODES_PER_TILE * AIMap.DistanceManhattan(from, to);
	budget = min(RAIL_NODES_MAX, max(RAIL_NODES_MIN, budget));
	local budget0 = budget;
	while (!open.IsEmpty() && budget-- > 0) {
		/* A search can take months: the maintenance tick keeps the lines served meanwhile (steward.nut),
		 * and the growth funnel closes a game year that ends inside it (funnel.nut FunnelRoll). Without
		 * the roll, a heartbeat whose searches ran from 1962 to 1964 on the fitness seed (a 258-tile
		 * double track, three searches of 80-96 days each) left 1963 with no FUNNEL line, and the
		 * fitness check failed `no_funnel`. */
		if (budget % MAINT_YIELD_NODES == 0) {
			PrismaticAI.MaintenanceYield();
			PrismaticAI.FunnelRoll();
		}
		local cur = open.Pop();
		if (cur == to) {
			RAIL_LAST_NODES = budget0 - budget;
			AILog.Info("PrismaticAI: rail path from tile " + from + " to " + to + " found: " + RAIL_LAST_NODES + " of " + budget0 + " nodes in " +
				(AIDate.GetCurrentDate() - day0) + " days");
			local path = [];
			for (local t = to; t != -1; t = came[t]) {
				path.push(t);
				if (bridge_from.rawin(t)) path.push(bridge_from[t]);
			}
			path.reverse();
			return path;
		}
		local prev = came[cur] == -1 ? from_end : came[cur];
		/* The direction the line arrives in: one step, or along the bridge it just left. */
		local in_dir = bridge_dir.rawin(cur) ? bridge_dir[cur] : (cur - prev);
		/* The direction it arrived at `prev` in; equal to in_dir on straight track, at a bridge or at
		 * the start. A corner on `cur` that turns back to it continues a diagonal. */
		local before = in_dir;
		if (!bridge_dir.rawin(cur)) {
			if (came[cur] == -1) {
				/* At `from`: the move into from_end, straight out of a platform unless a re-plan says. */
				if (from_in != null) before = from_in;
			} else if (came.rawin(prev) && came[prev] == -1) {
				/* One beyond `from`: the move into `from` came from from_end. Without this the corner on
				 * `from` was never compared with the next one, and a line could start with a 90-degree
				 * turn no train may take (forbid_90_deg). */
				before = prev - from_end;
			} else if (came.rawin(prev) && !bridge_dir.rawin(prev)) {
				before = prev - came[prev];
			}
		}
		/* A bridge across what blocks the line two tiles ahead (water, a road, rail, a valley): the
		 * near head is the next tile, the far head the first land 3 to 11 tiles on whose bridge the
		 * game accepts (AITestMode; slopes, clearance and length are the game's own rules). */
		local ahead = cur + in_dir;
		if (ahead != to && PrismaticAI.RailStepCost(ahead) >= 0 &&
				PrismaticAI.RailStepCost(ahead + in_dir) < 0 && !PrismaticAI.CanCross(ahead + in_dir, in_dir) && ahead + in_dir != to) {
			for (local k = 2; k <= 10; k++) {
				local h2 = ahead + k * in_dir;
				if (!AIMap.IsValidTile(h2) || h2 == to || PrismaticAI.RailStepCost(h2) < 0) continue;
				if (AIMap.DistanceManhattan(ahead, h2) != k) break; /* wrapped round the map edge */
				local bridge = PrismaticAI.BridgeType(ahead, h2);
				if (bridge == -1) continue;
				local ok = false;
				{
					/* The probe's own accounting. The game adds a test-mode estimate to the costs an
					 * AIAccounting counts (ScriptObject::DoCommand), and a nested one restores its
					 * caller's count on exit. Without it every probe of a re-plan while track A was
					 * laid counted as track A's cost: on the fitness seed 930967890 a 229-tile line
					 * "cost about 1630003" with funds of 157584, so the second track was refused and
					 * the line ran single track with two trains. */
					local probe = AIAccounting();
					local test = AITestMode();
					ok = AIBridge.BuildBridge(AIVehicle.VT_RAIL, bridge, ahead, h2);
				}
				if (!ok) continue;
				local c = cost[cur] + 3 * (3 + 8 * (k + 1));
				if (!cost.rawin(h2) || c < cost[h2]) {
					cost[h2] <- c;
					came[h2] <- cur;
					bridge_from[h2] <- ahead;
					bridge_dir[h2] <- in_dir;
					open.Insert(h2, c + H_TILE * AIMap.DistanceManhattan(h2, to));
				}
				break;
			}
		}
		/* A tunnel through the hill ahead: the next tile is a slope rising in the line's direction,
		 * the game finds the far portal (AITunnel.GetOtherTunnelEnd), and the tunnel is taken when it
		 * runs straight on, is at most 15 tiles, the line can go on beyond it, and the game accepts it
		 * (AITestMode). Like a bridge, its two portals sit in a row in the path. */
		if (ahead != to && PrismaticAI.RailStepCost(ahead) >= 0 && AITile.GetSlope(ahead) != AITile.SLOPE_FLAT) {
			local far = AITunnel.GetOtherTunnelEnd(ahead);
			local k = AIMap.IsValidTile(far) ? AIMap.DistanceManhattan(ahead, far) : 0;
			if (k >= 2 && k <= 15 && far == ahead + k * in_dir && far != to &&
					(PrismaticAI.RailStepCost(far + in_dir) >= 0 || far + in_dir == to)) {
				local ok = false;
				{
					local probe = AIAccounting();   /* as the bridge probe above */
					local test = AITestMode();
					ok = AITunnel.BuildTunnel(AIVehicle.VT_RAIL, ahead);
				}
				if (ok) {
					local c = cost[cur] + 3 * (3 + 10 * (k + 1));
					if (!cost.rawin(far) || c < cost[far]) {
						cost[far] <- c;
						came[far] <- cur;
						bridge_from[far] <- ahead;
						bridge_dir[far] <- in_dir;
						open.Insert(far, c + H_TILE * AIMap.DistanceManhattan(far, to));
					}
				}
			}
		}
		foreach (next in [cur - 1, cur + 1, cur - map_w, cur + map_w]) {
			if (next == prev) continue;
			if (avoid != null && avoid.rawin(next)) continue;
			/* Straight through a throat's own exit tile, and straight into the far one. */
			if (from_straight && came[cur] == -1 && (next - cur) != in_dir) continue;
			if (to_straight && next == to && (next - cur) != (to_end - to)) continue;
			/* A bridge's far head lets the line go on straight only. */
			if (bridge_dir.rawin(cur) && (next - cur) != in_dir) continue;
			local step = (next == to) ? 1 : PrismaticAI.RailStepCost(next);
			if (step < 0) {
				/* A level crossing: a straight road at right angles, on flat land (CanCross). It costs
				 * more than land, so the line crosses roads only where it must (a station walled in
				 * by its town), and the line passes it straight (no turn on the crossing below). */
				if (!PrismaticAI.CanCross(next, next - cur)) continue;
				step = 12;
			}
			/* No turn on a level crossing: its rail runs straight across the road. */
			if ((next - cur) != in_dir && AIRoad.IsRoadTile(cur)) continue;
			local diagonal = false;
			if ((next - cur) != in_dir) {
				/* A turn: the corner piece on `cur` must fit its slope (PieceFits). Allowing turns
				 * only on flat land left hilly station exits unreachable, and allowing them on any
				 * slope that is not steep built pieces the game refuses (ERR_LAND_SLOPED_WRONG): a
				 * 180-tile coal line failed both ways (OpenTTD research/results duel seed 20208). */
				local s = AITile.GetSlope(cur);
				if (!PrismaticAI.PieceFits(s, PrismaticAI.PieceBetween(cur, prev, next))) continue;
				if (PrismaticAI.Turns90(before, in_dir, next - cur)) continue; /* 90 degrees: two corners turning one way */
				diagonal = before != in_dir && (next - cur) == before;
				/* Entering a turn pays for it; going on along a diagonal does not. */
				if (!diagonal) step += (s == AITile.SLOPE_FLAT) ? 2 : 6;
			}
			/* The piece on the far exit joins this line to the far station. */
			if (next == to && !PrismaticAI.PieceFits(AITile.GetSlope(to), PrismaticAI.PieceBetween(to, cur, to_end))) continue;
			/* Nor a 90-degree turn into the far throat: the corner on `cur` and the one on `to`. */
			if (next == to && PrismaticAI.Turns90(in_dir, next - cur, to_end - to)) continue;
			/* A corner piece on a diagonal is two thirds of a straight one to travel. */
			local c = cost[cur] + (diagonal ? 2 * step : 3 * step);
			if (!cost.rawin(next) || c < cost[next]) {
				cost[next] <- c;
				came[next] <- cur;
				/* Reached on foot now, not over a bridge. */
				if (bridge_from.rawin(next)) { delete bridge_from[next]; delete bridge_dir[next]; }
				/* Weighted A* (H_TILE a tile): far fewer nodes than plain A*, the route in practice a
				 * few percent longer; the search no longer eats months of game time. */
				open.Insert(next, c + H_TILE * AIMap.DistanceManhattan(next, to));
			}
		}
	}
	AILog.Warning("PrismaticAI: no rail path from tile " + from + " to " + to + " (" + AIMap.DistanceManhattan(from, to) + " tiles): "
		+ (budget <= 0 ? "the node budget of " + budget0 + " ran out" : "every reachable tile tried, " + (budget0 - budget) + " nodes")
		+ " in " + (AIDate.GetCurrentDate() - day0) + " days");
	return null;
}

/* 1 when every tile of the w x h rectangle at `t` is flat, else 0 (a valuator for AITileList). */
function PrismaticAI::RectFlat(t, w, h) {
	local mw = AIMap.GetMapSizeX();
	for (local y = 0; y < h; y++) {
		for (local x = 0; x < w; x++) {
			if (AITile.GetSlope(t + y * mw + x) != AITile.SLOPE_FLAT) return 0;
		}
	}
	return 1;
}

/* The company's own train station nearest tile `origin`, at most `reach` tiles away, or null. */
function PrismaticAI::OwnRailStationNear(origin, reach) {
	local list = AIStationList(AIStation.STATION_TRAIN);
	list.Valuate(AIStation.GetDistanceManhattanToTile, origin);
	list.KeepBelowValue(reach + 1);
	list.Sort(AIList.SORT_BY_VALUE, AIList.SORT_ASCENDING);
	return list.IsEmpty() ? null : list.Begin();
}

/* How long a foreign rail station tile claims the producer it catches, in days from when this
 * company first saw the tile (ClaimFresh). The window has to outlast a line still being built: from
 * the planner's choice to the first train, PrismaticAI's lines took 0 to 18 months, most within 12
 * (quarter resolution, the referee's REF Q lines; the fitness seed's two sibling matches and
 * 10-year matches on seeds 15805 and 2927, 2026-11-23), most of it waiting for money (RailCashWait)
 * and rebuilding a double track as single track. A longer window keeps a producer from every
 * company for longer behind a station that only catches it, and the same window holds for a
 * processing industry, where a station delivering there also catches it (review of 2026-11-23). */
const CLAIM_FRESH_DAYS = 365;
/* How long check 2 of BuildFreightRailRoute waits before it yields to a company of a higher id
 * (ClaimBlocks): longer than that company takes to remove a source station it gave up. Removing one
 * is a few commands; AIs run every 2^(4 - competitor_speed) ticks, and a network game adds frame lag. */
const CLAIM_SETTLE_DAYS = 2;

/* Foreign rail station tiles near producers: tile -> [owner, station id, date first seen], saved
 * and restored (restore.nut), so a load does not make every station fresh again for a year. */
CLAIM_SEEN <- {};
/* The producers whose release ForeignRailStationAt has logged, so each is said once. */
CLAIM_LAPSED <- {};
/* Tiles of foreign stations the last ForeignRailStationAt found near its producer, none claiming it. */
CLAIM_LAST_LAPSED <- 0;

/* Whether the foreign rail station tile `t` of company `o` is fresh: first seen by this company
 * less than `days` ago. The first sight is per tile and per station, so a platform joined to an old
 * station, or a station rebuilt on a tile (after a yield or a bankruptcy), is fresh again. */
function PrismaticAI::ClaimFresh(t, o, days) {
	local sid = AIStation.GetStationID(t), now = AIDate.GetCurrentDate();
	local seen = CLAIM_SEEN.rawin(t) ? CLAIM_SEEN[t] : null;
	if (seen == null || seen[0] != o || seen[1] != sid) {
		seen = [o, sid, now];
		CLAIM_SEEN[t] <- seen;
	}
	return now - seen[2] < days;
}

/* After a restart (restore.nut Restore, from a save or from the map): every foreign rail station
 * tile near a producer that the first-seen table does not hold is taken as seen CLAIM_FRESH_DAYS ago,
 * so it claims nothing. A load restarts every AI of the game, so no company is building a line across
 * it; a station of unknown age that the table lacks (a save from before the table, or one never
 * scanned) would otherwise hold its producer for a year after every deploy into a running game.
 * Returns how many tiles it added. */
function PrismaticAI::ClaimsAfterRestart() {
	local me = AICompany.ResolveCompanyID(AICompany.COMPANY_SELF);
	local old = AIDate.GetCurrentDate() - CLAIM_FRESH_DAYS, n = 0;
	local radius = AIStation.GetCoverageRadius(AIStation.STATION_TRAIN);
	foreach (p, _ in AIIndustryList()) {
		local tiles = AITileList_IndustryProducing(p, radius);
		tiles.Valuate(AIRail.IsRailStationTile);
		tiles.KeepValue(1);
		foreach (t, _ in tiles) {
			local o = AITile.GetOwner(t);
			if (o == me || o == AICompany.COMPANY_INVALID || CLAIM_SEEN.rawin(t)) continue;
			CLAIM_SEEN[t] <- [o, AIStation.GetStationID(t), old];
			n++;
		}
	}
	return n;
}

/* Drops the first sights whose tile no longer holds the station they were taken of, so a station
 * later built there is fresh, and the table holds only standing stations. */
function PrismaticAI::ForgetGoneClaims() {
	local gone = [];
	foreach (t, seen in CLAIM_SEEN) {
		if (!AIRail.IsRailStationTile(t) || AITile.GetOwner(t) != seen[0] || AIStation.GetStationID(t) != seen[1]) gone.push(t);
	}
	foreach (t in gone) delete CLAIM_SEEN[t];
}

/* Another company whose rail station claims industry `p`, the lowest such company id, or null. A
 * source station is the claim on a producer: two lines at one mine split its output by station
 * rating, so the second line pays its full cost for about half the cargo. In the owner's live game
 * of 2026-11-22 four PrismaticAI companies of one map chose their lines with the same deterministic
 * score, and companies 0 and 1, 6 and 7, 1 and 6 each built a line from the same mine or forest: all
 * four stopped growing in 1952 while AAAHogEx reached 2.2M
 * (research/incident-2026-11-22-sibling-industries.md). Any owner counts, a sibling or a rival; a
 * neutral station (COMPANY_INVALID, an oil rig's) never does. Raw and processing industries alike.
 *
 * A claim is a fresh station: a foreign rail station tile catching `p` that this company first saw
 * less than CLAIM_FRESH_DAYS ago (ClaimFresh). That is the line still being built or waiting for its
 * first train, when nothing shows that it will pick up. Once a rival's station is older, what the
 * industry transports decides, not the station: NextOwnFreightLine skips a producer whose output
 * moved last month, and in rich mode (rich.nut) scores it on the share left below RICH_SERVED_BELOW.
 * A sibling's (IsSibling) older station still claims while cargo moves from `p`: in rich mode the
 * fitness check's sibling lineup (seed 31337) otherwise built a second coal line at Pruston Coal Mine
 * beside its sibling's, whose station rated the coal 49%. Before, any
 * foreign rail station near a raw industry claimed it for good, so a rival's working line held a
 * producer from rich mode too: in the owner's live game of 2026-11-23 the bot reported 19 to 21
 * producers "claimed" and 0 pairs tried from 1952-10 on, still in rich mode from 1954-03.
 *
 * Why the station alone cannot decide: one that catches a producer does not have to pick up from
 * it. A rival's passenger station in a town beside a mine, or a station delivering to a power
 * station next to one, catches the mine and takes nothing (with order.selectgoods an industry gives
 * cargo only to a station where a vehicle tried to load it). An AI cannot ask: a foreign station's
 * ratings, waiting cargo, vehicles, name and construction date are refused (AIStation.IsValidStation
 * is false for it), and the cargo monitor is for Game Scripts only. Its acceptance does not decide
 * either: a pickup station at a mine inside a town accepts passengers too. The one trace of a
 * pickup is the producer's transported amount (AIIndustry.GetLastMonthTransported), for the whole
 * industry and a month late.
 *
 * With `fresh_only` (BuildFreightRailRoute's checks after its stations) no release is logged. */
function PrismaticAI::ForeignRailStationAt(p, fresh_only = false) {
	local me = AICompany.ResolveCompanyID(AICompany.COMPANY_SELF);
	local tiles = AITileList_IndustryProducing(p, AIStation.GetCoverageRadius(AIStation.STATION_TRAIN));
	tiles.Valuate(AIRail.IsRailStationTile);
	tiles.KeepValue(1);
	/* The lowest claiming company id, so two companies that claim one producer at once settle it the
	 * same way (BuildFreightRailRoute: the higher id yields). */
	local holder = null, foreign = 0, moved = null;
	foreach (t, _ in tiles) {
		local o = AITile.GetOwner(t);
		if (o == me || o == AICompany.COMPANY_INVALID) continue;
		foreign++;
		local claims = PrismaticAI.ClaimFresh(t, o, CLAIM_FRESH_DAYS);
		if (!claims && !fresh_only && PrismaticAI.IsSibling(o)) {
			if (moved == null) moved = PrismaticAI.CargoMoved(p);
			claims = moved;
		}
		if (claims && (holder == null || o < holder)) holder = o;
	}
	CLAIM_LAST_LAPSED = (holder == null) ? foreign : 0;
	if (holder == null && foreign > 0 && !fresh_only && !CLAIM_LAPSED.rawin(p)) {
		CLAIM_LAPSED[p] <- true;
		if (moved == null) moved = PrismaticAI.CargoMoved(p);
		AILog.Info("PrismaticAI: no claim on " + AIIndustry.GetName(p) + ": " + foreign +
			" tile(s) of other companies' rail stations, all seen over " + CLAIM_FRESH_DAYS + " days ago; cargo moved last month: " +
			(moved ? "yes (scored by the share left)" : "no"));
	}
	return holder;
}

/* Whether any cargo moved from industry `p` last month, whoever moved it. */
function PrismaticAI::CargoMoved(p) {
	foreach (c, _ in AICargoList_IndustryProducing(p)) if (AIIndustry.GetLastMonthTransported(p, c) > 0) return true;
	return false;
}

/* Auto-renew as PrismaticAI sets it at every start (main.nut: SetAutoRenewMonths, SetAutoRenewMoney):
 * the mark of a sibling. test_ai_battle.py SiblingMark keeps the two in step. */
const SIBLING_RENEW_MONTHS = -6;
const SIBLING_RENEW_MONEY = 30000;

/* Whether company `o` runs PrismaticAI too, as far as an AI can tell. A company's name is the game's
 * own (disclose_name is off in battles and in the owner's games), but any company's auto-renew
 * settings are readable (AICompany.GetAutoRenewMonths, GetAutoRenewMoney), and PrismaticAI sets
 * them to a pair other than the game's defaults (6 months, 100000). A rival that happens to use the
 * same pair is taken for a sibling, which only keeps its working line from a second line of ours, as
 * master did for every station. */
function PrismaticAI::IsSibling(o) {
	return AICompany.GetAutoRenewStatus(o) && AICompany.GetAutoRenewMonths(o) == SIBLING_RENEW_MONTHS &&
		AICompany.GetAutoRenewMoney(o) == SIBLING_RENEW_MONEY;
}

/* Check 2 of BuildFreightRailRoute, after both stations of a line from producer `p` stand: whether
 * the line yields to a claim made in the same days. A lower company id wins at once, as at check 1.
 * A higher one may be removing a station it gave up at its own check 1, so it is read again after
 * CLAIM_SETTLE_DAYS, and the line yields only when it still stands: then that company kept its
 * station, because its check 1 came before ours was built. */
function PrismaticAI::ClaimBlocks(p, label) {
	local me = AICompany.ResolveCompanyID(AICompany.COMPANY_SELF);
	local rival = PrismaticAI.ForeignRailStationAt(p, true);
	if (rival == null) return false;
	if (rival > me) {
		AILog.Info("PrismaticAI: company " + (rival + 1) + " built a rail station at " + AIIndustry.GetName(p) +
			" in the same days; " + label + " checks again in " + CLAIM_SETTLE_DAYS + " days");
		AIController.Sleep(CLAIM_SETTLE_DAYS * 74);
		rival = PrismaticAI.ForeignRailStationAt(p, true);
		if (rival == null) {
			AILog.Info("PrismaticAI: the other claim on " + AIIndustry.GetName(p) + " is gone; " + label + " goes on");
			return false;
		}
	}
	AILog.Warning("PrismaticAI: freight line " + label + " given up at its second station: company " + (rival + 1) +
		" claimed " + AIIndustry.GetName(p) + " in the same days");
	return true;
}

/* test_claim_race (RESEARCH-ONLY, info.nut): the company waits that many days after the check before
 * its first source station, once a game, so a sibling that chose the same producer builds and checks
 * its station first: the order only check 2 (ClaimBlocks) settles. */
TEST_CLAIM_RACE_DONE <- false;
function PrismaticAI::TestClaimRace() {
	local days = AIController.GetSetting("test_claim_race");
	if (days <= 0 || TEST_CLAIM_RACE_DONE) return;
	TEST_CLAIM_RACE_DONE = true;
	AILog.Info("PrismaticAI: test_claim_race: waiting " + days + " days before the first source station");
	AIController.Sleep(days * 74);
}

/* The bounding box of a station's rail tiles: [x0, y0, x1, y1]. */
function PrismaticAI::RailStationBox(st) {
	local b = [AIMap.GetMapSizeX(), AIMap.GetMapSizeY(), 0, 0];
	foreach (t, _ in AITileList_StationType(st, AIStation.STATION_TRAIN)) {
		local x = AIMap.GetTileX(t), y = AIMap.GetTileY(t);
		b[0] = min(b[0], x); b[1] = min(b[1], y); b[2] = max(b[2], x); b[3] = max(b[3], y);
	}
	return b;
}

/* 1 when a w x h platform at tile `t` and the station box `b` together span at most `spread`
 * tiles each way (station.station_spread), so the platform can join that station. */
function PrismaticAI::FitsSpread(t, w, h, b, spread) {
	local x = AIMap.GetTileX(t), y = AIMap.GetTileY(t);
	return (max(b[2], x + w - 1) - min(b[0], x) < spread && max(b[3], y + h - 1) - min(b[1], y) < spread) ? 1 : 0;
}

/* How a w x h platform at `t` sits against a station box `b` = [x0, y0, x1, y1]: 0 when it lies right
 * beside the station's platforms, parallel to them (the same axis, touching their long side, its span
 * overlapping theirs), so the two form one station with side-by-side platforms; otherwise 100 plus its
 * distance to the box, so a scattered platform is the last resort. */
function PrismaticAI::JoinRank(t, w, h, b) {
	local x = AIMap.GetTileX(t), y = AIMap.GetTileY(t);
	local box_along_x = (b[2] - b[0]) >= (b[3] - b[1]);
	local along_x = w >= h;
	if (along_x == box_along_x) {
		if (along_x && (y == b[1] - 1 || y == b[3] + 1) && x <= b[2] && x + w - 1 >= b[0]) return 0;
		if (!along_x && (x == b[0] - 1 || x == b[2] + 1) && y <= b[3] && y + h - 1 >= b[1]) return 0;
	}
	local dx = max(0, max(b[0] - (x + w - 1), x - b[2])), dy = max(0, max(b[1] - (y + h - 1), y - b[3]));
	return 100 + dx + dy;
}

/* The station a new platform near `origin` joins: the company's own train station there, when the
 * game allows a distant join; null builds a new one. Two lines to one power station share one
 * station with two platforms: on seed 31337 the second coal line built "Slonham Transfer" four
 * tiles from the first line's "Slonham East", and the duplicate audit flagged the pair. */
function PrismaticAI::RailStationToJoin(origin, len) {
	local name = "station.distant_join_stations";
	if (AIGameSettings.IsValid(name) && AIGameSettings.GetValue(name) == 0) return null;
	return PrismaticAI.OwnRailStationNear(origin, 10 + len);
}

/* A terminus near tile `origin` for a line to tile `other`: `platforms` platforms of `len` tiles side
 * by side and, with more than one, a two-tile throat that merges them into one line.
 * `site` says what makes a good site: "accept" (cargo accepted, e.g. passengers in a town or coal at
 * a power station) or "produce" (cargo produced into the catchment, at a mine).
 *
 * Geometry, along the move `out` from the platforms' ends toward the line (one platform is the main
 * one, the other lies `sd` beside it):
 *
 *     platform main  ... end_m | T1m  M   L0 -> the line
 *     platform 2nd   ... end_s | T1s  M2
 *
 * T1m and T1s are straight pieces with a path signal facing out (a train leaves its platform only
 * when it can reserve its whole way into a free platform at the other end); M2 bends the second
 * platform onto M, where it joins the main one; L0 (`exit`) is the first tile of the line and M
 * (`end`) the tile the line search starts from. The bend is a diagonal, never 90 degrees. Without a
 * throat (one platform) `end` is the platform's end and `exit` the tile beyond it.
 *
 * Where it stands (StationOrientations, StationSurvey): the axis pointing at `other` first, then
 * the other axis (its exit toward `other`'s side first), then the first axis facing away. Each
 * orientation offers the buildable rectangles whose platforms catch the cargo, most cargo first:
 * flat ones, then, where construction.build_on_slopes allows it, sloped ones the game builds on
 * foundations (FitsOnFoundations, then RailStationFits in test mode). Only when no orientation has
 * such a site is one levelled (LevelRailStationSite). Sites used to be flat and on the first axis
 * only: in the owner's hilly 1950 settings (seeds 15805, 2927, 31337, 4242; 2026-11-22) 14 of 23
 * "no rail station site" had one here, on foundations on that axis (Findingville Coal Mine 7, seed
 * 2927) or on the other axis (Chontbourne Power Station 44 flat, seed 2927; Drinbourne Oil Refinery
 * 5 on foundations, seed 31337). Levelling was possible at 6 and refused each time for the
 * RAIL_LEVEL_KEEP margin (funds 35k to 64k after the source station). The other 9 had no buildable
 * catching rectangle with an exit in any orientation.
 *
 * Returns {tile, end, exit, step, platforms, len, platform_rect, throat, backs} or null:
 * `platform_rect` is this station's own platforms ([corner, opposite corner]), `throat` the plain
 * track of its throat (RemoveRailStation removes both and nothing else), `backs` the [depot tile,
 * platform tile] pairs behind each platform's
 * dead end (BuildRailDepotBehind). A company train station already near `origin` gets these
 * platforms (RailStationToJoin) wherever a site fits the station spread, in any orientation; only
 * when none does is a separate station built, with a warning. */
function PrismaticAI::BuildRailStationAt(origin, other, len, cargo, site, label, platforms = 1, hand = 0) {
	if (PrismaticAI.NoRailSiteKnown(origin, site, cargo, len, platforms, hand, label)) return null;
	local join = PrismaticAI.RailStationToJoin(origin, len);
	local box = (join == null) ? null : PrismaticAI.RailStationBox(join);
	local slopes = PrismaticAI.BuildsOnSlopes();
	local orients = PrismaticAI.StationOrientations(origin, other);
	local hows = ["", ", on the other axis", ", on the other axis", ", its exit reversed"];
	local surveys = [];
	local budget = {tried = 0, probes = 0, levelled = 0, blocked = 0, dear = 0, money = 0};
	/* Joining the station already there comes first, in every orientation; a new station next. */
	local passes = [];
	if (join != null) foreach (k, _ in orients) passes.append([k, join]);
	foreach (k, _ in orients) passes.append([k, AIStation.STATION_NEW]);
	local warned = false;
	foreach (pass in passes) {
		local k = pass[0], into = pass[1];
		while (surveys.len() <= k) {
			local o = orients[surveys.len()];
			local shape = PrismaticAI.StationOrientation(o[0], o[1], len, platforms, hand);
			shape.how <- hows[surveys.len()];
			surveys.append(PrismaticAI.StationSurvey(origin, shape, cargo, site, slopes, box));
		}
		local s = surveys[k];
		if (into == AIStation.STATION_NEW && join != null && !warned && s.flat.Count() + s.slope.Count() > 0) {
			warned = true;
			AILog.Warning("PrismaticAI: no site " + label + " joins " + AIStation.GetName(join) + "; a separate station");
		}
		local lists = (into == AIStation.STATION_NEW) ? [s.flat, s.slope] : [s.near_flat, s.near_slope];
		foreach (i, list in lists) {
			local made = PrismaticAI.TryRailStationSites(list, s.o, into, len, platforms, label, i == 1, budget, box);
			if (made != null) return made;
		}
	}
	foreach (s in surveys) {
		local made = PrismaticAI.LevelRailStationSite(s, len, platforms, label, budget);
		if (made != null) return made;
	}
	local counts = "";
	foreach (s in surveys) {
		counts += (counts == "" ? "" : ", ") + s.o.name + " " + (s.flat.Count() + s.near_flat.Count()) + "/" +
			(s.slope.Count() + s.near_slope.Count()) + "/" + s.level.Count();
	}
	AILog.Warning("PrismaticAI: no rail station site " + label + " (flat/foundation/levellable sites by orientation: " + counts +
		"; " + budget.tried + " built and refused, " + budget.probes + " refused before building, levelling within " + PrismaticAI.RailLevelStep() +
		" height step(s): " + budget.levelled + " levelled, " + budget.blocked + " blocked by a tile that does not move, " +
		budget.dear + " over " + PrismaticAI.RailLevelMax() + ", " + budget.money + " short of money)");
	PrismaticAI.RememberNoRailSite(origin, site, cargo, len, platforms, hand, budget.money > 0 ? NO_RAIL_SITE_MONEY_DAYS : NO_RAIL_SITE_DAYS, join != null);
	return null;
}

/* Termini that found no site (BuildRailStationAt), so the next line to or from the same place does
 * not survey it again: NoRailSiteKey -> the date until which it is believed. Without it, on seed
 * 4242 Cheningpool Steel Mill was surveyed 5 times in 13 game days, as the acceptor of three mines'
 * iron ore (each after the mine's station was built, and then removed) and as the producer of
 * steel for three factories. Keyed by the origin tile and the terminus's shape, not by the
 * industry id (ids are reused when an industry closes), and with the platform length exact (a
 * longer platform catches more, a shorter one fits more land). A record made while no station of
 * ours stood there to join no longer holds once one does (RailStationToJoin): joining it opens
 * other sites. Saved (restore.nut). */
NO_RAIL_SITE <- {};
/* How long a site search that found no land is believed: towns grow and land changes. Each
 * record is {until = date, join = whether a station to join stood there, any = its
 * NO_RAIL_SITE_ANY key}. */
NO_RAIL_SITE_DAYS <- 730;
/* ... and one that passed over a levellable site only for money, which comes in faster. */
NO_RAIL_SITE_MONEY_DAYS <- 30;

function PrismaticAI::NoRailSiteKey(origin, site, cargo, len, platforms, hand) {
	return origin + "-" + site + "-" + cargo + "-" + len + "-" + platforms + "-" + hand;
}

function PrismaticAI::RememberNoRailSite(origin, site, cargo, len, platforms, hand, days, joined) {
	local until = AIDate.GetCurrentDate() + days, any = origin + "-" + site + "-" + cargo;
	NO_RAIL_SITE[PrismaticAI.NoRailSiteKey(origin, site, cargo, len, platforms, hand)] <- {until = until, join = joined, any = any};
	PrismaticAI.NoRailSiteIndexAdd(any, until);
}

/* NO_RAIL_SITE by origin tile, site and cargo alone ("<tile>-<site>-<cargo>" -> the latest date a
 * record of it holds until), for the own-line search, which does not know a line's platform length
 * yet (BuildFreightRailRoute plans it). Rebuilt after a load (NoRailSiteReindex). */
NO_RAIL_SITE_ANY <- {};

function PrismaticAI::NoRailSiteIndexAdd(key, until) {
	if (!NO_RAIL_SITE_ANY.rawin(key) || NO_RAIL_SITE_ANY[key] < until) NO_RAIL_SITE_ANY[key] <- until;
}

function PrismaticAI::NoRailSiteReindex() {
	NO_RAIL_SITE_ANY.clear();
	foreach (k, rec in NO_RAIL_SITE) {
		if (typeof rec == "table" && ("until" in rec) && ("any" in rec)) PrismaticAI.NoRailSiteIndexAdd(rec.any, rec.until);
	}
}

/* Whether a terminus of any shape at `origin` for `cargo` as `site` is remembered without a site:
 * the own-line search (NextOwnFreightLine) then passes the industry over, so a failed try does not
 * use up own_fail_pause and the next best pair is chosen. In the owner's live game of seed 21647
 * (1992) the own plan picked five factories in a row with no buildable catching site at all
 * (0/0/0 in every orientation), each after building and removing the farm's station. Coarser than
 * NoRailSiteKnown (any platform length), and it expires the same way. */
function PrismaticAI::NoRailSiteFor(origin, site, cargo) {
	local key = origin + "-" + site + "-" + cargo;
	if (!NO_RAIL_SITE_ANY.rawin(key)) return false;
	if (NO_RAIL_SITE_ANY[key] > AIDate.GetCurrentDate()) return true;
	delete NO_RAIL_SITE_ANY[key];
	return false;
}

/* Whether the terminus asked for is remembered without a site (NO_RAIL_SITE); says so when it is.
 * An expired record is dropped here, and a hit never extends it. */
function PrismaticAI::NoRailSiteKnown(origin, site, cargo, len, platforms, hand, label) {
	local key = PrismaticAI.NoRailSiteKey(origin, site, cargo, len, platforms, hand);
	if (!NO_RAIL_SITE.rawin(key)) return false;
	local rec = NO_RAIL_SITE[key];
	if (typeof rec != "table" || !("until" in rec) || !("join" in rec) || rec.until <= AIDate.GetCurrentDate() ||
			(!rec.join && PrismaticAI.RailStationToJoin(origin, len) != null)) {
		delete NO_RAIL_SITE[key];
		return false;
	}
	local until = rec.until;
	AILog.Warning("PrismaticAI: no rail station site " + label + " (remembered until " + AIDate.GetYear(until) + "-" + AIDate.GetMonth(until) + "-" + AIDate.GetDayOfMonth(until) + ")");
	return true;
}

/* NO_RAIL_SITE without its expired records, at most NO_RAIL_SITE_SAVE_MAX of them, for a save. */
NO_RAIL_SITE_SAVE_MAX <- 200;
function PrismaticAI::NoRailSiteSave() {
	local now = AIDate.GetCurrentDate(), out = {};
	foreach (k, rec in NO_RAIL_SITE) {
		if (out.len() >= NO_RAIL_SITE_SAVE_MAX) break;
		if (typeof rec == "table" && ("until" in rec) && rec.until > now) out[k] <- rec;
	}
	return out;
}

/* Whether the game builds rail stations and track on foundations (construction.build_on_slopes, on
 * by default; a game without the setting is read as on). */
function PrismaticAI::BuildsOnSlopes() {
	local name = "construction.build_on_slopes";
	return !AIGameSettings.IsValid(name) || AIGameSettings.GetValue(name) != 0;
}

/* The orientations a terminus near `origin` is tried in, as [along_x, forward] pairs: the axis that
 * points at `other` with the exit toward it, the other axis with its exit toward `other`'s side and
 * then away from it, and last the first axis with its exit away from `other` (the line turns round
 * the station). */
function PrismaticAI::StationOrientations(origin, other) {
	local dx = AIMap.GetTileX(other) - AIMap.GetTileX(origin), dy = AIMap.GetTileY(other) - AIMap.GetTileY(origin);
	local along_x = abs(dx) >= abs(dy);
	local forward = along_x ? (dx >= 0) : (dy >= 0);
	local cross = along_x ? (dy >= 0) : (dx >= 0);
	return [[along_x, forward], [!along_x, cross], [!along_x, !cross], [along_x, !forward]];
}

/* The shape of a terminus of `platforms` platforms of `len` tiles in one orientation (`along_x`, the
 * exit on the + side when `forward`): the move along a platform (`step`), to the next platform
 * (`side`), toward the line (`out`), the double track's lane, the throat's length and the site's
 * rectangle (w x h), and where in it the platforms lie (sw x sh at `offset`). */
function PrismaticAI::StationOrientation(along_x, forward, len, platforms, hand) {
	local map_w = AIMap.GetMapSizeX();
	local step = along_x ? 1 : map_w;          /* along a platform */
	local side = along_x ? map_w : 1;          /* to the next platform: AIRail.BuildRailStation stacks them so */
	local out = forward ? step : -step;        /* from the platforms' ends toward the line */
	/* A double-track terminus (hand +1 at the source, -1 at the far end; DoubleTerminus) has two
	 * more throat tiles, L0 and the junction A1, where the two one-way tracks meet. The lane turns
	 * with `out`, so the second track leaves on the same side of the line in every orientation. */
	local throat = (hand != 0) ? 4 : ((platforms > 1) ? 2 : 0);
	return {along_x = along_x, forward = forward, name = (along_x ? "x" : "y") + (forward ? "+" : "-"),
		axis = along_x ? AIRail.RAILTRACK_NE_SW : AIRail.RAILTRACK_NW_SE, step = step, side = side, out = out,
		lane = hand * PrismaticAI.QuarterTurn(out), throat = throat,
		w = along_x ? len + throat : platforms, h = along_x ? platforms : len + throat,
		sw = along_x ? len : platforms, sh = along_x ? platforms : len, offset = forward ? 0 : throat * step};
}

/* The sites of a terminus in orientation `o` within 10 tiles of `origin`: buildable rectangles of
 * the platforms and the throat whose platforms catch the cargo (the throat is plain track: an
 * industry beside the throat but out of the platforms' reach would pass otherwise), split into
 * `flat`, `slope` (sloped, the station fits on foundations: FitsOnFoundations; empty when `slopes`
 * is off) and, when a station box `box` is given, the `near_flat` and `near_slope` ones within the
 * station spread of it, side by side first (JoinRank). Each list is sorted by the cargo caught, most
 * first. `level` holds the sloped sites whose corners span at most RailLevelStep() height steps,
 * lowest span first (LevelRailStationSite). */
function PrismaticAI::StationSurvey(origin, o, cargo, site, slopes, box) {
	local tiles = AITileList();
	/* A site's corner is its north tile, and an industry's tiles lie south-east of its location
	 * tile: a site north or west of it catches it (catchment radius 4) with a corner up to
	 * 3 + o.w (3 + o.h) tiles out, which 10 did not reach for a terminus with a 4-tile throat on
	 * that side (corner 11 out). Only that far, not further: every tile more is valuated in every
	 * orientation, and the script's time is the game's. */
	local nw_x = max(10, 3 + o.w), nw_y = max(10, 3 + o.h);
	tiles.AddRectangle(AIMap.GetTileIndex(max(2, AIMap.GetTileX(origin) - nw_x), max(2, AIMap.GetTileY(origin) - nw_y)),
		AIMap.GetTileIndex(min(AIMap.GetMapSizeX() - 3 - o.w, AIMap.GetTileX(origin) + 10), min(AIMap.GetMapSizeY() - 3 - o.h, AIMap.GetTileY(origin) + 10)));
	tiles.Valuate(AITile.IsBuildableRectangle, o.w, o.h);
	tiles.KeepValue(1);
	tiles.Valuate(PrismaticAI.PlatformCoverage, cargo, o.sw, o.sh, o.offset, site == "produce");
	tiles.KeepAboveValue(site == "produce" ? 0 : 7);
	local sloped = AITileList();
	sloped.AddList(tiles);
	tiles.Valuate(PrismaticAI.RectFlat, o.w, o.h);
	tiles.KeepValue(1);
	sloped.RemoveList(tiles);
	local level = AITileList();
	level.AddList(sloped);
	level.Valuate(PrismaticAI.RectHeightRange, o.w, o.h);
	level.KeepBelowValue(PrismaticAI.RailLevelStep() + 1);
	level.Sort(AIList.SORT_BY_VALUE, AIList.SORT_ASCENDING);
	if (slopes) {
		sloped.Valuate(PrismaticAI.FitsOnFoundations, o.sw, o.sh, o.offset, o.along_x);
		sloped.KeepValue(1);
	} else {
		sloped.Clear();
	}
	local s = {o = o, flat = tiles, slope = sloped, near_flat = AITileList(), near_slope = AITileList(), level = level};
	foreach (pair in [["flat", "near_flat"], ["slope", "near_slope"]]) {
		local all = s[pair[0]], near = s[pair[1]];
		if (box != null) {
			local spread = AIGameSettings.IsValid("station.station_spread") ? AIGameSettings.GetValue("station.station_spread") : 12;
			near.AddList(all);
			near.Valuate(PrismaticAI.FitsSpread, o.w, o.h, box, spread);
			near.KeepValue(1);
			all.RemoveList(near);
			/* Side by side first (JoinRank 0): one station with parallel platforms, a transfer yard
			 * for every line of that industry, rather than platforms scattered across the spread. */
			near.Valuate(PrismaticAI.JoinRank, o.w, o.h, box);
			near.Sort(AIList.SORT_BY_VALUE, AIList.SORT_ASCENDING);
		}
		/* Most cargo first: the coverage again, since the flatness test replaced it as the value. */
		all.Valuate(PrismaticAI.PlatformCoverage, cargo, o.sw, o.sh, o.offset, site == "produce");
		all.Sort(AIList.SORT_BY_VALUE, AIList.SORT_DESCENDING);
	}
	return s;
}

/* 1 when the sw x sh platforms at `r` + `offset` (along x when `along_x`) can stand on foundations, by
 * the game's rule for a rail station tile (station_cmd.cpp CheckBuildableTile with build_on_slopes):
 * no steep slope, every tile's highest corner at one height, and a sloped tile raised at a corner of
 * both edges the track leaves by (CanBuildDepotByTileh for both directions of the axis); 0 otherwise.
 * A flat rectangle answers 1 as well. A valuator; RailStationFits asks the game itself before a build. */
function PrismaticAI::FitsOnFoundations(r, sw, sh, offset, along_x) {
	local mw = AIMap.GetMapSizeX();
	local t0 = r + offset;
	local top = AITile.GetMaxHeight(t0);
	/* The corners of each edge the platform's track leaves by: NE (N, E) and SW (S, W) along x,
	 * NW (N, W) and SE (S, E) along y (slope bits W 1, S 2, E 4, N 8). */
	local a = along_x ? (AITile.SLOPE_N | AITile.SLOPE_E) : (AITile.SLOPE_N | AITile.SLOPE_W);
	local b = along_x ? (AITile.SLOPE_S | AITile.SLOPE_W) : (AITile.SLOPE_S | AITile.SLOPE_E);
	for (local y = 0; y < sh; y++) {
		for (local x = 0; x < sw; x++) {
			local t = t0 + y * mw + x;
			local slope = AITile.GetSlope(t);
			if (AITile.GetMaxHeight(t) != top) return 0;
			if (slope == AITile.SLOPE_FLAT) continue;
			if (AITile.IsSteepSlope(slope) || (slope & a) == 0 || (slope & b) == 0) return 0;
		}
	}
	return 1;
}

/* Whether every tile of a terminus throat (TerminusGeometry `g`) takes its track: land the line
 * may use (RailStepCost) carrying all its pieces together on its slope (PieceFits). The double
 * track's spur tile S1 lies beside the site's rectangle, so nothing else looked at it. True
 * without a throat. */
function PrismaticAI::ThroatFits(g) {
	if (g.throat == null) return true;
	local bits = {};
	foreach (pc in PrismaticAI.ThroatPieces(g)) {
		local b = PrismaticAI.PieceBetween(pc[1], pc[0], pc[2]);
		if (bits.rawin(pc[1])) bits[pc[1]] = bits[pc[1]] | b;
		else bits[pc[1]] <- b;
	}
	foreach (t, b in bits) {
		if (PrismaticAI.RailStepCost(t) < 0 || !PrismaticAI.PieceFits(AITile.GetSlope(t), b)) return false;
	}
	return true;
}

/* Whether the game builds a terminus at corner `t` (geometry `g`, TerminusGeometry) into station
 * `into`: the platforms and every throat piece, asked in test mode. Its own AIAccounting keeps the
 * test's estimate out of the costs an enclosing one counts (the engine adds it there). */
function PrismaticAI::RailStationFits(t, g, axis, platforms, len, into) {
	local ok = false;
	{
		local probe = AIAccounting();
		local test = AITestMode();
		ok = AIRail.BuildRailStation(t, axis, platforms, len, into);
		if (ok && g.throat != null) {
			foreach (pc in PrismaticAI.ThroatPieces(g)) {
				if (!AIRail.BuildRail(pc[0], pc[1], pc[2])) { ok = false; break; }
			}
		}
	}
	return ok;
}

/* The track pieces of a terminus throat (TerminusGeometry), as [from, tile, to]: both platforms
 * straight out onto T1m and T1s, the second bent onto the main row at M, and on a double track the
 * junction A1 with its spur to S1. */
function PrismaticAI::ThroatPieces(g) {
	local th = g.throat;
	local pieces = [[th.end_m, th.t1m, th.m], [th.t1m, th.m, th.l0], [th.end_s, th.t1s, th.m2],
		[th.t1s, th.m2, th.m], [th.m2, th.m, th.l0]];
	if ("dbl" in g) {
		local d = g.dbl;
		pieces.extend([[d.m, d.l0, d.a1], [d.l0, d.a1, d.a2], [d.l0, d.a1, d.s1], [d.a1, d.s1, d.b2]]);
	}
	return pieces;
}

/* Build a terminus on the first site of `list` (StationSurvey) in orientation `o` that the game
 * takes, into station `into` (STATION_NEW or the station whose box is `box`). Every site's throat
 * is checked against its slopes first (ThroatFits); with `foundations` the sites are sloped, and
 * each is also asked in test mode (RailStationFits). A refused one so costs neither a station built
 * and removed nor a pause. `budget` counts the sites built and refused (`tried`, at most 20 per
 * terminus) and those refused before (`probes`). Returns the terminus (see BuildRailStationAt) or
 * null. */
function PrismaticAI::TryRailStationSites(list, o, into, len, platforms, label, foundations, budget, box) {
	for (local r = list.Begin(); !list.IsEnd() && budget.tried < 20; r = list.Next()) {
		/* The station's own corner: the throat lies on the `out` side of the platforms. */
		local t = o.forward ? r : r + o.throat * o.step;
		local g = PrismaticAI.TerminusGeometry(t, len, platforms, o.step, o.side, o.out, o.lane);
		if (g == null) continue;
		if (!PrismaticAI.ThroatFits(g) || (foundations && !PrismaticAI.RailStationFits(t, g, o.axis, platforms, len, into))) {
			budget.probes++;
			continue;
		}
		budget.tried++;
		AILog.Info("PrismaticAI: survey rail station " + label + " at tile " + t);
		AIController.Sleep(PACE);
		local rank = (into == AIStation.STATION_NEW) ? -1 : PrismaticAI.JoinRank(t, o.w, o.h, box);
		local made = PrismaticAI.RailStationOn(t, g, o, len, platforms, into, "");
		if (made == null) continue;
		AILog.Info("PrismaticAI: built rail station " + label + " at tile " + t + ", " + platforms + " platform(s) of " + len
			+ ", exit " + o.name + o.how + (foundations ? ", on foundations" : "")
			+ (into == AIStation.STATION_NEW ? "" : ", platforms of " + AIStation.GetName(into)
				+ (rank == 0 ? " beside its others" : " (apart from its others)")));
		return made;
	}
	return null;
}

/* Build the platforms at corner `t` (orientation `o`, geometry `g`) into station `into` and, with
 * more than one platform, their throat. The platforms are one rectangle; the throat is plain track.
 * They are removed apart when the throat is refused: a station tile is never demolished, because
 * demolishing one removes every platform of the station, a joined line's included (station_cmd.cpp
 * RemoveRailStation). `what` names the site in the warnings ("" or "levelled "). Returns the
 * terminus or null. */
function PrismaticAI::RailStationOn(t, g, o, len, platforms, into, what) {
	if (!AIRail.BuildRailStation(t, o.axis, platforms, len, into)) {
		AILog.Warning("PrismaticAI: " + what + "rail station at tile " + t + " refused: " + AIError.GetLastErrorString());
		return null;
	}
	local corner = t + (platforms - 1) * o.side + (len - 1) * o.step;
	local throat_tiles = [];
	local rec = {tile = t, platform_rect = [t, corner], throat = throat_tiles, step = o.step};
	if (platforms > 1 && !PrismaticAI.BuildThroat(g, throat_tiles)) {
		AILog.Warning("PrismaticAI: throat of the " + what + "station at tile " + t + " refused: " + AIError.GetLastErrorString());
		PrismaticAI.RemoveRailStation(rec, len);
		return null;
	}
	local made = {tile = t, end = g.end, exit = g.exit, step = o.step, platforms = platforms, len = len,
		platform_rect = [t, corner], throat = throat_tiles, backs = g.backs};
	if (o.lane != 0) made.dbl <- g.dbl;
	return made;
}

/* No site in any orientation: level one of survey `s` (a new station). Without this, on OpenTTD
 * seed 260603278 (GA g271.3, slot 3) ten of the twelve own lines found no flat site at one end, and
 * the company kept 2.1M idle for 15 years on 3 trains. Only rectangles whose corners span at most
 * RAIL_LEVEL_STEP height steps are levelled (a bigger step moves corners outside the site, where
 * houses and roads stop it), only where no corner that moves touches a tile that refuses it
 * (LevelPlan), and only while the money left after a fourfold margin on the test price is at least
 * the proportionate keep (RailLevelKeep); at most 3 per terminus (`budget.levelled`). The sites
 * passed over are counted in `budget` (`blocked`, `dear`, `money`) for the failure message and the
 * no-site memory. Returns the terminus or null. */
function PrismaticAI::LevelRailStationSite(s, len, platforms, label, budget) {
	local o = s.o;
	for (local r = s.level.Begin(); !s.level.IsEnd() && budget.levelled < 3; r = s.level.Next()) {
		local t = o.forward ? r : r + o.throat * o.step;
		local g = PrismaticAI.TerminusGeometry(t, len, platforms, o.step, o.side, o.out, o.lane);
		if (g == null) continue;
		local plan = PrismaticAI.LevelPlan(r, o.w, o.h);
		if (plan.start == null) { budget.blocked++; continue; }
		local cost = 0;
		{
			local test = AITestMode();
			local acc = AIAccounting();
			if (!AITile.LevelTiles(plan.start, plan.far)) cost = 1 << 30;
			else cost = acc.GetCosts();
		}
		local funds = AICompany.GetBankBalance(AICompany.COMPANY_SELF) + AICompany.GetMaxLoanAmount() - AICompany.GetLoanAmount();
		if (cost > PrismaticAI.RailLevelMax()) { budget.dear++; continue; }
		local keep = PrismaticAI.RailLevelKeep(cost);
		if (funds - 4 * cost < keep) {
			budget.money++;
			AILog.Info("PrismaticAI: rail station site " + label + " at tile " + t + " not levelled: estimated " + cost +
				", funds " + funds + " (keeping " + keep + " after four times the estimate)");
			continue;
		}
		budget.levelled++;
		local real = AIAccounting();
		AILog.Info("PrismaticAI: levelling rail station site " + label + " at tile " + t + " to height " + plan.z + " (estimated " + cost + ")");
		AIController.Sleep(PACE);
		local levelled = AITile.LevelTiles(plan.start, plan.far);
		/* What the levelling cost, before the station adds its own price to the same count. */
		local paid = real.GetCosts();
		if (!levelled) {
			AILog.Warning("PrismaticAI: levelled rail station site at tile " + t + " for " + paid + " and it was still refused: " + AIError.GetLastErrorString());
			continue;
		}
		local made = PrismaticAI.RailStationOn(t, g, o, len, platforms, AIStation.STATION_NEW, "levelled ");
		if (made == null) {
			/* The town's answer holds for every other site of this terminus: the levelling cleared
			 * trees, and each costs the town's rating (knowledge levelling-an-airport-site-clears-
			 * trees-and-costs-town-rating). On seed 31337 a site levelled exactly was refused so. */
			if (AIError.GetLastError() == AIError.ERR_LOCAL_AUTHORITY_REFUSES) budget.levelled = 3;
			local left = PrismaticAI.RectHeightRange(r, o.w, o.h);
			if (left != 0) AILog.Warning("PrismaticAI: levelling site at tile " + t + " for " + paid + " left it uneven (" + left + " height steps)");
			continue;
		}
		AILog.Info("PrismaticAI: built rail station " + label + " at tile " + t + " after levelling it for " + paid + ", " + platforms + " platform(s) of " + len + ", exit " + o.name + o.how);
		return made;
	}
	return null;
}

/* What a rail station levelling estimated at `cost` must leave after four times that cost: the
 * fixed RAIL_LEVEL_KEEP once the levelling is dear, RAIL_LEVEL_KEEP_X times the estimate below it,
 * and never under RAIL_LEVEL_KEEP_MIN. The fixed keep alone refused every levelling below about
 * 64000 of funds: #240 counted 6 sites levelled for 500 to 4000 refused at funds of 35000 to 64000
 * after the source station, which then had no site at all. With 12 a cheap site is levelled while
 * four times its price is at most a quarter of the funds; a dear one is judged as before. */
const RAIL_LEVEL_KEEP_X = 12;
const RAIL_LEVEL_KEEP_MIN = 15000;
function PrismaticAI::RailLevelKeep(cost) {
	return max(RAIL_LEVEL_KEEP_MIN, min(RAIL_LEVEL_KEEP, RAIL_LEVEL_KEEP_X * cost));
}

/* A company with RAIL_LEVEL_RICH or more to spend (bank balance and loan headroom) levels rail
 * station sites whose corners span up to RAIL_LEVEL_STEP_RICH height steps, estimated at up to
 * RAIL_LEVEL_MAX_RICH, or a fiftieth of its funds when that is less: in the owner's live game of
 * seed 21647 (1992) company 0 held 34M idle and its only new line failed twice at "no rail station
 * site at Cunnford Coal Mine", with levelling held to one height step and 40000. LevelPlan keeps
 * the bigger levellings from paying for a site a house or road leaves uneven. */
const RAIL_LEVEL_RICH = 2000000;
const RAIL_LEVEL_STEP_RICH = 3;
const RAIL_LEVEL_MAX_RICH = 400000;
function PrismaticAI::RailLevelFunds() {
	return AICompany.GetBankBalance(AICompany.COMPANY_SELF) + AICompany.GetMaxLoanAmount() - AICompany.GetLoanAmount();
}
function PrismaticAI::RailLevelStep() {
	return PrismaticAI.RailLevelFunds() >= RAIL_LEVEL_RICH ? max(RAIL_LEVEL_STEP, RAIL_LEVEL_STEP_RICH) : RAIL_LEVEL_STEP;
}
function PrismaticAI::RailLevelMax() {
	local funds = PrismaticAI.RailLevelFunds();
	return funds >= RAIL_LEVEL_RICH ? max(RAIL_LEVEL_MAX, min(RAIL_LEVEL_MAX_RICH, funds / 50)) : RAIL_LEVEL_MAX;
}

/* The height span of the corners of the w x h rectangle at `t` (a valuator for AITileList). */
function PrismaticAI::RectHeightRange(t, w, h) {
	local lo = 255, hi = 0;
	for (local dx = 0; dx <= w; dx++) {
		for (local dy = 0; dy <= h; dy++) {
			local c = AIMap.GetTileIndex(min(AIMap.GetMapSizeX() - 1, AIMap.GetTileX(t) + dx),
				min(AIMap.GetMapSizeY() - 1, AIMap.GetTileY(t) + dy));
			local z = AITile.GetCornerHeight(c, AITile.CORNER_N);
			if (z < lo) lo = z;
			if (z > hi) hi = z;
		}
	}
	return hi - lo;
}

/* How far from a site levelling may move corners before LevelPlan gives the site up (tiles). */
const LEVEL_REACH = 8;
/* The most corner steps a levelling LevelPlan accepts: each step of each moved corner counts
 * against the company's terraform limit (construction.terraform_frame_burst, 4096 by default), and
 * CmdLevelLand reports success when it stops at that limit with the site half levelled. */
const LEVEL_STEPS_MAX = 400;

/* What tile `t` is, in the words of a levelling message. */
function PrismaticAI::LevelTileKind(t) {
	if (!AIMap.IsValidTile(t)) return "the map's edge";
	if (AIRoad.IsRoadTile(t)) return "a road";
	if (AIRail.IsRailTile(t)) return "a track";
	if (AITile.IsStationTile(t)) return "a station";
	if (AIIndustry.IsValidIndustry(AIIndustry.GetIndustryID(t))) return "an industry";
	if (AITile.IsCoastTile(t) || AITile.IsWaterTile(t)) return "water";
	return "a house or object";
}

/* Whether level-land may move a corner of tile `t`: clear land (farmland and rocks included) and
 * trees only. Every other tile refuses a corner that moves under it (terraform_cmd.cpp runs each
 * tile's terraform proc with DoCommandFlag::Auto: a house, road, track, station or industry says no;
 * a coast is left alone to be safe), and autoslope's exceptions are not counted on. */
function PrismaticAI::LevelTerraformable(t) {
	return AIMap.IsValidTile(t) && AITile.IsBuildable(t) && !AIRoad.IsRoadTile(t) && !AITile.IsCoastTile(t);
}

/* What levelling the corners x0..x1, y0..y1 to height `z` does, as the game's level-land does it
 * (CmdLevelLand steps every corner of the rectangle toward z; TerraformTileHeight drags a neighbour
 * corner along x or y that would differ by more than one step). On a map where those neighbours
 * differ by at most one step, a corner at distance d (steps along x plus along y) from the rectangle
 * ends at its height clamped to z - d .. z + d, and when no corner at distance d moves, none further
 * out does. Returns {state, steps, blocker}: state "ok" with the corner steps it takes, "blocked"
 * with the tile that refuses (LevelTerraformable), or "unknown" (corners move beyond LEVEL_REACH
 * tiles or next to the map's edge, or a corner height could not be read). */
function PrismaticAI::LevelPlanAt(x0, y0, x1, y1, z) {
	local mx = AIMap.GetMapSizeX(), my = AIMap.GetMapSizeY();
	local steps = 0, tiles = {};
	for (local d = 0; d <= LEVEL_REACH; d++) {
		local moved = false;
		for (local x = x0 - d; x <= x1 + d; x++) {
			/* Only the ring at distance d: the whole column inside the rectangle's rows when x alone
			 * is d away, else the two corners rem rows above and below it. */
			local rem = d - max(0, max(x0 - x, x - x1));
			local ys = (rem == 0) ? null : [y0 - rem, y1 + rem];
			for (local k = 0; k < (ys == null ? y1 - y0 + 1 : 2); k++) {
				local y = (ys == null) ? y0 + k : ys[k];
				/* A ring is read only when the one inside it moved; next to the map's edge a corner may
				 * not move (TerraformTileHeight) and a void one reads -1, so the answer is unknown. */
				if (x <= 1 || y <= 1 || x >= mx - 2 || y >= my - 2) return {state = "unknown", steps = steps, blocker = null};
				local c = AIMap.GetTileIndex(x, y);
				local h = AITile.GetCornerHeight(c, AITile.CORNER_N);
				if (h < 0) return {state = "unknown", steps = steps, blocker = c};
				local to = max(z - d, min(z + d, h));
				if (to == h) continue;
				moved = true;
				steps += abs(to - h);
				for (local ax = x - 1; ax <= x; ax++) {
					for (local ay = y - 1; ay <= y; ay++) tiles[AIMap.GetTileIndex(ax, ay)] <- true;
				}
			}
		}
		if (!moved) {
			foreach (t, _ in tiles) {
				if (!PrismaticAI.LevelTerraformable(t)) return {state = "blocked", steps = steps, blocker = t};
			}
			return {state = steps > LEVEL_STEPS_MAX ? "unknown" : "ok", steps = steps, blocker = null};
		}
	}
	return {state = "unknown", steps = steps, blocker = null};
}

/* How to level the w x h site at tile `t` so that every corner of it ends level. AITile.LevelTiles
 * levels a rectangle to the height of its start corner, so each of the site's four outer corners is
 * a candidate, and the one whose levelling no tile refuses (LevelPlanAt) with the fewest corner
 * steps is taken. Without this, a corner shared with a house or road stayed where it was while
 * LevelTiles reported success (CmdLevelLand succeeds when any step did) and the money was spent: on
 * seed 15805 three airport sites were levelled for 8266, 1377 and 6489 and refused
 * ERR_FLAT_LAND_REQUIRED, each beside a house, road or water this finds. Returns {start, far, z,
 * steps, why}: `start` and `far` are the corners to pass to LevelTiles, or null with `why` when no
 * candidate works. */
function PrismaticAI::LevelPlan(t, w, h) {
	local x0 = AIMap.GetTileX(t), y0 = AIMap.GetTileY(t), x1 = x0 + w, y1 = y0 + h;
	local best = {start = null, far = null, z = -1, steps = 0, why = ""};
	if (x1 >= AIMap.GetMapSizeX() || y1 >= AIMap.GetMapSizeY()) {
		best.why = "the site reaches the map's edge";
		return best;
	}
	foreach (c in [[x0, y0, x1, y1], [x1, y0, x0, y1], [x0, y1, x1, y0], [x1, y1, x0, y0]]) {
		local start = AIMap.GetTileIndex(c[0], c[1]);
		local z = AITile.GetCornerHeight(start, AITile.CORNER_N);
		local p = PrismaticAI.LevelPlanAt(x0, y0, x1, y1, z);
		if (p.state == "ok") {
			if (best.start == null || p.steps < best.steps) {
				best = {start = start, far = AIMap.GetTileIndex(c[2], c[3]), z = z, steps = p.steps, why = ""};
			}
		} else if (best.start == null && best.why == "") {
			best.why = (p.state == "blocked") ? "levelling would move a corner of " + PrismaticAI.LevelTileKind(p.blocker) +
				" at tile " + p.blocker : "levelling would move corners more than " + LEVEL_REACH + " tiles out, by the map's edge or more than " + LEVEL_STEPS_MAX + " steps";
		}
	}
	return best;
}

/* What a terminus whose site starts at `r` catches with its platforms (sw x sh tiles, `offset` from
 * `r`): the cargo produced into the catchment at a mine, accepted at a town or factory (a valuator). */
function PrismaticAI::PlatformCoverage(r, cargo, sw, sh, offset, produce) {
	return produce ? AITile.GetCargoProduction(r + offset, cargo, sw, sh, 4) : AITile.GetCargoAcceptance(r + offset, cargo, sw, sh, 4);
}

/* The tiles of a terminus whose station corner is `t` (see BuildRailStationAt), with the main
 * platform chosen so that its exit leads on: {end, exit, throat, backs}, or null when neither choice
 * has an exit the line search can leave (ExitLeadsOn, RailStepCost). */
function PrismaticAI::TerminusGeometry(t, len, platforms, step, side, out, lane = 0) {
	local forward = (out == step);
	local rows = [];
	for (local p = 0; p < platforms; p++) rows.push(t + p * side);
	foreach (k, main in rows) {
		local end_m = forward ? main + (len - 1) * step : main;
		local g = {end = end_m, exit = end_m + out, throat = null, backs = []};
		if (platforms > 1) {
			/* Two platforms: the other row lies sd beside the main one. */
			local sec = rows[platforms - 1 - k];
			/* Double track: the second platform lies on the lane side, where the second track leaves. */
			if (lane != 0 && sec - main != lane) continue;
			local end_s = end_m + (sec - main);
			g.throat = {end_m = end_m, end_s = end_s, t1m = end_m + out, t1s = end_s + out,
				m = end_m + 2 * out, m2 = end_s + 2 * out, l0 = end_m + 3 * out};
			g.end = g.throat.m;
			g.exit = g.throat.l0;
		}
		if (lane != 0) {
			g.dbl <- PrismaticAI.DoubleTerminus(g.throat, out, lane);
			g.end = g.dbl.a1;
			g.exit = g.dbl.a2;
			/* Both tracks leave straight, and the second must lead on as well as the first. */
			if (PrismaticAI.RailStepCost(g.dbl.b2) < 0 || !PrismaticAI.ExitLeadsOn(g.dbl.b2, out, true)) continue;
		}
		if (PrismaticAI.RailStepCost(g.exit) < 0) continue;
		/* The exit must lead on: a station between houses whose exit faces a road junction is a dead
		 * end the line can never leave (OpenTTD seed 20208, Ganingbury). */
		if (!PrismaticAI.ExitLeadsOn(g.exit, out, platforms > 1)) continue;
		foreach (row in rows) {
			local far = forward ? row : row + (len - 1) * step;
			g.backs.push([far - out, far]);
		}
		return g;
	}
	return null;
}

/* Lay a terminus throat (TerminusGeometry): both platforms straight out onto T1m and T1s, a path
 * signal on each facing away from the platforms, the second platform bent onto the main row at M.
 * Every tile laid is added to `built` (the throat's own track). Returns false when the game
 * refuses a piece. */
function PrismaticAI::BuildThroat(g, built) {
	local th = g.throat;
	foreach (i, pc in PrismaticAI.ThroatPieces(g)) {
		if (!AIRail.BuildRail(pc[0], pc[1], pc[2])) return false;
		/* A1 takes two pieces (6 and 7, from 0, in ThroatPieces); each tile is recorded once, as
		 * soon as it holds track. */
		if (i != 7) built.push(pc[1]);
	}
	if ("dbl" in g) {
		local d = g.dbl;
		AILog.Info("PrismaticAI: built the double-track junction at tile " + d.a1 + ", tracks leave at tiles " + d.a2 + " and " + d.b2);
	}
	/* AIRail.BuildSignal(tile, front) signals the trains that come from `front` (script_rail.cpp
	 * _possible_trackdirs): here those entering a platform. On a double track they face the trains
	 * leaving instead, so a train arriving at full platforms waits on its own track, at the last
	 * one-way signal before the junction, and never on M, the tile every departure needs. */
	local dbl = "dbl" in g;
	if (!AIRail.BuildSignal(th.t1m, dbl ? th.end_m : th.m, AIRail.SIGNALTYPE_PBS)) return false;
	if (!AIRail.BuildSignal(th.t1s, dbl ? th.end_s : th.m2, AIRail.SIGNALTYPE_PBS)) return false;
	AILog.Info("PrismaticAI: built the throat of two platforms at tile " + th.m + ", path signals at tiles " + th.t1m + " and " + th.t1s);
	return true;
}

/* The rail bridge for a span from head `a` to head `b`: the cheapest one faster than 100 km/h, else
 * the fastest there is; -1 when none is that long. */
function PrismaticAI::BridgeType(a, b) {
	local len = AIMap.DistanceManhattan(a, b) + 1;
	local list = AIBridgeList_Length(len);
	if (list.IsEmpty()) return -1;
	list.Valuate(AIBridge.GetMaxSpeed);
	local fast = AIList();
	fast.AddList(list);
	fast.KeepAboveValue(100);
	if (!fast.IsEmpty()) {
		fast.Valuate(AIBridge.GetPrice, len);
		fast.Sort(AIList.SORT_BY_VALUE, AIList.SORT_ASCENDING);
		return fast.Begin();
	}
	list.Sort(AIList.SORT_BY_VALUE, AIList.SORT_DESCENDING);
	return list.Begin();
}

/* Days a line being laid may wait in all for the money of its next piece (RailCashWait). */
RAIL_CASH_WAIT_DAYS <- 120;

/* After a piece refused for ERR_NOT_ENOUGH_CASH: borrow up to the limit, then wait a day at a time
 * for the income of the lines already running, and build the piece again (`kind` "rail" with the
 * tiles a, b, c; "bridge" with the type a and heads b, c; "tunnel" from the portal a) until it is
 * built or the line has waited RAIL_CASH_WAIT_DAYS in all (`budget.days`, shared by the line's
 * pieces). True when the piece was built. A re-plan around the tile cannot help: money is short,
 * not land. Before this, the owner's live game of 2026-11-22 (seed 15805) logged six
 * "rail at tile N failed: ERR_NOT_ENOUGH_CASH" in its first three years: each line was re-planned
 * twice, then given up with its track already laid and paid for. */
function PrismaticAI::RailCashWait(budget, kind, a, b = 0, c = 0) {
	if (AIError.GetLastError() != AIError.ERR_NOT_ENOUGH_CASH) return false;
	local start = budget.days;
	while (budget.days < RAIL_CASH_WAIT_DAYS) {
		AICompany.SetLoanAmount(AICompany.GetMaxLoanAmount());
		local ok = false;
		if (kind == "rail") ok = AIRail.BuildRail(a, b, c);
		else if (kind == "bridge") ok = AIBridge.BuildBridge(AIVehicle.VT_RAIL, a, b, c);
		else ok = AITunnel.BuildTunnel(AIVehicle.VT_RAIL, a);
		if (ok) {
			if (budget.days > start) AILog.Info("PrismaticAI: waited " + (budget.days - start) + " day(s) for the money of the next rail piece");
			return true;
		}
		if (AIError.GetLastError() != AIError.ERR_NOT_ENOUGH_CASH) return false;
		AIController.Sleep(74);
		budget.days++;
	}
	AILog.Warning("PrismaticAI: still short of money after waiting " + budget.days + " day(s) for a rail line");
	return false;
}

/* Lays `path` piece by piece between the station end tiles `a_end` and `b_end`. Returns the tiles
 * actually laid, or null. A piece the game refuses (a house, a rival's track or a vehicle took the
 * tile since the search, ERR_AREA_NOT_CLEAR) is not the end of the line: from the last laid tile the
 * search runs again around the refused one, the last piece is relaid toward the new route, and the
 * rest follows (at most `replans` times). On seed 31337 a 212-tile coal line was dropped this way.
 * While it lays, MAINT_LAYING says so, and no maintenance round runs (steward.nut MaintenanceYield). */
function PrismaticAI::BuildRailAlong(path, a_end, b_end, replans = 2, to_straight = false, from_straight = false, cash = null) {
	MAINT_LAYING++;
	local laid = PrismaticAI.LayRailAlong(path, a_end, b_end, replans, to_straight, from_straight, cash);
	MAINT_LAYING--;
	return laid;
}

/* BuildRailAlong's body. */
function PrismaticAI::LayRailAlong(path, a_end, b_end, replans, to_straight, from_straight, cash) {
	local n = path.len();
	local corners = 0;
	if (cash == null) cash = {days = 0};
	for (local i = 0; i < n; i++) {
		/* Two portals in a row that are not neighbours and a tunnel joins: the tunnel FindRailPath
		 * planned (the game digs from the first to the far end it finds itself). */
		if (i + 1 < n && AIMap.DistanceManhattan(path[i], path[i + 1]) > 1 &&
				AITile.GetSlope(path[i]) != AITile.SLOPE_FLAT && AITunnel.GetOtherTunnelEnd(path[i]) == path[i + 1]) {
			if (!AITunnel.BuildTunnel(AIVehicle.VT_RAIL, path[i]) && !PrismaticAI.RailCashWait(cash, "tunnel", path[i])) {
				AILog.Warning("PrismaticAI: rail tunnel from tile " + path[i] + " to " + path[i + 1] + " failed: " + AIError.GetLastErrorString());
				return null;
			}
			AILog.Info("PrismaticAI: built rail tunnel from tile " + path[i] + " to " + path[i + 1]);
			i++;
			continue;
		}
		/* Two heads in a row that are not neighbours: the bridge FindRailPath planned between them. */
		if (i + 1 < n && AIMap.DistanceManhattan(path[i], path[i + 1]) > 1) {
			local bridge = PrismaticAI.BridgeType(path[i], path[i + 1]);
			if (bridge == -1 || (!AIBridge.BuildBridge(AIVehicle.VT_RAIL, bridge, path[i], path[i + 1]) &&
					!PrismaticAI.RailCashWait(cash, "bridge", bridge, path[i], path[i + 1]))) {
				AILog.Warning("PrismaticAI: rail bridge from tile " + path[i] + " to " + path[i + 1] + " failed: " + AIError.GetLastErrorString());
				return null;
			}
			AILog.Info("PrismaticAI: built rail bridge " + AIBridge.GetName(bridge, AIVehicle.VT_RAIL) + " from tile " + path[i] + " to " + path[i + 1]);
			i++;
			continue;
		}
		local prev = (i == 0) ? a_end : path[i - 1];
		local next = (i == n - 1) ? b_end : path[i + 1];
		if (next - path[i] != path[i] - prev) corners++;
		if (!AIRail.BuildRail(prev, path[i], next) && !PrismaticAI.RailCashWait(cash, "rail", prev, path[i], next)) {
			AILog.Warning("PrismaticAI: rail at tile " + path[i] + " failed: " + AIError.GetLastErrorString());
			if (replans <= 0 || i < 1) return null;
			local f = path[i - 1], e = (i == 1) ? a_end : path[i - 2], g = path[i];
			local avoid = {};
			avoid[g] <- 1;
			/* The move that arrived at e, so the new route's first corner is checked against e's
			 * (Turns90): the old search started as if out of a straight platform. */
			local from_in = null;
			if (i >= 3) from_in = PrismaticAI.UnitStep(path[i - 3], path[i - 2]);
			else if (i == 2) from_in = PrismaticAI.UnitStep(a_end, path[0]);
			/* Re-planning from the throat's own exit tile (f is path[0]) keeps it straight too. */
			local rest = PrismaticAI.FindRailPath(f, path[n - 1], e, b_end, avoid, from_in, from_straight && i == 1, to_straight);
			if (rest == null || rest.len() < 2) return null;
			/* The piece on f still points at g; it is relaid toward the new route's first step. */
			AIRail.RemoveRail(e, f, g);
			AILog.Info("PrismaticAI: re-planned from tile " + f + " around tile " + g + ", " + rest.len() + " tiles to go");
			local tail = PrismaticAI.BuildRailAlong(rest, e, b_end, replans - 1, to_straight, from_straight && i == 1, cash);
			if (tail == null) return null;
			local laid = path.slice(0, i - 1);
			laid.extend(tail);
			return laid;
		}
		AILog.Info("PrismaticAI: built rail at tile " + path[i]);
		if (i % 4 == 0) AIController.Sleep(ROAD_PACE);
	}
	AILog.Info("PrismaticAI: line laid, " + n + " tiles, " + corners + " corner pieces (diagonal track and turns)");
	return path;
}

/* The unit move from tile a toward tile b along one axis (a bridge or tunnel jump counts as its
 * direction): +-1 or +-map width. */
function PrismaticAI::UnitStep(a, b) {
	local k = max(1, AIMap.DistanceManhattan(a, b));
	return (b - a) / k;
}

/* Walk a laid line from station end `a_end` through `path` to `b_end` and check that every plain
 * piece joins its neighbours (AIRail.AreTilesConnected, the game's own track bits); bridge and
 * tunnel heads stand as the game built them. Returns -1 when the line is whole, else the first tile
 * where it is broken. A broken line is one where trains get lost: found here, the missing piece is
 * laid once more, and a line that stays broken gets no train. */
function PrismaticAI::VerifyRailLine(path, a_end, b_end) {
	local n = path.len();
	for (local i = 0; i < n; i++) {
		local prev = (i == 0) ? a_end : path[i - 1];
		local next = (i == n - 1) ? b_end : path[i + 1];
		if (AIMap.DistanceManhattan(prev, path[i]) != 1 || AIMap.DistanceManhattan(path[i], next) != 1) continue;
		/* A bridge or tunnel head. A tile whose only piece is gone has no rail at all and is laid
		 * again below: skipping every tile that is not a rail tile left such a gap for good. */
		if (AIBridge.IsBridgeTile(path[i]) || AITunnel.IsTunnelTile(path[i])) continue;
		if (AIRail.AreTilesConnected(prev, path[i], next)) continue;
		if (AIRail.BuildRail(prev, path[i], next) && AIRail.AreTilesConnected(prev, path[i], next)) {
			AILog.Info("PrismaticAI: relaid the missing piece at tile " + path[i]);
			continue;
		}
		return path[i];
	}
	return -1;
}

/* A depot behind the dead end of one of a station's platforms (`st.backs`, [depot tile, platform
 * tile] pairs from BuildRailStationAt): trains reach it through their own platform, turn in it, and
 * leave inside the station's signals, so a depot never lets a train onto the single track past
 * them. Returns its tile or null. */
function PrismaticAI::BuildRailDepotBehind(st) {
	if (!("backs" in st)) return null;
	foreach (b in st.backs) {
		if (PrismaticAI.RailStepCost(b[0]) < 0) continue;
		if (AIRail.BuildRailDepot(b[0], b[1])) {
			AILog.Info("PrismaticAI: built rail depot at tile " + b[0] + " behind a platform");
			return b[0];
		}
	}
	return null;
}

/* A depot beside the line near its start, joined to the line both ways, only where the line runs
 * straight: on a corner the two joining pieces meet the line's own corner at 90 degrees, and with
 * forbid_90_deg a train leaving the depot could take neither way. Returns its tile or null. */
function PrismaticAI::BuildRailDepotBeside(path) {
	local map_w = AIMap.GetMapSizeX();
	for (local k = 1; k < min(path.len() - 1, 12); k++) {
		local p = path[k];
		if (AIMap.DistanceManhattan(path[k - 1], p) != 1 || AIMap.DistanceManhattan(p, path[k + 1]) != 1) continue;
		if (p - path[k - 1] != path[k + 1] - p) continue; /* not straight here */
		foreach (cand in [p - 1, p + 1, p - map_w, p + map_w]) {
			if (cand == path[k - 1] || cand == path[k + 1] || PrismaticAI.RailStepCost(cand) < 0) continue;
			if (!AIRail.BuildRailDepot(cand, p)) continue;
			if (AIRail.BuildRail(cand, p, path[k + 1]) && AIRail.BuildRail(cand, p, path[k - 1])) {
				AILog.Info("PrismaticAI: built rail depot at tile " + cand);
				return cand;
			}
			AITile.DemolishTile(cand);
		}
	}
	return null;
}

/* Removes what a station record built: its own platforms (AIRail.RemoveRailStationTileRectangle,
 * never DemolishTile, which would take every platform of a joined station) and its throat. A record
 * without `platform_rect` (from before the throat) demolishes its platform tiles: the step along the platform comes from the
 * record, since derived from end - tile it was 0 for a station facing the other way and the cleanup
 * after a failed line demolished one tile of three (OpenTTD seed 31337, 2026-11-17). */
function PrismaticAI::RemoveRailStation(st, len) {
	if ("platform_rect" in st) {
		/* Only this station's own platforms (keep_rail false: their track goes too), never the
		 * joined station's others, then the throat's plain track. */
		AIRail.RemoveRailStationTileRectangle(st.platform_rect[0], st.platform_rect[1], false);
		foreach (t in st.throat) AITile.DemolishTile(t);
		return;
	}
	local step = ("step" in st) ? st.step : (st.end - st.tile) / max(1, len - 1);
	for (local i = 0; i < len; i++) AITile.DemolishTile(st.tile + i * step);
}

/* A line that will run no train leaves no station: both of its termini go (RemoveRailStation); the
 * track already laid stays. A station left standing costs upkeep, earns nothing, and keeps the
 * bot's next own line away from its producer (NextOwnFreightLine skips a producer with a rail
 * station of ours near it). In the owner's live game (seed 15805) 10 of company 0's 12 rail
 * stations had no train by 1958 (research/incident-2026-11-23-trainless-lines.md). */
function PrismaticAI::AbandonLineStations(sa, sb, len, label, why) {
	AILog.Warning("PrismaticAI: rail line " + label + " given up (" + why + "); its two stations are removed");
	PrismaticAI.RemoveRailStation(sa, len);
	PrismaticAI.RemoveRailStation(sb, len);
}

/* A finished line whose first train was not bought keeps its place: it goes into RAIL_LINES with no
 * train and logs its record (so the Rhai rail monitor reports it `unserved`), and KeepLinesServed
 * buys its first train once the bank holds the reserve. Before, the line returned 0 and was
 * forgotten: in the owner's live game two finished iron ore lines (180 and 123 rail pieces, four
 * stations, two depots) stood without a train for years while 535k lay in the bank. Returns 1. */
function PrismaticAI::KeepTrainlessLine(line) {
	RAIL_LINES.push(line);
	PrismaticAI.Funnel("rail", "built");
	PrismaticAI.Funnel("rail", "trainless");
	AILog.Warning("PrismaticAI: rail line " + line.label + " is built but its first train was not bought (" +
		line.train_fail + "); it waits for one");
	AILog.Info(PrismaticAI.LineRecord(RAIL_LINES.len(), line));
	return 1;
}

/* Whether a failed BuildTrain (`line.train_fail`) leaves a line a later train can serve: money, or
 * no engine for the cargo yet. A wagon that carries nothing or orders the game refuses would fail
 * the same way again on the same line. */
function PrismaticAI::TrainCanComeLater(line) {
	return ("train_fail" in line) && (line.train_fail == "cash" || line.train_fail == "engine");
}

/* The depot of a line just laid between the termini `sa` and `sb` along `laid`: behind a platform of
 * either, else beside the track. {depot (a tile, or null), cash (true when it was refused for money,
 * the test hook included)}. After RailCashWait the track is laid, and the depot met the same shortage
 * that the train did (#240, seed 15805): a laid line must not be dropped for it. */
function PrismaticAI::LineDepot(sa, sb, laid) {
	if (PrismaticAI.TestRefuseDepot()) return {depot = null, cash = true};
	local depot = PrismaticAI.BuildRailDepotBehind(sa);
	if (depot == null) depot = PrismaticAI.BuildRailDepotBehind(sb);
	if (depot == null) depot = PrismaticAI.BuildRailDepotBeside(laid);
	return {depot = depot, cash = depot == null && AIError.GetLastError() == AIError.ERR_NOT_ENOUGH_CASH};
}

/* A line laid whose depot was refused for money: kept like a line whose train was (KeepTrainlessLine),
 * with depot -1 and `depot_pending`; KeepLinesServed builds the depot, then the first train, once
 * the bank holds both. */
function PrismaticAI::KeepDepotlessLine(line) {
	line.depot_pending <- true;
	line.train_fail <- "cash";
	AILog.Warning("PrismaticAI: rail line " + line.label + " is laid but its depot was refused for money; it waits for one");
	return PrismaticAI.KeepTrainlessLine(line);
}

/* Builds the depot of a kept line that has none (KeepDepotlessLine), in its own rail type, behind a
 * platform of its stations or beside its track; true when it stands. */
function PrismaticAI::BuildPendingDepot(line) {
	AIRail.SetCurrentRailType(line.railtype);
	local d = null;
	foreach (st in line.stations) if (d == null) d = PrismaticAI.BuildRailDepotBehind(st);
	if (d == null && line.path.len() > 2) d = PrismaticAI.BuildRailDepotBeside(line.path);
	if (d == null) return false;
	line.depot = d;
	delete line.depot_pending;
	AILog.Info("PrismaticAI: line " + line.label + " got its depot at tile " + d);
	return true;
}

/* test_trainless (RESEARCH-ONLY, info.nut): every other new line's depot is refused as for money. */
TEST_DEPOT_TURN <- 0;
function PrismaticAI::TestRefuseDepot() {
	if (AIController.GetSetting("test_trainless") == 0) return false;
	TEST_DEPOT_TURN++;
	if (TEST_DEPOT_TURN % 2 == 1) return false;
	AILog.Warning("PrismaticAI: rail depot not built: refused by test_trainless");
	return true;
}

/* The line between two termini, its depot and one train for `cargo`. The first stop loads with
 * `first_flags` (full load at a mine; none for passengers). Returns 1 when a train runs, else 0. A
 * finished line logs its record (lines.nut LineRecord): its geometry for the Rhai steward, which
 * sets trains up to trains_max and wagons up to wagons_fit. */
function PrismaticAI::BuildRailLineAndTrain(sa, sb, len, cargo, wagons, first_flags, label) {
	AILog.Info("PrismaticAI: survey rail line from tile " + sa.exit + " to tile " + sb.exit);
	PrismaticAI.Funnel("rail", "stations");
	local throat_a = ("platforms" in sa) && sa.platforms > 1, throat_b = ("platforms" in sb) && sb.platforms > 1;
	local path = PrismaticAI.FindRailPath(sa.exit, sb.exit, sa.end, sb.end, null, null, throat_a, throat_b);
	if (path == null) {
		AILog.Warning("PrismaticAI: no rail path between the stations; removing them");
		PrismaticAI.Funnel("rail", "path_fail");
		PrismaticAI.RemoveRailStation(sa, len); PrismaticAI.RemoveRailStation(sb, len);
		return 0;
	}
	/* What was actually laid: a re-plan around a refused piece changes the route. */
	local laid = PrismaticAI.BuildRailAlong(path, sa.end, sb.end, 2, throat_b, throat_a);
	if (laid == null) { PrismaticAI.AbandonLineStations(sa, sb, len, label, "the track could not be laid"); return 0; }
	local broken = PrismaticAI.VerifyRailLine(laid, sa.end, sb.end);
	if (broken != -1) {
		AILog.Warning("PrismaticAI: rail line " + label + " is broken at tile " + broken + "; no train on a line it cannot run");
		PrismaticAI.AbandonLineStations(sa, sb, len, label, "broken at tile " + broken);
		return 0;
	}
	AILog.Info("PrismaticAI: rail line " + label + " done, " + laid.len() + " tiles");
	PrismaticAI.Funnel("rail", "laid");
	local ld = PrismaticAI.LineDepot(sa, sb, laid);
	if (ld.depot == null && !ld.cash) {
		AILog.Warning("PrismaticAI: no rail depot behind a platform or beside the line");
		PrismaticAI.AbandonLineStations(sa, sb, len, label, "no depot");
		return 0;
	}
	local depot = (ld.depot == null) ? -1 : ld.depot;
	local platforms = min(("platforms" in sa) ? sa.platforms : 1, ("platforms" in sb) ? sb.platforms : 1);
	/* Everything the line owns (its stations' own platforms and throats, its track, its depot), so
	 * closing it (lines.nut CloseLine) clears it to the last tile and nothing of another line. */
	local line = {a = sa.tile, b = sb.tile, depot = depot, cargo = cargo, wagons = wagons, flags = first_flags,
		label = label, railtype = AIRail.GetCurrentRailType(), restored = 0, len = len, trains_max = platforms,
		path = laid, stations = [sa, sb], closed = false};
	if (depot == -1) return PrismaticAI.KeepDepotlessLine(line);
	local train = PrismaticAI.BuildTrain(line);
	if (train == -1) {
		if (PrismaticAI.TrainCanComeLater(line)) return PrismaticAI.KeepTrainlessLine(line);
		PrismaticAI.AbandonLineStations(sa, sb, len, label, "no train can run on it: " + line.train_fail);
		return 0;
	}
	RAIL_LINES.push(line);
	PrismaticAI.Funnel("rail", "built");
	AILog.Info("PrismaticAI: built a rail route " + label + " with a train of " + AIVehicle.GetNumWagons(train) + " (" + AIEngine.GetName(AIVehicle.GetEngineType(train)) + ")");
	AILog.Info(PrismaticAI.LineRecord(RAIL_LINES.len(), line));
	return 1;
}

/*
 * Double track (RESEARCH-ONLY, opt-in: setting rail_double=1, the bot's own freight lines only).
 * After AAAHogEx, whose rail earns 2-3M a year by 1970 while a single-track line here holds two
 * trains: one track each way, one-way path signals every RAIL_SIGNAL_SPACING tiles, so a line holds
 * as many trains as it has signal blocks and the in-bot steward adds them while cargo waits.
 *
 * Each terminus is the two-platform throat (BuildThroat) and two more tiles, L0 and the junction A1,
 * where the two tracks part (`lane` is the side the second track leaves on):
 *
 *     platform main | T1m  M   L0  A1  A2 -> track A
 *     platform 2nd  | T1s  M2          S1  B2 -> track B
 *
 * A1 carries a straight piece (track A) and a corner toward S1, S1 a corner back onto the axis. L0 is
 * straight, so neither platform's route turns 90 degrees into either track (Turns90: M2 to M, then
 * two straight moves, then A1 to S1). Seen from the source, the second track leaves on one side,
 * and at the far end on the same side of the map (hand +1 and -1 in BuildRailStationAt), so track A
 * runs A2 to the far A2 and track B B2 to the far B2 without crossing.
 *
 * Track A carries the trains from the source, track B the trains back: one-way path signals facing
 * that way (PlaceLaneSignals) make the other way a dead end to the pathfinder. A train leaves its
 * platform at the throat's path signal, which faces departures on a double track (BuildThroat),
 * shares M-L0-A1 with arriving trains only, and an arriving train waits at the last one-way signal
 * before the junction until it can reserve its way into a free platform. Both tracks come from the same A*
 * search (FindRailPath); the second runs with the first's tiles left out, and both are walked
 * (VerifyRailLine, VerifyJunction) before a train is bought. When the second track cannot be found,
 * laid or walked, the junction's spur is taken out again and the line runs single track with
 * trains_max = platforms, as rail_double=0 builds it.
 */

/* Tiles between the one-way path signals of a double track: one block holds a train as long as
 * the longest the game allows by default (vehicle.max_train_length 7). */
const RAIL_SIGNAL_SPACING = 7;

/* The move a quarter turn from `d` (+-1 along x, +-map width along y), the same way for every d. */
function PrismaticAI::QuarterTurn(d) {
	local map_w = AIMap.GetMapSizeX();
	if (d == 1) return map_w;
	if (d == map_w) return -1;
	if (d == -1) return -map_w;
	return 1;
}

/* The junction tiles of a double-track terminus from its throat (TerminusGeometry), the move `out`
 * toward the other end and the side `lane` the second track leaves on (see above). */
function PrismaticAI::DoubleTerminus(th, out, lane) {
	local a1 = th.l0 + out, s1 = a1 + lane;
	return {m = th.m, l0 = th.l0, a1 = a1, a2 = a1 + out, s1 = s1, b2 = s1 + out, out = out, lane = lane};
}

/* The pieces of a junction (`d`, DoubleTerminus): the first two carry track A, the last two join
 * track B. Returns the first tile whose piece is missing, or -1. */
function PrismaticAI::VerifyJunction(d, track_b) {
	local pieces = [[d.m, d.l0, d.a1], [d.l0, d.a1, d.a2]];
	if (track_b) pieces.extend([[d.l0, d.a1, d.s1], [d.a1, d.s1, d.b2]]);
	foreach (pc in pieces) {
		if (!AIRail.AreTilesConnected(pc[0], pc[1], pc[2])) return pc[1];
	}
	return -1;
}

/* One-way path signals along a laid track `path` (between the junction tiles `a_end` and `b_end`)
 * for trains that run from path[0] toward its end (`forward`) or back: on the first tile they enter,
 * then every RAIL_SIGNAL_SPACING tiles, and on the last tile before the far junction, so a train
 * waits for a platform there and never on the junction. A tile that takes no signal (a bridge or
 * tunnel head, a level crossing) passes the signal on to the next one. Returns the signals built. */
function PrismaticAI::PlaceLaneSignals(path, a_end, b_end, forward) {
	local n = path.len(), placed = 0, last = -RAIL_SIGNAL_SPACING;
	for (local k = 0; k < n; k++) {
		if (k - last < RAIL_SIGNAL_SPACING && k != n - 1) continue;
		local i = forward ? k : n - 1 - k;
		local p = path[i];
		local front = forward ? ((i == n - 1) ? b_end : path[i + 1]) : ((i == 0) ? a_end : path[i - 1]);
		local back = forward ? ((i == 0) ? a_end : path[i - 1]) : ((i == n - 1) ? b_end : path[i + 1]);
		if (AIMap.DistanceManhattan(p, front) != 1 || AIMap.DistanceManhattan(p, back) != 1) continue;
		if (!AIRail.IsRailTile(p) || AIRail.IsLevelCrossingTile(p)) continue;
		/* The tile the trains come from: BuildSignal signals trains arriving from its second tile. */
		if (AIRail.BuildSignal(p, back, AIRail.SIGNALTYPE_PBS_ONEWAY)) {
			placed++;
			last = k;
		}
	}
	return placed;
}

/* Take out a junction's spur onto track B (A1 to S1 and S1 itself): the line runs single track. */
function PrismaticAI::RemoveDoubleSpur(d) {
	AIRail.RemoveRail(d.l0, d.a1, d.s1);
	AIRail.RemoveRail(d.a1, d.s1, d.b2);
}

/* The double-track line between two termini built with a junction (BuildRailStationAt with hand),
 * its depot and one train as long as the platform; the steward adds the rest (steward.nut). Returns
 * 1 when a train runs, else 0. The line's record says trains_max: the platforms plus the signal
 * blocks of the shorter track, half of what both tracks and all four platforms hold, so a full
 * line still has room to move (a train waits for a platform on its own track, never on the other). */
function PrismaticAI::BuildDoubleRailLineAndTrain(sa, sb, len, cargo, first_flags, label) {
	local da = sa.dbl, db = sb.dbl;
	AILog.Info("PrismaticAI: survey double-track rail line from tile " + sa.exit + " to tile " + sb.exit);
	PrismaticAI.Funnel("rail", "stations");
	/* Track A keeps clear of where track B leaves each junction. */
	local avoid = {};
	foreach (d in [da, db]) {
		for (local k = 0; k < 3; k++) avoid[d.b2 + k * d.out] <- 1;
	}
	local path = PrismaticAI.FindRailPath(sa.exit, sb.exit, sa.end, sb.end, avoid, null, true, true);
	/* Track A's own search, before a re-plan while laying it overwrites RAIL_LAST_NODES. */
	local nodes_a = RAIL_LAST_NODES;
	if (path == null) {
		AILog.Warning("PrismaticAI: no rail path between the stations; removing them");
		PrismaticAI.Funnel("rail", "path_fail");
		PrismaticAI.RemoveRailStation(sa, len); PrismaticAI.RemoveRailStation(sb, len);
		PrismaticAI.Funnel("rail", "single_track");
		/* -1: nothing of the line is left, so the caller may try it single track (straight exits
		 * and the second track's reserved tiles are what a hilly map most often refuses). */
		return -1;
	}
	local acc_a = AIAccounting();
	/* One wait for money (RailCashWait) for both tracks of the line, not RAIL_CASH_WAIT_DAYS each. */
	local cash = {days = 0};
	local laid = PrismaticAI.BuildRailAlong(path, sa.end, sb.end, 2, true, true, cash);
	local cost_a = acc_a.GetCosts();
	if (laid == null) { PrismaticAI.AbandonLineStations(sa, sb, len, label, "the track could not be laid"); return 0; }
	local broken = PrismaticAI.VerifyRailLine(laid, sa.end, sb.end);
	if (broken == -1) broken = PrismaticAI.VerifyJunction(da, false);
	if (broken == -1) broken = PrismaticAI.VerifyJunction(db, false);
	if (broken != -1) {
		AILog.Warning("PrismaticAI: rail line " + label + " is broken at tile " + broken + "; no train on a line it cannot run");
		PrismaticAI.AbandonLineStations(sa, sb, len, label, "broken at tile " + broken);
		return 0;
	}
	/* Track B only when, after it, the money still buys the train: track B costs about what track A
	 * did. On OpenTTD seed 1516196607 (company 4) both tracks took the whole loan, the locomotive
	 * was refused for ERR_NOT_ENOUGH_CASH, and the company went bankrupt in 1961 with two buses. */
	local engines = PrismaticAI.PickRailEngines(AIRail.GetCurrentRailType(), cargo);
	local train_price = (engines.loco == -1) ? 0 : AIEngine.GetPrice(engines.loco) +
		(engines.wagon == -1 ? 0 : 2 * len * AIEngine.GetPrice(engines.wagon));
	local funds = AICompany.GetBankBalance(AICompany.COMPANY_SELF) + AICompany.GetMaxLoanAmount() - AICompany.GetLoanAmount();
	local afford_b = funds >= cost_a + train_price;
	if (!afford_b) AILog.Warning("PrismaticAI: no second track for " + label + ": it would cost about " + cost_a +
		" and leave less than the train's " + train_price + " (funds " + funds + ")");
	local taken = {};
	foreach (t in laid) taken[t] <- 1;
	local laid2 = null;
	/* Track B runs beside track A: where it is found, its search took 0.88 to 1.09 times track A's
	 * nodes (the fitness match above, four double lines), so twice that is its budget. */
	local path2 = afford_b ? PrismaticAI.FindRailPath(da.b2, db.b2, da.s1, db.s1, taken, da.lane, true, true, 2 * nodes_a) : null;
	if (path2 != null) laid2 = PrismaticAI.BuildRailAlong(path2, da.s1, db.s1, 2, true, true, cash);
	if (laid2 != null) {
		broken = PrismaticAI.VerifyRailLine(laid2, da.s1, db.s1);
		if (broken == -1) broken = PrismaticAI.VerifyJunction(da, true);
		if (broken == -1) broken = PrismaticAI.VerifyJunction(db, true);
		if (broken != -1) {
			AILog.Warning("PrismaticAI: second track of " + label + " is broken at tile " + broken);
			laid2 = null;
		}
	}
	local platforms = min(sa.platforms, sb.platforms);
	local trains_max = platforms;
	if (laid2 != null) {
		local sig_a = PrismaticAI.PlaceLaneSignals(laid, sa.end, sb.end, true);
		local sig_b = PrismaticAI.PlaceLaneSignals(laid2, da.s1, db.s1, false);
		if (sig_a == 0 || sig_b == 0) {
			AILog.Warning("PrismaticAI: no one-way signal on a track of " + label + " (" + sig_a + ", " + sig_b + "); no train on it");
			PrismaticAI.AbandonLineStations(sa, sb, len, label, "no one-way signal");
			return 0;
		}
		trains_max = platforms + min(sig_a, sig_b);
		AILog.Info("PrismaticAI: double track " + label + ": " + laid.len() + " and " + laid2.len() + " tiles, " + sig_a + " and " +
			sig_b + " one-way path signals, trains_max " + trains_max);
	} else {
		PrismaticAI.RemoveDoubleSpur(da);
		PrismaticAI.RemoveDoubleSpur(db);
		AILog.Warning("PrismaticAI: no second track for " + label + "; it runs single track, " + platforms + " trains");
		PrismaticAI.Funnel("rail", "single_track");
	}
	AILog.Info("PrismaticAI: rail line " + label + " done, " + laid.len() + " tiles");
	PrismaticAI.Funnel("rail", "laid");
	local ld = PrismaticAI.LineDepot(sa, sb, laid);
	if (ld.depot == null && !ld.cash) {
		AILog.Warning("PrismaticAI: no rail depot behind a platform or beside the line");
		PrismaticAI.AbandonLineStations(sa, sb, len, label, "no depot");
		return 0;
	}
	local depot = (ld.depot == null) ? -1 : ld.depot;
	/* wagons 0: every train as long as the platform. */
	local line = {a = sa.tile, b = sb.tile, depot = depot, cargo = cargo, wagons = 0, flags = first_flags,
		label = label, railtype = AIRail.GetCurrentRailType(), restored = 0, len = len, trains_max = trains_max,
		path = laid, stations = [sa, sb], closed = false, platforms = platforms};
	if (laid2 != null) line.path2 <- laid2;
	if (depot == -1) return PrismaticAI.KeepDepotlessLine(line);
	local train = PrismaticAI.BuildTrain(line);
	if (train == -1) {
		if (PrismaticAI.TrainCanComeLater(line)) return PrismaticAI.KeepTrainlessLine(line);
		PrismaticAI.AbandonLineStations(sa, sb, len, label, "no train can run on it: " + line.train_fail);
		return 0;
	}
	RAIL_LINES.push(line);
	PrismaticAI.Funnel("rail", "built");
	AILog.Info("PrismaticAI: built a rail route " + label + " with a train of " + AIVehicle.GetNumWagons(train) + " (" + AIEngine.GetName(AIVehicle.GetEngineType(train)) + ")");
	AILog.Info(PrismaticAI.LineRecord(RAIL_LINES.len(), line));
	return 1;
}

/* The engines a line runs: the fastest buildable locomotive with power on `railtype` (ties: the
 * more powerful) and the wagon of the largest capacity that carries or refits to `cargo`. {loco,
 * wagon}, -1 where there is none. A locomotive must have power, not only run: an electric one can
 * run on plain rail but not be built in a plain depot (CmdBuildRailVehicle: CMD_ERROR, which a
 * script reads as ERR_UNKNOWN). Picking it by CanRunOnRail once electric engines appeared left
 * finished lines without a train and the replacement loop failing every heartbeat (774 times in
 * one 20-year default replay; g288.brainSafe's 320-tile STEL line in m-0009 of 20a48c81). */
function PrismaticAI::PickRailEngines(railtype, cargo) {
	local loco = -1, wagon = -1;
	local engines = AIEngineList(AIVehicle.VT_RAIL);
	for (local e = engines.Begin(); !engines.IsEnd(); e = engines.Next()) {
		if (!AIEngine.IsBuildable(e) || !AIEngine.CanRunOnRail(e, railtype)) continue;
		if (AIEngine.IsWagon(e)) {
			if ((AIEngine.GetCargoType(e) == cargo || AIEngine.CanRefitCargo(e, cargo)) &&
					(wagon == -1 || AIEngine.GetCapacity(e) > AIEngine.GetCapacity(wagon))) wagon = e;
		} else if (!AIEngine.HasPowerOnRail(e, railtype)) {
			continue;
		} else if (loco == -1 || AIEngine.GetMaxSpeed(e) > AIEngine.GetMaxSpeed(loco) ||
				(AIEngine.GetMaxSpeed(e) == AIEngine.GetMaxSpeed(loco) && AIEngine.GetPower(e) > AIEngine.GetPower(loco))) {
			loco = e;
		}
	}
	return {loco = loco, wagon = wagon};
}

/* A train for `line` ({a, b, depot, cargo, wagons, flags, railtype, len}): PickRailEngines' engines,
 * `wagons` wagons (0: as many as fit), never longer than the platforms (`len` tiles, sixteen
 * sixteenths a tile, AIVehicle.GetLength), orders to both stations (the first with the line's load
 * flags), started. Returns the train or -1. */
function PrismaticAI::BuildTrain(line) {
	/* test_trainless (RESEARCH-ONLY, lines.nut): a new line's first train is refused as for money. */
	if (!("first" in line) && !("test_refused" in line) && AIController.GetSetting("test_trainless") > 0) {
		line.test_refused <- true;
		line.train_fail <- "cash";
		AILog.Warning("PrismaticAI: locomotive not bought: refused by test_trainless");
		PrismaticAI.Funnel("rail", "loco_refused_cash");
		return -1;
	}
	local e = PrismaticAI.PickRailEngines(line.railtype, line.cargo);
	if (e.loco == -1 || e.wagon == -1) {
		line.train_fail <- "engine";
		AILog.Warning("PrismaticAI: no locomotive or wagon for " + AICargo.GetCargoLabel(line.cargo));
		return -1;
	}
	local train = AIVehicle.BuildVehicle(line.depot, e.loco);
	/* Why a train was not bought, for the line it was bought for (rail.nut TrainCanComeLater). */
	if (!AIVehicle.IsValidVehicle(train)) {
		line.train_fail <- (AIError.GetLastError() == AIError.ERR_NOT_ENOUGH_CASH) ? "cash" : AIError.GetLastErrorString();
		AILog.Warning("PrismaticAI: locomotive not bought: " + AIError.GetLastErrorString());
		if (line.train_fail == "cash") PrismaticAI.Funnel("rail", "loco_refused_cash");
		return -1;
	}
	local max_len = (("len" in line) ? line.len : 3) * 16;
	for (local i = 0; line.wagons <= 0 || i < line.wagons; i++) {
		if (!PrismaticAI.AddWagon(train, line.depot, e.wagon, line.cargo, max_len)) break;
	}
	/* A train that carries nothing earns nothing: a NewGRF may refuse every wagon or refit. */
	if (AIVehicle.GetCapacity(train, line.cargo) <= 0) {
		AILog.Warning("PrismaticAI: no wagon of " + AIEngine.GetName(e.wagon) + " would carry " + AICargo.GetCargoLabel(line.cargo) + " behind " + AIEngine.GetName(e.loco) + "; the locomotive is sold");
		/* No wagon for money is a money failure (the loop above stops at the first refused wagon). */
		line.train_fail <- (AIError.GetLastError() == AIError.ERR_NOT_ENOUGH_CASH) ? "cash" : "wagon";
		AIVehicle.SellVehicle(train);
		return -1;
	}
	/* A train started without its orders would wander, earn nothing and never be found again. */
	if (!AIOrder.AppendOrder(train, line.a, line.flags) || !AIOrder.AppendOrder(train, line.b, AIOrder.OF_NONE)) {
		AILog.Warning("PrismaticAI: train for " + line.label + " got no orders to tiles " + line.a + " and " + line.b + ": " +
			AIError.GetLastErrorString() + "; the train is sold");
		AIVehicle.SellVehicle(train);
		line.train_fail <- "orders";
		return -1;
	}
	/* The line's trains are this one and its clones (shared orders, lines.nut TrainsOf). */
	if (!("first" in line)) PrismaticAI.Funnel("rail", "trained");
	line.first <- train;
	if ("train_fail" in line) delete line.train_fail;
	AIVehicle.StartStopVehicle(train);
	return train;
}

/* Buy one `wagon` in `depot`, refit it to `cargo` and couple it behind `train`, keeping the train
 * within `max_len` sixteenths of a tile. A wagon the game will not refit or couple (a NewGRF rule)
 * is sold again, never left in the depot. Returns true when the train has one more wagon. */
function PrismaticAI::AddWagon(train, depot, wagon, cargo, max_len) {
	local w = AIVehicle.BuildVehicle(depot, wagon);
	if (!AIVehicle.IsValidVehicle(w)) return false;
	if (AIEngine.GetCargoType(wagon) != cargo && !AIVehicle.RefitVehicle(w, cargo)) { AIVehicle.SellVehicle(w); return false; }
	if (!AIVehicle.MoveWagon(w, 0, train, AIVehicle.GetNumWagons(train) - 1)) { AIVehicle.SellVehicle(w); return false; }
	if (AIVehicle.GetLength(train) > max_len) {
		/* One too many for the platform: a train longer than its platform blocks the throat. */
		AIVehicle.SellWagon(train, AIVehicle.GetNumWagons(train) - 1);
		return false;
	}
	return true;
}

/* Platform length and wagons for a line of `dist` tiles carrying `supply` units of `cargo` a month,
 * on `platforms` platforms at each end. Every number is derived:
 * - the engines PickRailEngines would buy, their speed (0.036 tiles a day per km/h, .ai knowledge
 *   cargo-payment-and-vehicle-speed) and the wagon's capacity;
 * - the round trip, both ways plus a week at the stations;
 * - the trains the line may run (trains_max = platforms) and the load each must take per trip to
 *   move the month's supply: wagons = ceil(supply * trip_days / 30 / trains / capacity);
 * - the platform: the locomotive and those wagons at half a tile each (the default vehicle length;
 *   BuildTrain checks the real one), at least 2 tiles, at most the game's vehicle.max_train_length
 *   and station.station_spread.
 * `override_len` and `override_wagons` (the settings rail_len, rail_wagons) win when above 0.
 * Returns {len, wagons, trip_days, why}. */
function PrismaticAI::PlatformPlan(cargo, supply, dist, platforms, override_len, override_wagons) {
	local e = PrismaticAI.PickRailEngines(AIRail.GetCurrentRailType(), cargo);
	local kmh = (e.loco == -1) ? 100 : max(1, AIEngine.GetMaxSpeed(e.loco));
	local cap = (e.wagon == -1) ? 30 : max(1, AIEngine.GetCapacity(e.wagon));
	local trip_days = (2.0 * dist / (kmh * 0.036)).tointeger() + 7;
	local cap_len = 7;
	if (AIGameSettings.IsValid("vehicle.max_train_length")) cap_len = AIGameSettings.GetValue("vehicle.max_train_length");
	if (AIGameSettings.IsValid("station.station_spread")) cap_len = min(cap_len, AIGameSettings.GetValue("station.station_spread"));
	local trains = max(1, platforms);
	local need = (supply * trip_days + 30 * trains * cap - 1) / (30 * trains * cap);
	local wagons = max(1, need);
	local len = max(2, min(cap_len, (1 + wagons + 1) / 2));
	/* Never more wagons than the platform takes at half a tile each. */
	wagons = min(wagons, 2 * len - 1);
	if (override_len > 0) len = min(cap_len, override_len);
	if (override_wagons > 0) wagons = override_wagons;
	local why = "supply " + supply + "/month, " + dist + " tiles, trip " + trip_days + " days at " + kmh + " km/h, " + trains +
		" train(s) of " + wagons + " x " + cap + " -> platform " + len + " tiles (max " + cap_len + ")";
	return {len = len, wagons = (override_wagons > 0) ? override_wagons : 0, wagons_planned = wagons, trip_days = trip_days, why = why};
}

/* What a first train of `line` costs now: its locomotive and as many wagons as its platform holds
 * (BuildTrain); 0 when the line has no engine yet (BuildTrain then says why). */
function PrismaticAI::FirstTrainPrice(line) {
	local e = PrismaticAI.PickRailEngines(line.railtype, line.cargo);
	if (e.loco == -1 || e.wagon == -1) return 0;
	local wagons = line.wagons > 0 ? line.wagons : 2 * (("len" in line) ? line.len : 3);
	return AIEngine.GetPrice(e.loco) + wagons * AIEngine.GetPrice(e.wagon);
}

/* Every rail line the bot built keeps a train: one that was lost (a crash at a level crossing, a
 * sale) is replaced in the line's depot, at most twice per line, so a line that keeps losing its
 * train for money reasons is not rebuilt forever. Old trains are renewed by the company's own
 * autorenew (main.nut). */
function PrismaticAI::KeepLinesServed(reserve) {
	foreach (i, line in RAIL_LINES) {
		/* A line whose track cannot be repaired or whose trains get lost gets no train (events.nut). */
		if (line.restored >= 2 || line.closed || PrismaticAI.LineHalted(line) || PrismaticAI.LineTarget(i + 1, "trains") == 0) continue;
		/* A train of this line: a rail vehicle with orders to both of its stations (two lines may
		 * share a station since a second platform joins it). */
		/* Its own trains only (lines.nut TrainsOf): a line sharing both stations with another is not
		 * served by the other's trains. */
		local served = PrismaticAI.TrainsOf(line).len() > 0;
		if (served) continue;
		if (AICompany.GetBankBalance(AICompany.COMPANY_SELF) < reserve) return;
		/* -1: a line rebuilt from the map with no depot near it, or one whose depot was refused for
		 * money (KeepDepotlessLine), which gets it once the bank holds the depot and the train. */
		if (line.depot == -1) {
			if (!("depot_pending" in line)) continue;
			local need = reserve + PrismaticAI.FirstTrainPrice(line) + AIRail.GetBuildCost(line.railtype, AIRail.BT_DEPOT);
			if (AICompany.GetBankBalance(AICompany.COMPANY_SELF) < need || !PrismaticAI.BuildPendingDepot(line)) continue;
		}
		/* A line that never ran a train (KeepTrainlessLine, AdoptTrainlessLines) waits until the bank
		 * holds the train's price above the reserve, so it is not refused for money every heartbeat;
		 * its first train is no replacement. */
		local first_train = !("first" in line);
		/* test_trainless=2 (RESEARCH-ONLY): a refused line waits for TestForgetTrainless and the adoption. */
		if (first_train && ("test_refused" in line) && !("adopted" in line) && AIController.GetSetting("test_trainless") >= 2) continue;
		if (first_train && AICompany.GetBankBalance(AICompany.COMPANY_SELF) < reserve + PrismaticAI.FirstTrainPrice(line)) continue;
		local train = PrismaticAI.BuildTrain(line);
		if (train == -1) continue;
		if (first_train) {
			AILog.Info("PrismaticAI: line " + line.label + " got its first train (" + AIEngine.GetName(AIVehicle.GetEngineType(train)) + ")");
			AILog.Info(PrismaticAI.LineRecord(i + 1, line));
			continue;
		}
		line.restored++;
		AILog.Info("PrismaticAI: line " + line.label + " had no train; a new one runs (" + AIEngine.GetName(AIVehicle.GetEngineType(train)) +
			", replacement " + line.restored + " of 2)");
	}
}

/* The rail type a new line is built in: with upgrade_track, the one whose best locomotive for
 * `cargo` is best (infra.nut BestRailType); else, or when none is found, the first available. */
function PrismaticAI::UseFirstRailType(cargo = -1) {
	local types = AIRailTypeList();
	if (types.IsEmpty()) { AILog.Warning("PrismaticAI: no rail type available"); return false; }
	local rt = (RICH_NOW && AIController.GetSetting("upgrade_track") == 1) ? PrismaticAI.BestRailType(cargo) : AIRail.RAILTYPE_INVALID;
	AIRail.SetCurrentRailType(rt != AIRail.RAILTYPE_INVALID ? rt : types.Begin());
	return true;
}

/* The rail settings a route is built with, from Behaviour(): platforms per terminus and the
 * overrides of the derived platform length and wagons (0: derive). */
function PrismaticAI::RailShape(b, own = false) {
	/* rail_double applies to the bot's own freight lines only (main.nut passes own = true there). */
	return {platforms = max(1, b.rail_platforms), len = b.rail_len, wagons = b.rail_wagons,
		double = own && b.rail_double == 1};
}

function PrismaticAI::BuildRailRoute(a, b, shape) {
	if (!AITown.IsValidTown(a) || !AITown.IsValidTown(b) || a == b) {
		AILog.Warning("PrismaticAI: rail route needs two different towns (rail_a=" + a + ", rail_b=" + b + ")");
		return 0;
	}
	if (!PrismaticAI.UseFirstRailType()) return 0;
	PrismaticAI.Funnel("rail", "tried");
	local pax = PrismaticAI.PassengerCargo();
	local dist = AIMap.DistanceManhattan(AITown.GetLocation(a), AITown.GetLocation(b));
	AILog.Info("PrismaticAI: planner pair by rail: " + AITown.GetName(a) + " - " + AITown.GetName(b) + " (" + dist + " tiles)");
	/* A station near the centre catches about a third of what a town produces; the smaller town
	 * bounds what flows both ways. */
	local supply = min(AITown.GetLastMonthProduction(a, pax), AITown.GetLastMonthProduction(b, pax)) / 3;
	local plan = PrismaticAI.PlatformPlan(pax, supply, dist, shape.platforms, shape.len, shape.wagons);
	AILog.Info("PrismaticAI: rail plan " + AITown.GetName(a) + " - " + AITown.GetName(b) + ": " + plan.why);
	local sa = PrismaticAI.BuildRailStationAt(AITown.GetLocation(a), AITown.GetLocation(b), plan.len, pax, "accept",
		"in " + AITown.GetName(a) + " (axis toward " + AITown.GetName(b) + ")", shape.platforms);
	if (sa == null) return 0;
	local sb = PrismaticAI.BuildRailStationAt(AITown.GetLocation(b), AITown.GetLocation(a), plan.len, pax, "accept",
		"in " + AITown.GetName(b) + " (axis toward " + AITown.GetName(a) + ")", shape.platforms);
	if (sb == null) { PrismaticAI.RemoveRailStation(sa, plan.len); return 0; }
	return PrismaticAI.BuildRailLineAndTrain(sa, sb, plan.len, pax, plan.wagons, AIOrder.OF_NONE, AITown.GetName(a) + " - " + AITown.GetName(b));
}

/* Freight by rail: from the producing industry `p` to the accepting industry `q` for `cargo`, the
 * planner's portfolio pair (settings fr_from, fr_to, fr_cargo) or the bot's own. The train waits
 * for a full load at the producer. */
function PrismaticAI::BuildFreightRailRoute(p, q, cargo, shape) {
	if (!AIIndustry.IsValidIndustry(p) || !AIIndustry.IsValidIndustry(q) || p == q || !AICargo.IsValidCargo(cargo)) {
		AILog.Warning("PrismaticAI: freight route needs two industries and a cargo (fr_from=" + p + ", fr_to=" + q + ", fr_cargo=" + cargo + ")");
		return 0;
	}
	if (!PrismaticAI.UseFirstRailType(cargo)) return 0;
	/* A double track retried single (built == -1 below) is a second try: new stations, a new search. */
	PrismaticAI.Funnel("rail", "tried");
	local pl = AIIndustry.GetLocation(p), ql = AIIndustry.GetLocation(q);
	local label = AICargo.GetCargoLabel(cargo) + " " + AIIndustry.GetName(p) + " - " + AIIndustry.GetName(q);
	local dist = AIMap.DistanceManhattan(pl, ql);
	AILog.Info("PrismaticAI: planner freight by rail: " + label + " (" + dist + " tiles)");
	/* The pre-check, right before the source station is built, so it holds for the planner's lines
	 * (fr_*), the bot's own and the steward's alike, and for a rival that claimed the producer
	 * after the line was chosen. */
	local holder = PrismaticAI.ForeignRailStationAt(p);
	if (holder != null) {
		AILog.Warning("PrismaticAI: freight line " + label + " skipped: company " + (holder + 1) +
			" already has a rail station claiming " + AIIndustry.GetName(p));
		return 0;
	}
	PrismaticAI.TestClaimRace();
	/* What a served station gets of the production: the station rating, about 70% once trains call
	 * regularly. */
	local supply = AIIndustry.GetLastMonthProduction(p, cargo) * 70 / 100;
	local double = ("double" in shape) && shape.double;
	/* Double track: two platforms at each end (the junction joins two), trains as long as the platform. */
	local platforms = double ? 2 : shape.platforms;
	local plan = PrismaticAI.PlatformPlan(cargo, supply, dist, platforms, shape.len, shape.wagons);
	AILog.Info("PrismaticAI: rail plan " + label + ": " + plan.why + (double ? ", double track" : ""));
	/* An acceptor remembered without a site fails here, before the source station is built and removed. */
	if (PrismaticAI.NoRailSiteKnown(ql, "accept", cargo, plan.len, platforms, double ? -1 : 0, "at " + AIIndustry.GetName(q))) return 0;
	local sa = PrismaticAI.BuildRailStationAt(pl, ql, plan.len, cargo, "produce", "at " + AIIndustry.GetName(p), platforms, double ? 1 : 0);
	if (sa == null) return 0;
	/* Two companies that chose one producer in the same days both pass the pre-check: in the fitness
	 * check's sibling match (seed 930967890) both PrismaticAI companies picked Wontford Coal Mine in
	 * their first heartbeat, the second's track ran into the first's and its line failed. Two checks
	 * settle it, both counting only claims made since the pre-check (fresh tiles):
	 * - check 1, here, after the source station: a lower company id wins, before any more is built.
	 *   That settles the common race, where both start in one heartbeat: the lower id runs first in
	 *   every tick, takes the first site of the same survey and builds first.
	 * - check 2 (ClaimBlocks), after the destination station: a lower id still wins, and a higher one
	 *   that still stands after a short settle wins too, because it kept its station at its own
	 *   check 1, which then came before ours was built.
	 * With check 1 alone, the company of the higher id could finish its station and check before the
	 * other's site search ended, see nothing and keep it, while the lower one then saw a higher rival
	 * and kept its own too (review, 2026-11-23). Now two lines are never both kept: of two keepers,
	 * the one whose check 2 came later saw the other's source station standing and yielded. Both
	 * yield only when one's check 2 falls in the days the other takes to remove a station it gave
	 * up; then nobody builds there. A 10-day wait before check 1 also closed the window, but delayed
	 * every line: on the fitness seed the six PrismaticAI lineups summed 7.87M with it and 9.36M
	 * without. These checks cost nothing without a rival: a scan of the tiles around the producer. */
	local me = AICompany.ResolveCompanyID(AICompany.COMPANY_SELF);
	local rival = PrismaticAI.ForeignRailStationAt(p, true);
	if (rival != null && rival < me) {
		AILog.Warning("PrismaticAI: freight line " + label + " given up: company " + (rival + 1) +
			" built a rail station at " + AIIndustry.GetName(p) + " in the same days");
		PrismaticAI.RemoveRailStation(sa, plan.len);
		return 0;
	}
	local sb = PrismaticAI.BuildRailStationAt(ql, pl, plan.len, cargo, "accept", "at " + AIIndustry.GetName(q), platforms, double ? -1 : 0);
	if (sb == null) { PrismaticAI.RemoveRailStation(sa, plan.len); return 0; }
	if (PrismaticAI.ClaimBlocks(p, label)) {
		PrismaticAI.RemoveRailStation(sa, plan.len); PrismaticAI.RemoveRailStation(sb, plan.len);
		return 0;
	}
	local built = double ? PrismaticAI.BuildDoubleRailLineAndTrain(sa, sb, plan.len, cargo, AIOrder.OF_FULL_LOAD_ANY, label)
		: PrismaticAI.BuildRailLineAndTrain(sa, sb, plan.len, cargo, plan.wagons, AIOrder.OF_FULL_LOAD_ANY, label);
	/* No double-track path: the same line single track. On a hilly map (OpenTTD seed 1807087905)
	 * every double-track search stopped after 5 nodes and the bot ended last with a few buses, while
	 * the single-track line on the same pair was built and ran 7 trains. */
	if (built == -1) {
		AILog.Info("PrismaticAI: no double-track path for " + label + "; trying it single track");
		shape = clone shape;
		shape.double = false;
		return PrismaticAI.BuildFreightRailRoute(p, q, cargo, shape);
	}
	/* The industries it serves, so a saturated line can get a twin (steward.nut TwinLines). */
	if (built > 0) { RAIL_LINES.top().from <- p; RAIL_LINES.top().to <- q; }
	return built;
}

/* The bot's own next freight line: the best untried pair of a producing industry nobody serves yet
 * (no cargo transported last month, no station of ours near it, no claim of another company's
 * rail station on it: ForeignRailStationAt) and an industry accepting that cargo
 * 100 to 260 tiles away, the distance the planner's lines run. Scored by what the game pays for the
 * cargo (AICargo.GetCargoIncome) on that distance and transit time, per day of a round trip, times
 * last month's production. `tried` holds the "from-to-cargo" keys already attempted. Returns
 * {from, to, cargo, distance, production, score} or null.
 * An industry built on water (an oil rig) has no land for a rail station. Its large output put it
 * first, and every attempt ended at "no rail station site", yet still counted against own_lines and
 * own_fail_pause: on OpenTTD seed 930967890 its own plans 9 to 12 of 32 were all oil rigs.
 * With `rich` (rich.nut), a producer others serve is a candidate while under RICH_SERVED_BELOW
 * percent of its output was transported, scored on the part left, and acceptors may be
 * RICH_MIN_TILES to RICH_MAX_TILES away: with eight companies on the owner's map no unserved
 * producer was left after 1958, and the bot held 3.7M for years. */
function PrismaticAI::NextOwnFreightLine(tried, feed = 0, rich = false) {
	local best = null, best_score = 0;
	/* Transit time at the speed of a typical early steam engine: about 100 km/h, some 3.7 tiles a day. */
	local tiles_a_day = 3.7;
	local near = rich ? RICH_MIN_TILES : 100, far = rich ? RICH_MAX_TILES : 260;
	/* Why nothing was found, said once a game year (NO_OWN_LINE_YEAR): without it a bot that found
	 * no candidate went silent, as company 0 of the owner's live game (seed 15805) did for years. */
	local why = {producers = 0, ours = 0, claimed = 0, released = 0, small = 0, served = 0, no_acceptor = 0, tried = 0, no_site = 0};
	PrismaticAI.ForgetGoneClaims();
	foreach (p, _ in AIIndustryList()) {
		if (AIIndustry.IsBuiltOnWater(p)) continue;
		why.producers++;
		if (PrismaticAI.OwnRailStationNear(AIIndustry.GetLocation(p), 8) != null) { why.ours++; continue; }
		/* Another company's rail station claims it, in every mode (ForeignRailStationAt); one that only
		 * catches it does not, and is counted as released. */
		if (PrismaticAI.ForeignRailStationAt(p) != null) { why.claimed++; continue; }
		if (CLAIM_LAST_LAPSED > 0) why.released++;
		foreach (c, _ in AICargoList_IndustryProducing(p)) {
			if (!AICargo.IsFreight(c)) continue;
			local prod = AIIndustry.GetLastMonthProduction(p, c);
			if (prod < 40) { why.small++; continue; }
			if (PrismaticAI.NoRailSiteFor(AIIndustry.GetLocation(p), "produce", c)) { why.no_site++; continue; }
			if (AIIndustry.GetLastMonthTransported(p, c) > 0) {
				if (!rich) { why.served++; continue; }
				local served = AIIndustry.GetLastMonthTransportedPercentage(p, c);
				if (served >= RICH_SERVED_BELOW) { why.served++; continue; }
				prod = prod * (100 - served) / 100;
				if (prod < 40) continue;
			}
			local accs = AIIndustryList_CargoAccepting(c);
			accs.Valuate(AIIndustry.GetDistanceManhattanToTile, AIIndustry.GetLocation(p));
			accs.KeepBetweenValue(near - 1, far + 1);
			if (accs.IsEmpty()) why.no_acceptor++;
			foreach (q, dist in accs) {
				if (tried.rawin(p + "-" + q + "-" + c)) { why.tried++; continue; }
				if (PrismaticAI.NoRailSiteFor(AIIndustry.GetLocation(q), "accept", c)) { why.no_site++; continue; }
				local days = (dist / tiles_a_day).tointeger() + 1;
				local income = AICargo.GetCargoIncome(c, dist, days);
				/* Multi-source (rail_feed): the unserved producers of the same cargo a station at p
				 * can join (growth.nut ClusterProduction) count towards the line's supply. */
				local supply = prod + ((feed > 0) ? PrismaticAI.ClusterProduction(p, c, feed) : 0);
				local score = (supply * income / (2 * days + 10)).tointeger();
				if (score > best_score) {
					best_score = score;
					best = {from = p, to = q, cargo = c, distance = dist, production = prod, score = score};
				}
			}
		}
	}
	PrismaticAI.FunnelCensus(why, best != null);
	local year = AIDate.GetYear(AIDate.GetCurrentDate());
	if (best == null && NO_OWN_LINE_YEAR != year) {
		NO_OWN_LINE_YEAR = year;
		AILog.Info("PrismaticAI: no own freight line: " + why.producers + " producers on land, " + why.ours +
			" ours, " + why.claimed + " claimed by another company's rail station, " + why.released +
			" with only another company's rail station that no longer claims it, cargoes: " + why.small +
			" under 40 a month, " + why.served + " transported by others" + (rich ? " (rich: past RICH_SERVED_BELOW)" : "") +
			", " + why.no_acceptor + " with no acceptor " + near + "-" + far + " tiles away, " + why.tried + " pairs tried, " +
			why.no_site + " passed over for an end remembered without a station site");
	}
	return best;
}

/* The game year NextOwnFreightLine last said why it found nothing. */
NO_OWN_LINE_YEAR <- -1;
