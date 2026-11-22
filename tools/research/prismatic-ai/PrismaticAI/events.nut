/*
 * The game's events and the vehicles they name (RESEARCH-ONLY). Until 2026-11-22 the bot's only
 * event consumer was modernize.nut AcceptPreviews, which ran only with modernize=1 on idle money
 * and dropped every other event unread. In the owner's co-play game the news showed trains that
 * "can't find a path to continue" and "is lost" again and again, a train whose profit last year was
 * -2302, and a train and earlier a bus that stood in their depots with no orders, bought and never
 * used. The bot never saw any of it.
 *
 * HandleEvents drains AIEventController once a heartbeat (main.nut, before ManageFleet):
 * - ET_VEHICLE_LOST. A train's line is checked with the builders' own checks: VerifyRailLine lays
 *   a missing piece of a recorded track again, RailWalk follows the track the game has from one
 *   station to the other (power included, so a tile of a rail type the locomotive has no power on
 *   is converted to the line's type), and RepairDepot joins the depot to the line again or builds a
 *   new one where the line's builders would. A line that cannot be repaired is marked `broken`: the
 *   train is sent to be sold (the fleet's selling path, fleet.nut; one that cannot reach a depot is
 *   stopped where it stands, SellOrPark), KeepLinesServed and
 *   ReconcileLines put no train on it, and once a month LookAfterVehicles walks it again. On a whole
 *   single-track line the train is turned around (a train facing a dead end is lost on a good line);
 *   one that reaches no station within LOST_SELL_DAYS, or gets lost again, is sold. When two trains
 *   of a whole line are lost at once, or one stays lost that long, the line takes no new train
 *   (`lost_trains`, LineHalted) until one of them reaches a station. A road vehicle
 *   whose stops no road joins (RoadConnected, roadstop.nut) is sold; other types are watched.
 * - ET_VEHICLE_UNPROFITABLE. The same route check first: a broken line or road can never earn, so
 *   its vehicle is sold at once. A whole one is left to the rules that already judge losses: the
 *   fleet sells a vehicle over two years old that lost money last year and this year (ManageFleet)
 *   and the steward closes or shrinks a losing line (steward.nut SelfSteward). Each is logged.
 * - ET_ENGINE_PREVIEW. Kept for AcceptPreviews (modernize.nut), which accepts them as before, only
 *   while modernize=1 and money is idle.
 * Once a game month LookAfterVehicles finds vehicles stopped in a depot with no orders. One seen a
 * month earlier too (the owner may be giving it orders meanwhile) gets the orders of the line or
 * route served from that depot, the train its wagons first (lines.nut ResizeTrain); a vehicle no
 * line or route there can use is sold. A half-built train (a save while BuildTrain was buying its
 * wagons) is the usual cause: a restart forgets it.
 * Every decision logs one AILog line, "PrismaticAI: lost: ...", "PrismaticAI: unprofitable: ..." or
 * "PrismaticAI: no orders: ...", so the panel's ai.log in DuckDB holds the reason.
 *
 * The game raises no event for a train that "can't find a path to continue" (train_cmd.cpp
 * TrainStuck): it waits for a path reservation out of a station or depot and is not lost.
 */

/* Days a lost vehicle has to reach a station of its orders before it is sold. */
const LOST_SELL_DAYS = 90;
/* Track states RailWalk visits at most: more than any line the bot builds holds. */
const RAIL_WALK_MAX = 20000;
/* Road tiles RoadConnected visits at most when it checks a lost or losing vehicle's stops. */
const ROAD_CHECK_MAX = 20000;
/* Engine previews kept for AcceptPreviews (modernize.nut) at most; older ones are dropped. */
const PREVIEWS_KEPT = 16;

/* The previews AcceptPreviews (modernize.nut) has not taken yet. */
PENDING_PREVIEWS <- [];

