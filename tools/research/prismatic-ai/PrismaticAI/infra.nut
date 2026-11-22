/*
 * Track that keeps up (RESEARCH-ONLY, setting upgrade_track=1): the owner saw other AIs upgrade
 * their track and trains while this bot ran plain RAIL for the whole game. Every line was laid in
 * the first rail type of AIRailTypeList and nothing ever called AIRail.ConvertRailType, so the
 * engines of a better type (electric on ELRL) never had a track to run on and modernize.nut's
 * autoreplace never found them.
 *
 * - New lines: BestRailType, the available type whose best locomotive (PickRailEngines) is the
 *   fastest at the track's own speed limit (AIRail.GetMaxSpeed, 0: none), ties to the more powerful,
 *   then to the cheaper track. Before electric engines exist RAIL and ELRL tie and RAIL wins.
 * - Existing lines (UpgradeTrack, at most one network a call): a line converts to `rt` only when
 *   (1) the engines of its type have power on rt (AIRail.TrainHasPowerOnRail; vanilla: RAIL engines
 *   on ELRL yes, anything on MONO/MAGLEV no), so the game converts it as an upgrade (CmdConvertRail:
 *   compatible types need no empty track, rail.h RailConvertCost charges 1/8 of the new track plus
 *   the price difference) and never as a rebuild under running trains;
 *   (2) every locomotive the company runs keeps power on rt and every wagon can run on it, so
 *   nothing stalls, whatever the conversion touches;
 *   (3) rt's best locomotive for the line's cargo is strictly better (speed, then power) than the
 *   best on the line's current type.
 * - What is converted is the line's whole rail network, not its path: every own rail tile
 *   connected to its stations and depot (flood over own tiles that carry rail, bridges and tunnels
 *   jumped to their other end). A restored line (restore.nut, path = []) has no tile list, and a
 *   tile missed on the way (a throat, a double track's junction) would stop an electric train dead,
 *   since an electric engine runs on plain rail without power. Station tiles, depots, level
 *   crossings and bridge/tunnel heads all convert (CmdConvertRail cases Station, Road crossing,
 *   TunnelBridge; a bridge or tunnel head converts both ends). Other companies' tiles are skipped.
 * - The cost is measured first (AITestMode + AIAccounting) and the network is converted only when
 *   the bank holds that plus track_keep_budgets line budgets (AI setting, info.nut; default 2, the
 *   margin this was first built with). Afterwards every tile is checked; only then does
 *   every line on that network get line.railtype = rt (modernize.nut and BuildTrain buy for it). A
 *   network left partly converted keeps its old railtype (its trains have power on both) and the
 *   line is not tried for rt again that game year (track_fail).
 */

/* Tiles a network flood takes at most: far more than any line the bot builds. */
const INFRA_FLOOD_MAX = 20000;

/* `loco`'s speed on track `rt`: its own top speed, capped by the track's limit (0: none). */
function PrismaticAI::LocoSpeedOn(loco, rt) {
	local speed = AIEngine.GetMaxSpeed(loco);
	local cap = AIRail.GetMaxSpeed(rt);
	return (cap > 0) ? min(speed, cap) : speed;
}

/* Whether locomotive `a` on `rt_a` beats `b` on `rt_b`: faster, then more powerful. */
function PrismaticAI::LocoBetter(a, rt_a, b, rt_b) {
	if (b == -1) return a != -1;
	if (a == -1) return false;
	local sa = PrismaticAI.LocoSpeedOn(a, rt_a), sb = PrismaticAI.LocoSpeedOn(b, rt_b);
	if (sa != sb) return sa > sb;
	return AIEngine.GetPower(a) > AIEngine.GetPower(b);
}

/* The rail type new lines are built in: the available type with the best locomotive (LocoBetter),
 * ties to the cheaper track. With `cargo` (>= 0) only types with a wagon for it count; without, any
 * buildable wagon that runs there. AIRail.RAILTYPE_INVALID when no type has a locomotive. */
