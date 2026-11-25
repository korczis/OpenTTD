/*
 * Phase 2 strategy: no longer a pure idle skeleton. On start, the AI tries
 * to build one real bus route -- two stops within the town with the
 * largest population, a depot, and 1-3 buses (scaled by the externally
 * controlled `aggressiveness` setting) -- then falls back to the same
 * heartbeat loop as before either way. Deliberately scoped to a
 * single-town route: both stops sit on that town's own, already-connected
 * road network, so no intercity pathfinding is needed -- that remains a
 * separate, larger follow-up (see the bridge repo's README roadmap).
 *
 * Every build step is "try, check the actual return value, log and bail
 * cleanly on failure" -- this intentionally never assumes terrain/placement
 * rules match what's expected; OpenTTD's own API return values are the
 * source of truth.
 */

/* Ticks between two visible build steps, so a watching player sees each one happen. */
PACE <- 20;

/* Every rail line the bot built ({a, b, depot, cargo, wagons, flags, label, railtype, restored}),
 * so KeepLinesServed can put a train back on a line that lost its own. */
RAIL_LINES <- [];
/* air_full_load of the current heartbeat: BuildAirRoute is a static function, so it reads this. */
PRISMATIC_FULL_LOAD <- 0;
/* The most levelling one airport site may cost (setting air_level_max, in thousands; air.nut). */
AIR_LEVEL_MAX <- 15000;
/* Levelling a rail station site, the last resort (rail.nut LevelRailStationSite): at most this
 * estimated cost per site, corners within RAIL_LEVEL_STEP height steps, and RAIL_LEVEL_KEEP left. */
RAIL_LEVEL_MAX <- 40000;
RAIL_LEVEL_STEP <- 1;
RAIL_LEVEL_KEEP <- 60000;
/* Whether the fleet seller keeps the last plane of an air route (setting air_keep_last; fleet.nut). */
AIR_KEEP_LAST <- 1;
/* Whether one airport search stops levelling after a levelled site was refused (setting air_level_once). */
AIR_LEVEL_ONCE <- 0;
/* The most height steps between the corners of a site that is still levelled (setting air_level_step). */
AIR_LEVEL_STEP <- 0;
/* Whether a levelled airport site was refused in this game (air_level_step 0 adapts to it). */
AIR_LEVEL_REFUSED <- false;


class PrismaticAI extends AIController {
	name = null;
	used_towns = null; /* town id -> true for every town that already has one of our routes */
	routes = 0;
	air_done = ""; /* the planner air pair ("a-b") already tried */
	rail_done = ""; /* the planner rail pair ("a-b") already tried */
	pair_due_since = -1; /* the date the planner rail pair first was due (PairDue; saved, restore.nut) */
	pair_due_key = ""; /* the pair ("a-b") pair_due_since is for */
	freight_tried = null; /* the planner freight lines ("from-to-cargo") already tried, a table */
	own_built = 0; /* freight lines the bot planned itself and built (own_lines caps them) */
	own_failed = 0; /* own freight lines in a row that were not built (OWN_FAIL_PAUSE) */
	own_failed_year = 0; /* the game year own_failed counts for */
	adopt_year = -1; /* the period AdoptTrainlessLines last ran in (lines.nut AdoptKey) */
	rescued_at = -1; /* the date StayAfloat last borrowed to keep the company alive */
	was_frozen = false; /* whether the last heartbeat froze construction (BuildFrozen) */
	selling = null; /* vehicle id -> true while it drives to its depot to be sold */
	resizing = null; /* train id -> true while it drives to its depot to change its wagons (lines.nut) */
	lost = null; /* vehicle id -> the date its ET_VEHICLE_LOST arrived, until it reaches a station (events.nut) */
	orphans = null; /* vehicle id -> true: stopped in a depot with no orders at the last monthly look (events.nut) */
	month = -1; /* the game month the line records were last logged in */
	idle = null; /* route key -> heartbeats in a row with almost nothing waiting */
	self_targets = null; /* slot -> {trains, wagons}: the in-bot steward's targets (steward.nut) */
	steward_on = false; /* this heartbeat's Behaviour().self_steward == 1 (steward.nut) */
	auto_air = 0; /* autopilot air routes built (autopilot.nut) */
	air_failed = 0; /* autopilot air pairs that could not be built this game year */
	air_year = 0; /* the game year air_failed counts for */
	air_cap = 10; /* planes per route with the steward on: this heartbeat's invest_air_cap */
	air_add_waiting = 0; /* this heartbeat's air_add_waiting (fleet.nut) */
	ringed = null; /* station id -> the game year its catchment ring was last grown (catchment.nut) */
	twins = 0; /* twin rail lines built (steward.nut TwinLines) */
	full_load = 1; /* this heartbeat's air_full_load, for air routes built from here on */
	air_towns = null; /* town id -> true where an autopilot airport stands */
	air_tried = null; /* "a-b" town pairs the autopilot tried for air */
	loaded = null; /* {version, data} handed to Load(), read by Restore() (restore.nut) */
	rich = false; /* this heartbeat's Behaviour().rich (rich.nut) */
	ship_routes = -1; /* ship routes running (ships.nut; -1: not counted from the map yet) */
	ship_tried = null; /* ship route candidates already checked or built, a table (ships.nut) */
	ship_idle_year = -1; /* the game year no ship route was left to try in (ships.nut) */
	ship_cap = 6; /* this heartbeat's ships_cap: most ships per route (fleet.nut) */

	constructor() {
		this.name = AICompany.GetName(AICompany.COMPANY_SELF);
		this.used_towns = {};
		this.freight_tried = {};
		this.selling = {};
		this.resizing = {};
		this.lost = {};
		this.orphans = {};
		this.idle = {};
		this.self_targets = {};
		this.air_towns = {};
		this.air_tried = {};
		this.ringed = {};
		this.ship_tried = {};
	}
}

/* Autopilot floors (Behaviour): the profile of the battle's "solo-max" contestant, which reached
 * second place with the in-bot steward (research/ai-battle/README.md). */
const AUTOPILOT_OWN_LINES = 10;
/* With own_fail_pause=0, still at most this many failed own lines a game year. Since a failed line
 * no longer uses up own_lines (PR #234), nothing else bounds them, and g227.0 went bankrupt trying
 * failing lines one after another. */
const OWN_FAIL_YEAR_MAX = 8;
/* Below START_FLEET vehicles, the bot borrows up to its reserve plus START_ROUTE_MONEY for its next
 * road route (main.nut). */
const START_FLEET = 20;
const START_ROUTE_MONEY = 30000;
const AUTOPILOT_MAX_ROUTES = 30;
const AUTOPILOT_CASH_RESERVE = 5000;