/* One pass over the event queue (see the head of this file). */
function PrismaticAI::HandleEvents(b) {
	local lost = {}, losing = {};
	while (AIEventController.IsEventWaiting()) {
		local e = AIEventController.GetNextEvent();
		if (e == null) continue;
		local type = e.GetEventType();
		if (type == AIEvent.ET_ENGINE_PREVIEW) {
			PENDING_PREVIEWS.push(AIEventEnginePreview.Convert(e));
			if (PENDING_PREVIEWS.len() > PREVIEWS_KEPT) PENDING_PREVIEWS.remove(0);
		} else if (type == AIEvent.ET_VEHICLE_LOST) {
			lost[AIEventVehicleLost.Convert(e).GetVehicleID()] <- true;
		} else if (type == AIEvent.ET_VEHICLE_UNPROFITABLE) {
			losing[AIEventVehicleUnprofitable.Convert(e).GetVehicleID()] <- true;
		}
	}
	/* One check per line in this pass, however many of its trains the events name. */
	local checked = {};
	foreach (v, _ in lost) this.OnVehicleLost(v, checked);
	foreach (v, _ in losing) if (!lost.rawin(v)) this.OnVehicleUnprofitable(v, checked);
	this.FollowLost();
}

/* Send `v` to its depot to be sold there (ManageFleet sells it once it stands in the depot), and
 * log `msg`. The fleet's one selling path: fleet.nut, lines.nut and this file all use it. Returns
 * true when it is on its way (or was already). */
function PrismaticAI::SendToSell(v, msg) {
	if (this.selling.rawin(v)) return true;
	if (!AIVehicle.SendVehicleToDepot(v)) return false;
	this.selling[v] <- true;
	PrismaticAI.ProtectFromReplace(v); /* autoreplace would give it a new id there (modernize.nut) */
	AILog.Info(msg);
	return true;
}

/* The slot index (0-based) in RAIL_LINES of the open line `train` runs on, or -1. */
function PrismaticAI::LineOfTrain(train) {
	foreach (i, line in RAIL_LINES) {
		if (line.closed) continue;
		foreach (t in PrismaticAI.TrainsOf(line)) if (t == train) return i;
	}
	return -1;
}

/* The line's record as one log phrase. */
function PrismaticAI::LineName(i) {
	return "rail line " + (i + 1) + " (" + RAIL_LINES[i].label + ")";
}

/* Whether `line` takes no new train: its track cannot be repaired (`broken`), or its trains get
 * lost on a track that joins its stations (`lost_trains`, the day it was marked). KeepLinesServed
 * (rail.nut), ReconcileLines (lines.nut) and OrdersFor ask this. */
function PrismaticAI::LineHalted(line) {
	return ("broken" in line) || ("lost_trains" in line);
}

/* Mark line i as one whose trains get lost on a whole track, once, and say why. */
function PrismaticAI::HaltLostLine(i, why) {
	local line = RAIL_LINES[i];
	if ("lost_trains" in line) return;
	line.lost_trains <- AIDate.GetCurrentDate();
	AILog.Warning("PrismaticAI: lost: " + PrismaticAI.LineName(i) + " gets no new train: " + why +
		"; it runs again once one of its lost trains reaches a station");
}

/* Whether the track the game has joins a platform of the station at `from_tile` to one of the
 * station at `to_tile`: a walk over the pieces a train could follow (AIRail.AreTilesConnected),
 * through bridges and tunnels, from every platform tile of the first. A tile whose rail type
 * `loco` (-1: any) has no power on stops the walk there. Signals and the 90-degree rule are not
 * looked at. Returns {joined, unpowered (the first such tile or -1), exhausted (RAIL_WALK_MAX was
 * reached first)}. */
