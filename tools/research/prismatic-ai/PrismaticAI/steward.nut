/*
 * The in-bot steward (RESEARCH-ONLY, opt-in: setting self_steward=1, default 0).
 *
 * The bot never decides what a rail line runs: the Rhai steward does
 * (tools/research/mcp-panel/rhai/rail_steward.rhai with rail_lib.rhai) and hands its targets over as
 * the settings line<slot>_trains / line<slot>_wagons (lines.nut). A bot started without the panel
 * has no steward, so every line keeps the one train it was built with, and the money its lines earn
 * piles up unspent. In the LAN AI battle (research/ai-battle/README.md) PrismaticAI solo-max grew to
 * about 20 vehicles by its second year, then fell to 11 while its bank balance rose to millions.
 *
 * With self_steward=1 this file applies the same policy as rail_lib.rhai::target, inside the bot,
 * to every line that has no external target (setting -1). An external target always wins, so a bot
 * the panel steers behaves exactly as before. Rules (the policy of rail_lib.rhai::policy):
 *   - close: the oldest train is judge_after_days old and the line lost money last year and this year
 *   - lengthen: cargo waiting >= grow_loads trainloads and the trains are shorter than the platform
 *   - add a train: as above with full-length trains, while trains < trains_max
 *   - add a train for the rating (setting steward_rating): the line earns, its source station rates
 *     the cargo below steward_rating percent, a source platform is free and the newest train is
 *     STEWARD_SETTLE_DAYS old, while trains < trains_max (see below)
 *   - shrink: more than one train, waiting < shrink_loads trainloads, and the newest train is old
 *     enough to judge and lost money last year
 * One change per line per game month, like the Rhai steward's rounds.
 */

const STEWARD_JUDGE_AFTER_DAYS = 400;
const STEWARD_GROW_LOADS = 1.0;
const STEWARD_SHRINK_LOADS = 0.25;

/* Why waiting cargo alone kept lines at one or two trains. Every line runs full-load orders, so a
 * train at the source takes the cargo as it comes, and the cargo waiting reaches a trainload only
 * when the trains cannot carry it at all. A line with too few trains shows up instead as a low
 * station rating: the rating falls with the days since the last pickup (station_cmd.cpp
 * UpdateStationRating), and the industry moves only rating-percent of its output to the station.
 * In six-year matches of master f4c02d0aac (2026-11-22, live settings, start 1950, seeds 15805, 2927,
 * 31337 and 4242, battle.py ablate) the steward decided one to four trains per match, all by the
 * double-track rule; the 27 lines peaked at one to three trains, the double-track ones of 15-37
 * allowed, the single-track ones mostly at one of two, and the company ended with 23k-1.44M in the
 * bank, while AAAHogEx ran 38-59 trains at about the same profit per train. The rating rule grows
 * such a line by one train, and one more for every STEWARD_RATING_STEP points below the threshold,
 * judged again once the newest has run STEWARD_SETTLE_DAYS. It stops where every source platform
 * holds a loading train (more trains would only queue for cargo), at trains_max, or once the line
 * stops earning. research/rail-steward.md has the measurement. */
const STEWARD_SETTLE_DAYS = 90;
const STEWARD_RATING_STEP = 15;
/* The game year SelfSteward last logged every line's state in. */
STEWARD_STATE_YEAR <- -1;

/* STEWARD_ROUND_NOTE: a round may add more than one train to a line. The steward runs once a game
 * month at most, on a heartbeat, and a heartbeat can take most of a year: on the fitness seed
 * (930967890, the self-steward lineup) the line records, logged on the same monthly gate, appeared
 * about once a game year from 1963 on, while rail path searches took 90-178 game days each.
 * ReconcileLines buys every missing train of a target at once. */

/* Whether `s` (LineStats) earns: the line's profit last year and this year together is positive,
 * and its newest train, once old enough to judge, did not lose money last year. */