/* After the class: intercity.nut adds methods to it. */
require("roadstop.nut");
require("intercity.nut");
require("air.nut");
require("rail.nut");
require("fleet.nut");
require("lines.nut");
require("funnel.nut");
require("steward.nut");
require("autopilot.nut");
require("catchment.nut");
require("statues.nut");
require("growth.nut");
require("rich.nut");
require("pressure.nut");
require("infra.nut");
require("modernize.nut");
require("restore.nut");
require("events.nut");
require("ships.nut");
require("capital.nut");

/* Cheapest buildable road engine that carries a passenger-class cargo. */
function PrismaticAI::PickPassengerEngine() {
	local engines = AIEngineList(AIVehicle.VT_ROAD);
	local best = -1;
	local best_price = -1;

	for (local engine = engines.Begin(); !engines.IsEnd(); engine = engines.Next()) {
		if (!AIEngine.IsBuildable(engine)) continue;
		if (!AICargo.HasCargoClass(AIEngine.GetCargoType(engine), AICargo.CC_PASSENGERS)) continue;

		local price = AIEngine.GetPrice(engine);
		if (best == -1 || price < best_price) {
			best = engine;
			best_price = price;
		}
	}
	return best;
}

/* Towns by current population, largest first: the candidates for a starter route. */
function PrismaticAI::TownsByPopulation() {
	local towns = AITownList();
	towns.Valuate(AITown.GetPopulation);
	towns.Sort(AIList.SORT_BY_VALUE, AIList.SORT_DESCENDING);
	return towns;
}

/*
 * Builds one bus route: two stops, a depot, and 1-3 buses depending on
 * `aggressiveness` (0-100), in the largest town where it fits, trying the
 * `max_towns` largest towns in turn. Returns true if at least one vehicle
 * ended up running, false if no town worked (each failure is logged).
 */
function PrismaticAI::BuildStarterRoute(aggressiveness, used_towns) {
	AIRoad.SetCurrentRoadType(AIRoad.ROADTYPE_ROAD);

	local towns = PrismaticAI.TownsByPopulation();
	if (towns.IsEmpty()) {
		AILog.Warning("PrismaticAI: no towns on the map, staying idle");
		return -1;
	}
	local max_towns = 10;
	local tried = 0;
	for (local town = towns.Begin(); !towns.IsEnd() && tried < max_towns; town = towns.Next()) {
		if (used_towns.rawin(town)) continue;
		tried++;
		if (PrismaticAI.BuildRouteInTown(town, aggressiveness)) return town;
	}
	/* Every town already has a route: nothing was tried, so there is nothing to warn about. With
	 * rich growth lifting max_routes the bot reached this every heartbeat, and the narrator relayed
	 * "no new route in the 0 largest unused town(s)" into the owner's chat (2026-11-22). */
	if (tried > 0) AILog.Warning("PrismaticAI: no new route in the " + tried + " largest unused town(s)");
	return -1;
}

/* One attempt at a bus route in `town`. See BuildStarterRoute. */
function PrismaticAI::BuildRouteInTown(town, aggressiveness) {
	PrismaticAI.Funnel("road", "tried");
	local center = AITown.GetLocation(town);
	local stop_a = PrismaticAI.BuildBusStopNear(center, 15, "A in " + AITown.GetName(town));
	if (stop_a == null) { PrismaticAI.Funnel("road", "stop_fail"); return false; }
	AIController.Sleep(PACE);
	/* Stop B in the same town, at least 8 tiles from A, so the two catchments do not overlap (center + 20 used to land in the next town). */
	local stop_b = PrismaticAI.BuildBusStopAway(center, 12, "B in " + AITown.GetName(town), stop_a);
	if (stop_b == null || stop_b == stop_a) {
		AILog.Warning("PrismaticAI: could not find a distinct site for stop B in " + AITown.GetName(town));
		PrismaticAI.Funnel("road", "stop_fail");
		return false;
	}
	if (!PrismaticAI.RoadConnected(stop_a, stop_b, 4000)) {
		AILog.Warning("PrismaticAI: stops at tiles " + stop_a + " and " + stop_b + " are on separate road networks; no buses bought");
		PrismaticAI.Funnel("road", "network_fail");
		return false;
	}
	AIController.Sleep(PACE);

	local depot = PrismaticAI.BuildDepotNear(stop_a, 10);
	if (depot == null) depot = PrismaticAI.BuildDepotNear(stop_b, 10);
	if (depot == null) {
		AILog.Warning("PrismaticAI: could not build a road depot near either stop");
		PrismaticAI.Funnel("road", "depot_fail");
		return false;
	}
	AIController.Sleep(PACE);

	local engine = PrismaticAI.PickPassengerEngine();
	if (engine == -1) {
		AILog.Warning("PrismaticAI: no buildable passenger road vehicle available");
		PrismaticAI.Funnel("road", "vehicle_fail");
		return false;
	}

	local vehicle_count = 1 + aggressiveness / 67; /* start small: 1, or 2 when aggressive; fleet management grows it */
	local built = 0;
	for (local i = 0; i < vehicle_count; i++) {
		local vehicle = AIVehicle.BuildVehicle(depot, engine);
		if (!AIVehicle.IsValidVehicle(vehicle)) {
			AILog.Warning("PrismaticAI: failed to build vehicle " + i + ": " + AIError.GetLastErrorString());
			continue;
		}
		AIOrder.AppendOrder(vehicle, stop_a, AIOrder.OF_NONE);
		AIOrder.AppendOrder(vehicle, stop_b, AIOrder.OF_NONE);
		AIVehicle.StartStopVehicle(vehicle);
		built++;
		AILog.Info("PrismaticAI: started bus " + vehicle + " at tile " + depot);
		AIController.Sleep(PACE);
	}

	AILog.Info("PrismaticAI: built a bus route in " + AITown.GetName(town) +
		" with " + built + "/" + vehicle_count + " vehicle(s) (aggressiveness=" + aggressiveness + ")");
	PrismaticAI.Funnel("road", (built > 0) ? "built" : "vehicle_fail");
	return built > 0;
}

/* Append " (AI)" to the company's name once the game has given it one (not "Unnamed"); names are
 * unique, so a taken one gets a number. Returns true when the name says so. */
function PrismaticAI::DiscloseName() {
	local name = AICompany.GetName(AICompany.COMPANY_SELF);
	if (name == null || name.find("(AI") != null) return true;
	if (name.find("Unnamed") != null) return false;
	foreach (i, suffix in [" (AI)", " (AI 2)", " (AI 3)"]) {
		if (AICompany.SetName(name + suffix)) {
			AILog.Info("PrismaticAI: the company says it is an AI: " + name + suffix);
			return true;
		}
	}
	return false;
}