function PrismaticAI::RailWalk(from_tile, to_tile, loco = -1) {
	local out = {joined = false, unpowered = -1, exhausted = false};
	local st_from = AIStation.GetStationID(from_tile), st_to = AIStation.GetStationID(to_tile);
	if (!AIStation.IsValidStation(st_from) || !AIStation.IsValidStation(st_to)) return out;
	local map_w = AIMap.GetMapSizeX();
	local dirs = [1, -1, map_w, -map_w];
	local seen = {}, todo = [];
	/* A state is [p, t]: the walk stands on t, having come from its neighbour p. */
	foreach (s, _ in AITileList_StationType(st_from, AIStation.STATION_TRAIN)) {
		foreach (d in [1, map_w]) {
			if (!AIRail.AreTilesConnected(s - d, s, s + d)) continue;
			todo.push([s, s + d]);
			todo.push([s, s - d]);
		}
	}
	local budget = RAIL_WALK_MAX;
	while (todo.len() > 0) {
		if (budget-- <= 0) { out.exhausted = true; return out; }
		local st = todo.pop();
		local p = st[0], t = st[1];
		if (!AIMap.IsValidTile(t) || !AITile.HasTransportType(t, AITile.TRANSPORT_RAIL)) continue;
		local key = t * 4 + PrismaticAI.SideOf(p - t, map_w);
		if (seen.rawin(key)) continue;
		seen[key] <- true;
		if (loco != -1 && !AIEngine.HasPowerOnRail(loco, AIRail.GetRailType(t))) {
			if (out.unpowered == -1) out.unpowered = t;
			continue;
		}
		if (AIRail.IsRailStationTile(t) && AIStation.GetStationID(t) == st_to) { out.joined = true; return out; }
		local other = -1;
		if (AIBridge.IsBridgeTile(t)) other = AIBridge.GetOtherBridgeEnd(t);
		else if (AITunnel.IsTunnelTile(t)) other = AITunnel.GetOtherTunnelEnd(t);
		if (other != -1) {
			/* Into a head along its axis only; out past the other head the same way. */
			local dir = PrismaticAI.UnitStep(t, other);
			if (t - p == dir) todo.push([other, other + dir]);
			continue;
		}
		if (!AIRail.IsRailTile(t)) continue;
		foreach (d in dirs) {
			local q = t + d;
			if (q != p && AIRail.AreTilesConnected(p, t, q)) todo.push([t, q]);
		}
	}
	return out;
}

/* Whether `depot` is our rail depot and a piece on its front tile leads out of it. */
function PrismaticAI::DepotJoined(depot) {
	if (depot < 0 || !AIRail.IsRailDepotTile(depot) || !AICompany.IsMine(AITile.GetOwner(depot))) return false;
	local front = AIRail.GetRailDepotFrontTile(depot);
	local d = front - depot, side = (d == 1 || d == -1) ? AIMap.GetMapSizeX() : 1;
	foreach (n in [front + d, front + side, front - side]) if (AIRail.AreTilesConnected(depot, front, n)) return true;
	return false;
}

/* Make `line` (slot index i) have a depot its trains can reach: a depot beside the line gets its
 * two joining pieces again (rail.nut BuildRailDepotBeside lays them), and a depot that is gone or
 * stays cut off is replaced where the line's builders would put one (BuildRailDepotBehind, then
 * BuildRailDepotBeside); a line restored from the map, which has no station records, takes the
 * nearest joined depot no other line uses (restore.nut RestoreDepot). Returns true when the line
 * has a joined depot afterwards. */
function PrismaticAI::RepairDepot(i, line) {
	if (PrismaticAI.DepotJoined(line.depot)) return true;
	local old = line.depot;
	if (old >= 0 && AIRail.IsRailDepotTile(old) && AICompany.IsMine(AITile.GetOwner(old))) {
		local front = AIRail.GetRailDepotFrontTile(old);
		local d = front - old, side = (d == 1 || d == -1) ? AIMap.GetMapSizeX() : 1;
		/* Only beside a straight run of plain track, as it was built. */
		if (AIRail.IsRailTile(front) && !AIRail.IsRailStationTile(front) && AIRail.AreTilesConnected(front - side, front, front + side)) {
			AIRail.BuildRail(old, front, front + side);
			AIRail.BuildRail(old, front, front - side);
		}
		if (PrismaticAI.DepotJoined(old)) {
			AILog.Info("PrismaticAI: lost: " + PrismaticAI.LineName(i) + ": depot at tile " + old + " joined to the line again");
			return true;
		}
	}
	local depot = null;
	foreach (st in line.stations) if (depot == null) depot = PrismaticAI.BuildRailDepotBehind(st);
	if (depot == null && line.path.len() > 2) depot = PrismaticAI.BuildRailDepotBeside(line.path);
	if (depot == null && line.stations.len() == 0) {
		local taken = {};
		foreach (other in RAIL_LINES) if (other != line) taken[other.depot] <- true;
		local near = PrismaticAI.RestoreDepot(line.a, line.b, taken);
		if (near != -1 && near != old && !taken.rawin(near) && PrismaticAI.DepotJoined(near)) depot = near;
	}
	if (depot == null) {
		AILog.Warning("PrismaticAI: lost: " + PrismaticAI.LineName(i) + ": depot at tile " + old +
			" is gone or cut off and no new one could be placed: " + AIError.GetLastErrorString());
		return false;
	}
	line.depot = depot;
	AILog.Info("PrismaticAI: lost: " + PrismaticAI.LineName(i) + ": depot at tile " + depot + " replaces the one at tile " + old);
	return true;
}