function PrismaticAI::BestRailType(cargo = -1) {
	local best = AIRail.RAILTYPE_INVALID, best_loco = -1;
	foreach (rt, _ in AIRailTypeList()) {
		local loco = -1;
		if (cargo >= 0) {
			local e = PrismaticAI.PickRailEngines(rt, cargo);
			if (e.wagon == -1) continue;
			loco = e.loco;
		} else {
			loco = PrismaticAI.PickRailEngines(rt, -1).loco;
			if (!PrismaticAI.AnyWagonOn(rt)) continue;
		}
		if (loco == -1) continue;
		if (best == AIRail.RAILTYPE_INVALID || PrismaticAI.LocoBetter(loco, rt, best_loco, best)) {
			best = rt; best_loco = loco;
		} else if (!PrismaticAI.LocoBetter(best_loco, best, loco, rt) &&
				AIRail.GetBuildCost(rt, AIRail.BT_TRACK) < AIRail.GetBuildCost(best, AIRail.BT_TRACK)) {
			best = rt; best_loco = loco;
		}
	}
	return best;
}

/* Whether some buildable wagon runs on `rt`. */
function PrismaticAI::AnyWagonOn(rt) {
	foreach (e, _ in AIEngineList(AIVehicle.VT_RAIL)) {
		if (AIEngine.IsBuildable(e) && AIEngine.IsWagon(e) && AIEngine.CanRunOnRail(e, rt)) return true;
	}
	return false;
}

/* Whether every train of the company keeps going on track `rt`: each locomotive has power there,
 * each wagon can run there. Company-wide, because the flood may join networks of several lines. */
function PrismaticAI::FleetKeepsPowerOn(rt) {
	foreach (v, _ in AIVehicleList()) {
		if (AIVehicle.GetVehicleType(v) != AIVehicle.VT_RAIL) continue;
		for (local i = 0; i < AIVehicle.GetNumWagons(v); i++) {
			local e = AIVehicle.GetWagonEngineType(v, i);
			if (!AIEngine.IsValidEngine(e)) continue;
			if (AIEngine.IsWagon(e) ? !AIEngine.CanRunOnRail(e, rt) : !AIEngine.HasPowerOnRail(e, rt)) return false;
		}
	}
	return true;
}

/* Every own rail tile connected to `seeds`: {tiles, set, single, skip}. `single` are the tiles
 * converted one by one (level crossings, bridge and tunnel heads), `skip` the far ends of bridges
 * and tunnels (their near end converts both; converting both would charge the test twice and fail
 * the second execution). Returns null when the flood passes INFRA_FLOOD_MAX. */
function PrismaticAI::RailNetwork(seeds) {
	local map_w = AIMap.GetMapSizeX();
	local set = {}, single = {}, skip = {}, tiles = [], todo = [];
	/* A restored line may have no depot (-1, restore.nut RestoreDepot). */
	foreach (s in seeds) if (s != null && s >= 0) todo.push(s);
	while (todo.len() > 0) {
		local t = todo.pop();
		if (set.rawin(t) || !AIMap.IsValidTile(t)) continue;
		if (!AITile.HasTransportType(t, AITile.TRANSPORT_RAIL) || !AICompany.IsMine(AITile.GetOwner(t))) continue;
		set[t] <- true;
		tiles.push(t);
		if (tiles.len() > INFRA_FLOOD_MAX) return null;
		local other = -1;
		if (AIBridge.IsBridgeTile(t)) other = AIBridge.GetOtherBridgeEnd(t);
		else if (AITunnel.IsTunnelTile(t)) other = AITunnel.GetOtherTunnelEnd(t);
		if (other != -1 && AIMap.IsValidTile(other)) {
			single[t] <- true;
			if (!set.rawin(other)) {
				/* The end reached second is converted with the first. */
				skip[other] <- true;
				todo.push(other);
			}
		}
		if (AIRail.IsLevelCrossingTile(t)) single[t] <- true;
		foreach (n in [t - 1, t + 1, t - map_w, t + map_w]) if (n >= 0 && !set.rawin(n)) todo.push(n);
	}
	return {tiles = tiles, set = set, single = single, skip = skip};
}

