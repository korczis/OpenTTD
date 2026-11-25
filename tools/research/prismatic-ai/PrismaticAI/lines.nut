/*
 * Rail lines as desired state (RESEARCH-ONLY): the bot builds a line, the Rhai steward
 * (tools/research/mcp-panel/rhai/rail_steward.rhai) decides what it should run, and the bot makes it
 * so. The bot never decides here; it reconciles.
 *
 * - Every line has a slot: its place in RAIL_LINES, from 1. The steward reads each line's geometry
 *   from the record the bot logs (LineRecord, "PrismaticAI: rail line <slot>: label=..|.."), at
 *   once when the line is built and again on the first heartbeat of every game month, so a steward
 *   started late or a panel whose event buffer rolled over still finds every line.
 * - The steward's targets are this AI's settings line<slot>_trains and line<slot>_wagons (info.nut
 *   declares LINE_SLOTS of each; -1 means no target). They are CONFIG_INGAME, so a change goes
 *   through openttd.ai.set_setting and the owner's approval gate, like every other change.
 * - trains: 0 closes the line (its trains go to the depot and are sold, then its track, stations
 *   and depot are demolished, so it stops costing maintenance); 1..trains_max is the number of
 *   trains (a clone of the first, shared orders; above trains_max the platforms could not hold them).
 * - wagons: wagons per train, at most wagons_fit (what the platform holds at the wagon's real
 *   length). A train with another count is sent to the depot, resized there and started again.
 */

/* The trains of `line`: its first train (rail.nut BuildTrain) and the clones that share its orders.
 * Telling them by their stations alone counted another line's trains as this line's wherever two
 * lines share both stations (side-by-side platforms of one station): on 07472abe/m-0006 lines 5 and
 * 11 both ran to Rarningworth Falls Sawmill, line 5 sold line 11's trains as surplus and line 11
 * bought them again, 372 trains bought and 360 sent to be sold in one game. Without a valid first
 * train, the trains that visit both stations and belong to no other line's order group. */
function PrismaticAI::TrainsOf(line) {
	local out = [];
	if (("first" in line) && AIVehicle.IsValidVehicle(line.first)) {
		foreach (v, _ in AIVehicleList_SharedOrders(line.first)) out.push(v);
		return out;
	}
	local st_a = AIStation.GetStationID(line.a), st_b = AIStation.GetStationID(line.b);
	if (!AIStation.IsValidStation(st_a) || !AIStation.IsValidStation(st_b)) return out;
	local claimed = {};
	foreach (other in RAIL_LINES) {
		if (other == line || !("first" in other) || !AIVehicle.IsValidVehicle(other.first)) continue;
		foreach (v, _ in AIVehicleList_SharedOrders(other.first)) claimed[v] <- true;
	}
	local list = AIVehicleList_Station(st_a);
	for (local v = list.Begin(); !list.IsEnd(); v = list.Next()) {
		if (AIVehicle.GetVehicleType(v) != AIVehicle.VT_RAIL || claimed.rawin(v)) continue;
		for (local i = 0; i < AIOrder.GetOrderCount(v); i++) {
			if (AIStation.GetStationID(AIOrder.GetOrderDestination(v, i)) == st_b) { out.push(v); break; }
		}
	}
	/* A train found this way leads the line from now on. */
	if (out.len() > 0) line.first <- out[0];
	return out;
}

/* The wagons a platform of `line.len` tiles holds behind `train`'s locomotive: the train's real
 * length (AIVehicle.GetLength, sixteenths of a tile) shared evenly over its parts, which is exact
 * for the default vehicles (half a tile each) and an estimate for a NewGRF set; ResizeTrain checks
 * the real length of every wagon it adds, so the estimate never makes a train longer than its
 * platform. */
function PrismaticAI::WagonsFit(line, train) {
	local n = AIVehicle.GetNumWagons(train);
	local part = max(1, AIVehicle.GetLength(train) / max(1, n));
	return max(1, line.len * 16 / part - 1);
}