/* What a quarter of the fleet's running costs comes to: the balance the bot never lets fall below. */
function PrismaticAI::QuarterRunningCost() {
	local cost = 0;
	foreach (v, _ in AIVehicleList()) cost += AIVehicle.GetRunningCost(v);
	return cost / 4;
}

/* Survival borrowing, independent of invest_borrow (which only governs borrowing to invest): OpenTTD
 * bankrupts a company whose balance stays negative quarter after quarter, and a GA mutation that spent
 * the opener on six planes went bankrupt in 1962 with half its loan unused (edition b8b04ae2). */
function PrismaticAI::StayAfloat(floor) {
	local money = AICompany.GetBankBalance(AICompany.COMPANY_SELF);
	if (money >= floor) return false;
	local loan = AICompany.GetLoanAmount(), max_loan = AICompany.GetMaxLoanAmount();
	if (loan >= max_loan) return true;
	local step = AICompany.GetLoanInterval();
	local want = min(max_loan, loan + ((floor - money) / step + 1) * step);
	if (AICompany.SetLoanAmount(want)) {
		AILog.Info("PrismaticAI: borrowed " + (want - loan) + " to stay afloat (balance was " + money + ", floor " + floor + ")");
	}
	return true;
}

/* The second half of the guard: a company that had to borrow to survive builds nothing new for a
 * while once more than half of its loan is drawn, and nothing while its money and loan headroom
 * would not run the fleet for two quarters. Survival borrowing alone could be outspent. g227.0
 * (edition 2befd3c8, seed 446449271, own_fail_pause=0) tried failing own lines one after another.
 * Stations rose from 33 to 40 while its vehicles fell from 7 to 5, and it went bankrupt in 1967. */
const FREEZE_DAYS = 90;
function PrismaticAI::BuildFrozen(afloat) {
	local money = AICompany.GetBankBalance(AICompany.COMPANY_SELF);
	local max_loan = AICompany.GetMaxLoanAmount(), loan = AICompany.GetLoanAmount();
	if (money + max_loan - loan < 2 * afloat) return true;
	return this.rescued_at >= 0 && AIDate.GetCurrentDate() - this.rescued_at < FREEZE_DAYS && 2 * loan > max_loan;
}

/* Repay the loan down to what keeps `keep` in the bank, in the game's loan steps. */
function PrismaticAI::RepaySurplus(keep) {
	local loan = AICompany.GetLoanAmount();
	if (loan <= 0) return;
	local surplus = AICompany.GetBankBalance(AICompany.COMPANY_SELF) - keep;
	local step = AICompany.GetLoanInterval();
	local repay = (surplus / step) * step;
	if (repay <= 0) return;
	local target = max(0, loan - repay);
	if (AICompany.SetLoanAmount(target)) {
		AILog.Info("PrismaticAI: repaid " + (loan - target) + " of the loan (now " + target + ")");
	}
}

/* The owner's behaviour knobs, read again at every heartbeat: all are CONFIG_INGAME, so a change made
 * while the AI runs (the settings GUI, or an MCP tool that sets them) takes effect without a restart. */
function PrismaticAI::Behaviour() {
	local b = {
		aggressiveness = AIController.GetSetting("aggressiveness"),
		heartbeat = AIController.GetSetting("heartbeat_seconds"),
		max_routes = AIController.GetSetting("max_routes"),
		cash_reserve = AIController.GetSetting("cash_reserve"),
		intercity = AIController.GetSetting("intercity"),
		air_a = AIController.GetSetting("air_a"),
		air_b = AIController.GetSetting("air_b"),
		air_planes = AIController.GetSetting("air_planes"),
		rail_a = AIController.GetSetting("rail_a"),
		rail_b = AIController.GetSetting("rail_b"),
		rail_wagons = AIController.GetSetting("rail_wagons"),
		rail_len = AIController.GetSetting("rail_len"),
		rail_platforms = AIController.GetSetting("rail_platforms"),
		rail_after = AIController.GetSetting("rail_after"),
		fr_from = AIController.GetSetting("fr_from"),
		fr_to = AIController.GetSetting("fr_to"),
		fr_cargo = AIController.GetSetting("fr_cargo"),
		freight = PrismaticAI.FreightLines(),
		line_budget = AIController.GetSetting("line_budget"),
		own_lines = AIController.GetSetting("own_lines"),
		planned_freight_only = AIController.GetSetting("planned_freight_only"),
		self_steward = AIController.GetSetting("self_steward"),
		autopilot = AIController.GetSetting("autopilot"),
		/* Strategy knobs: every one is an AI setting, so the tilde console (`bot set`), MCP
		 * openttd.ai.set_setting, Rhai api_botSetting, the panel, battle settings strings and the
		 * battle's genetic algorithm all steer them, live (info.nut). */
		air_routes_max = AIController.GetSetting("air_routes_max"),
		air_towns = AIController.GetSetting("air_towns"),
		air_min_tiles = AIController.GetSetting("air_min_tiles"),
		air_max_tiles = AIController.GetSetting("air_max_tiles"),
		air_level_max = AIController.GetSetting("air_level_max") * 1000,
		air_keep_last = AIController.GetSetting("air_keep_last"),
		air_grow_money = AIController.GetSetting("air_grow_money") * 1000,
		air_reuse = AIController.GetSetting("air_reuse"),
		air_level_once = AIController.GetSetting("air_level_once"),
		air_level_step = AIController.GetSetting("air_level_step"),
		own_fail_pause = AIController.GetSetting("own_fail_pause"),
		foreign_airports = AIController.GetSetting("foreign_airports"),
		invest_min = AIController.GetSetting("invest_min") * 1000,
		invest_ppv_div = AIController.GetSetting("invest_ppv_div"),
		invest_road_cap = AIController.GetSetting("invest_road_cap"),
		invest_air_cap = AIController.GetSetting("invest_air_cap"),
		reinvest_budgets = AIController.GetSetting("reinvest_budgets"),
		catchment_stops = AIController.GetSetting("catchment_stops"),
		catchment_spacing = AIController.GetSetting("catchment_spacing"),
		statue_money = AIController.GetSetting("statue_money"),
		air_full_load = AIController.GetSetting("air_full_load"),
		invest_borrow = AIController.GetSetting("invest_borrow"),
		air_add_waiting = AIController.GetSetting("air_add_waiting"),
		rail_twin_max = AIController.GetSetting("rail_twin_max"),
		rail_grow_max = AIController.GetSetting("rail_grow_max"),
		rail_feed = AIController.GetSetting("rail_feed"),
		rail_double = AIController.GetSetting("rail_double"),
		rich_growth = AIController.GetSetting("rich_growth"),
		rich_air = AIController.GetSetting("rich_air"),
		modernize = AIController.GetSetting("modernize"),
		compete = AIController.GetSetting("compete"),
		upgrade_track = AIController.GetSetting("upgrade_track"),
		rich_budgets = AIController.GetSetting("rich_budgets"),
		rich_air_money = AIController.GetSetting("rich_air_money") * 1000,
		rich_air_cap_max = AIController.GetSetting("rich_air_cap_max"),
		rich_invest_rounds = AIController.GetSetting("rich_invest_rounds"),
		rich_served_below = AIController.GetSetting("rich_served_below"),
		rich_min_tiles = AIController.GetSetting("rich_min_tiles"),
		rich_max_tiles = AIController.GetSetting("rich_max_tiles"),
		airport_fill = AIController.GetSetting("airport_fill"),
		pressure_low = AIController.GetSetting("pressure_low"),
		pressure_mid = AIController.GetSetting("pressure_mid"),
		pressure_high = AIController.GetSetting("pressure_high"),
		track_keep_budgets = AIController.GetSetting("track_keep_budgets"),
		ships_max = AIController.GetSetting("ships_max"),
		ships_cap = AIController.GetSetting("ships_cap"),
		ships_max_tiles = AIController.GetSetting("ships_max_tiles"),
	};
	/* Autopilot (default on): a bot nobody gave a plan (no planner line, air or rail pair) grows on
	 * its own instead of stopping at a few buses. In the LAN AI battle the unplanned default ended
	 * last in every match (research/ai-battle/README.md). It runs the in-bot steward, plans its own
	 * freight lines, connects towns and keeps a small reserve. autopilot=0 restores the unplanned bot
	 * that duels used as their rival before 2026-11-19.
	 *
	 * With a plan, the planner's lines are added on top of the autopilot, not used instead of it
	 * (`grows`): the same floors, the in-bot steward (an external steward's target always wins), its
	 * own air routes after the planner's pair, and idle money buying vehicles. A planner's own_lines
	 * or max_routes above the floor stand; intercity is on and cash_reserve at most 5000, as without a
	 * plan (planned_freight_only=1 is how a plan turns the bot's own lines off). Without the growth
	 * layer a planned bot in held-start duels ended with ~20 vehicles and no plane while the unplanned
	 * autopilot rival ran ~190 with 100 planes and won (openttd-1e, seed 4242, 120 months). Without
	 * the floors, the live
	 * settings' own_lines=4 held a planned bot at four own lines for a year while its bank doubled
	 * (ablation seed 15805, 2026-11-23: 156k to 303k in 1952 on 3-5 trains). */
	b.planned <- b.freight.len() > 0 || (b.air_a >= 0 && b.air_b >= 0) || (b.rail_a >= 0 && b.rail_b >= 0);
	b.autopiloted <- b.autopilot == 1 && !b.planned;
	b.grows <- b.autopilot == 1;
	if (b.grows) {
		b.self_steward = 1;
		b.own_lines = max(b.own_lines, AUTOPILOT_OWN_LINES);
		b.max_routes = max(b.max_routes, AUTOPILOT_MAX_ROUTES);
		b.intercity = 1;
		b.cash_reserve = min(b.cash_reserve, AUTOPILOT_CASH_RESERVE);
	}
	/* Money the caps leave idle lifts them, planned or not (rich.nut). */
	PrismaticAI.ApplyRich(b, AICompany.GetBankBalance(AICompany.COMPANY_SELF));
	/* Behind the strongest company with money idle: the fleet limits loosen with the gap (pressure.nut). */
	if (b.rich) PrismaticAI.ApplyPressure(b, AICompany.GetBankBalance(AICompany.COMPANY_SELF));
	else b.pressure <- 0;
	return b;
}

