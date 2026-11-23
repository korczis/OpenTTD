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
 * the loan unused, while its buses earned about 370 a year each. */
function PrismaticAI::FundTrain(line, train, reserve) {
	local price = PrismaticAI.TrainPrice(train);
	local bank = AICompany.GetBankBalance(AICompany.COMPANY_SELF);
	local loan = AICompany.GetLoanAmount();
	if (bank >= reserve + price) return loan;
	if (AIController.GetSetting("steward_borrow") != 1 || !PrismaticAI.LineEarns(this.LineStats(line))) return -1;
	local afloat = PrismaticAI.QuarterRunningCost();
	if (this.BuildFrozen(afloat)) return -1;
	local max_loan = AICompany.GetMaxLoanAmount();
	local step = AICompany.GetLoanInterval();
	local want = min(max_loan, loan + ((reserve + price - bank) / step + 1) * step);
	/* The loan stays at most half its limit, and the headroom after it still holds a line budget and
	 * two quarters of running costs. Early in a game the loan pays for the lines: in a replay of
	 * seed 4242 a train borrowed for with 280k of 300k drawn left the bank at the reserve, the next
	 * quarter's survival borrowing froze construction (BuildFrozen) and the company built four
	 * freight lines where master built seven. */
	local line_budget = AIController.GetSetting("line_budget");
	if (2 * want > max_loan || max_loan - want < line_budget + 2 * afloat) return -1;
	if (!AICompany.SetLoanAmount(want)) return -1;
	AILog.Info("PrismaticAI: borrowed " + (want - loan) + " for a train of " + line.label + " (price " + price + ", bank was " + bank + ")");
	return loan;
}

/* Make every line what its targets say (see the head of this file). */
function PrismaticAI::ReconcileLines(reserve) {
	foreach (i, line in RAIL_LINES) {
		if (line.closed) continue;
		local slot = i + 1;
		/* The external steward's setting, else the in-bot steward's target (steward.nut). */
		local want_trains = this.EffectiveTarget(slot, "trains");
		local want_wagons = this.EffectiveTarget(slot, "wagons");
		if (want_trains == 0) { this.CloseLine(slot, line); continue; }
		local trains = PrismaticAI.TrainsOf(line);
		if (want_wagons > 0) {
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
			local running = 0;
			foreach (t in trains) if (!this.selling.rawin(t)) running++;
			/* A line whose track cannot be repaired or whose trains get lost gets no new train (events.nut). */
			if (running < want_trains && running > 0 && !PrismaticAI.LineHalted(line)) {
				/* Every missing train at once (steward.nut STEWARD_ROUND_NOTE), each paid from the bank
				 * or, for a line that earns, from the loan (FundTrain). */
				while (running < want_trains) {
					local loan_before = this.FundTrain(line, trains[0], reserve);
					if (loan_before < 0) break;
					local nv = AIVehicle.CloneVehicle(line.depot, trains[0], true);
					if (!AIVehicle.IsValidVehicle(nv)) {
						if (AICompany.GetLoanAmount() > loan_before) AICompany.SetLoanAmount(loan_before);
						break;
					}
					AIVehicle.StartStopVehicle(nv);
					running++;
					AILog.Info("PrismaticAI: rail line " + slot + ": added train " + nv + " (" + running + " of " + want_trains + ")");
				}
			} else if (running > want_trains) {
				local newest = -1;
				foreach (t in trains) if (!this.selling.rawin(t) && (newest == -1 || AIVehicle.GetAge(t) < AIVehicle.GetAge(newest))) newest = t;
				if (newest != -1) this.SendToSell(newest, "PrismaticAI: rail line " + slot + ": train " + newest + " to the depot to be sold (" +
					running + " for " + want_trains + ")");
			}
		}
	}
}