/* One machine-readable record of a line, for the steward (rail_lib.rhai parse_record). */
function PrismaticAI::LineRecord(slot, line) {
	local trains = PrismaticAI.TrainsOf(line);
	local fit = trains.len() > 0 ? PrismaticAI.WagonsFit(line, trains[0]) : max(1, 2 * line.len - 1);
	line.wagons_fit <- fit;
	/* The station ids, so a reader matches the line's stations exactly, never by catchment. */
	local st_a = AIStation.GetStationID(line.a), st_b = AIStation.GetStationID(line.b);
	return "PrismaticAI: rail line " + slot + ": label=" + line.label + "|a=" + line.a + "|b=" + line.b +
		"|st_a=" + (AIStation.IsValidStation(st_a) ? st_a : -1) + "|st_b=" + (AIStation.IsValidStation(st_b) ? st_b : -1) +
		"|cargo=" + line.cargo + "|platforms=" + (("platforms" in line) ? line.platforms : line.trains_max) + "|len=" + line.len +
		"|depot=" + line.depot + "|trains_max=" + line.trains_max + "|wagons_fit=" + fit +
		"|tiles=" + (line.path.len() + (("path2" in line) ? line.path2.len() : 0)) +
		/* A double-track line (rail.nut BuildDoubleRailLineAndTrain) says so; a single one as before. */
		(("path2" in line) ? "|double=1" : "") + "|state=" + (line.closed ? "closed" : "open");
}

/* The game month now, to log every line's record once a month. */
function PrismaticAI::MonthKey() {
	local d = AIDate.GetCurrentDate();
	return AIDate.GetYear(d) * 12 + AIDate.GetMonth(d);
}

/* Log every line's record (LineRecord). */
function PrismaticAI::LogLines() {
	foreach (i, line in RAIL_LINES) AILog.Info(PrismaticAI.LineRecord(i + 1, line));
}

/* A setting of this AI, or -1 where info.nut declares none (more lines than slots). */
function PrismaticAI::LineTarget(slot, what) {
	local name = "line" + slot + "_" + what;
	local v = AIController.GetSetting(name);
	return (v == null) ? -1 : v;
}

/* Resize `train`, stopped in its depot, to `want` wagons of the type it has (or the line's best),
 * never longer than the platform. Returns the wagons it has afterwards. */
function PrismaticAI::ResizeTrain(line, train, want) {
	local depot = AIVehicle.GetLocation(train);
	local wagon = AIVehicle.GetNumWagons(train) > 1 ? AIVehicle.GetWagonEngineType(train, 1) : PrismaticAI.PickRailEngines(line.railtype, line.cargo).wagon;
	while (AIVehicle.GetNumWagons(train) - 1 > want) AIVehicle.SellWagon(train, AIVehicle.GetNumWagons(train) - 1);
	while (AIVehicle.GetNumWagons(train) - 1 < want && wagon != -1 && AIEngine.IsBuildable(wagon)) {
		if (!PrismaticAI.AddWagon(train, depot, wagon, line.cargo, line.len * 16)) break;
	}
	return AIVehicle.GetNumWagons(train) - 1;
}

/* Close `line`: every train to the depot and sold (this.selling, ManageFleet sells it there); once
 * none is left, its track, its own platforms and throats and its depot are removed (a crossing keeps
 * its road, a joined station keeps the other lines' platforms). Returns true when it is gone. */