/* The planner's passenger pair (rail_a, rail_b) is due once its rail_after freight lines were tried
 * (pending slots count as untried, FreightLines); `pair_due_since` holds the date it first was.
 * Producers nobody serves are taken by rivals within the first years, and a pair of towns is not.
 * So the heartbeat gives the pair only a heartbeat in which the bot started no own freight line
 * (none left, its cap, a failure pause, no budget), or any heartbeat once the pair has waited
 * PAIR_WAIT_DAYS. Built first, the pair held the first own line back for months of path search:
 * on ablation seed 15805 (2026-11-23) the first heartbeat ran to 1950-11-15 instead of
 * 1950-04-13, Plondham - Ponninghall failed after 176 days, and the coal mine the unplanned bot
 * claimed first was gone. Over four seeds (15805, 2927, 31337, 4242; ablations against the seven
 * live AIs, 2026-11-24) a planned start cost 21.8% (geometric, planned against unplanned in one
 * tree) on master, 22.5% with the pair after the first own line only, and 3.5% with this order
 * (+10.6%, -2.5%, -21.2%, +2.1%; research/results ablation-20261124T1739* to T1820*). On seed
 * 31337 the pair, tried once its bound ran out (heartbeat 1951-04 to 1952-08), still cost 21%;
 * without the bound it was never built there, and two of four seeds never built it at all. The
 * wait is saved (restore.nut): a load keeps it, a restore from the map starts it again. */
const PAIR_WAIT_DAYS = 730;

function PrismaticAI::PairDue(b) {
	if (b.rail_a < 0 || b.rail_b < 0 || this.rail_done == b.rail_a + "-" + b.rail_b) return false;
	local tried = 0;
	foreach (line in b.freight) if (this.freight_tried.rawin(line.from + "-" + line.to + "-" + line.cargo)) tried++;
	if (tried < min(b.rail_after, b.freight.len() + FREIGHT_SLOTS_WAITING)) return false;
	/* A pair the settings changed to waits from when it is due, not from when the last one was. */
	local key = b.rail_a + "-" + b.rail_b;
	if (this.pair_due_since < 0 || this.pair_due_key != key) {
		this.pair_due_since = AIDate.GetCurrentDate();
		this.pair_due_key = key;
	}
	return true;
}

/* The planner's passenger pair, tried once, on borrowed money, when PairDue said it is due: it waits
 * like the planner's next freight line for money plus loan headroom to reach line_budget, and reads
 * the construction freeze again, since the builders before it in the heartbeat can take months.
 * Returns true when it was tried. */
function PrismaticAI::TryRailPair(b, afloat) {
	if (this.BuildFrozen(afloat)) return false;
	local headroom = AICompany.GetMaxLoanAmount() - AICompany.GetLoanAmount();
	if (AICompany.GetBankBalance(AICompany.COMPANY_SELF) + headroom < b.line_budget) return false;
	this.rail_done = b.rail_a + "-" + b.rail_b;
	/* A line pays back over years: borrow up to the limit to build it, as the planner's budget
	 * assumes; when nothing gets built, give the money back so it does not sit there at interest. */
	local loan_before = AICompany.GetLoanAmount();
	AICompany.SetLoanAmount(AICompany.GetMaxLoanAmount());
	if (PrismaticAI.BuildRailRoute(b.rail_a, b.rail_b, PrismaticAI.RailShape(b)) > 0) {
		this.routes++;
	} else {
		AICompany.SetLoanAmount(loan_before);
		AILog.Info("PrismaticAI: no rail route built; loan back to " + AICompany.GetLoanAmount());
	}
	return true;
}