function PrismaticAI::LineEarns(s) {
	if (s.trains == 0 || s.profit_last + s.profit_this <= 0) return false;
	return !(s.newest_age >= STEWARD_JUDGE_AFTER_DAYS && s.newest_profit_last < 0);
}

/* The rating, in percent, of `line`'s cargo at its source station, or -1 while it has none. */
function PrismaticAI::SourceRating(line) {
	local st = AIStation.GetStationID(line.a);
	if (!AIStation.IsValidStation(st) || !AIStation.HasCargoRating(st, line.cargo)) return -1;
	return AIStation.GetCargoRating(st, line.cargo);
}
/* Beyond own_lines, a further own line needs this many line budgets in the bank (not on loan). */

/* No more own lines than this however rich the bot is (the planner's own cap is 16 handed lines). */
const STEWARD_OWN_LINES_MAX = 32;

/* What `line`'s trains carry and earn: {trains, capacity, wagons, profit_last, profit_this, oldest,
 * newest, newest_profit_last}. Trains on their way to be sold do not count. */
function PrismaticAI::LineStats(line) {
	local s = { trains = 0, capacity = 0, wagons = 0, profit_last = 0, profit_this = 0, oldest = 0,
		newest = -1, newest_age = 0, newest_profit_last = 0 };
	foreach (t in PrismaticAI.TrainsOf(line)) {
		if (this.selling.rawin(t)) continue;
		s.trains++;
		s.capacity += max(0, AIVehicle.GetCapacity(t, line.cargo));
		s.wagons = max(s.wagons, AIVehicle.GetNumWagons(t) - 1);
		s.profit_last += AIVehicle.GetProfitLastYear(t);
		s.profit_this += AIVehicle.GetProfitThisYear(t);
		local age = AIVehicle.GetAge(t);
		s.oldest = max(s.oldest, age);
		if (s.newest == -1 || age < s.newest_age) {
			s.newest = t;
			s.newest_age = age;
			s.newest_profit_last = AIVehicle.GetProfitLastYear(t);
		}
	}
	return s;
}

/* The most of the line's cargo waiting at either of its stations. */
function PrismaticAI::LineWaiting(line) {
	local w = 0;
	foreach (tile in [line.a, line.b]) {
		local st = AIStation.GetStationID(tile);
		if (AIStation.IsValidStation(st)) w = max(w, AIStation.GetCargoWaiting(st, line.cargo));
	}
	return w;
}

/* Whether a train of `line` stands loading or unloading at the station of `tile`. */
function PrismaticAI::TrainLoadingAt(line, tile) {
	return PrismaticAI.TrainsLoadingAt(line, tile) > 0;
}

/* How many trains of `line` stand loading or unloading at the station of `tile`. */
function PrismaticAI::TrainsLoadingAt(line, tile) {
	local st = AIStation.GetStationID(tile), n = 0;
	foreach (t in PrismaticAI.TrainsOf(line)) {
		if (AIVehicle.GetState(t) != AIVehicle.VS_AT_STATION) continue;
		if (AIStation.GetStationID(AIVehicle.GetLocation(t)) == st) n++;
	}
	return n;
}

/* The target for `slot` (lines.nut reads this instead of the setting alone): the external steward's
 * setting when it set one, else the in-bot steward's own target when self_steward is on, else -1. */
function PrismaticAI::EffectiveTarget(slot, what) {
	local external = PrismaticAI.LineTarget(slot, what);
	if (external >= 0 || !this.steward_on) return external;
	if (!this.self_targets.rawin(slot)) return -1;
	return this.self_targets[slot][what];
}