function PrismaticAI::CloseLine(slot, line) {
	local trains = PrismaticAI.TrainsOf(line);
	foreach (t in trains) {
		if (this.selling.rawin(t)) continue;
		this.SendToSell(t, "PrismaticAI: rail line " + slot + " closing: train " + t + " to the depot to be sold");
	}
	if (trains.len() > 0) return false;
	local n = PrismaticAI.ClearPath(line.path);
	/* A double-track line's second track (rail.nut BuildDoubleRailLineAndTrain). */
	if ("path2" in line) n += PrismaticAI.ClearPath(line.path2);
	/* The stations' own platforms and throats only: a platform of another line joined to the same
	 * station stays (RemoveRailStation, never DemolishTile on a station tile). */
	foreach (st in line.stations) PrismaticAI.RemoveRailStation(st, line.len);
	/* A line rebuilt from the map (restore.nut) has a guessed depot that may serve another line. */
	if (line.stations.len() > 0) AITile.DemolishTile(line.depot);
	line.closed = true;
	AILog.Info("PrismaticAI: rail line " + slot + " closed: " + line.label + ", " + n + " tiles cleared");
	AILog.Info(PrismaticAI.LineRecord(slot, line));
	/* An own line that closed gives its own_lines slot back, the way a line that was never built
	 * does (main.nut, PR #234): own_lines bounds the own lines that run, and a closed one runs
	 * nothing and holds no train. Kept, it held the slot for the rest of the game: in the owner's
	 * game 21647 line 7 (OIL Runnley Oil Wells) closed on 1957-08-25 and was never replaced. Its pair
	 * stays in freight_tried, so the next own line is another pair. Never raised here: a line
	 * adopted from the map (AdoptTrainlessLines) is not in own_built until a load counts it. */
	if ("from" in line) {
		local open = PrismaticAI.OwnLinesOpen();
		if (open < this.own_built) {
			AILog.Info("PrismaticAI: own lines " + this.own_built + " -> " + open + ": closed line " + slot + " gives its slot back");
			this.own_built = open;
		}
	}
	return true;
}

/* Clear the track a line laid along `path`; returns the tiles cleared. */
function PrismaticAI::ClearPath(path) {
	local n = 0, count = path.len();
	foreach (i, t in path) {
		/* A level crossing carries a town's road: take only the rail off it (a crossing is always a
		 * straight piece, so its neighbours along the line say which). DemolishTile would clear the
		 * road too and cut the town's network. */
		if (count >= 2 && AIRail.IsLevelCrossingTile(t)) {
			local prev = (i > 0) ? path[i - 1] : t - (path[1] - t);
			local next = (i + 1 < count) ? path[i + 1] : t + (t - path[i - 1]);
			if (AIRail.RemoveRail(prev, t, next)) n++;
			continue;
		}
		if (AITile.DemolishTile(t)) n++;
	}
	return n;
}

/* What a clone of `train` costs now: every part at its engine's price. */
function PrismaticAI::TrainPrice(train) {
	local price = 0;
	for (local i = 0; i < AIVehicle.GetNumWagons(train); i++) price += AIEngine.GetPrice(AIVehicle.GetWagonEngineType(train, i));
	return price;
}

/* Money for one more train of `line`, a clone of `train`. Returns the loan before the purchase, or
 * -1 when the train cannot be paid.
 * - The bank balance pays when it holds `reserve` plus the price.
 * - Otherwise, with setting steward_borrow, a line whose trains earn (steward.nut LineEarns) borrows
 *   what is missing while the company is not frozen (main.nut BuildFrozen), the loan stays at most
 *   half its limit and the headroom left still holds a line budget and two quarters of running
 *   costs.
 * RepaySurplus (main.nut) pays the loan down to the reserve after every heartbeat, so the bank alone
 * almost never held a train's price: in the owner's live game (seed 15805, 1958-08) a coal line
 * earning 33.9k a year per train kept its one train of trains_max 2 with 13k in the bank and 250k of
 * the loan unused, while its buses earned about 370 a year each.
 * `inside`: a round inside a rail path search (steward.nut MaintenanceYield) never borrows. With
 * test_borrow (RESEARCH-ONLY) the bank counts as holding only the reserve, so every train needs
 * the loan and this path runs. */