/* The rectangles ConvertRailType takes for the tiles of `net` not yet of type `rt`: runs along x,
 * then the leftovers along y, singles alone. Each rectangle holds network tiles only. */
function PrismaticAI::ConvertRuns(net, rt) {
	local map_w = AIMap.GetMapSizeX();
	local want = {}, list = [];
	foreach (t in net.tiles) {
		if (net.skip.rawin(t) || AIRail.GetRailType(t) == rt) continue;
		want[t] <- true;
		list.push(t);
	}
	list.sort();
	local runs = [], used = {}, left = [];
	foreach (t in list) {
		if (used.rawin(t)) continue;
		if (net.single.rawin(t)) { runs.push([t, t]); used[t] <- true; continue; }
		local e = t;
		while (want.rawin(e + 1) && !net.single.rawin(e + 1) && !used.rawin(e + 1) &&
				AIMap.GetTileY(e + 1) == AIMap.GetTileY(t)) e++;
		if (e == t) { left.push(t); continue; }
		for (local k = t; k <= e; k++) used[k] <- true;
		runs.push([t, e]);
	}
	foreach (t in left) {
		if (used.rawin(t)) continue;
		local e = t;
		used[t] <- true;
		while (want.rawin(e + map_w) && !net.single.rawin(e + map_w) && !used.rawin(e + map_w)) {
			e += map_w;
			used[e] <- true;
		}
		runs.push([t, e]);
	}
	return runs;
}

/* What converting `runs` to `rt` costs, from the game's own test pass: {cost, why}, cost -1 and
 * the game's reason when a run would fail. */
function PrismaticAI::ConvertCost(runs, rt) {
	local mode = AITestMode();
	local acc = AIAccounting();
	foreach (r in runs) {
		if (!AIRail.ConvertRailType(r[0], r[1], rt)) {
			return {cost = -1, why = "tiles " + r[0] + ".." + r[1] + " would not convert: " + AIError.GetLastErrorString()};
		}
	}
	return {cost = acc.GetCosts(), why = ""};
}

/* PickRailEngines once per rail type and cargo within one call of UpgradeTrack. */
function PrismaticAI::EnginesMemo(memo, rt, cargo) {
	local key = rt + "/" + cargo;
	if (!memo.rawin(key)) memo[key] <- PrismaticAI.PickRailEngines(rt, cargo);
	return memo[key];
}

/* The rail type `line` should run, or RAILTYPE_INVALID: conditions (1) to (3) of the head. A type
 * whose conversion failed this game year (track_fail "<rt>/<year>") waits for the next. */
function PrismaticAI::UpgradeTarget(line, memo, year) {
	local cur = line.railtype;
	if (!AIRail.IsRailTypeAvailable(cur)) return AIRail.RAILTYPE_INVALID;
	local best = AIRail.RAILTYPE_INVALID, best_loco = PrismaticAI.EnginesMemo(memo, cur, line.cargo).loco, best_rt = cur;
	foreach (rt, _ in AIRailTypeList()) {
		if (rt == cur || (("track_fail" in line) && line.track_fail == rt + "/" + year)) continue;
		if (!AIRail.TrainHasPowerOnRail(cur, rt)) continue;
		local e = PrismaticAI.EnginesMemo(memo, rt, line.cargo);
		if (e.loco == -1 || e.wagon == -1) continue;
		if (!PrismaticAI.LocoBetter(e.loco, rt, best_loco, best_rt)) continue;
		if (!PrismaticAI.FleetKeepsPowerOn(rt)) continue;
		best = rt; best_loco = e.loco; best_rt = rt;
	}
	return best;
}