/* Handed slots already reported as incoherent ("fr3_:31-5-6" -> true), so each state is logged once. */
FREIGHT_SLOT_BAD <- {};
/* Each handed slot's tuple at the previous read ("fr3_" -> "31-5-6"): a slot counts once two reads in
 * a row agree. */
FREIGHT_SLOT_SEEN <- {};
/* Reads in a row each handed slot was set (a field >= 0) but did not count ("fr3_" -> 2). */
FREIGHT_SLOT_PENDING <- {};
/* Slots of the last read that are set, do not count yet, and have not been pending for
 * SLOT_PENDING_READS reads: the heartbeat treats them as untried lines (handed_done, the pair's
 * rail_after), so a slot being rewritten does not let the bot's own line or the passenger pair run
 * ahead of it. A slot pending longer (a planner that left it incoherent) stops holding them back. */
FREIGHT_SLOTS_WAITING <- 0;
const SLOT_PENDING_READS = 6;

/* The planner's freight rail lines in its order: fr_* then fr2_* to fr16_* (as info.nut declares
 * them), those whole and coherent. The panel and the re-planner change a slot one setting at a time,
 * each approved on its own and in any order, so a heartbeat can read a slot half updated: the
 * owner's game of 2026-11-23 (:8800) tried "WOOD Grutford Forest - Ganingstone Factory" and then
 * "GRAI Grutford Forest - Ganingstone Factory", which no industry produces or accepts, and each
 * landed in freight_tried. A slot counts only when from, to and cargo are all set, both industries
 * exist, the producer makes the cargo and the acceptor takes it (FreightSlotWhyNot), and the
 * previous read held the same tuple. A slot that does not is left out of this read: not tried, not in
 * freight_tried, not counted by pair_due, handed_done or b.planned; an incoherent one is logged once
 * per state. The two reads catch a mixed tuple that is coherent by chance (an old coal line's
 * producer with a new coal line's acceptor) only while it lasts less than a heartbeat: a writer that
 * leaves it longer still gets it built. Behaviour() reads once a heartbeat, and twice at the start. */
function PrismaticAI::FreightLines() {
	local out = [];
	local waiting = 0;
	foreach (slot in PrismaticAI.FreightSlots()) {
		/* An unused slot (the usual case: most of the 16 are) costs nothing more than before. */
		if (slot[1] < 0 && slot[2] < 0 && slot[3] < 0) {
			if (FREIGHT_SLOT_SEEN.rawin(slot[0])) { delete FREIGHT_SLOT_SEEN[slot[0]]; FREIGHT_SLOT_PENDING[slot[0]] <- 0; }
			continue;
		}
		local line = {from = slot[1], to = slot[2], cargo = slot[3]};
		local tuple = line.from + "-" + line.to + "-" + line.cargo;
		local stable = FREIGHT_SLOT_SEEN.rawin(slot[0]) && FREIGHT_SLOT_SEEN[slot[0]] == tuple;
		FREIGHT_SLOT_SEEN[slot[0]] <- tuple;
		local counts = false;
		if (line.from >= 0 && line.to >= 0 && line.cargo >= 0) {
			local why = PrismaticAI.FreightSlotWhyNot(line);
			if (why != null && !FREIGHT_SLOT_BAD.rawin(slot[0] + ":" + tuple)) {
				FREIGHT_SLOT_BAD[slot[0] + ":" + tuple] <- true;
				AILog.Warning("PrismaticAI: handed freight slot " + slot[0] + " (" + tuple + ") left out: " + why);
			}
			counts = why == null && stable;
		}
		if (counts) {
			out.push(line);
			FREIGHT_SLOT_PENDING[slot[0]] <- 0;
			continue;
		}
		local reads = FREIGHT_SLOT_PENDING.rawin(slot[0]) ? FREIGHT_SLOT_PENDING[slot[0]] : 0;
		FREIGHT_SLOT_PENDING[slot[0]] <- reads + 1;
		if (reads < SLOT_PENDING_READS) waiting++;
	}
	::FREIGHT_SLOTS_WAITING = waiting;
	return out;
}

/* The handed slots as [prefix, from, to, cargo], fr_ then fr2_ to fr16_ (as info.nut declares them),
 * or the test hook's (TestFreightSlots). */
function PrismaticAI::FreightSlots() {
	if (AIController.GetSetting("test_freight_slots") == 1) return PrismaticAI.TestFreightSlots();
	local out = [];
	for (local i = 1; i <= 16; i++) {
		local p = (i == 1) ? "fr_" : ("fr" + i + "_");
		out.push([p, AIController.GetSetting(p + "from"), AIController.GetSetting(p + "to"), AIController.GetSetting(p + "cargo")]);
	}
	return out;
}

/* test_freight_slots (RESEARCH-ONLY): the line the hook hands over and the reads so far, set once. */
TEST_SLOTS <- null;
/* Reads per phase of the test hook (TestFreightSlots). */
const TEST_SLOT_READS = 6;

/* RESEARCH-ONLY test hook (setting test_freight_slots=1): handed slots written the way the panel and
 * the re-planner write them, one setting at a time, so a headless match shows that FreightLines
 * neither tries nor marks a slot until it is whole, coherent and stable. (P, Q, c) is the bot's own
 * best freight candidate when the hook first runs; Q2 is another industry accepting c. Phases of
 * TEST_SLOT_READS reads each (a read a heartbeat): 1. fr_ half written (P set, to and cargo -1);
 * 2. fr_ = P to an industry that does not accept c, and fr2_ = P to Q with a cargo P does not
 * produce; 3. fr_ flips between P - Q and P - Q2 at every read (coherent, never stable); 4. fr_ =
 * P - Q with c, built once. Run it with planned_freight_only=1, so no own line takes P first. */