function PrismaticAI::FundTrain(line, train, reserve, inside = false) {
	local price = PrismaticAI.TrainPrice(train);
	local bank = AICompany.GetBankBalance(AICompany.COMPANY_SELF);
	local loan = AICompany.GetLoanAmount();
	if (!inside && AIController.GetSetting("test_borrow") == 1) bank = min(bank, reserve);
	if (bank >= reserve + price) return loan;
	if (inside || AIController.GetSetting("steward_borrow") != 1 || !PrismaticAI.LineEarns(this.LineStats(line))) return -1;
	local afloat = PrismaticAI.QuarterRunningCost();
	if (this.BuildFrozen(afloat)) return -1;
	local max_loan = AICompany.GetMaxLoanAmount();
	local step = AICompany.GetLoanInterval();
	/* The shortfall in whole loan steps, rounded up. */
	local want = min(max_loan, loan + ((reserve + price - bank + step - 1) / step) * step);
	/* The loan stays at most half its limit, and the headroom after it still holds a line budget and
	 * two quarters of running costs. Early in a game the loan pays for the lines: in a replay of
	 * seed 4242 a train borrowed for with 280k of 300k drawn left the bank at the reserve, the next
	 * quarter's survival borrowing froze construction (BuildFrozen) and the company built four
	 * freight lines where master built seven. */
	local line_budget = AIController.GetSetting("line_budget");
	if (2 * want > max_loan || max_loan - want < line_budget + 2 * afloat) {
		/* Once per line and reason, so the bound shows in a log without repeating every heartbeat. */
		local why = 2 * want > max_loan ? "the loan would pass half its limit" : "the headroom left would not hold a line budget and two quarters";
		if (!("borrow_refused" in line) || line.borrow_refused != why) {
			line.borrow_refused <- why;
			AILog.Info("PrismaticAI: no loan for a train of " + line.label + ": " + why + " (loan " + loan + ", wanted " + want +
				" of " + max_loan + ", line budget " + line_budget + ", a quarter " + afloat + ")");
		}
		return -1;
	}
	if (!AICompany.SetLoanAmount(want)) return -1;
	if ("borrow_refused" in line) delete line.borrow_refused;
	AILog.Info("PrismaticAI: borrowed " + (want - loan) + " for a train of " + line.label + " (price " + price + ", bank was " + bank +
		"; loan now " + want + " of " + max_loan + ", headroom " + (max_loan - want) + " for a line budget of " + line_budget + " and a quarter of " + afloat + ")");
	return loan;
}

/* Make every line what its targets say (see the head of this file). `inside`: a round inside a rail
 * path search (steward.nut MaintenanceYield), which only adds trains, at most MAINT_YIELD_TRAINS,
 * and never borrows; closing, resizing and shrinking wait for the heartbeat. */
