/*
 * Competitive pressure (RESEARCH-ONLY; setting compete, default 1): the further the bot falls
 * behind the strongest other company while its money sits idle, the more its fleet limits
 * loosen. Limits that were tuned once and never move let a rival build, improve and modernize past
 * us while our bank fills up.
 *
 * The incident: in the owner's live co-play game (2026-11-22, seed 20261122) our company 0 ran 37
 * vehicles with a company value of 4.3M and 3.7M idle in the bank, while AAAHogEx ran 562 vehicles
 * at a value of 11.7M (2.7 times ours). The owner: limits must change dynamically; it is not
 * acceptable that other AIs build, improve and modernize while ours does not. Rich growth
 * (rich.nut) lifts the caps that ran out once the bank holds rich_budgets line budgets (default
 * ten); it does not look
 * at the rivals, and InvestSurplus (fleet.nut) still wanted every route to earn a third of a
 * vehicle's price per vehicle and stopped at six road vehicles or planes per route.
 *
 * Once a game quarter (Behaviour runs every heartbeat, so the comparison is cached per quarter in
 * PRESSURE) the bot compares its current company value with the highest one among the other
 * companies (ScriptCompany::GetQuarterlyCompanyValue, CURRENT_QUARTER: the live value, also for a
 * company younger than one quarter). The script API exposes no vehicle count of another company
 * (ScriptVehicleList and the group counts are own-company only), so value is the only measure. The
 * level is leader / ours: below pressure_low percent none, then low, mid from pressure_mid, high
 * from pressure_high (AI settings, info.nut, read at every heartbeat; their defaults 125, 200 and
 * 400 are the thresholds this was first built with). The quarter caches only the measurement
 * (both values and whether money was idle); the level is derived from it and the thresholds of
 * `b` at every heartbeat, so a threshold changed live (Rhai, openttd.ai.set_setting, the genetic
 * algorithm) applies at once. It applies only while the bank holds idle money, the same surplus
 * InvestSurplus spends (above invest_min and INVEST_SURPLUS_RESERVES reserves); a bot that is
 * behind and poor needs its reserve, not looser limits.
 *
 * Per level (ApplyPressure, after ApplyRich in Behaviour, main.nut), each bounded by the setting's
 * max_value in info.nut:
 * - invest_ppv_div +1: a route gets one more vehicle while its vehicles earn price / div a year,
 *   so a larger divisor asks less of it (3 -> 4, 5, 6);
 * - invest_road_cap and invest_air_cap + PRESSURE_CAP_STEP: more vehicles per route (planes still
 *   only while every airport of the route has room, rich.nut AirportHasRoom);
 * - air_routes_max + PRESSURE_AIR_ROUTES_STEP;
 * - air_add_waiting, if off (0), PRESSURE_AIR_WAITING: a plane more while both airports have
 *   passengers waiting (fleet.nut);
 * - rail_twin_max at least PRESSURE_TWIN_MIN from mid.
 * Untouched: cash_reserve, invest_min, invest_borrow, StayAfloat, BuildFrozen and RepaySurplus
 * (main.nut), and the guard against a vehicle more on a route that loses money (fleet.nut
 * RouteEarns).
 */

/* Vehicles per route (road and air) one level adds. */
const PRESSURE_CAP_STEP = 2;
/* Air routes one level adds to air_routes_max. */
const PRESSURE_AIR_ROUTES_STEP = 5;
/* air_add_waiting under pressure when the setting has it off. */
const PRESSURE_AIR_WAITING = 50;
/* Twin rail lines at least, from mid. */
const PRESSURE_TWIN_MIN = 2;
/* The settings' own maximum values (info.nut). */
const PRESSURE_PPV_DIV_MAX = 10;
const PRESSURE_ROAD_CAP_MAX = 40;
const PRESSURE_AIR_CAP_MAX = 30;
const PRESSURE_AIR_ROUTES_MAX = 30;

/* Level names, indexed by level, for the log. */
PRESSURE_NAMES <- ["none", "low", "mid", "high"];

/* The comparison of the current quarter: ApplyPressure is static, so its state lives here.
 * quarter: year * 4 + quarter index it was made in (-1: never, or compete was off); idle: whether
 * the bank held idle money then; level: 0-3, derived at every heartbeat (ApplyPressure); logged:
 * the level last logged. */
