/*
 * The capital allocator (RESEARCH-ONLY; setting capital_alloc, default 3). Once a heartbeat, in
 * place of the plain loan repayment, it decides where the money above the reserve goes: into the
 * investment with the best measured return when one clears its hurdle, else into the loan.
 *
 * Why (the owner's live game, seed 12609, company 0, 1950-1953, read from the panel store and
 * openttd://vehicles):
 * - RepaySurplus (main.nut) repaid everything above twice the reserve at the end of every
 *   heartbeat: 42 repayments, 1.19M, while the bank held about 10k. Every spend that reads the
 *   bank balance saw that 10k: InvestSurplus (fleet.nut) wants 20 reserves (100k) with
 *   invest_borrow 0 and bought its first plane on 1953-09-23, although the planes of route 0,10
 *   earned 20-24k a year on a price of about 30k (payback 1.5 years) and flew 53% full from 1952.
 * - The loan was 0 from 1953-04 and the bank rose to 182k by 1954-01, below the 250k at which
 *   rich growth (rich.nut) spends idle money, so nothing spent it: the company reinvested about
 *   44% of its operating cash in 1953, AAAHogEx about 100%.
 * - RepaySurplus left the balance less than one loan interval above StayAfloat's floor (a quarter
 *   of the fleet's running costs); a few days of costs later StayAfloat "rescued" the company by
 *   borrowing 10k, and BuildFrozen froze all construction for 90 days while the loan was above
 *   half its limit. Four such rescues in 1952 (278-994 below the floor) kept it frozen from
 *   1952-01-02 to 1952-06-20.
 * - No new air route since 1951-07: AirRoutesPay (autopilot.nut) wants every air route to earn
 *   price / invest_ppv_div a plane, and two weak routes refused every new one while the air fleet
 *   as a whole earned 16.9k a plane.
 * The 372k in the bank with a 300k loan seen on 1953-06 was TwinLines' whole-loan draw for a
 * 34-day twin build, repaid when it was built, not idle money; interest in 1953 came to about 1.2k.
 *
 * Per heartbeat (AllocateCapital), by level:
 *   1  keep = max(2 * cash_reserve, quarter running cost) + one loan interval, then RepaySurplus.
 *      StayAfloat borrows in loan intervals, so only a real shortfall of a whole interval now
 *      triggers a rescue (and the freeze behind it).
 *   2  and before repaying: the best measured investment (CapitalBest), bought while the bank holds
 *      keep plus its price, then the candidates are measured again. One more vehicle on a road, air
 *      or water route; or one more train on a rail line that earns, under trains_max, with half a
 *      trainload waiting or its source station rating the cargo below steward_rating, a source
 *      platform free and its newest train STEWARD_SETTLE_DAYS old (the in-bot steward's target
 *      follows). InvestSurplus (fleet.nut) never buys a train, and the steward's monthly round asks
 *      for one only on its stricter rules, so idle money did not reach rail: in the owner's game of
 *      seed 10487 company 0 held 165M with 40 trains, and the fitness check's self-steward
 *      contestant went from 61k to 885k on 32 vehicles in 1964-65. Where an external steward set a
 *      line's target, only a train it wants and lacks is funded: the money is held and
 *      ReconcileLines (lines.nut FundTrain) buys it. What the best candidate still needs is held
 *      back from repayment, so the next heartbeats' earnings pay for it; a hold that lasts over a
 *      full quarter is dropped and its candidate skipped for that quarter. With no candidate, the
 *      loan is repaid as before.
 *   3  and a new air route once the previous quarter left money idle at every call (no loan, at
 *      least one plane's price above keep) and the air fleet is proven (CapitalAirProven), one
 *      unproven route at a time, built by AutopilotAir before the repayment.
 *   0  master's RepaySurplus(max(2 * cash_reserve, quarter running cost)) unchanged.
 *
 * The hurdle is the bot's own: payback within invest_ppv_div years (profit a year x invest_ppv_div
 * >= price), the test InvestSurplus and AirRoutesPay apply. Loan interest is a few percent a year,
 * so an investment that clears it beats repaying. The profit is last year's, of the vehicles that
 * ran all of last year (CapitalFullYear): average, not marginal, so a candidate also passes every
 * saturation guard the fleet already has (caps after rich and pressure, RouteHasAirRoom,
 * AirRouteHasDemand, RouteFillsMore, fewer unjudged vehicles than judged ones).
 *
 * The state (CAPITAL) is not saved: after a load the quarter starts fresh, which only delays a new
 * air route by one quarter.
 */