function PrismaticAI::ReconcileLines(reserve, inside = false) {
	if (!inside) MAINT_RESERVE = reserve;
	local added = 0;
	foreach (i, line in RAIL_LINES) {
		if (line.closed) continue;
		local slot = i + 1;
		/* The external steward's setting, else the in-bot steward's target (steward.nut). */
		local want_trains = this.EffectiveTarget(slot, "trains");
		local want_wagons = this.EffectiveTarget(slot, "wagons");
		if (want_trains == 0) {
			if (!inside) this.CloseLine(slot, line);
			continue;
		}
		local trains = PrismaticAI.TrainsOf(line);
		if (want_wagons > 0 && !inside) {
			if (!("wagons_fit" in line)) PrismaticAI.LineRecord(slot, line);
			want_wagons = min(want_wagons, line.wagons_fit);
			foreach (t in trains) {
				if (this.selling.rawin(t)) continue;
				local have = AIVehicle.GetNumWagons(t) - 1;
				if (have == want_wagons) {
					if (this.resizing.rawin(t)) {
						/* The target came back to what the train has while it was on its way: start it
						 * again, or cancel its depot order (sending it again cancels), never leave it
						 * standing in the depot. */
						if (AIVehicle.IsStoppedInDepot(t)) AIVehicle.StartStopVehicle(t);
						else AIVehicle.SendVehicleToDepot(t);
						delete this.resizing[t];
						AILog.Info("PrismaticAI: rail line " + slot + ": train " + t + " keeps its " + have + " wagons and runs again");
					}
					continue;
				}
				if (AIVehicle.IsStoppedInDepot(t)) {
					local got = PrismaticAI.ResizeTrain(line, t, want_wagons);
					AIVehicle.StartStopVehicle(t);
					if (this.resizing.rawin(t)) delete this.resizing[t];
					AILog.Info("PrismaticAI: rail line " + slot + ": train " + t + " resized from " + have + " to " + got + " wagons");
				} else if (!this.resizing.rawin(t) && AIVehicle.SendVehicleToDepot(t)) {
					this.resizing[t] <- true;
					PrismaticAI.ProtectFromReplace(t); /* autoreplace would give it a new id there (modernize.nut) */
					AILog.Info("PrismaticAI: rail line " + slot + ": train " + t + " to the depot for " + want_wagons + " wagons (has " + have + ")");
				}
			}
		}
		if (want_trains > 0) {
			want_trains = min(want_trains, line.trains_max);
			local running = 0, source = -1;
			foreach (t in trains) {
				if (this.selling.rawin(t)) continue;
				running++;
				/* A clone copies a train that keeps running, never one on its way to be sold. */
				if (source == -1) source = t;
			}
			/* A line whose track cannot be repaired or whose trains get lost gets no new train (events.nut). */
			if (running < want_trains && running > 0 && !PrismaticAI.LineHalted(line)) {
				/* Every missing train at once (steward.nut STEWARD_ROUND_NOTE), each paid from the bank
				 * or, for a line that earns, from the loan (FundTrain). */
				while (running < want_trains && !(inside && added >= MAINT_YIELD_TRAINS)) {
					local loan_before = this.FundTrain(line, source, reserve, inside);
					if (loan_before < 0) break;
					local nv = AIVehicle.CloneVehicle(line.depot, source, true);
					if (!AIVehicle.IsValidVehicle(nv) || !AIVehicle.StartStopVehicle(nv)) {
						/* A clone that will not start is sold again at once, not left in the depot. */
						if (AIVehicle.IsValidVehicle(nv)) AIVehicle.SellVehicle(nv);
						if (AICompany.GetLoanAmount() > loan_before) AICompany.SetLoanAmount(loan_before);
						break;
					}
					running++;
					added++;
					AILog.Info("PrismaticAI: rail line " + slot + ": added train " + nv + " (" + running + " of " + want_trains + ")" +
						(inside ? " during a rail path search" : ""));
				}
			} else if (running > want_trains && !inside) {
				local newest = -1;
				foreach (t in trains) if (!this.selling.rawin(t) && (newest == -1 || AIVehicle.GetAge(t) < AIVehicle.GetAge(newest))) newest = t;
				if (newest != -1) this.SendToSell(newest, "PrismaticAI: rail line " + slot + ": train " + newest + " to the depot to be sold (" +
					running + " for " + want_trains + ")");
			}
		}
	}
}

/* Industries further than this (tiles, to their location) are not looked at for a station: past any
 * industry's size plus the station spread the game allows by default (12) and the catchment. */
const ADOPT_INDUSTRY_REACH = 32;

/* Adopting trainless lines: a pair of our rail stations, at least one of them with no train of ours
 * calling (the other may be a station a served line joined), that no line of RAIL_LINES runs between
 * and no train of ours serves both of. The first station's catchment holds a producer of a freight
 * cargo, the second's an industry that accepts it (CaughtIndustry), the two are joined by our own track (RailWalk, events.nut), and a joined rail depot is near
 * one of them (RestoreDepot, DepotJoined). Such a pair is a finished line that lost its train before
 * KeepTrainlessLine existed, or one a restart forgot: it goes into RAIL_LINES with no train, and
 * KeepLinesServed buys its first train once the bank holds the train's price. In the owner's live
 * game (seed 15805) two finished iron ore lines stood like this from 1952 and 1953 while 535k lay in
 * the bank by 1962 (research/incident-2026-11-23-trainless-lines.md). A pair the walk cannot join is
 * left alone; a train that still gets lost is handled as on any line (events.nut: sold, the line
 * halted). Runs from Start after Restore and once a game year (AdoptKey). Returns the lines adopted. */
