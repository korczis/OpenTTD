/*
 * Modernize the fleet (setting modernize; main.nut calls Modernize once a game month when it is 1).
 *
 * Why: the bot never upgraded an engine. A new route buys the best engine of the day (rail.nut
 * PickRailEngines, air.nut PickPlane), but a route grows by AIVehicle.CloneVehicle (fleet.nut,
 * lines.nut), which copies the old engine, and autorenew (main.nut Start) renews like for like. In
 * the owner's live game in 1960 company 0 ran 17 steam Ginzu 'A4' (1935) while the diesel UU '37'
 * (1960) was buildable.
 *
 * How: for every engine the company runs, a successor that is strictly better and fits every
 * vehicle of that engine becomes an autoreplace rule on AIGroup.GROUP_ALL. The game does the rest
 * (src/autoreplace_cmd.cpp, src/vehicle.cpp, src/script/api/script_group.cpp):
 * - AIGroup.SetAutoReplace requires the new engine buildable and sets the rule with "when old"
 *   off, so a vehicle is replaced at its next depot visit, not at its old age. A clone of an old
 *   vehicle is replaced at its first service.
 * - The bot creates no groups of its own for running vehicles, and a vehicle of GROUP_DEFAULT falls
 *   back to the GROUP_ALL rule (autoreplace.cpp EngineReplacement).
 * - Vehicle::NeedsServicing schedules the depot visit only while the company has engine_renew_money
 *   (30000, main.nut Start) plus twice the new engine's price; the bot never borrows for it.
 * - The game's own check (CheckAutoreplaceValidity) only asks that the rail types be compatible,
 *   so a steam engine could be "replaced" by an electric one that has no power on plain rail: a
 *   successor must have power on the old engine's rail type and on the track its vehicles stand on
 *   (AIEngine.HasPowerOnRail). Road vehicles keep a road type the old one has power on (which also
 *   keeps trams trams); a plane never becomes a helicopter or the reverse (the game refuses it), a
 *   small plane never becomes a big one (it would crash on a small airport), and a plane never gets
 *   a shorter range (AIEngine.GetMaximumOrderDistance, 0 = unlimited).
 * - AIGroup.EnableWagonRemoval(true): a longer locomotive drops wagons from the end, so a train never
 *   grows longer than its platform.
 * A rule is set once per change and logged as "PrismaticAI: modernize: <old> -> <new>". A rule the
 * bot did not set (a person in the company, the panel) is left alone.
 *
 * Protection: the game also replaces a vehicle that drives to the depot to stop there. A vehicle the
 * bot sends to be sold (fleet.nut and lines.nut this.selling) or resized (lines.nut this.resizing)
 * would come out of the replacement with a new vehicle id, stopped, and the bot would wait for the
 * old id forever (src/vehicle.cpp VehicleEnteredDepotThisTick and the Halt branch of
 * VehicleEnterDepot: a halting vehicle is still queued for autoreplace). ProtectFromReplace moves
 * such a vehicle into a group of its type with autoreplace protection (AIGroup.
 * EnableAutoReplaceProtection), which ignores the GROUP_ALL rule. A clone inherits its source's
 * group (src/vehicle_cmd.cpp CmdCloneVehicle), so Modernize moves every protected vehicle that is
 * no longer selling or resizing back to GROUP_DEFAULT once a month. Protection does not stop the
 * game's like-for-like autorenew of a vehicle past its maximum age (autoreplace_cmd.cpp
 * GetNewEngineType).
 */

/* Vehicle type -> the id of the bot's autoreplace-protected group of that type (ProtectedGroup). */
MODERNIZE_GROUPS <- {};
/* The name of those groups, so a reloaded bot finds them again and never takes someone else's. */
MODERNIZE_GROUP_NAME <- "PrismaticAI hold";
/* Old engine -> the engine the bot set as its replacement on GROUP_ALL. */
MODERNIZE_SET <- {};
/* "old-new" -> true once a refused or foreign rule was logged, so it is logged once. */
MODERNIZE_LOGGED <- {};

/* An engine's name for the log, or its id when the game has none. */
function PrismaticAI::EngineLabel(e) {
	local n = AIEngine.GetName(e);
	return n == null ? ("engine " + e) : n;
}

/* The cargo `v` carries most of (its whole consist), or -1 when it carries nothing. */
function PrismaticAI::CarriedCargo(v) {
	local best = -1, best_cap = 0;
	foreach (c, _ in AICargoList()) {
		local cap = AIVehicle.GetCapacity(v, c);
		if (cap > best_cap) { best_cap = cap; best = c; }
	}
	return best;
}