/* quarter: PressureQuarter of the calls counted below (-1: none yet); start: the day the quarter's
 * count began (its first day, or the first call after a load); idle: every call of the quarter left
 * money idle; idle_last: the previous quarter did; calls, bought, spent, holds: the quarter's
 * report; frozen: the quarter's days under BuildFrozen, counted from call to call (last, was_frozen:
 * the day and state of the previous call); skip: route keys skipped for the rest of the quarter;
 * hold_key, hold_quarter: the candidate money is held for and the quarter the hold began; b: the
 * last heartbeat's Behaviour(), for the rounds inside a rail search (AllocateCapitalInside). */
CAPITAL <- {quarter = -1, start = -1, idle = false, idle_last = false, calls = 0, bought = 0, spent = 0, holds = 0,
	frozen = 0, last = -1, was_frozen = false, skip = {}, hold_key = "", hold_quarter = -1, b = null};

/* How many days a vehicle must be old today to have run all of last year: its profit last year is
 * then a whole year's (GetProfitLastYear is the previous calendar year, not the last 365 days). */
function PrismaticAI::CapitalFullYear() {
	local now = AIDate.GetCurrentDate();
	return now - AIDate.GetDate(AIDate.GetYear(now) - 1, 1, 1);
}

/* The candidate {key, price, profit, sample, rail, ...} with the best return (profit a year per
 * unit of price) whose payback is within invest_ppv_div years, or null. Vehicles: one more on a
 * road, air or water route, behind the fleet's guards. Rail: one more train on a line that is not
 * halted and runs under trains_max, by the demand rules of level 2 above (`buys`), or one an
 * external steward's target asks for (`buys` false: held for ReconcileLines). */
function PrismaticAI::CapitalBest(b) {
	local full = PrismaticAI.CapitalFullYear(), div = max(1, b.invest_ppv_div);
	local best = null;
	local routes = {};
	foreach (v, _ in AIVehicleList()) {
		if (this.selling.rawin(v)) continue;
		local type = AIVehicle.GetVehicleType(v);
		if (type != AIVehicle.VT_ROAD && type != AIVehicle.VT_AIR && type != AIVehicle.VT_WATER) continue;
		local k = PrismaticAI.RouteKey(v);
		if (k == "" || CAPITAL.skip.rawin(k)) continue;
		if (!routes.rawin(k)) routes[k] <- {sample = v, type = type, count = 0, judged = 0, profit = 0};
		local r = routes[k];
		r.count++;
		if (AIVehicle.GetAge(v) >= full) { r.judged++; r.profit += AIVehicle.GetProfitLastYear(v); }
	}
	foreach (k, r in routes) {
		local cap = (r.type == AIVehicle.VT_AIR) ? b.invest_air_cap : ((r.type == AIVehicle.VT_WATER) ? b.ships_cap : b.invest_road_cap);
		if (r.judged == 0 || r.count >= cap || r.count - r.judged >= r.judged) continue;
		local engine = AIVehicle.GetEngineType(r.sample);
		if (!AIEngine.IsBuildable(engine)) continue;
		local c = {key = k, price = AIEngine.GetPrice(engine), profit = r.profit / r.judged, sample = r.sample, rail = false};
		if (!PrismaticAI.CapitalBetter(c, best, div)) continue;
		if (r.type == AIVehicle.VT_AIR && (!PrismaticAI.RouteHasAirRoom(r.sample) || !PrismaticAI.AirRouteHasDemand(r.sample))) continue;
		if (!PrismaticAI.RouteFillsMore(k)) continue;
		best = c;
	}
	foreach (i, line in RAIL_LINES) {
		if (line.closed || PrismaticAI.LineHalted(line)) continue;
		local slot = i + 1, k = "line" + slot;
		if (CAPITAL.skip.rawin(k)) continue;
		local trains = PrismaticAI.TrainsOf(line), running = 0, judged = 0, profit = 0, newest = -1, capacity = 0;
		foreach (t in trains) {
			if (this.selling.rawin(t)) continue;
			running++;
			capacity += max(0, AIVehicle.GetCapacity(t, line.cargo));
			if (newest == -1 || AIVehicle.GetAge(t) < newest) newest = AIVehicle.GetAge(t);
			if (AIVehicle.GetAge(t) >= full) { judged++; profit += AIVehicle.GetProfitLastYear(t); }
		}
		if (running == 0 || judged == 0 || running >= line.trains_max) continue;
		/* An external steward's target decides the line: only a train it wants and lacks is funded. */
		local external = PrismaticAI.LineTarget(slot, "trains");
		if (external >= 0 && running >= external) continue;
		/* Otherwise demand is the steward's own signals, read here so idle money buys the train the
		 * monthly round has not asked for yet: half a trainload waiting (the double-track rule), or the
		 * source station rating the cargo below steward_rating; a source platform free for it, and the
		 * newest train STEWARD_SETTLE_DAYS old, so each train's effect is measured before the next. */
		local buys = external < 0;
		if (buys) {
			if (!this.steward_on || newest < STEWARD_SETTLE_DAYS) continue;
			local free = (("platforms" in line) ? line.platforms : 1) - PrismaticAI.TrainsLoadingAt(line, line.a);
			local rating = PrismaticAI.SourceRating(line), rating_low = AIController.GetSetting("steward_rating");
			local wants = 2 * PrismaticAI.LineWaiting(line) * running >= capacity || (rating_low > 0 && rating >= 0 && rating < rating_low);
			if (free <= 0 || !wants) continue;
		}
		local c = {key = k, price = PrismaticAI.TrainPrice(trains[0]), profit = profit / judged, sample = trains[0], rail = true,
			slot = slot, line = line, running = running, buys = buys};
		if (PrismaticAI.CapitalBetter(c, best, div)) best = c;
	}
	return best;
}