function PrismaticAI::AdoptTrainlessLines() {
	local pairs = {}, depots = {};
	foreach (line in RAIL_LINES) {
		local a = AIStation.GetStationID(line.a), b = AIStation.GetStationID(line.b);
		pairs[a + "-" + b] <- true;
		pairs[b + "-" + a] <- true;
		if (line.depot >= 0) depots[line.depot] <- true;
	}
	/* Our rail stations by their first platform tile, with the rail vehicles of ours that call. */
	local all = [], orphans = 0;
	foreach (st, _ in AIStationList(AIStation.STATION_TRAIN)) {
		local tiles = AITileList_StationType(st, AIStation.STATION_TRAIN);
		if (tiles.IsEmpty()) continue;
		local calls = AIVehicleList_Station(st);
		calls.Valuate(AIVehicle.GetVehicleType);
		calls.KeepValue(AIVehicle.VT_RAIL);
		all.push({st = st, tile = tiles.Begin(), calls = calls});
		if (calls.IsEmpty()) orphans++;
	}
	if (orphans == 0) return 0;
	local adopted = 0, used = {};
	foreach (src in all) {
		if (used.rawin(src.st)) continue;
		foreach (c, _ in AICargoList()) {
			if (!AICargo.IsFreight(c) || used.rawin(src.st)) continue;
			local p = PrismaticAI.CaughtIndustry(AIIndustryList_CargoProducing(c), src, true);
			if (p == -1) continue;
			foreach (dst in all) {
				if (dst.st == src.st || used.rawin(dst.st) || pairs.rawin(src.st + "-" + dst.st)) continue;
				if (!src.calls.IsEmpty() && !dst.calls.IsEmpty()) continue;
				/* A train of ours that calls at both already serves the pair. */
				local both = false;
				foreach (v, _ in src.calls) if (dst.calls.HasItem(v)) { both = true; break; }
				if (both) continue;
				local q = PrismaticAI.CaughtIndustry(AIIndustryList_CargoAccepting(c), dst, false);
				if (q == -1 || q == p) continue;
				if (!PrismaticAI.RailWalk(src.tile, dst.tile).joined) continue;
				local depot = PrismaticAI.RestoreDepot(src.tile, dst.tile, depots);
				if (depot == -1 || !PrismaticAI.DepotJoined(depot)) continue;
				local pa = PrismaticAI.RestorePlatform(src.tile), pb = PrismaticAI.RestorePlatform(dst.tile);
				local platforms = max(1, min(min(pa.rows, pb.rows), 2));
				local line = {a = src.tile, b = dst.tile, depot = depot, cargo = c, wagons = 0,
					flags = AIOrder.OF_FULL_LOAD_ANY, railtype = AIRail.GetRailType(src.tile), restored = 0,
					len = max(1, min(pa.len, pb.len)), trains_max = platforms, platforms = platforms,
					path = [], stations = [], closed = false, from = p, to = q, adopted = true,
					label = AICargo.GetCargoLabel(c) + " " + AIIndustry.GetName(p) + " - " + AIIndustry.GetName(q)};
				RAIL_LINES.push(line);
				depots[depot] <- true;
				pairs[src.st + "-" + dst.st] <- true;
				pairs[dst.st + "-" + src.st] <- true;
				/* An orphan joins one line; a station a served line joined may join more. */
				if (src.calls.IsEmpty()) used[src.st] <- true;
				if (dst.calls.IsEmpty()) used[dst.st] <- true;
				this.freight_tried[p + "-" + q + "-" + c] <- true;
				adopted++;
				AILog.Warning("PrismaticAI: adopted trainless rail line " + line.label + " (stations " + src.st + " and " +
					dst.st + ", depot " + depot + "); it waits for its first train");
				AILog.Info(PrismaticAI.LineRecord(RAIL_LINES.len(), line));
				break;
			}
		}
	}
	return adopted;
}