/* The buildable road engine with the best capacity x speed that carries or refits to `cargo`; with
 * `roadtype` (not -1) one that has power on it, and with `articulated` false not an articulated one
 * (an articulated road vehicle cannot use a bay stop, roadstop.nut BuildRoadStation). -1 if none. */
function PrismaticAI::BestRoadEngine(cargo, roadtype = -1, articulated = true) {
	local best = -1, best_value = -1;
	foreach (e, _ in AIEngineList(AIVehicle.VT_ROAD)) {
		if (!AIEngine.IsBuildable(e)) continue;
		if (AIEngine.GetCargoType(e) != cargo && !AIEngine.CanRefitCargo(e, cargo)) continue;
		if (roadtype != -1 && !AIEngine.HasPowerOnRoad(e, roadtype)) continue;
		if (!articulated && AIEngine.IsArticulated(e)) continue;
		local value = AIEngine.GetCapacity(e) * AIEngine.GetMaxSpeed(e);
		if (value > best_value) { best_value = value; best = e; }
	}
	return best;
}

/* The engine that should replace `old` in vehicle `v`, strictly better than `old`, or -1.
 * - rail: PickRailEngines' locomotive for the old engine's rail type and the train's cargo
 *   (passengers for a train without wagons), with power on the track `v` stands on, and faster, or
 *   as fast and more powerful;
 * - road: BestRoadEngine of the same cargo and road type, articulated only where the old one is,
 *   with a better capacity x speed;
 * - air: PickPlane's measure (capacity x speed, passengers or the plane's cargo), the same class
 *   (helicopter stays helicopter, a small plane stays small) and no shorter range. */
function PrismaticAI::SuccessorEngine(v, old) {
	local type = AIVehicle.GetVehicleType(v);
	local cargo = PrismaticAI.CarriedCargo(v);
	if (cargo == -1) cargo = PrismaticAI.PassengerCargo();
	if (cargo == -1 || !AIEngine.IsValidEngine(old)) return -1;
	local old_value = AIEngine.GetCapacity(old) * AIEngine.GetMaxSpeed(old);

	if (type == AIVehicle.VT_RAIL) {
		if (AIEngine.IsWagon(old)) return -1;
		/* The rail type of the line's track, read at its first station (never a bridge or a tunnel):
		 * once infra.nut converted a line to electrified rail, the best locomotive is an electric one,
		 * which the old engine's own rail type would never offer. */
		local rt = AIRail.GetRailType(AIOrder.GetOrderDestination(v, 0));
		if (rt == AIRail.RAILTYPE_INVALID) rt = AIEngine.GetRailType(old);
		if (rt == AIRail.RAILTYPE_INVALID) return -1;
		local loco = PrismaticAI.PickRailEngines(rt, cargo).loco;
		if (loco == -1 || loco == old) return -1;
		/* On a bridge or in a tunnel's middle the tile has no rail type: the station's type decides. */
		local track = AIRail.GetRailType(AIVehicle.GetLocation(v));
		if (!AIEngine.HasPowerOnRail(loco, rt)) return -1;
		if (track != AIRail.RAILTYPE_INVALID && !AIEngine.HasPowerOnRail(loco, track)) return -1;
		local s = AIEngine.GetMaxSpeed(loco), s0 = AIEngine.GetMaxSpeed(old);
		if (s > s0 || (s == s0 && AIEngine.GetPower(loco) > AIEngine.GetPower(old))) return loco;
		return -1;
	}

	if (type == AIVehicle.VT_ROAD) {
		local road = PrismaticAI.BestRoadEngine(cargo, AIEngine.GetRoadType(old), AIEngine.IsArticulated(old));
		if (road == -1 || road == old) return -1;
		return AIEngine.GetCapacity(road) * AIEngine.GetMaxSpeed(road) > old_value ? road : -1;
	}

	if (type == AIVehicle.VT_AIR) {
		local pt = AIEngine.GetPlaneType(old);
		local heli = pt == AIAirport.PT_HELICOPTER;
		local range = AIEngine.GetMaximumOrderDistance(old);
		local best = -1, best_value = old_value;
		foreach (e, _ in AIEngineList(AIVehicle.VT_AIR)) {
			if (e == old || !AIEngine.IsBuildable(e)) continue;
			if (AIEngine.GetCargoType(e) != cargo && !AIEngine.CanRefitCargo(e, cargo)) continue;
			local p = AIEngine.GetPlaneType(e);
			if ((p == AIAirport.PT_HELICOPTER) != heli) continue;
			if (pt == AIAirport.PT_SMALL_PLANE && p == AIAirport.PT_BIG_PLANE) continue;
			local r = AIEngine.GetMaximumOrderDistance(e);
			if (r != 0 && (range == 0 || r < range)) continue;
			local value = AIEngine.GetCapacity(e) * AIEngine.GetMaxSpeed(e);
			if (value > best_value) { best_value = value; best = e; }
		}
		return best;
	}
	return -1;
}