/* Whether candidate `c` clears the hurdle (profit > 0 and profit x div >= price) and returns more
 * per unit of price than `best` (null: any). Cross-multiplied, so no integer division rounds. */
function PrismaticAI::CapitalBetter(c, best, div) {
	if (c.profit <= 0 || c.price <= 0 || c.profit * div < c.price) return false;
	return best == null || c.profit * best.price > best.profit * c.price;
}

/* Whether the air fleet vouches for one more air route: every air route has a plane that ran all
 * of last year (one unproven route at a time), and the planes that did earned together at least
 * their price / invest_ppv_div. Fleet level, where AirRoutesPay (autopilot.nut) asks it of every
 * route: in the live game two weak routes (375 and -1,088 a plane) held the fleet at three routes
 * while it earned 16.9k a plane. */
function PrismaticAI::CapitalAirProven(b) {
	local full = PrismaticAI.CapitalFullYear();
	local routes = {}, profit = 0, price = 0;
	foreach (v, _ in AIVehicleList()) {
		if (AIVehicle.GetVehicleType(v) != AIVehicle.VT_AIR || this.selling.rawin(v)) continue;
		local k = PrismaticAI.RouteKey(v);
		if (!routes.rawin(k)) routes[k] <- false;
		if (AIVehicle.GetAge(v) < full) continue;
		routes[k] = true;
		profit += AIVehicle.GetProfitLastYear(v);
		price += AIEngine.GetPrice(AIVehicle.GetEngineType(v));
	}
	if (routes.len() == 0) return false;
	foreach (k, judged in routes) if (!judged) return false;
	return profit > 0 && profit * max(1, b.invest_ppv_div) >= price;
}

/* The first day of game quarter `q` (PressureQuarter's year * 4 + 0..3). */
function PrismaticAI::CapitalQuarterStart(q) {
	return AIDate.GetDate(q / 4, (q % 4) * 3 + 1, 1);
}

/* A new game quarter: the previous one's report is logged, its idle verdict kept, the counters and
 * the skipped candidates cleared. The report is a warning, so a headless match logs it at
 * `-d script=3` without the info chatter, and battle.py fitness reads its frozen= and days= fields
 * (the gate on construction frozen in a contestant's first years). In the owner's live game
 * (seed 12609) construction stood frozen on 525 of the 1,096 days of 1950-1952 (47.9%). */
