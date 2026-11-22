/*
 * Rich growth (RESEARCH-ONLY; setting rich_growth, default 1): money the bot holds and cannot spend
 * lifts the caps that ran out, instead of piling up in the bank.
 *
 * The incident: in the owner's live co-play game (2026-11-22, seed 20261122, company 0, a planned
 * bot: nine planner freight lines and a rail pair) the fleet stayed at 35-37 vehicles from 1955 to
 * 1960 while the bank balance rose from 0.6M to 3.77M. Every growth sink had hit a limit that money
 * never moved:
 * - air: every route ran invest_air_cap (6) planes, 539 passengers waited at one airport, and the
 *   pool of fresh town pairs was empty (foreign_airports marked 18 of 25 towns, air_reuse=0 left out
 *   our own 7);
 * - own freight lines: NextOwnFreightLine wants a producer nobody transports from, and with eight
 *   companies on a 256x256 map none was left after 1958; one farm with no station site took five of
 *   own_fail_pause's six tries in January 1957, one per acceptor;
 * - road: routes 23 >= max_routes 4, because air and rail routes count too, and a planned bot does
 *   not get the autopilot's floors.
 *
 * Rich means the bank holds rich_budgets line budgets. Then (Behaviour, main.nut): max_routes
 * is at least the autopilot's, air_reuse is on (a new route may start at an airport we already
 * have), invest_air_cap rises by one plane per rich_air_money above the threshold up to
 * rich_air_cap_max, own lines have no cap but own_fail_pause and money (steward.nut OwnLinesCap),
 * and NextOwnFreightLine (rail.nut) also takes producers others serve poorly (below
 * rich_served_below percent), within a wider distance window (rich_min_tiles to rich_max_tiles).
 * Planes are added only while every airport of the route has room (AirportHasRoom, scaled by
 * airport_fill): a small airport jams long before thirty planes. StayAfloat, BuildFrozen and
 * RepaySurplus (main.nut) are untouched: rich growth spends only money above the threshold.
 *
 * Every number is an AI setting (info.nut, CONFIG_INGAME), read at every heartbeat, so Rhai
 * policies, openttd.ai.set_setting and the battle's genetic algorithm steer them live. The values
 * the incident above led to (10 line budgets, since lowered to 5 so the bot spends below the fitness hoards floor, one plane per 250000, at most 30 planes, served below
 * 60%, 80-320 tiles, airports filled to 100%) are now the settings' defaults.
 */

/* Rich: the producer share and distance window NextOwnFreightLine (rail.nut, no `b` there) reads.
 * Globals, not consts: ApplyRich assigns them from b.rich_served_below, b.rich_min_tiles and
 * b.rich_max_tiles at every heartbeat; the initial values are the settings' defaults. */
RICH_SERVED_BELOW <- 60;
RICH_MIN_TILES <- 80;
RICH_MAX_TILES <- 320;
/* The percent of AirportPlanesMax an airport is filled to (AirportHasRoom, static and called from
 * fleet.nut without `b`): ApplyRich assigns it from b.airport_fill at every heartbeat. */
AIRPORT_FILL <- 100;
/* This heartbeat's Behaviour().rich, for the static helpers that have no `b` (rail.nut, fleet.nut,
 * modernize.nut): every mechanism of this change acts only while money is idle, so a bot that is
 * not rich plays, and is timed, as before it (the fitness and ablation matches of 2026-11-22). */
RICH_NOW <- false;

/* Whether `money` makes the bot rich under the behaviour `b` (rich_growth on). */
function PrismaticAI::Rich(b, money) {
	return b.rich_growth == 1 && money >= b.rich_budgets * b.line_budget;
}

/* Planes per air route when rich: invest_air_cap plus one per b.rich_air_money above the
 * threshold, at most b.rich_air_cap_max (never below invest_air_cap). */
function PrismaticAI::RichAirCap(b, money) {
	local above = max(0, money - b.rich_budgets * b.line_budget);
	local per = max(1, b.rich_air_money);
	return min(max(b.invest_air_cap, b.rich_air_cap_max), max(b.invest_air_cap, b.invest_air_cap + 1 + above / per));
}

/* The planes one airport of `type` serves before it jams: its hangars and terminals by the game's
 * default layouts, about two planes per terminal and the hangar, scaled by AIRPORT_FILL percent
 * (setting airport_fill; at least one plane). Unknown types count as small. */
function PrismaticAI::AirportPlanesMax(type) {
	local base = 8;
	if (type == AIAirport.AT_INTERNATIONAL || type == AIAirport.AT_INTERCON) base = 24;
	else if (type == AIAirport.AT_METROPOLITAN) base = 18;
	else if (type == AIAirport.AT_LARGE) base = 14;
	else if (type == AIAirport.AT_COMMUTER) base = 10;
	return max(1, base * AIRPORT_FILL / 100);
}

/* Whether the airport station `st` takes one plane more: fewer aircraft call there (every route)
 * than AirportPlanesMax of its type. A station that is no airport always has room. */
function PrismaticAI::AirportHasRoom(st) {
	local tile = AIStation.GetLocation(st);
	if (!AIAirport.IsAirportTile(tile)) return true;
	local planes = 0;
	foreach (v, _ in AIVehicleList_Station(st)) {
		if (AIVehicle.GetVehicleType(v) == AIVehicle.VT_AIR) planes++;
	}
	return planes < PrismaticAI.AirportPlanesMax(AIAirport.GetAirportType(tile));
}

/* Whether every airport in the orders of plane `v` has room for one more (AirportHasRoom). */
function PrismaticAI::RouteHasAirRoom(v) {
	if (!RICH_NOW) return true; /* below the rich caps no route outgrows its airports */
	for (local i = 0; i < AIOrder.GetOrderCount(v); i++) {
		local st = AIStation.GetStationID(AIOrder.GetOrderDestination(v, i));
		if (AIStation.IsValidStation(st) && !PrismaticAI.AirportHasRoom(st)) return false;
	}
	return true;
}

/* Applies rich growth to the behaviour `b` (main.nut Behaviour), for the bank balance `money`. */
function PrismaticAI::ApplyRich(b, money) {
	/* The statics rail.nut and fleet.nut read without `b` follow the settings at every heartbeat. */
	::RICH_SERVED_BELOW = b.rich_served_below;
	::RICH_MIN_TILES = b.rich_min_tiles;
	::RICH_MAX_TILES = max(b.rich_min_tiles, b.rich_max_tiles);
	::AIRPORT_FILL = b.airport_fill;
	b.rich <- PrismaticAI.Rich(b, money);
	::RICH_NOW = b.rich;
	if (!b.rich) return;
	b.max_routes = max(b.max_routes, AUTOPILOT_MAX_ROUTES);
	b.air_reuse = 1;
	b.invest_air_cap = PrismaticAI.RichAirCap(b, money);
}