/* The industry of `list` that the rail station `stn` ({st, tile}) serves: whose producing tiles
 * (`producing`) or accepting tiles the station's catchment covers, the nearest first; -1 when none.
 * By catchment and not by distance: a station can stand 19 tiles from its industry's location
 * (the industry's north corner), past RESTORE_INDUSTRY_REACH, and still serve it (fitness seed
 * 930967890, Pronhattan Forest). */
function PrismaticAI::CaughtIndustry(list, stn, producing) {
	local radius = AIStation.GetCoverageRadius(AIStation.STATION_TRAIN);
	list.Valuate(AIIndustry.GetDistanceManhattanToTile, stn.tile);
	list.KeepBelowValue(ADOPT_INDUSTRY_REACH + 1);
	list.Sort(AIList.SORT_BY_VALUE, AIList.SORT_ASCENDING);
	foreach (i, _ in list) {
		local tiles = producing ? AITileList_IndustryProducing(i, radius) : AITileList_IndustryAccepting(i, radius);
		foreach (t, _ in tiles) {
			if (AIRail.IsRailStationTile(t) && AIStation.GetStationID(t) == stn.st) return i;
		}
	}
	return -1;
}

/* test_trainless=2 (RESEARCH-ONLY, info.nut): once a game month, before AdoptTrainlessLines, the bot
 * forgets its lines that never ran a train, as a restart from the map would (restore.nut rebuilds a
 * line only from its trains). Their stations, track and depot stay, so the adoption must find them
 * again on the map. Returns the number of lines forgotten. */
function PrismaticAI::TestForgetTrainless() {
	if (AIController.GetSetting("test_trainless") < 2) return 0;
	local keep = [], forgot = 0;
	foreach (line in RAIL_LINES) {
		/* A line still waiting for its depot has none for the adoption to find (open finding of
		 * research/incident-2026-11-23-trainless-lines.md): it is kept. */
		if (!("first" in line) && !line.closed && !("adopted" in line) && !("depot_pending" in line)) { forgot++; continue; }
		keep.push(line);
	}
	if (forgot == 0) return 0;
	RAIL_LINES.clear();
	foreach (line in keep) RAIL_LINES.push(line);
	AILog.Warning("PrismaticAI: test_trainless forgot " + forgot + " trainless line(s)");
	return forgot;
}

/* The period AdoptTrainlessLines runs once in: the game year, or the game month with
 * test_trainless=2 (RESEARCH-ONLY), so a forgotten line waits a month and not a year. */
function PrismaticAI::AdoptKey() {
	if (AIController.GetSetting("test_trainless") >= 2) return PrismaticAI.MonthKey();
	return AIDate.GetYear(AIDate.GetCurrentDate());
}

/* Days a line halted by lost trains (events.nut, `lost_trains`) may stand with no train at all
 * before it is closed: LOST_SELL_DAYS already sold the train that got lost, and the line takes no
 * new one until a lost train reaches a station, which a sold train never does. */
const HALTED_CLOSE_DAYS = 180;

/* Closes every line halted by lost trains that has had no train for HALTED_CLOSE_DAYS (CloseLine:
 * track, stations and depot go). Such a line otherwise keeps its stations, upkeep and producer for
 * the rest of the game: on the fitness seed after the merge of 2026-11-23, `WOOD Hartburg Forest -
 * Pronhattan Sawmill` (double track, 599 tiles) stood like this from its train's sale to the end
 * (research/incident-2026-11-23-trainless-lines.md). Runs with AdoptTrainlessLines. Returns the
 * number of lines closed. */
function PrismaticAI::CloseHaltedLines() {
	local now = AIDate.GetCurrentDate(), closed = 0;
	foreach (i, line in RAIL_LINES) {
		if (line.closed || !("lost_trains" in line) || now - line.lost_trains < HALTED_CLOSE_DAYS) continue;
		if (PrismaticAI.TrainsOf(line).len() > 0) continue;
		AILog.Warning("PrismaticAI: rail line " + (i + 1) + " (" + line.label + ") has been halted by lost trains for " +
			(now - line.lost_trains) + " days with no train; closing it");
		if (this.CloseLine(i + 1, line)) closed++;
	}
	return closed;
}