function PrismaticAI::CapitalQuarter(q, keep, level) {
	if (CAPITAL.quarter == q) return;
	local start = PrismaticAI.CapitalQuarterStart(q);
	if (CAPITAL.quarter >= 0 && CAPITAL.calls > 0) {
		/* The previous call's state holds up to the end of its quarter. */
		if (CAPITAL.was_frozen && CAPITAL.last >= 0 && start > CAPITAL.last) CAPITAL.frozen += start - CAPITAL.last;
		AILog.Warning("PrismaticAI: capital report " + (CAPITAL.quarter / 4) + "Q" + (CAPITAL.quarter % 4 + 1) + ": frozen=" + CAPITAL.frozen +
			" days=" + max(0, start - CAPITAL.start) + " calls=" + CAPITAL.calls + " bought=" + CAPITAL.bought + " spent=" + CAPITAL.spent +
			" holds=" + CAPITAL.holds + " idle=" + (CAPITAL.idle ? 1 : 0) + " money=" + AICompany.GetBankBalance(AICompany.COMPANY_SELF) +
			" loan=" + AICompany.GetLoanAmount() + " keep=" + keep + " level=" + level);
	}
	CAPITAL.idle_last = CAPITAL.quarter >= 0 && CAPITAL.calls > 0 && CAPITAL.idle;
	CAPITAL.quarter = q;
	/* After a load the count starts at this call; a quarter's first call counts from its first day. */
	CAPITAL.start = (CAPITAL.last >= 0) ? start : AIDate.GetCurrentDate();
	if (CAPITAL.last >= 0 && CAPITAL.last < start) CAPITAL.last = start;
	CAPITAL.idle = true;
	CAPITAL.calls = 0;
	CAPITAL.bought = 0;
	CAPITAL.spent = 0;
	CAPITAL.holds = 0;
	CAPITAL.frozen = 0;
	CAPITAL.skip = {};
}

/* The purchases of one allocation: the best candidate (CapitalBest) bought while the bank holds
 * `keep` plus its price and the purchase leaves two quarters of running costs (`afloat` is one
 * quarter's) in money and loan headroom, then the candidates measured again. With `may_hold`, a
 * candidate the bank cannot pay yet keeps its price from repayment (for a quarter at most, game
 * quarter `q`). Returns {best, held}: the last candidate seen and the money held for it. */
function PrismaticAI::CapitalBuy(b, keep, afloat, q, may_hold) {
	local held = 0, best = null;
	while (true) {
		best = this.CapitalBest(b);
		if (best == null) break;
		local bank = AICompany.GetBankBalance(AICompany.COMPANY_SELF);
		/* Never a purchase that freezes construction: after it, money and loan headroom still cover
		 * two quarters of running costs, the new vehicle's included (BuildFrozen's first test). In a
		 * replay of seed 4242 (variants.txt line 1) 39 purchases without this test left the company
		 * frozen on 26.7% of the days of 1950-1952, against 0.4% with level 0. */
		local headroom = AICompany.GetMaxLoanAmount() - AICompany.GetLoanAmount();
		local afloat_after = afloat + AIVehicle.GetRunningCost(best.sample) / 4;
		local safe = bank - best.price + headroom >= 2 * afloat_after;
		if ((!best.rail || best.buys) && bank - keep >= best.price && safe) {
			local nv = AIVehicle.CloneVehicle(best.rail ? best.line.depot : PrismaticAI.DepotFor(best.sample), best.sample, true);
			if (!AIVehicle.IsValidVehicle(nv)) {
				AILog.Warning("PrismaticAI: capital: no vehicle more on " + best.key + ": " + AIError.GetLastErrorString() + "; skipped this quarter");
				CAPITAL.skip[best.key] <- true;
				continue;
			}
			AIVehicle.StartStopVehicle(nv);
			/* The in-bot steward's target follows, so ReconcileLines keeps the train instead of selling
			 * it as one above target (lines.nut). */
			if (best.rail) {
				local cur = this.self_targets.rawin(best.slot) ? this.self_targets[best.slot] : {trains = -1, wagons = -1};
				this.self_targets[best.slot] <- {trains = best.running + 1, wagons = cur.wagons};
			}
			CAPITAL.bought++;
			CAPITAL.spent += best.price;
			AILog.Info("PrismaticAI: capital: " + (best.rail ? "train " : "vehicle ") + nv + " on " + (best.rail ? "rail " : "route ") + best.key + ", " + best.profit + " a year a vehicle on a price of " +
				best.price + " (bank " + bank + ", keep " + keep + (may_hold ? ")" : ", inside a rail search)"));
			continue;
		}
		if (!safe || !may_hold) break;
		/* Not yet affordable, or a train an external steward wants (ReconcileLines buys it): the money
		 * stays for it, for a quarter at most. */
		if (CAPITAL.hold_key != best.key) {
			CAPITAL.hold_key = best.key;
			CAPITAL.hold_quarter = q;
			AILog.Info("PrismaticAI: capital: holding " + best.price + " for " + (best.rail ? "a train of rail " : "a vehicle on route ") + best.key +
				" (" + best.profit + " a year on that price) instead of repaying the loan");
		} else if (q - CAPITAL.hold_quarter >= 2) {
			AILog.Info("PrismaticAI: capital: the money held for " + best.key + " since quarter " + CAPITAL.hold_quarter + " was not spent; skipped this quarter");
			CAPITAL.skip[best.key] <- true;
			CAPITAL.hold_key = "";
			continue;
		}
		held = best.price;
		CAPITAL.holds++;
		break;
	}
	return {best = best, held = held};
}