/* Check line i and repair what can be repaired (see the head of this file): returns null when
 * its trains have a track between its stations afterwards, else why not. `loco` is the engine of
 * the train that asked (-1: the line's own). */
function PrismaticAI::CheckLine(i, line, loco) {
	foreach (tile in [line.a, line.b]) {
		if (!AIRail.IsRailStationTile(tile) || !AIStation.IsValidStation(AIStation.GetStationID(tile))) return "its station at tile " + tile + " is gone";
	}
	local s = line.stations;
	if (line.path.len() > 0 && s.len() == 2 && ("end" in s[0]) && ("end" in s[1])) {
		local cut = PrismaticAI.VerifyRailLine(line.path, s[0].end, s[1].end);
		if (cut != -1) return "its track is cut at tile " + cut + " and the piece could not be laid again (" + AIError.GetLastErrorString() + ")";
	}
	if (("path2" in line) && line.path2.len() > 0 && s.len() == 2 && ("dbl" in s[0]) && ("dbl" in s[1])) {
		local cut = PrismaticAI.VerifyRailLine(line.path2, s[0].dbl.s1, s[1].dbl.s1);
		if (cut != -1) return "its second track is cut at tile " + cut + " and the piece could not be laid again (" + AIError.GetLastErrorString() + ")";
	}
	if (loco == -1) loco = PrismaticAI.PickRailEngines(line.railtype, line.cargo).loco;
	local walk = PrismaticAI.RailWalk(line.a, line.b, loco);
	if (!walk.joined && walk.unpowered != -1 && loco != -1 && AIEngine.HasPowerOnRail(loco, line.railtype) &&
			AIRail.ConvertRailType(walk.unpowered, walk.unpowered, line.railtype)) {
		AILog.Info("PrismaticAI: lost: " + PrismaticAI.LineName(i) + ": tile " + walk.unpowered + " converted to " +
			PrismaticAI.RailTypeLabel(line.railtype) + ", where its trains had no power");
		walk = PrismaticAI.RailWalk(line.a, line.b, loco);
	}
	/* A walk that ran out of budget proves nothing either way: the line is taken as whole. */
	if (!walk.joined && !walk.exhausted) {
		return "no track joins its stations at tiles " + line.a + " and " + line.b +
			(walk.unpowered != -1 ? " (no power at tile " + walk.unpowered + ")" : "");
	}
	PrismaticAI.RepairDepot(i, line);
	return null;
}

/* CheckLine once per line within one pass (`checked`: slot index -> reason or null). */
function PrismaticAI::CheckLineOnce(i, loco, checked) {
	if (!checked.rawin(i)) checked[i] <- PrismaticAI.CheckLine(i, RAIL_LINES[i], loco);
	return checked[i];
}

/* The first pair of consecutive stops of road vehicle `v` that no road joins: [a, b], or null. */
function PrismaticAI::RoadGap(v) {
	local n = AIOrder.GetOrderCount(v);
	for (local k = 0; k < n && n > 1; k++) {
		local a = AIOrder.GetOrderDestination(v, k), b = AIOrder.GetOrderDestination(v, (k + 1) % n);
		if (a == b || !AIMap.IsValidTile(a) || !AIMap.IsValidTile(b)) continue;
		if (!PrismaticAI.RoadConnected(a, b, ROAD_CHECK_MAX)) return [a, b];
	}
	return null;
}

/* A sale that could not start (no depot the vehicle can reach) is logged; it stays where it is. */
function PrismaticAI::SellOrSay(v, msg) {
	if (this.SendToSell(v, msg)) return;
	AILog.Warning(msg + " - but it cannot be sent to a depot: " + AIError.GetLastErrorString());
}

/* A train of line i, whose track cannot be repaired, to be sold; one that cannot reach a depot
 * either is stopped where it stands (a stopped train has no running costs) and started again by
 * LookAfterVehicles once the line is whole. In a headless match three trains of a cut line could
 * reach no depot and each lost 1828 a year. */