/* The bot's autoreplace-protected group for vehicles of `vtype`: the cached one, else one of that
 * name found among the company's groups (a reloaded bot), else, with `create`, a new one. null when
 * there is none or the game refused it. */
function PrismaticAI::ProtectedGroup(vtype, create) {
	if (MODERNIZE_GROUPS.rawin(vtype) && AIGroup.IsValidGroup(MODERNIZE_GROUPS[vtype])) return MODERNIZE_GROUPS[vtype];
	foreach (id, _ in AIGroupList()) {
		if (AIGroup.GetVehicleType(id) != vtype || AIGroup.GetName(id) != MODERNIZE_GROUP_NAME) continue;
		if (!AIGroup.GetAutoReplaceProtection(id) && !AIGroup.EnableAutoReplaceProtection(id, true)) continue;
		MODERNIZE_GROUPS[vtype] <- id;
		return id;
	}
	if (!create) return null;
	local g = AIGroup.CreateGroup(vtype, AIGroup.GROUP_INVALID);
	if (!AIGroup.IsValidGroup(g)) {
		AILog.Warning("PrismaticAI: modernize: no protected group: " + AIError.GetLastErrorString());
		return null;
	}
	if (!AIGroup.EnableAutoReplaceProtection(g, true)) {
		AILog.Warning("PrismaticAI: modernize: group " + g + " not protected: " + AIError.GetLastErrorString());
		AIGroup.DeleteGroup(g);
		return null;
	}
	AIGroup.SetName(g, MODERNIZE_GROUP_NAME);
	MODERNIZE_GROUPS[vtype] <- g;
	return g;
}

/* Keep `v` from being autoreplaced on its way to the depot to be sold or resized: into the
 * protected group of its type. Does nothing when the group cannot be had. */
function PrismaticAI::ProtectFromReplace(v) {
	if (!AIVehicle.IsValidVehicle(v) || !AIVehicle.IsPrimaryVehicle(v)) return;
	/* No rule of the bot's, nothing to be protected from: no group is made (and no command spent). */
	if (MODERNIZE_SET.len() == 0) return;
	local g = PrismaticAI.ProtectedGroup(AIVehicle.GetVehicleType(v), true);
	if (g != null && AIVehicle.GetGroupID(v) != g) AIGroup.MoveVehicle(g, v);
}

/* Back to GROUP_DEFAULT, and so to the GROUP_ALL rules, every protected vehicle that is neither
 * selling nor resizing any more: a train that runs again, a clone of a protected vehicle. */
function PrismaticAI::ReleaseProtected() {
	foreach (vtype in [AIVehicle.VT_RAIL, AIVehicle.VT_ROAD, AIVehicle.VT_WATER, AIVehicle.VT_AIR]) {
		local g = PrismaticAI.ProtectedGroup(vtype, false);
		if (g == null) continue;
		foreach (v, _ in AIVehicleList_Group(g)) {
			if (this.selling.rawin(v) || this.resizing.rawin(v)) continue;
			AIGroup.MoveVehicle(AIGroup.GROUP_DEFAULT, v);
		}
	}
}

/* Once a game month (b.modernize == 1): a GROUP_ALL autoreplace rule for every engine the company
 * runs that has a strictly better successor fitting every one of its vehicles. */