/* A round inside a long rail path search (steward.nut MaintenanceRound, which runs a bounded round
 * once a game month while FindRailPath works). A heartbeat on the fitness seed's planned lineup
 * came once in 183-366 days while own lines were searched and laid, and the money grew from 528k
 * to 1.25M net of the loan in 1964-65 on 21-23 vehicles. Here only purchases, from the money beyond
 * `hold` (the round's own: all of the loan, the cash reserve and one line budget, since the builder
 * lays the line from the loan it took): no repayment, no hold, no new route, never borrowing. */
function PrismaticAI::AllocateCapitalInside(hold) {
	local b = CAPITAL.b;
	local level = AIController.GetSetting("capital_alloc");
	if (b == null || level < 2) return;
	local afloat = PrismaticAI.QuarterRunningCost();
	if (this.BuildFrozen(afloat)) return;
	local keep = max(hold, max(2 * b.cash_reserve, afloat) + AICompany.GetLoanInterval());
	/* A round in a new quarter opens it, so its purchases count in that quarter's report and the
	 * last quarter's skipped candidates are tried again (a search can outlast a quarter). */
	local q = PrismaticAI.PressureQuarter();
	PrismaticAI.CapitalQuarter(q, keep, level);
	this.CapitalBuy(b, keep, afloat, q, false);
}

/* One heartbeat's allocation (see the head of this file). `afloat`: StayAfloat's floor, a quarter
 * of the fleet's running costs (main.nut QuarterRunningCost). The quarter's bookkeeping and its
 * report run at every level, 0 included, so a match measures master's frozen days the same way. */
function PrismaticAI::AllocateCapital(b, afloat) {
	local level = AIController.GetSetting("capital_alloc");
	local floor = max(2 * b.cash_reserve, afloat);
	local keep = floor + (level >= 1 ? AICompany.GetLoanInterval() : 0);
	local frozen = this.BuildFrozen(afloat);
	local q = PrismaticAI.PressureQuarter();
	PrismaticAI.CapitalQuarter(q, keep, level);
	local today = AIDate.GetCurrentDate();
	if (CAPITAL.was_frozen && CAPITAL.last >= 0 && today > CAPITAL.last) CAPITAL.frozen += today - CAPITAL.last;
	CAPITAL.last = today;
	CAPITAL.was_frozen = frozen;
	CAPITAL.calls++;
	CAPITAL.b = b;
	if (level <= 0) {
		PrismaticAI.RepaySurplus(floor);
		return;
	}
	local held = 0, best = null;
	if (level >= 2 && !frozen) {
		local r = this.CapitalBuy(b, keep, afloat, q, true);
		best = r.best;
		held = r.held;
	}
	if (best == null) CAPITAL.hold_key = "";
	/* A new air route where the fleet is proven, after a quarter of idle money; before repaying,
	 * because AutopilotAir draws the whole loan and keeps it once the route is built. */
	if (level >= 3 && !frozen && best == null && CAPITAL.idle_last) {
		CAPITAL.idle_last = false;
		if (this.CapitalAirProven(b)) {
			local b2 = clone b;
			b2.air_routes_max = this.auto_air + 1;
			b2.air_grow_money = 0;
			AILog.Info("PrismaticAI: capital: money idle a quarter and the air fleet proven; trying air route " + (this.auto_air + 1));
			this.AutopilotAir(b2);
		}
	}
	local type = PrismaticAI.AirportType(), pax = PrismaticAI.PassengerCargo();
	local plane = (type == -1 || pax == -1) ? -1 : PrismaticAI.PickPlane(type, pax);
	local unit = plane == -1 ? AICompany.GetLoanInterval() : AIEngine.GetPrice(plane);
	if (held > 0 || AICompany.GetLoanAmount() > 0 || AICompany.GetBankBalance(AICompany.COMPANY_SELF) - keep < unit) CAPITAL.idle = false;
	PrismaticAI.RepaySurplus(keep + held);
}