/* A rail type's name for the log. */
function PrismaticAI::RailTypeLabel(rt) {
	local name = AIRail.GetName(rt);
	return (name == null) ? ("railtype " + rt) : name;
}

/* Log `msg` for `line` once per `note` (rail type and year), so a line that waits says so once a
 * year, not every month. */
function PrismaticAI::TrackNote(line, note, msg) {
	if (("track_note" in line) && line.track_note == note) return;
	line.track_note <- note;
	AILog.Info("PrismaticAI: upgrade track: line " + line.label + ": " + msg);
}

/* Once a game month (b.upgrade_track == 1, construction not frozen): convert the network of the
 * first line that has a better rail type (UpgradeTarget) and whose conversion the bank can pay
 * with b.track_keep_budgets line budgets to spare. At most one network a call. Returns true when
 * one converted. */
function PrismaticAI::UpgradeTrack(b) {
	local margin = b.track_keep_budgets * b.line_budget;
	local bank = AICompany.GetBankBalance(AICompany.COMPANY_SELF);
	if (bank <= margin) return false;
	local year = AIDate.GetYear(AIDate.GetCurrentDate());
	local memo = {};
	foreach (i, line in RAIL_LINES) {
		if (line.closed) continue;
		local rt = PrismaticAI.UpgradeTarget(line, memo, year);
		if (rt == AIRail.RAILTYPE_INVALID) continue;
		local to = PrismaticAI.RailTypeLabel(rt), note = rt + "/" + year;
		local seeds = [line.a, line.b, line.depot];
		if ("path" in line) seeds.extend(line.path);
		if ("path2" in line) seeds.extend(line.path2);
		local net = PrismaticAI.RailNetwork(seeds);
		if (net == null) { PrismaticAI.TrackNote(line, note, "network over " + INFRA_FLOOD_MAX + " tiles; not converted"); continue; }
		if (net.tiles.len() == 0) { PrismaticAI.TrackNote(line, note, "no own rail at its stations or depot; not converted"); continue; }
		local runs = PrismaticAI.ConvertRuns(net, rt);
		if (runs.len() == 0) continue;
		local test = PrismaticAI.ConvertCost(runs, rt);
		if (test.cost < 0) { line.track_fail <- note; PrismaticAI.TrackNote(line, note, "-> " + to + " refused: " + test.why); continue; }
		if (bank < test.cost + margin) {
			PrismaticAI.TrackNote(line, note, "-> " + to + " costs " + test.cost + " (" + net.tiles.len() + " tiles); waits for " +
				(test.cost + margin) + " in the bank (has " + bank + ")");
			continue;
		}
		local acc = AIAccounting();
		foreach (r in runs) {
			if (!AIRail.ConvertRailType(r[0], r[1], rt)) {
				AILog.Warning("PrismaticAI: upgrade track: tiles " + r[0] + ".." + r[1] + " not converted: " + AIError.GetLastErrorString());
			}
		}
		local spent = acc.GetCosts();
		local missed = -1;
		foreach (t in net.tiles) if (AIRail.GetRailType(t) != rt) { missed = t; break; }
		if (missed != -1) {
			/* Its trains have power on both types: the line keeps its railtype and runs on. */
			line.track_fail <- note;
			AILog.Warning("PrismaticAI: upgrade track: line " + line.label + " -> " + to + " left tile " + missed +
				" unconverted (spent " + spent + "); it keeps " + PrismaticAI.RailTypeLabel(line.railtype) + " this year");
			return false;
		}
		/* Every line on this network runs rt now (a joined station joins their networks). */
		foreach (other in RAIL_LINES) {
			if (other.closed || other.railtype == rt || !net.set.rawin(other.a)) continue;
			AILog.Info("PrismaticAI: upgrade track: line " + other.label + " " + PrismaticAI.RailTypeLabel(other.railtype) + " -> " + to +
				", cost " + spent + " (" + net.tiles.len() + " tiles, " + runs.len() + " conversions)");
			other.railtype = rt;
		}
		return true;
	}
	return false;
}