function PrismaticAI::Modernize(b) {
	PrismaticAI.ForgetReplacedTrains();
	AIGroup.EnableWagonRemoval(true);
	this.ReleaseProtected();

	/* Old engine -> the vehicles that run it, leaving out those on their way to be sold or resized. */
	local fleet = {};
	foreach (v, _ in AIVehicleList()) {
		if (this.selling.rawin(v) || this.resizing.rawin(v)) continue;
		local e = AIVehicle.GetEngineType(v);
		if (!fleet.rawin(e)) fleet[e] <- [];
		fleet[e].push(v);
	}

	/* SuccessorEngine depends on the engine, the cargo and (rail) the track: one call per kind. */
	local seen = {};
	foreach (old, vehicles in fleet) {
		local best = -1;
		foreach (v in vehicles) {
			local track = AIVehicle.GetVehicleType(v) == AIVehicle.VT_RAIL ? AIRail.GetRailType(AIVehicle.GetLocation(v)) : -1;
			local key = old + "/" + PrismaticAI.CarriedCargo(v) + "/" + track;
			if (!seen.rawin(key)) seen[key] <- PrismaticAI.SuccessorEngine(v, old);
			local s = seen[key];
			/* One rule serves every vehicle of `old`: all of them must take the same successor. */
			if (s == -1 || (best != -1 && s != best)) { best = -1; break; }
			best = s;
		}
		if (best == -1) continue;

		local cur = AIGroup.GetEngineReplacement(AIGroup.GROUP_ALL, old);
		if (cur == best) continue;
		local ours = MODERNIZE_SET.rawin(old) && MODERNIZE_SET[old] == cur;
		if (AIEngine.IsValidEngine(cur) && AIEngine.IsBuildable(cur) && !ours) {
			local note = old + "-" + cur;
			if (!MODERNIZE_LOGGED.rawin(note)) {
				MODERNIZE_LOGGED[note] <- true;
				AILog.Info("PrismaticAI: modernize: " + PrismaticAI.EngineLabel(old) + " already replaced by " +
					PrismaticAI.EngineLabel(cur) + " (a rule the bot did not set); " + PrismaticAI.EngineLabel(best) + " not set");
			}
			continue;
		}
		if (AIGroup.SetAutoReplace(AIGroup.GROUP_ALL, old, best)) {
			MODERNIZE_SET[old] <- best;
			AILog.Info("PrismaticAI: modernize: " + PrismaticAI.EngineLabel(old) + " -> " + PrismaticAI.EngineLabel(best) +
				" (" + vehicles.len() + " vehicles, each replaced at its next depot visit)");
		} else {
			local note = old + "-" + best;
			if (!MODERNIZE_LOGGED.rawin(note)) {
				MODERNIZE_LOGGED[note] <- true;
				AILog.Warning("PrismaticAI: modernize: " + PrismaticAI.EngineLabel(old) + " -> " + PrismaticAI.EngineLabel(best) +
					" refused: " + AIError.GetLastErrorString());
			}
		}
	}
}

/* Accept every engine preview the game offers: a year of a new engine before anyone else, which
 * PickRailEngines, PickPlane and SuccessorEngine then find buildable. The previews come from
 * events.nut HandleEvents, the bot's one reader of AIEventController, which keeps them here. */
function PrismaticAI::AcceptPreviews() {
	foreach (p in PENDING_PREVIEWS) {
		local name = p.GetName();
		if (p.AcceptPreview()) AILog.Info("PrismaticAI: modernize: accepted the preview of " + (name == null ? "an engine" : name));
	}
	PENDING_PREVIEWS.clear();
}

/* Not rich (rich.nut): the bot's own replacement rules are lifted, so money that is not idle
 * is not spent on replacing working vehicles. In the headless fitness match (seed 930967890, solo-max
 * lineup) modernizing on every budget cost 22% of the final value in six years: a replacement costs
 * a new vehicle's full price and pays back over its life. A rule someone else set stays. */
function PrismaticAI::ModernizeHold() {
	if (MODERNIZE_SET.len() > 0) PrismaticAI.ForgetReplacedTrains();
	foreach (old, rule in MODERNIZE_SET) {
		if (AIGroup.GetEngineReplacement(AIGroup.GROUP_ALL, old) == rule) AIGroup.StopAutoReplace(AIGroup.GROUP_ALL, old);
	}
	MODERNIZE_SET.clear();
}

/* A train replaced under these rules gets a new id, and the next bus may take the old one, which a
 * line still names as its first train (lines.nut TrainsOf). Once a month a line forgets a first
 * train that is gone or is no train, and TrainsOf finds its trains by their stations again. Kept
 * out of TrainsOf on purpose: one call more on that hot path shifts the script's timing, and the
 * headless match of a bot that never modernizes played a different game (2026-11-22). */
function PrismaticAI::ForgetReplacedTrains() {
	foreach (line in RAIL_LINES) {
		if (!("first" in line)) continue;
		if (!AIVehicle.IsValidVehicle(line.first) || AIVehicle.GetVehicleType(line.first) != AIVehicle.VT_RAIL) delete line.first;
	}
}