function PrismaticAI::SellOrPark(v, i, msg) {
	if (this.SendToSell(v, msg)) return;
	local line = RAIL_LINES[i];
	if (AIVehicle.GetState(v) != AIVehicle.VS_STOPPED && !AIVehicle.StartStopVehicle(v)) {
		AILog.Warning(msg + " - but it can neither reach a depot nor be stopped: " + AIError.GetLastErrorString());
		return;
	}
	if (!("parked" in line)) line.parked <- [];
	line.parked.push(v);
	AILog.Warning(msg + " - but no depot can be reached, so it stands stopped until the line is whole again");
}

/* ET_VEHICLE_LOST for `v` (see the head of this file). */
function PrismaticAI::OnVehicleLost(v, checked) {
	if (!AIVehicle.IsValidVehicle(v)) return;
	local type = AIVehicle.GetVehicleType(v), where = AIVehicle.GetLocation(v);
	local what = "vehicle " + v + " (unit " + AIVehicle.GetUnitNumber(v) + ")";
	if (type == AIVehicle.VT_RAIL) {
		local i = PrismaticAI.LineOfTrain(v);
		if (i == -1) {
			this.SellOrSay(v, "PrismaticAI: lost: train " + what + " at tile " + where + " runs on no line the bot knows; sending it to be sold");
			return;
		}
		local line = RAIL_LINES[i];
		local why = PrismaticAI.CheckLineOnce(i, AIVehicle.GetEngineType(v), checked);
		if (why != null) {
			line.broken <- why;
			this.SellOrPark(v, i, "PrismaticAI: lost: train " + what + " of " + PrismaticAI.LineName(i) + " at tile " + where + ": " + why +
				"; the line cannot be repaired, so the train is sent to be sold and the line gets no new one");
			return;
		}
		if (this.selling.rawin(v)) {
			AILog.Info("PrismaticAI: lost: train " + what + " of " + PrismaticAI.LineName(i) + " at tile " + where +
				" on its way to be sold; the line is whole and its depot checked");
			return;
		}
		if (this.lost.rawin(v)) {
			this.SellOrSay(v, "PrismaticAI: lost: train " + what + " of " + PrismaticAI.LineName(i) + " at tile " + where +
				" is lost again on a whole line (first on day " + this.lost[v] + "); sending it to be sold");
			delete this.lost[v];
			return;
		}
		/* Another train of this line is lost and has not found its way yet: the fault is the line's
		 * (a turn or signal its trains cannot take), and every train added there is lost too. In a
		 * headless match a double-track line's steward added five trains, each lost on its first trip. */
		foreach (u, _ in this.lost) {
			if (u != v && AIVehicle.IsValidVehicle(u) && PrismaticAI.LineOfTrain(u) == i) {
				PrismaticAI.HaltLostLine(i, "trains " + u + " and " + v + " are both lost on its whole track");
				break;
			}
		}
		this.lost[v] <- AIDate.GetCurrentDate();
		/* On a double track a train turned around would face the back of a one-way signal. */
		local turned = !("path2" in line) && AIVehicle.ReverseVehicle(v);
		AILog.Info("PrismaticAI: lost: train " + what + " of " + PrismaticAI.LineName(i) + " at tile " + where + ": the line is whole" +
			(turned ? "; turned it around" : "") + "; sold if it reaches no station within " + LOST_SELL_DAYS + " days");
		return;
	}
	if (type == AIVehicle.VT_ROAD) {
		local gap = PrismaticAI.RoadGap(v);
		if (gap != null) {
			this.SellOrSay(v, "PrismaticAI: lost: road " + what + " at tile " + where + ": no road joins its stops at tiles " + gap[0] +
				" and " + gap[1] + "; sending it to be sold");
			return;
		}
	}
	if (this.selling.rawin(v) || this.lost.rawin(v)) {
		if (this.lost.rawin(v)) {
			this.SellOrSay(v, "PrismaticAI: lost: " + what + " at tile " + where + " is lost again (first on day " + this.lost[v] + "); sending it to be sold");
			delete this.lost[v];
		}
		return;
	}
	this.lost[v] <- AIDate.GetCurrentDate();
	AILog.Info("PrismaticAI: lost: " + what + " at tile " + where + ": its route is whole; sold if it reaches no station within " + LOST_SELL_DAYS + " days");
}

