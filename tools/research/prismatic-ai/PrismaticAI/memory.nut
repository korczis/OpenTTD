/*
 * Failure memory (RESEARCH-ONLY): "tried this, it failed for this reason, do not try it again
 * until a date, or until the cause can have changed". One mechanism for every such record, so a
 * new kind of failure adds a store and a clear condition, never another copy of the bookkeeping.
 *
 * A store is a global table, key -> record. A record is a table of primitives (it is saved:
 * restore.nut) with `until`, the date it stops holding, and whatever its clear condition reads.
 * Changes only what the bot tries; observer effect: one table walk per save.
 *
 * Stores: ROAD_TOWN_FAILED (a town whose bus route failed, below). NO_RAIL_SITE of PR #291 (a
 * terminus without a station site) has the same shape and can use these three functions.
 */

/* Remember `key` in `store` for `days` days: `rec` (a table) gets `until` and replaces any earlier
 * record of the key. Returns `rec`. */
function PrismaticAI::MemoRemember(store, key, days, rec) {
	rec.until <- AIDate.GetCurrentDate() + days;
	store[key] <- rec;
	return rec;
}

/* The record of `key` while it still holds, else null. A record past its `until`, malformed (a
 * save of another layout), or one `cleared(key, rec)` says no longer holds (its cause can have
 * changed; null for none) is dropped here. A hit never extends a record. */
function PrismaticAI::MemoHeld(store, key, cleared) {
	if (!store.rawin(key)) return null;
	local rec = store[key];
	if (typeof rec != "table" || !("until" in rec) || typeof rec.until != "integer" ||
			rec.until <= AIDate.GetCurrentDate() || (cleared != null && cleared(key, rec))) {
		delete store[key];
		return null;
	}
	return rec;
}

/* The records of `store` that still hold by date, at most `max` of them, for a save: the ones that
 * expire last are kept, so the cut does not depend on the table's iteration order. */
function PrismaticAI::MemoSave(store, max) {
	local now = AIDate.GetCurrentDate(), live = [];
	foreach (k, rec in store) {
		if (typeof rec == "table" && ("until" in rec) && typeof rec.until == "integer" && rec.until > now) live.push([rec.until, k, rec]);
	}
	live.sort(function(x, y) { return (y[0] > x[0]) ? 1 : ((y[0] < x[0]) ? -1 : 0); });
	local out = {};
	for (local i = 0; i < live.len() && i < max; i++) out[live[i][1]] <- live[i][2];
	return out;
}

/*
 * ROAD_TOWN_FAILED: town id -> {why, until, ...} for a town whose bus route (main.nut
 * BuildRouteInTown) failed. BuildStarterRoute skips it while the record holds.
 *
 * Seen in live game 21647 (1995-2000): nothing remembered a failed town, so every heartbeat that
 * tried road walked the same eight towns again. Three of them surveyed about 40 sites each, one
 * game day per site, and ended refused by the local authority on the same tiles; the other five
 * reused two stops "on separate road networks" (one of each pair was an airport's or a platform's
 * location tile: roadstop.nut OwnBusStationNear). A heartbeat then lasted about four months: 3 a year
 * instead of 88, and AllocateCapital ran once a quarter while 34-44M sat idle.
 *
 * Why, and how long it is believed (days), and what clears it before that:
 *  - "authority": the authority refused a stop. Cleared once the company's rating band in the
 *    town (AITown.GetRating) is above the one it had at the refusal. A town whose band surely
 *    refuses is skipped before any survey anyway (RoadTownRefuses); this record keeps
 *    out a town refused all the same, e.g. by the authority of a neighbour closer to the site
 *    (CheckIfAuthorityAllowsNewStation asks the town closest to the tile). The rating rises by
 *    at most RATING_GROWTH_UP_STEP a month without a served station, so it takes months.
 *  - "site": no connected site for a stop. Cleared once the town grew by ROAD_FAIL_GROWTH (new
 *    houses come with new road).
 *  - "network": the two stops are on separate road networks. Only expires: the failed attempt
 *    removes the stops nobody serves (roadstop.nut RemoveUnservedBusStops), so "a stop is gone"
 *    says nothing, and a retry after it expires is the reconnect check.
 */
ROAD_TOWN_FAILED <- {};
ROAD_FAIL_DAYS <- {authority = 365, site = 365, network = 180};
/* Percent of growth after which a town remembered without a site is tried again. */
ROAD_FAIL_GROWTH <- 10;
/* At most this many towns are saved. */
ROAD_FAIL_SAVE_MAX <- 200;

/* Remember that the route in `town` failed for `why` ("authority", "site" or "network"); `a` and
 * `b` are the two stops of a "network" failure. Logged once, here. */
function PrismaticAI::RememberRoadTown(town, why, a, b) {
	local rec = {why = why, pop = AITown.GetPopulation(town), band = AITown.GetRating(town, AICompany.COMPANY_SELF), a = a, b = b};
	PrismaticAI.MemoRemember(ROAD_TOWN_FAILED, town, ROAD_FAIL_DAYS[why], rec);
	local u = rec.until;
	AILog.Info("PrismaticAI: road route in " + AITown.GetName(town) + " remembered as failed (" + why + ") until " +
		AIDate.GetYear(u) + "-" + AIDate.GetMonth(u) + "-" + AIDate.GetDayOfMonth(u) + ", rating band " + rec.band);
}

/* Whether the authority of `town` surely refuses us a new station: CheckIfAuthorityAllowsNewStation
 * (src/town_cmd.cpp) wants a rating above RATING_VERYPOOR (-200), and the bands AITown.GetRating
 * gives end there exactly (VERY_POOR is -399 .. -200, POOR starts at -199), so the band answers it.
 * Never with a permissive council (town_council_tolerance 3), which allows every station. Written
 * here and not from town.nut: town_model.nut does not compile as Squirrel (its bands use the
 * keyword `enum` as a key), so town.nut is not loaded. */
function PrismaticAI::RoadTownRefuses(town) {
	if (AIGameSettings.IsValid("difficulty.town_council_tolerance") &&
			AIGameSettings.GetValue("difficulty.town_council_tolerance") == 3) return false;
	local r = AITown.GetRating(town, AICompany.COMPANY_SELF);
	return r == AITown.TOWN_RATING_APPALLING || r == AITown.TOWN_RATING_VERY_POOR;
}

/* Whether the failure remembered for `town` can no longer hold (see ROAD_TOWN_FAILED). */
function PrismaticAI::RoadTownCleared(town, rec) {
	if (!("why" in rec)) return true;
	if (rec.why == "authority") return !("band" in rec) || AITown.GetRating(town, AICompany.COMPANY_SELF) > rec.band;
	if (rec.why == "site") return !("pop" in rec) || AITown.GetPopulation(town) * 100 >= rec.pop * (100 + ROAD_FAIL_GROWTH);
	return rec.why != "network";
}

/* The record that keeps BuildStarterRoute out of `town`, or null. */
function PrismaticAI::RoadTownFailed(town) {
	return PrismaticAI.MemoHeld(ROAD_TOWN_FAILED, town, PrismaticAI.RoadTownCleared);
}