function PrismaticAI::TestFreightSlots() {
	if (TEST_SLOTS == null) {
		local own = PrismaticAI.NextOwnFreightLine({});
		if (own == null) return [];
		local bad_to = -1, bad_cargo = -1, to2 = own.to;
		foreach (i, _ in AIIndustryList()) {
			local accepts = AIIndustry.IsCargoAccepted(i, own.cargo) != AIIndustry.CAS_NOT_ACCEPTED;
			if (bad_to == -1 && i != own.from && !accepts) bad_to = i;
			if (to2 == own.to && i != own.to && i != own.from && accepts) to2 = i;
		}
		local made = AICargoList_IndustryProducing(own.from);
		foreach (c, _ in AICargoList()) {
			if (AICargo.IsFreight(c) && !made.HasItem(c)) { bad_cargo = c; break; }
		}
		::TEST_SLOTS = {reads = 0, phase = 0, line = own, bad_to = bad_to, bad_cargo = bad_cargo, to2 = to2};
		AILog.Info("PrismaticAI: test_freight_slots: hands " + own.from + "-" + own.to + "-" + own.cargo + " (" +
			AIIndustry.GetName(own.from) + " - " + AIIndustry.GetName(own.to) + "), alternate acceptor " + to2 +
			", non-acceptor " + bad_to + ", cargo not produced " + bad_cargo);
	}
	local t = TEST_SLOTS, l = TEST_SLOTS.line;
	local phase = min(4, 1 + t.reads / TEST_SLOT_READS);
	if (phase != t.phase) {
		t.phase = phase;
		AILog.Info("PrismaticAI: test_freight_slots: phase " + phase + " at read " + t.reads);
	}
	t.reads++;
	if (phase == 1) return [["fr_", l.from, -1, -1]];
	if (phase == 2) return [["fr_", l.from, t.bad_to, l.cargo], ["fr2_", l.from, l.to, t.bad_cargo]];
	if (phase == 3) return [["fr_", l.from, (t.reads % 2 == 0) ? l.to : t.to2, l.cargo]];
	return [["fr_", l.from, l.to, l.cargo]];
}

/* Why the handed line {from, to, cargo} cannot run, or null when it can. */
function PrismaticAI::FreightSlotWhyNot(line) {
	if (!AICargo.IsValidCargo(line.cargo)) return "no such cargo";
	if (!AIIndustry.IsValidIndustry(line.from)) return "no industry " + line.from;
	if (!AIIndustry.IsValidIndustry(line.to)) return "no industry " + line.to;
	if (line.from == line.to) return "one industry at both ends";
	local cargo = AICargo.GetCargoLabel(line.cargo);
	if (!AICargoList_IndustryProducing(line.from).HasItem(line.cargo)) return AIIndustry.GetName(line.from) + " does not produce " + cargo;
	if (AIIndustry.IsCargoAccepted(line.to, line.cargo) == AIIndustry.CAS_NOT_ACCEPTED) return AIIndustry.GetName(line.to) + " does not accept " + cargo;
	return null;
}