/* ET_VEHICLE_UNPROFITABLE for `v` (see the head of this file). */
function PrismaticAI::OnVehicleUnprofitable(v, checked) {
	if (!AIVehicle.IsValidVehicle(v) || this.selling.rawin(v)) return;
	local type = AIVehicle.GetVehicleType(v);
	local what = "vehicle " + v + " (unit " + AIVehicle.GetUnitNumber(v) + ")";
	local profit = AIVehicle.GetProfitLastYear(v);
	if (type == AIVehicle.VT_RAIL) {
		local i = PrismaticAI.LineOfTrain(v);
		if (i != -1) {
			local why = PrismaticAI.CheckLineOnce(i, AIVehicle.GetEngineType(v), checked);
			if (why != null) {
				RAIL_LINES[i].broken <- why;
				this.SellOrPark(v, i, "PrismaticAI: unprofitable: train " + what + " of " + PrismaticAI.LineName(i) + " made " + profit +
					" last year: " + why + "; sending it to be sold");
				return;
			}
			AILog.Info("PrismaticAI: unprofitable: train " + what + " of " + PrismaticAI.LineName(i) + " made " + profit +
				" last year; the line is whole, so the steward and the fleet's two-year rule judge it");
			return;
		}
	} else if (type == AIVehicle.VT_ROAD) {
		local gap = PrismaticAI.RoadGap(v);
		if (gap != null) {
			this.SellOrSay(v, "PrismaticAI: unprofitable: road " + what + " made " + profit + " last year: no road joins its stops at tiles " +
				gap[0] + " and " + gap[1] + "; sending it to be sold");
			return;
		}
	}
	AILog.Info("PrismaticAI: unprofitable: " + what + " made " + profit + " last year; the fleet sells it once it is two years old and still losing");
}

/* Every heartbeat: a lost vehicle seen at a station found its way; one that reached none within
 * LOST_SELL_DAYS is sent to be sold. */
function PrismaticAI::FollowLost() {
	if (this.lost.len() == 0) return;
	local now = AIDate.GetCurrentDate(), done = [];
	foreach (v, since in this.lost) {
		if (!AIVehicle.IsValidVehicle(v) || this.selling.rawin(v)) { done.push(v); continue; }
		local i = (AIVehicle.GetVehicleType(v) == AIVehicle.VT_RAIL) ? PrismaticAI.LineOfTrain(v) : -1;
		if (AIVehicle.GetState(v) == AIVehicle.VS_AT_STATION) {
			AILog.Info("PrismaticAI: lost: vehicle " + v + " found its way: at station " + AIStation.GetName(AIStation.GetStationID(AIVehicle.GetLocation(v))) +
				((i != -1 && ("lost_trains" in RAIL_LINES[i])) ? "; " + PrismaticAI.LineName(i) + " may take new trains again" : ""));
			if (i != -1 && ("lost_trains" in RAIL_LINES[i])) delete RAIL_LINES[i].lost_trains;
			done.push(v);
		} else if (now - since >= LOST_SELL_DAYS) {
			if (i != -1) PrismaticAI.HaltLostLine(i, "train " + v + " reached no station in " + (now - since) + " days on its whole track");
			this.SellOrSay(v, "PrismaticAI: lost: vehicle " + v + " reached no station in " + (now - since) + " days; sending it to be sold");
			done.push(v);
		}
	}
	foreach (v in done) delete this.lost[v];
}

/* The vehicle of ours that `v`, stopped in its depot, can share orders with: one of its type with
 * orders that is served from the same depot (a rail line's depot, fleet.nut DepotFor otherwise)
 * and carries a cargo `v` can carry. A train gets the line's wagons first (lines.nut ResizeTrain)
 * and only joins a line with room for it. Returns {main, label} or null. */