PRESSURE <- {quarter = -1, idle = false, level = 0, leader = "", leader_value = 0, ours = 0, logged = 0};

/* The game quarter of today, as year * 4 + 0..3. */
function PrismaticAI::PressureQuarter() {
	local d = AIDate.GetCurrentDate();
	return AIDate.GetYear(d) * 4 + (AIDate.GetMonth(d) - 1) / 3;
}

/* The level for the value `leader` of the strongest rival against `ours`, by the thresholds of the
 * behaviour `b` (pressure_low, pressure_mid, pressure_high; percent). A threshold set below the one
 * before it counts as that one, so the levels stay in order whatever the settings say. */
function PrismaticAI::PressureLevel(b, leader, ours) {
	if (leader <= 0) return 0;
	if (ours <= 0) return 3;
	local pct = leader * 100 / ours;
	local low = b.pressure_low;
	local mid = max(low, b.pressure_mid);
	local high = max(mid, b.pressure_high);
	if (pct < low) return 0;
	if (pct < mid) return 1;
	if (pct < high) return 2;
	return 3;
}

/* Compares us with the strongest other company and stores the result in PRESSURE, for the bank
 * balance `money` under the behaviour `b`. */
function PrismaticAI::MeasurePressure(b, money) {
	local us = AICompany.ResolveCompanyID(AICompany.COMPANY_SELF);
	local ours = AICompany.GetQuarterlyCompanyValue(us, AICompany.CURRENT_QUARTER);
	local leader = -1, leader_value = 0;
	for (local c = AICompany.COMPANY_FIRST; c < AICompany.COMPANY_LAST; c++) {
		if (c == us || AICompany.ResolveCompanyID(c) == AICompany.COMPANY_INVALID) continue;
		local v = AICompany.GetQuarterlyCompanyValue(c, AICompany.CURRENT_QUARTER);
		if (v > leader_value) { leader = c; leader_value = v; }
	}
	PRESSURE.idle = money > max(b.invest_min, INVEST_SURPLUS_RESERVES * b.cash_reserve);
	local name = (leader == -1) ? null : AICompany.GetName(leader);
	PRESSURE.leader = (name == null) ? "-" : name;
	PRESSURE.leader_value = leader_value;
	PRESSURE.ours = ours;
}

/* Applies competitive pressure to the behaviour `b` (main.nut Behaviour, after ApplyRich), for the
 * bank balance `money`: measured once a game quarter, applied at every heartbeat. */
function PrismaticAI::ApplyPressure(b, money) {
	b.pressure <- 0;
	if (!("compete" in b) || b.compete != 1) {
		PRESSURE.quarter = -1; /* measure again as soon as it is switched back on */
		PRESSURE.logged = 0;
		return;
	}
	local q = PrismaticAI.PressureQuarter();
	if (q != PRESSURE.quarter) {
		PRESSURE.quarter = q;
		PrismaticAI.MeasurePressure(b, money);
	}
	local level = PRESSURE.idle ? PrismaticAI.PressureLevel(b, PRESSURE.leader_value, PRESSURE.ours) : 0;
	PRESSURE.level = level;
	if (level != PRESSURE.logged) {
		PRESSURE.logged = level;
		AILog.Info("PrismaticAI: pressure " + PRESSURE_NAMES[level] + ": leader " + PRESSURE.leader +
			" value " + PRESSURE.leader_value + " vs ours " + PRESSURE.ours);
	}
	if (level == 0) return;
	b.pressure = level;
	b.invest_ppv_div = min(PRESSURE_PPV_DIV_MAX, b.invest_ppv_div + level);
	b.invest_road_cap = min(PRESSURE_ROAD_CAP_MAX, b.invest_road_cap + PRESSURE_CAP_STEP * level);
	b.invest_air_cap = min(PRESSURE_AIR_CAP_MAX, b.invest_air_cap + PRESSURE_CAP_STEP * level);
	local routes = b.air_routes_max + PRESSURE_AIR_ROUTES_STEP * level;
	b.air_routes_max = min(PRESSURE_AIR_ROUTES_MAX, routes);
	if (b.air_add_waiting == 0) b.air_add_waiting = PRESSURE_AIR_WAITING;
	if (level >= 2) b.rail_twin_max = max(b.rail_twin_max, PRESSURE_TWIN_MIN);
}