/* One round of the in-bot steward: at most one change per line, logged like the Rhai steward's. */
function PrismaticAI::SelfSteward() {
	local rating_low = AIController.GetSetting("steward_rating");
	if (rating_low == null) rating_low = 0;
	/* On the first round of every game year each line's state, so a log shows what held each line
	 * (the evidence above); a round can come months late (STEWARD_ROUND_NOTE). */
	local year = AIDate.GetYear(AIDate.GetCurrentDate());
	local yearly = year != STEWARD_STATE_YEAR;
	STEWARD_STATE_YEAR = year;
	foreach (i, line in RAIL_LINES) {
		if (line.closed) continue;
		local slot = i + 1;
		if (PrismaticAI.LineTarget(slot, "trains") >= 0 || PrismaticAI.LineTarget(slot, "wagons") >= 0) continue;
		if (!("wagons_fit" in line)) PrismaticAI.LineRecord(slot, line);
		local s = this.LineStats(line);
		if (s.trains == 0) continue;
		local cur = this.self_targets.rawin(slot) ? this.self_targets[slot] : { trains = -1, wagons = -1 };
		local want = { trains = cur.trains, wagons = cur.wagons };
		local why = null;
		if (s.oldest >= STEWARD_JUDGE_AFTER_DAYS && s.profit_last < 0 && s.profit_this < 0) {
			want.trains = 0;
			why = "close: lost " + s.profit_last + " last year and " + s.profit_this + " this year";
		} else if (s.capacity > 0) {
			local waiting = PrismaticAI.LineWaiting(line);
			local loads = waiting.tofloat() / s.capacity.tofloat();
			local fit = ("wagons_fit" in line) ? line.wagons_fit : s.wagons;
			local free_platforms = (("platforms" in line) ? line.platforms : 1) - PrismaticAI.TrainsLoadingAt(line, line.a);
			local rating = PrismaticAI.SourceRating(line);
			if (yearly) {
				AILog.Info("PrismaticAI: steward state line " + slot + " (" + line.label + "): " + s.trains + " of " + line.trains_max +
					" trains" + (("path2" in line) ? " (double track)" : "") + ", " + waiting + " waiting, trainload " + (s.capacity / s.trains) +
					", rating " + rating + "% at the source, " + free_platforms + " source platform(s) free, profit " + s.profit_last +
					" last year and " + s.profit_this + " this year, newest train " + s.newest_age + " days");
			}
			if (("path2" in line) && s.trains < line.trains_max && 2 * waiting * s.trains >= s.capacity &&
					free_platforms > 0) {
				/* Double track (rail.nut): after AAAHogEx, a train more while half a trainload waits
				 * and a source platform is free to take it. "No train loading" almost never held with a
				 * full load at the source, so the second track carried no extra train. One train more for
				 * every full trainload waiting beyond that (STEWARD_ROUND_NOTE). */
				want.trains = min(line.trains_max, s.trains + max(1, waiting * s.trains / s.capacity));
				why = "add_train (double track): " + waiting + " waiting, half a train is " + (s.capacity / (2 * s.trains)) +
					", " + free_platforms + " source platform(s) free; " + s.trains + " -> " + want.trains + " of " + line.trains_max;
			} else if (loads >= STEWARD_GROW_LOADS && s.wagons < fit) {
				local per_wagon = max(1, s.capacity / max(1, s.trains * s.wagons));
				local more = max(1, min(fit - s.wagons, (waiting + per_wagon - 1) / per_wagon));
				want.wagons = s.wagons + more;
				why = "lengthen: " + waiting + " waiting is " + loads + " trainloads; " + s.wagons + " -> " + want.wagons + " wagons (platform holds " + fit + ")";
			} else if (loads >= STEWARD_GROW_LOADS && s.trains < line.trains_max) {
				want.trains = s.trains + 1;
				why = "add_train: " + waiting + " waiting with full-length trains; " + s.trains + " -> " + want.trains;
			} else if (rating_low > 0 && rating >= 0 && rating < rating_low && s.trains < line.trains_max && free_platforms > 0 &&
					s.newest_age >= STEWARD_SETTLE_DAYS && PrismaticAI.LineEarns(s)) {
				/* One train, and one more for every STEWARD_RATING_STEP points the rating is lower. */
				want.trains = min(line.trains_max, s.trains + 1 + (rating_low - rating) / STEWARD_RATING_STEP);
				why = "add_train (rating): " + rating + "% at the source, below " + rating_low + "%, " + free_platforms +
					" source platform(s) free, profit " + s.profit_last + " last year and " + s.profit_this + " this year; " +
					s.trains + " -> " + want.trains + " of " + line.trains_max;
			} else if (s.trains > 1 && loads < STEWARD_SHRINK_LOADS && s.newest_age >= STEWARD_JUDGE_AFTER_DAYS && s.newest_profit_last < 0) {
				want.trains = s.trains - 1;
				why = "shrink: " + waiting + " waiting and train " + s.newest + " lost " + s.newest_profit_last;
			}
		}
		if (why == null || (want.trains == cur.trains && want.wagons == cur.wagons)) continue;
		this.self_targets[slot] <- want;
		AILog.Info("PrismaticAI: self-steward line " + slot + " (" + line.label + "): " + why);
	}
}