function PrismaticAI::OrdersFor(v) {
	local depot = AIVehicle.GetLocation(v), type = AIVehicle.GetVehicleType(v);
	if (type == AIVehicle.VT_RAIL) {
		local loco = AIVehicle.GetEngineType(v);
		foreach (i, line in RAIL_LINES) {
			if (line.closed || PrismaticAI.LineHalted(line) || line.depot != depot || !AIEngine.HasPowerOnRail(loco, line.railtype)) continue;
			local trains = PrismaticAI.TrainsOf(line), running = 0;
			foreach (t in trains) if (t != v && !this.selling.rawin(t)) running++;
			local want = this.EffectiveTarget(i + 1, "trains");
			if (want == 0 || running >= (want > 0 ? min(want, line.trains_max) : line.trains_max)) continue;
			if (AIVehicle.GetCapacity(v, line.cargo) <= 0 || (trains.len() > 0 && AIVehicle.GetNumWagons(v) < AIVehicle.GetNumWagons(trains[0]))) {
				local wagons = (trains.len() > 0) ? AIVehicle.GetNumWagons(trains[0]) - 1 : (line.wagons > 0 ? line.wagons : 2 * line.len - 1);
				PrismaticAI.ResizeTrain(line, v, wagons);
			}
			if (AIVehicle.GetCapacity(v, line.cargo) <= 0) continue;
			return {main = (trains.len() > 0) ? trains[0] : -1, line = line, label = PrismaticAI.LineName(i)};
		}
		return null;
	}
	foreach (u, _ in AIVehicleList()) {
		if (u == v || AIVehicle.GetVehicleType(u) != type || AIOrder.GetOrderCount(u) == 0 || this.selling.rawin(u)) continue;
		if (PrismaticAI.DepotFor(u) != depot) continue;
		local cargo = PrismaticAI.CarriedCargo(u);
		if (cargo == -1 || AIVehicle.GetCapacity(v, cargo) <= 0) continue;
		return {main = u, line = null, label = "route " + PrismaticAI.RouteKey(u)};
	}
	return null;
}

/* Once a game month: vehicles stopped in a depot with no orders (see the head of this file), and
 * broken lines walked again. */
function PrismaticAI::LookAfterVehicles() {
	local seen = {};
	foreach (v, _ in AIVehicleList()) {
		if (AIOrder.GetOrderCount(v) > 0 || this.selling.rawin(v) || this.resizing.rawin(v) || !AIVehicle.IsStoppedInDepot(v)) continue;
		local what = "vehicle " + v + " (unit " + AIVehicle.GetUnitNumber(v) + ") in the depot at tile " + AIVehicle.GetLocation(v);
		if (!this.orphans.rawin(v)) {
			seen[v] <- true;
			AILog.Info("PrismaticAI: no orders: " + what + "; given a line's orders or sold next month if it still has none");
			continue;
		}
		local to = this.OrdersFor(v);
		if (to != null) {
			local ok = false;
			if (to.main != -1) ok = AIOrder.ShareOrders(v, to.main);
			else ok = AIOrder.AppendOrder(v, to.line.a, to.line.flags) && AIOrder.AppendOrder(v, to.line.b, AIOrder.OF_NONE);
			if (ok && AIVehicle.StartStopVehicle(v)) {
				if (to.main == -1) to.line.first <- v;
				AILog.Info("PrismaticAI: no orders: " + what + " now runs on " + to.label);
				continue;
			}
			AIOrder.UnshareOrders(v);
			while (AIOrder.GetOrderCount(v) > 0) AIOrder.RemoveOrder(v, 0);
		}
		local value = AIVehicle.GetCurrentValue(v);
		if (AIVehicle.SellVehicle(v)) {
			AILog.Info("PrismaticAI: no orders: sold " + what + " for " + value + ": no line or route served from that depot can use it");
		} else {
			seen[v] <- true;
			AILog.Warning("PrismaticAI: no orders: " + what + " could not be sold: " + AIError.GetLastErrorString());
		}
	}
	this.orphans = seen;
	foreach (i, line in RAIL_LINES) {
		if (line.closed || !("broken" in line)) continue;
		local why = PrismaticAI.CheckLine(i, line, -1);
		if (why == null) {
			delete line.broken;
			local started = 0;
			if ("parked" in line) {
				foreach (u in line.parked) {
					if (AIVehicle.IsValidVehicle(u) && AIVehicle.GetState(u) == AIVehicle.VS_STOPPED && AIVehicle.StartStopVehicle(u)) started++;
				}
				delete line.parked;
			}
			AILog.Info("PrismaticAI: lost: " + PrismaticAI.LineName(i) + " is whole again; it may run trains" +
				(started > 0 ? ", and its " + started + " stopped train(s) run again" : ""));
		} else if (why != line.broken) {
			line.broken = why;
		}
	}
}