function PrismaticAI::Start() {
	local b = PrismaticAI.Behaviour();
	AILog.Info("PrismaticAI starting for " + this.name + " (aggressiveness=" + b.aggressiveness +
		", max_routes=" + b.max_routes + ", cash_reserve=" + b.cash_reserve + ", heartbeat_seconds=" + b.heartbeat +
		", air_a=" + b.air_a + ", air_b=" + b.air_b + ", air_planes=" + b.air_planes +
		", rail_a=" + b.rail_a + ", rail_b=" + b.rail_b + ", rail_wagons=" + b.rail_wagons +
		", rail_len=" + b.rail_len + ", rail_platforms=" + b.rail_platforms + ")");

	this.Sleep(10); /* let the world/company finish settling before touching it */
	/* Old vehicles are renewed by the game itself, six months before their maximum age, when the
	 * company has 30000 to spare: in a 21-year duel the bot's trains fell from five to three. */
	AICompany.SetAutoRenewStatus(true);
	AICompany.SetAutoRenewMonths(-6);
	AICompany.SetAutoRenewMoney(30000);
	/* A restarted bot (a server save and load, or a new script deployed into a running game) takes
	 * its lines, routes and tried plans from its save, or else from the map (restore.nut): without
	 * this it would build the planner's lines a second time next to the ones it runs. */
	this.Restore(PrismaticAI.Behaviour());
	/* A finished line that lost its first train before a restart, or before the bot kept such lines
	 * (rail.nut KeepTrainlessLine), is taken back as a line; then once a game year (lines.nut). */
	this.adopt_year = PrismaticAI.AdoptKey();
	this.AdoptTrainlessLines();
	this.MaintenanceAttach(b.cash_reserve); /* the steward's rounds inside long path searches (steward.nut) */

	local last = "";
	local disclosed = false;
	while (true) {
		b = PrismaticAI.Behaviour();
		/* A real game against people: the company says it is an AI (research/real-game.md). The game
		 * names a company after its first station's town, so this waits for that name. */
		if (!disclosed && AIController.GetSetting("disclose_name") == 1) disclosed = PrismaticAI.DiscloseName();
		this.steward_on = b.self_steward == 1;
		/* The rounds inside this heartbeat's rail searches buy with these settings (capital.nut
		 * AllocateCapitalInside). Set only by AllocateCapital at the heartbeat's end, it was null
		 * through the first heartbeat after a start or a load, so no round bought anything there. */
		CAPITAL.b = b;
		local afloat = PrismaticAI.QuarterRunningCost();
		if (PrismaticAI.StayAfloat(afloat)) this.rescued_at = AIDate.GetCurrentDate();
		local frozen = this.BuildFrozen(afloat);
		if (frozen != this.was_frozen) {
			AILog.Info("PrismaticAI: construction " + (frozen ? "frozen: the company borrowed to stay afloat" : "resumed"));
			this.was_frozen = frozen;
		}
		/* Before anything is built this heartbeat: the first air route reads it too. */
		PRISMATIC_FULL_LOAD = b.air_full_load;
		AIR_LEVEL_MAX = b.air_level_max;
		AIR_KEEP_LAST = b.air_keep_last;
		AIR_LEVEL_ONCE = b.air_level_once;
		AIR_LEVEL_STEP = b.air_level_step;
		local knobs = "aggressiveness=" + b.aggressiveness + " max_routes=" + b.max_routes + " cash_reserve=" + b.cash_reserve +
			" intercity=" + b.intercity + " self_steward=" + b.self_steward + " own_lines=" + b.own_lines +
			(b.autopiloted ? " autopilot=engaged" : (b.grows ? " autopilot=growth-on-plan" : "")) +
			(b.planned_freight_only == 1 ? " planned_freight_only=1" : "") +
			(b.rich ? " rich=1" : "") +
			(b.ships_max > 0 ? " ships_max=" + b.ships_max : "");
		if (knobs != last) {
			AILog.Info("PrismaticAI behaviour: " + knobs);
			last = knobs;
		}

		local money = AICompany.GetBankBalance(AICompany.COMPANY_SELF);
		/* The planner's air route first: by the game's payment formula the long route between big
		 * towns earns the most per passenger. Tried once per pair the owner gives. */
		local air_key = b.air_a + "-" + b.air_b;
		if (!frozen && b.air_a >= 0 && b.air_b >= 0 && this.air_done != air_key && money > b.cash_reserve) {
			this.air_done = air_key;
			/* Built on borrowed money like every other route: on cash alone the airports were built
			 * and the plane was not (ERR_NOT_ENOUGH_CASH), leaving two empty airports. */
			local loan_before = AICompany.GetLoanAmount();
			AICompany.SetLoanAmount(AICompany.GetMaxLoanAmount());
			local flying = PrismaticAI.BuildAirRoute(b.air_a, b.air_b, b.air_planes);
			if (flying == 0) AICompany.SetLoanAmount(loan_before);
			if (flying > 0) {
				this.routes++;
				/* The planner's towns hold airports now: the autopilot's own pairs leave them out. */
				this.air_towns[b.air_a] <- true;
				this.air_towns[b.air_b] <- true;
			}
			money = AICompany.GetBankBalance(AICompany.COMPANY_SELF);
		}
		/* Without a plan, the autopilot's own air routes come first: the gap to the winner opens in the
		 * first quarters, and a plane between two big towns earns the most per vehicle (autopilot.nut). */
		if (!frozen && this.AutopilotAir(b)) money = AICompany.GetBankBalance(AICompany.COMPANY_SELF);
		/* The portfolio's freight first: by the game's payment rates, coal and wood by rail pay most.
		 * The planner's first line is built at once; each next one (fr2_* to fr16_*) once the bank
		 * balance plus the loan headroom reach line_budget, one line per heartbeat, each tried once. */
		local first = true;
		foreach (line in b.freight) {
			local fr_key = line.from + "-" + line.to + "-" + line.cargo;
			if (this.freight_tried.rawin(fr_key)) { first = false; continue; }
			if (frozen) break;
			local headroom = AICompany.GetMaxLoanAmount() - AICompany.GetLoanAmount();
			if (!first && money + headroom < b.line_budget) break;
			this.freight_tried[fr_key] <- true;
			local loan_before = AICompany.GetLoanAmount();
			AICompany.SetLoanAmount(AICompany.GetMaxLoanAmount());
			if (!first) AILog.Info("PrismaticAI: money " + money + " and loan headroom " + headroom + " reach " + b.line_budget + ": the planner's next freight line");
			if (PrismaticAI.BuildFreightRailRoute(line.from, line.to, line.cargo, PrismaticAI.RailShape(b)) > 0) {
				this.routes++;
			} else {
				AICompany.SetLoanAmount(loan_before);
				AILog.Info("PrismaticAI: no freight route built; loan back to " + AICompany.GetLoanAmount());
			}
			money = AICompany.GetBankBalance(AICompany.COMPANY_SELF);
			break;
		}
		/* Ships (ships.nut; ships_max, default 0: none): one water route a heartbeat, from an oil rig
		 * or a coastal producer to an acceptor by the sea, or between two coastal towns, before the
		 * bot's own rail lines take the money. */
		this.ship_cap = b.ships_cap;
		if (!frozen && this.ShipRoutes(b)) money = AICompany.GetBankBalance(AICompany.COMPANY_SELF);
		/* The bot's own plan: once every freight line the planner handed over was tried, money no
		 * longer waits for a new plan from outside. It does not wait for the planner's passenger pair
		 * either (TryRailPair): the pair is added on top of the autopilot's growth, not instead of it.
		 * With the script budget it was meant to have, the bot built all fourteen planned lines by month
		 * 24 on OpenTTD seed 31337 and then held 529983 idle at month 36 (research/results
		 * duel-20261117T112500).
		 * One line per heartbeat, when money plus loan headroom reach line_budget, at most own_lines. */
		/* The planner's passenger pair (TryRailPair) is added on top of the bot's own freight lines: it
		 * takes a heartbeat in which no own line was started, or any heartbeat once it has waited
		 * PAIR_WAIT_DAYS since it became due (PairDue). */
		local pair_due = this.PairDue(b);
		local own_started = false;
		local handed_done = FREIGHT_SLOTS_WAITING == 0;
		foreach (line in b.freight) if (!this.freight_tried.rawin(line.from + "-" + line.to + "-" + line.cargo)) handed_done = false;
		local headroom_own = AICompany.GetMaxLoanAmount() - AICompany.GetLoanAmount();
		local own_cap = this.OwnLinesCap(b, money);
		/* A failed line still costs its stations and part of its track. With own_fail_pause, after that
		 * many failures in a row the bot tries no more own lines that game year (0: never pauses): on OpenTTD seed 1439892342 it tried line
		 * after line on a map whose stations had no way out, held the whole loan and ended at value 1. */
		local year_now = AIDate.GetYear(AIDate.GetCurrentDate());
		if (year_now != this.own_failed_year) { this.own_failed_year = year_now; this.own_failed = 0; }
		/* planned_freight_only (RESEARCH-ONLY): one authority outside the bot decides every freight
		 * line, so the bot plans none of its own; the handed-over lines above still get built. */
		if (!frozen && b.planned_freight_only == 0 && handed_done && this.own_built < own_cap && this.own_failed < (b.own_fail_pause == 0 ? OWN_FAIL_YEAR_MAX : b.own_fail_pause) &&
				money + headroom_own >= b.line_budget) {
			local own = PrismaticAI.NextOwnFreightLine(this.freight_tried, b.rail_feed, b.rich);
			if (own != null) {
				own_started = true;
				this.freight_tried[own.from + "-" + own.to + "-" + own.cargo] <- true;
				this.own_built++;
				AILog.Info("PrismaticAI: own plan " + this.own_built + "/" + own_cap + ": " + AICargo.GetCargoLabel(own.cargo) + " " +
					AIIndustry.GetName(own.from) + " - " + AIIndustry.GetName(own.to) + ", " + own.distance + " tiles, " +
					own.production + " a month, score " + own.score);
				local loan_before = AICompany.GetLoanAmount();
				AICompany.SetLoanAmount(AICompany.GetMaxLoanAmount());
				if (PrismaticAI.BuildFreightRailRoute(own.from, own.to, own.cargo, PrismaticAI.RailShape(b, true)) > 0) {
					this.routes++;
					this.own_failed = 0;
					/* Multi-source: the cluster the planner scored joins the new source station (growth.nut). */
					if (b.rail_feed > 0) this.FeedNewLine(RAIL_LINES.len(), b.rail_feed);
				} else {
					this.own_failed++;
					/* A line that was not built does not use up own_lines: only own_fail_pause limits
					 * the failures, per game year. Counted as built, three lines refused for "no rail
					 * station site" on the hills of the owner's live game (seed 15805, 1954) took
					 * three of its four own lines, and the company ran one train from then on. A
					 * load already counted only the built ones (restore.nut). */
					this.own_built--;
					AICompany.SetLoanAmount(loan_before);
					AILog.Info("PrismaticAI: no freight route built; loan back to " + AICompany.GetLoanAmount());
				}
				money = AICompany.GetBankBalance(AICompany.COMPANY_SELF);
			}
		}
		if (!frozen && pair_due && (!own_started || AIDate.GetCurrentDate() - this.pair_due_since >= PAIR_WAIT_DAYS) &&
				this.TryRailPair(b, afloat)) money = AICompany.GetBankBalance(AICompany.COMPANY_SELF);
		/* A small fleet with no air or rail route borrows for its next road route: RepaySurplus pays the loan down to twice the
		 * reserve after every heartbeat, and a bot whose air and rail openers failed then had too
		 * little in the bank for any route. On OpenTTD seed 988774025 (company 3) it kept 3 buses on
		 * about 1000 with 100000 of loan unused, and went bankrupt. Unspent, the loan is repaid again
		 * at the end of the heartbeat. */
		if (!frozen && this.routes < b.max_routes && money <= b.cash_reserve + START_ROUTE_MONEY &&
				this.auto_air == 0 && RAIL_LINES.len() == 0 && AIVehicleList().Count() < START_FLEET) {
			local step = AICompany.GetLoanInterval();
			local want = AICompany.GetLoanAmount() + ((b.cash_reserve + START_ROUTE_MONEY - money) / step + 1) * step;
			if (AICompany.SetLoanAmount(min(want, AICompany.GetMaxLoanAmount()))) {
				money = AICompany.GetBankBalance(AICompany.COMPANY_SELF);
			}
		}
		if (!frozen && this.routes < b.max_routes && money > b.cash_reserve && PrismaticAI.RoadPays()) {
			local pair = (b.intercity == 1) ? PrismaticAI.BuildIntercityRoute(b.aggressiveness, this.used_towns) : null;
			if (pair != null) {
				this.used_towns[pair[0]] <- true;
				this.used_towns[pair[1]] <- true;
				this.routes++;
				AILog.Info("PrismaticAI: route " + this.routes + "/" + b.max_routes + " in " + AITown.GetName(pair[0]));
			} else {
				local town = PrismaticAI.BuildStarterRoute(b.aggressiveness, this.used_towns);
				if (town != -1) {
					this.used_towns[town] <- true;
					this.routes++;
					AILog.Info("PrismaticAI: route " + this.routes + "/" + b.max_routes + " in " + AITown.GetName(town));
				}
			}
		}

		/* The game's events: lost and unprofitable vehicles are checked, their line repaired or the
		 * vehicle sold, engine previews kept for AcceptPreviews (events.nut). */
		this.HandleEvents(b);
		/* Capacity follows demand: add where cargo piles up, sell what keeps losing money. */
		this.air_cap = b.invest_air_cap;
		this.air_add_waiting = b.air_add_waiting;
		this.ManageFleet(b.cash_reserve);
		/* With the steward on, idle money buys vehicles where vehicles earn most (fleet.nut). */
		/* Rich: several vehicles a heartbeat, because a heartbeat whose route builds fail can take
		 * months; the self-steward fitness bot bought one vehicle per heartbeat and its money rose
		 * from 485k to 1.1M on 27 vehicles in 1963-65 (rich.nut, setting rich_invest_rounds). */
		if (b.self_steward == 1) {
			for (local i = 0; i < (b.rich ? b.rich_invest_rounds : 1); i++) this.InvestSurplus(b);
		}
		/* Mechanisms learned from AAAHogEx, each behind its own setting (catchment.nut, statues.nut). */
		if (!frozen) {
			this.GrowCatchment(b);
			this.BuildStatues(b);
		}
		/* Adopting builds nothing (lines.nut), so a frozen company does it too: the purchase that
		 * follows waits for money in KeepLinesServed. */
		if (PrismaticAI.AdoptKey() != this.adopt_year) {
			this.adopt_year = PrismaticAI.AdoptKey();
			PrismaticAI.TestForgetTrainless();
			this.CloseHaltedLines();
			this.AdoptTrainlessLines();
		}
		/* Every rail line keeps a train: one lost at a level crossing or sold is replaced, and a line
		 * that never had one (KeepTrainlessLine, AdoptTrainlessLines) gets its first. */
		PrismaticAI.KeepLinesServed(b.cash_reserve);
		/* The growth funnel's heartbeat counts and, once a game year, its lines (funnel.nut). */
		PrismaticAI.FunnelTick(b);
		/* Each line runs what the steward's targets say (lines.nut), and its record is logged again
		 * once a game month so a steward started late still finds it. */
		/* With self_steward, the bot sets its own line targets once a game month (steward.nut). */
		if (b.self_steward == 1 && PrismaticAI.MonthKey() != this.month) {
			this.SelfSteward();
			if (!frozen) this.TwinLines(b);
			if (!frozen) this.GrowPlatforms(b);
			if (!frozen) this.FeedStations(b);
		}
		this.ReconcileLines(b.cash_reserve);
		/* Better engines than the ones it runs replace them at their next depot visit (modernize.nut). */
		/* Only with money idle (b.rich): a replacement costs a new vehicle's price (modernize.nut). */
		if (b.modernize == 1 && PrismaticAI.MonthKey() != this.month) {
			if (b.rich) this.Modernize(b);
			else PrismaticAI.ModernizeHold();
		}
		/* Track follows engines: one network a month moves to a better rail type it can afford (infra.nut). */
		if (!frozen && b.rich && b.upgrade_track == 1 && PrismaticAI.MonthKey() != this.month) this.UpgradeTrack(b);
		/* New models offered to us first: taken, like AAAHogEx does (modernize.nut). */
		if (b.modernize == 1 && b.rich) PrismaticAI.AcceptPreviews();
		/* Vehicles stopped in a depot with no orders get a line's orders or are sold; broken lines are
		 * walked again (events.nut). */
		if (PrismaticAI.MonthKey() != this.month) this.LookAfterVehicles();
		if (PrismaticAI.MonthKey() != this.month) { this.month = PrismaticAI.MonthKey(); PrismaticAI.LogLines(); }
		/* Money above the reserve buys the best measured investment, else repays the loan (capital.nut). */
		this.AllocateCapital(b, afloat);
		PrismaticAI.StayAfloat(afloat);

		AILog.Info("PrismaticAI heartbeat: date=" + AIDate.GetCurrentDate() +
			" money=" + AICompany.GetBankBalance(AICompany.COMPANY_SELF) +
			" vehicles=" + AIVehicleList().Count() + " routes=" + this.routes + "/" + b.max_routes +
			/* The caps in force, not the raw settings: own_lines=8 read as the cap while the real one
			 * was 32, and hid where growth stopped (rich.nut). */
			" own_cap=" + own_cap + " air_cap=" + b.invest_air_cap + " " + knobs);

		local ticks = b.heartbeat * 74 / 30; // ~heartbeat_seconds of ticks (74 ticks/sec baseline)
		this.Sleep(ticks > 0 ? ticks : 1);
	}
}