/* How many own lines the bot may plan: own_lines, or up to STEWARD_OWN_LINES_MAX with self_steward
 * once the bank (not the loan) holds `reinvest_budgets` line budgets. A rich bot (rich.nut) has no
 * cap but money and own_fail_pause: on the owner's map 27 of 32 tries were spent by 1958 with 3.7M
 * idle. */
function PrismaticAI::OwnLinesCap(b, money) {
	if (("rich" in b) && b.rich) return max(STEWARD_OWN_LINES_MAX, this.own_built + 1);
	if (b.self_steward == 1 && this.own_built >= b.own_lines && money >= b.reinvest_budgets * b.line_budget) {
		return STEWARD_OWN_LINES_MAX;
	}
	return b.own_lines;
}

/* Twin lines (setting rail_twin_max; after AAAHogEx's double track, which let its rail reach ~2M a
 * year). A line holds two trains on its single track. When one runs all of them, has run at least
 * TWIN_MIN_AGE days, and still has a full trainload waiting, a second line goes between the same
 * two industries: its own track and platforms beside the first one's, its own trains. One twin per
 * line, at most rail_twin_max, one per game month, when money and loan headroom reach line_budget. */
const TWIN_MIN_AGE = 180;

function PrismaticAI::TwinLines(b) {
	if (b.rail_twin_max <= 0 || this.twins >= b.rail_twin_max) return;
	local headroom = AICompany.GetMaxLoanAmount() - AICompany.GetLoanAmount();
	if (AICompany.GetBankBalance(AICompany.COMPANY_SELF) + headroom < b.line_budget) return;
	foreach (i, line in RAIL_LINES) {
		if (line.closed || !("from" in line) || ("twinned" in line)) continue;
		if (!AIIndustry.IsValidIndustry(line.from) || !AIIndustry.IsValidIndustry(line.to)) continue;
		local s = this.LineStats(line);
		if (s.trains < line.trains_max || s.capacity <= 0 || s.oldest < TWIN_MIN_AGE) continue;
		local waiting = PrismaticAI.LineWaiting(line);
		if (waiting < s.capacity / max(1, s.trains)) continue;
		line.twinned <- true;
		AILog.Info("PrismaticAI: twin for rail line " + (i + 1) + " (" + line.label + "): " + s.trains + " trains running, " + waiting + " waiting");
		local loan_before = AICompany.GetLoanAmount();
		AICompany.SetLoanAmount(AICompany.GetMaxLoanAmount());
		if (PrismaticAI.BuildFreightRailRoute(line.from, line.to, line.cargo, PrismaticAI.RailShape(b)) > 0) {
			this.twins++;
			this.routes++;
			RAIL_LINES.top().twinned <- true;
		} else {
			AICompany.SetLoanAmount(loan_before);
			AILog.Info("PrismaticAI: twin not built; loan back to " + AICompany.GetLoanAmount());
		}
		return;
	}
}
