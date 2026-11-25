/*
 * Minimal company AI whose whole purpose is to be externally steerable via
 * the bridge's /ai/start and /ai/reload/{company_id} endpoints (which wrap
 * OpenTTD's `start_ai`/`reload_ai` console commands over the Admin Network).
 *
 * It intentionally does NOT implement real gameplay strategy yet -- see
 * main.nut. It's the hook point: settings below become tunable from outside
 * the game via the bridge, without touching OpenTTD's C++ core.
 */
class PrismaticAI extends AIInfo {
	function GetAuthor()      { return "prismatic-platform"; }
	function GetName()        { return "PrismaticAI"; }
	function GetShortName()   { return "PRIS"; }
	function GetDescription() { return "Company AI steerable from outside the game via the openttd-prismatic-bridge REST API."; }
	function GetVersion()     { return 2; }
	/* Saved data of any version since 1 loads; Restore() rebuilds from the map when it cannot use it. */
	function MinVersionToLoad() { return 1; }
	function GetAPIVersion()  { return "16"; }
	function GetDate()        { return "2026-11-18"; }
	function CreateInstance() { return "PrismaticAI"; }
	function UseAsRandomAI()  { return false; }

	function GetSettings() {
		AddSetting({
			name = "aggressiveness",
			description = "How aggressively to expand (0 = passive, 100 = maximal). Set via start_ai/reload_ai settings string, e.g. 'aggressiveness=75'.",
			min_value = 0,
			max_value = 100,
			easy_value = 30, medium_value = 30, hard_value = 30, custom_value = 30,
			flags = CONFIG_INGAME,
		});
		AddSetting({
			name = "heartbeat_seconds",
			description = "How often (in-game seconds) the AI logs a heartbeat -- useful to confirm the bridge can see it's alive via /rcon list_ai and the Admin console feed.",
			min_value = 1,
			max_value = 300,
			easy_value = 30,
			medium_value = 30,
			hard_value = 30,
			custom_value = 30,
			flags = CONFIG_INGAME,
		});
		AddSetting({
			name = "max_routes",
			description = "How many bus routes (one per town) the company may run. Raising it while the AI runs makes it expand.",
			min_value = 1,
			max_value = 30,
			easy_value = 4,
			medium_value = 4,
			hard_value = 4,
			custom_value = 4,
			flags = CONFIG_INGAME,
		});
		AddSetting({
			name = "cash_reserve",
			description = "Money the company keeps in the bank: no new route is built while the balance is below it.",
			min_value = 0,
			max_value = 500000,
			easy_value = 25000,
			medium_value = 25000,
			hard_value = 25000,
			custom_value = 25000,
			step_size = 5000,
			flags = CONFIG_INGAME,
		});
		AddSetting({
			name = "intercity",
			description = "1: connect pairs of towns with a road the bot builds tile by tile and run buses between them; 0: routes inside one town only.",
			min_value = 0,
			max_value = 1,
			easy_value = 1,
			medium_value = 1,
			hard_value = 1,
			custom_value = 1,
			flags = CONFIG_INGAME | CONFIG_BOOLEAN,
		});
		AddSetting({
			name = "air_a",
			description = "First town of the air route the owner's planner chose (town id; -1: none). Set by the panel from mcp-plan by_mode.air.",
			min_value = -1,
			max_value = 65535,
			easy_value = -1,
			medium_value = -1,
			hard_value = -1,
			custom_value = -1,
			flags = CONFIG_INGAME,
		});
		AddSetting({
			name = "air_b",
			description = "Second town of the planner's air route (town id; -1: none).",
			min_value = -1,
			max_value = 65535,
			easy_value = -1,
			medium_value = -1,
			hard_value = -1,
			custom_value = -1,
			flags = CONFIG_INGAME,
		});
		AddSetting({
			name = "rail_a",
			description = "First town of the rail route the owner's planner chose (town id; -1: none). Set by the panel from mcp-plan by_mode.rail.",
			min_value = -1, max_value = 65535,
			easy_value = -1, medium_value = -1, hard_value = -1, custom_value = -1,
			flags = CONFIG_INGAME,
		});
		AddSetting({
			name = "rail_b",
			description = "Second town of the planner's rail route (town id; -1: none).",
			min_value = -1, max_value = 65535,
			easy_value = -1, medium_value = -1, hard_value = -1, custom_value = -1,
			flags = CONFIG_INGAME,
		});
		AddSetting({
			name = "rail_after",
			description = "The planner's freight lines built before its passenger rail pair: those that score more (1: after the first, as before).",
			min_value = 0, max_value = 16,
			easy_value = 1, medium_value = 1, hard_value = 1, custom_value = 1,
			flags = CONFIG_INGAME,
		});
		AddSetting({
			name = "rail_wagons",
			description = "Wagons behind the locomotive of a new rail line's train (0: derive from the line's monthly supply, distance and speed, and fill the platform; rail.nut PlatformPlan).",
			min_value = 0, max_value = 30,
			easy_value = 0, medium_value = 0, hard_value = 0, custom_value = 0, step_size = 1,
			flags = CONFIG_INGAME,
		});
		AddSetting({
			name = "rail_len",
			description = "Platform length in tiles of a new rail line (0: derive from demand, capped by the game's vehicle.max_train_length and station.station_spread).",
			min_value = 0, max_value = 16,
			/* 4 by default: the length derived from supply at build time gave 2-3 tiles and short
			 * trains. Over 9 replayed arena matches (ablate --match, 2026-11-22) 4 beat 0 in all 9,
			 * +13.5% geometric, +1.4% at least; 5 and 7 did worse than 4. */
			easy_value = 4, medium_value = 4, hard_value = 4, custom_value = 4, step_size = 1,
			flags = CONFIG_INGAME,
		});
		AddSetting({
			name = "rail_platforms",
			description = "Platforms at each terminus of a new rail line: 2 builds a throat with path signals so two trains share the single track (each waits in a platform, never on the line); 1 is one platform and one train.",
			min_value = 1, max_value = 2,
			easy_value = 2, medium_value = 2, hard_value = 2, custom_value = 2, step_size = 1,
			flags = CONFIG_INGAME,
		});
		AddSetting({
			name = "fr_from",
			description = "Producing industry of the freight rail route the owner's planner chose (industry id; -1: none). Set by the panel from the planner's portfolio.",
			min_value = -1, max_value = 65535,
			easy_value = -1, medium_value = -1, hard_value = -1, custom_value = -1,
			flags = CONFIG_INGAME,
		});
		AddSetting({
			name = "fr_to",
			description = "Accepting industry of the planner's freight rail route (industry id; -1: none).",
			min_value = -1, max_value = 65535,
			easy_value = -1, medium_value = -1, hard_value = -1, custom_value = -1,
			flags = CONFIG_INGAME,
		});
		AddSetting({
			name = "fr_cargo",
			description = "Cargo of the planner's freight rail route (cargo id; -1: none).",
			min_value = -1, max_value = 63,
			easy_value = -1, medium_value = -1, hard_value = -1, custom_value = -1,
			flags = CONFIG_INGAME,
		});
		/* The planner's next freight rail lines, in its order (fr2_* to fr16_*): the bot builds each
		 * one once the money and the loan headroom cover a new line. Sixteen: after four lines a
		 * 21-game-year duel left the bot's money idle, and after eight the best 36-month run on
		 * OpenTTD seed 31337 held 308074 idle with its loan repaid. FreightLines (main.nut) reads the
		 * same sixteen. */
		for (local i = 2; i <= 16; i++) {
			AddSetting({
				name = "fr" + i + "_from",
				description = "Producing industry of the planner's freight rail line " + i + " (industry id; -1: none).",
				min_value = -1, max_value = 65535,
				easy_value = -1, medium_value = -1, hard_value = -1, custom_value = -1,
				flags = CONFIG_INGAME,
			});
			AddSetting({
				name = "fr" + i + "_to",
				description = "Accepting industry of the planner's freight rail line " + i + " (industry id; -1: none).",
				min_value = -1, max_value = 65535,
				easy_value = -1, medium_value = -1, hard_value = -1, custom_value = -1,
				flags = CONFIG_INGAME,
			});
			AddSetting({
				name = "fr" + i + "_cargo",
				description = "Cargo of the planner's freight rail line " + i + " (cargo id; -1: none).",
				min_value = -1, max_value = 63,
				easy_value = -1, medium_value = -1, hard_value = -1, custom_value = -1,
				flags = CONFIG_INGAME,
			});
		}
		/* The Rhai steward's targets for each rail line, by slot (lines.nut; set through
		 * openttd.ai.set_setting, gated like every change): trains (0 closes the line) and wagons per
		 * train. 24 slots: the planner's 16 freight lines, its passenger pair and the bot's own. The
		 * steward derives the slot count from these declarations, never from a copy of it. */
		for (local i = 1; i <= 24; i++) {
			AddSetting({
				name = "line" + i + "_trains",
				description = "Trains on rail line " + i + " (the steward's target; 0 closes the line, -1: no target).",
				min_value = -1, max_value = 8,
				easy_value = -1, medium_value = -1, hard_value = -1, custom_value = -1,
				flags = CONFIG_INGAME,
			});
			AddSetting({
				name = "line" + i + "_wagons",
				description = "Wagons per train on rail line " + i + " (the steward's target; -1: no target).",
				min_value = -1, max_value = 30,
				easy_value = -1, medium_value = -1, hard_value = -1, custom_value = -1,
				flags = CONFIG_INGAME,
			});
		}
		AddSetting({
			name = "own_lines",
			description = "Freight rail lines the bot plans itself once every line the planner handed over was tried and money waits. With autopilot=1, planned or not, at least 10 (main.nut Behaviour); 0 means none only with autopilot=0, and planned_freight_only=1 turns them off under any plan.",
			min_value = 0, max_value = 32,
			easy_value = 18, medium_value = 18, hard_value = 18, custom_value = 18,
			flags = CONFIG_INGAME,
		});
		/* RESEARCH-ONLY: one authority for the freight-line decision (the battlebrain service). With it
		 * set, the bot builds only the freight lines handed over in fr_* to fr16_* and never picks one
		 * itself, whatever own_lines, self_steward and autopilot say (main.nut, the own plan). */
		AddSetting({
			name = "planned_freight_only",
			description = "1: the bot builds only the freight rail lines handed over in fr_* to fr16_* and plans none itself (no own lines from own_lines, the in-bot steward's reinvestment or the autopilot); twins (rail_twin_max) and the passenger pair still follow their own settings. 0 (default): unchanged.",
			min_value = 0, max_value = 1,
			easy_value = 0, medium_value = 0, hard_value = 0, custom_value = 0,
			flags = CONFIG_INGAME | CONFIG_BOOLEAN,
		});
		AddSetting({
			name = "line_budget",
			description = "Money (bank balance plus loan headroom) the bot wants at hand before it builds the planner's next freight rail line.",
			min_value = 10000, max_value = 1000000,
			easy_value = 50000, medium_value = 50000, hard_value = 50000, custom_value = 50000, step_size = 10000,
			flags = CONFIG_INGAME,
		});
		AddSetting({
			name = "disclose_name",
			description = "Name the company so it reads as an AI the host steers (\"<name> (AI)\"), for a real game against people (research/real-game.md). 0 keeps the game's own name.",
			min_value = 0, max_value = 1,
			easy_value = 0, medium_value = 0, hard_value = 0, custom_value = 0,
			flags = CONFIG_INGAME | CONFIG_BOOLEAN,
		});
		AddSetting({
			name = "air_routes_max",
			description = "Autopilot: most own air routes (autopilot.nut).",
			min_value = 0, max_value = 30,
			easy_value = 15, medium_value = 15, hard_value = 15, custom_value = 15, step_size = 1,
			flags = CONFIG_INGAME,
		});
		AddSetting({
			name = "air_towns",
			description = "Autopilot: how many of the most populous towns air pairs are chosen from.",
			min_value = 4, max_value = 60,
			easy_value = 50, medium_value = 50, hard_value = 50, custom_value = 50, step_size = 1,
			flags = CONFIG_INGAME,
		});
		AddSetting({
			name = "air_min_tiles",
			description = "Autopilot: shortest air pair, in tiles.",
			min_value = 20, max_value = 400,
			easy_value = 60, medium_value = 60, hard_value = 60, custom_value = 60, step_size = 10,
			flags = CONFIG_INGAME,
		});
		AddSetting({
			name = "air_max_tiles",
			description = "Autopilot: longest air pair, in tiles.",
			min_value = 60, max_value = 800,
			easy_value = 400, medium_value = 400, hard_value = 400, custom_value = 400, step_size = 10,
			flags = CONFIG_INGAME,
		});
		AddSetting({
			name = "air_level_max",
			description = "Most levelling one airport site may cost, in thousands (a hilly map needs more).",
			min_value = 0, max_value = 200,
			easy_value = 10, medium_value = 10, hard_value = 10, custom_value = 10, step_size = 5,
			flags = CONFIG_INGAME,
		});
		AddSetting({
			name = "air_grow_money",
			description = "Autopilot: one more air route beyond air_routes_max per this much bank money above the reserve, in thousands (0: never more than air_routes_max).",
			min_value = 0, max_value = 10000,
			easy_value = 2500, medium_value = 2500, hard_value = 2500, custom_value = 2500, step_size = 250,
			flags = CONFIG_INGAME,
		});
		AddSetting({
			name = "air_reuse",
			description = "Autopilot: once no pair of fresh towns is left, a new air route may fly from an airport of ours (a hub); 0: one route per airport.",
			min_value = 0, max_value = 1,
			easy_value = 0, medium_value = 0, hard_value = 0, custom_value = 0,
			flags = CONFIG_BOOLEAN | CONFIG_INGAME,
		});
		AddSetting({
			name = "air_level_step",
			description = "Most height steps between the corners of an airport site that is still levelled (a bigger step moves corners outside the site, where houses and roads stop it); 0: any step until a levelled site is refused in this game, then 1.",
			min_value = 0, max_value = 8,
			easy_value = 8, medium_value = 8, hard_value = 8, custom_value = 8, step_size = 1,
			flags = CONFIG_INGAME,
		});
		AddSetting({
			name = "air_level_once",
			description = "1: an airport search levels no further site once a levelled site was refused anyway (a corner shared with a house or road does not move); 0: it keeps levelling.",
			min_value = 0, max_value = 1,
			easy_value = 0, medium_value = 0, hard_value = 0, custom_value = 0,
			flags = CONFIG_BOOLEAN | CONFIG_INGAME,
		});
		AddSetting({
			name = "own_fail_pause",
			description = "After this many own freight lines in a row could not be built, try no more that game year (a failed line still costs its stations); 0: at most 8 a game year (main.nut OWN_FAIL_YEAR_MAX).",
			min_value = 0, max_value = 10,
			easy_value = 6, medium_value = 6, hard_value = 6, custom_value = 6, step_size = 1,
			flags = CONFIG_INGAME,
		});
		AddSetting({
			name = "air_keep_last",
			description = "Never sell the last plane of an air route for losing money (a dead route never flies again).",
			min_value = 0, max_value = 1,
			easy_value = 1, medium_value = 1, hard_value = 1, custom_value = 1,
			flags = CONFIG_BOOLEAN | CONFIG_INGAME,
		});
		AddSetting({
			name = "foreign_airports",
			description = "Autopilot: skip a town with this many airports of other companies within 16 tiles (0: never skip).",
			min_value = 0, max_value = 5,
			easy_value = 1, medium_value = 1, hard_value = 1, custom_value = 1, step_size = 1,
			flags = CONFIG_INGAME,
		});
		AddSetting({
			name = "invest_min",
			description = "Fleet: idle money (in thousands) above which InvestSurplus buys vehicles where they earn most.",
			min_value = 0, max_value = 2000,
			easy_value = 50, medium_value = 50, hard_value = 50, custom_value = 50, step_size = 10,
			flags = CONFIG_INGAME,
		});
		AddSetting({
			name = "invest_ppv_div",
			description = "Fleet: a route gets one more vehicle while its vehicles earn more than price / this per year.",
			min_value = 1, max_value = 10,
			easy_value = 3, medium_value = 3, hard_value = 3, custom_value = 3, step_size = 1,
			flags = CONFIG_INGAME,
		});
		AddSetting({
			name = "invest_road_cap",
			description = "Fleet: most road vehicles per route that InvestSurplus grows to.",
			min_value = 1, max_value = 40,
			easy_value = 6, medium_value = 6, hard_value = 6, custom_value = 6, step_size = 1,
			flags = CONFIG_INGAME,
		});
		AddSetting({
			name = "invest_air_cap",
			description = "Fleet: most planes per route (InvestSurplus, and fleet growth with the steward on).",
			min_value = 1, max_value = 30,
			easy_value = 6, medium_value = 6, hard_value = 6, custom_value = 6, step_size = 1,
			flags = CONFIG_INGAME,
		});
		AddSetting({
			name = "reinvest_budgets",
			description = "Steward: own lines beyond own_lines need this many line budgets in the bank.",
			min_value = 1, max_value = 10,
			easy_value = 2, medium_value = 2, hard_value = 2, custom_value = 2, step_size = 1,
			flags = CONFIG_INGAME,
		});
		AddSetting({
			name = "catchment_stops",
			description = "Catchment ring (learned from AAAHogEx): most joined drive-through road stops around each passenger airport, so it collects the whole town (catchment.nut; default 8 since GA g150.0 became the default on 2026-11-20; 2 before, from the GA elite of 2026-11-19; 0: off).",
			min_value = 0, max_value = 30,
			easy_value = 8, medium_value = 8, hard_value = 8, custom_value = 8, step_size = 1,
			flags = CONFIG_INGAME,
		});
		AddSetting({
			name = "catchment_spacing",
			description = "Catchment ring: tiles between two of its road stops.",
			min_value = 2, max_value = 10,
			easy_value = 4, medium_value = 4, hard_value = 4, custom_value = 4, step_size = 1,
			flags = CONFIG_INGAME,
		});
		AddSetting({
			name = "statue_money",
			description = "Statues (learned from AAAHogEx): money in thousands above which the bot builds a statue in each town next to its stations (statues.nut; default 0: off, it cost ~30% on the fitness seed at 6 years, and the battle's GA no longer explores it: on seed 853334149 statues took g279.3 from 3.19M to 0.66M, and over 1,140 GA variants those with statues averaged 14% less value).",
			min_value = 0, max_value = 5000,
			easy_value = 0, medium_value = 0, hard_value = 0, custom_value = 0, step_size = 50,
			flags = CONFIG_INGAME,
		});
		AddSetting({
			name = "air_full_load",
			description = "1: planes wait for a full load at both airports (learned from AAAHogEx); 0 (default): they leave at once; full load cost ~5% on the fitness seed at 6 years.",
			min_value = 0, max_value = 1,
			easy_value = 0, medium_value = 0, hard_value = 0, custom_value = 0, step_size = 1,
			flags = CONFIG_INGAME,
		});
		AddSetting({
			name = "invest_borrow",
			description = "1: InvestSurplus may borrow up to the loan limit for a vehicle that pays back (learned from AAAHogEx, which keeps its cash near zero); 0 (default): bank money only; borrowing changed nothing on the fitness seed at 6 years.",
			min_value = 0, max_value = 1,
			easy_value = 0, medium_value = 0, hard_value = 0, custom_value = 0, step_size = 1,
			flags = CONFIG_INGAME,
		});
		AddSetting({
			name = "air_add_waiting",
			description = "Planes (learned from AAAHogEx): add a plane to an air route while the passengers waiting at BOTH of its airports exceed this, up to invest_air_cap (fleet.nut; default 20 since GA g150.0 became the default on 2026-11-20; 0: the old rule of 1.5 loads at one end, the default of the GA elite of 2026-11-19; the earlier default 10 with catchment_stops=8 had +14.8% mean value over 36 seeds against the bot before it).",
			min_value = 0, max_value = 500,
			easy_value = 20, medium_value = 20, hard_value = 20, custom_value = 20, step_size = 10,
			flags = CONFIG_INGAME,
		});
		AddSetting({
			name = "air_add_full",
			description = "Air yield (after AAAHogEx, whose planes fly 89% full): one more plane calls at an airport of ours only while the planes flying from it are this percent full: a plane added to an air route (fleet growth, InvestSurplus) needs the route's flying planes that full, a new autopilot route from an airport we have (air_reuse) the planes flying from that airport (fleet.nut AirRouteHasDemand, autopilot.nut BestAirPair; 0: off).",
			min_value = 0, max_value = 100,
			easy_value = 0, medium_value = 0, hard_value = 0, custom_value = 0, step_size = 5,
			flags = CONFIG_INGAME,
		});
		AddSetting({
			name = "rail_twin_max",
			description = "Rail capacity (after AAAHogEx's double track): most twin lines, each a second line with its own track and platforms between the same two industries, built when a line runs all its trains and still has a trainload waiting (steward.nut TwinLines; 0: off).",
			min_value = 0, max_value = 24,
			easy_value = 6, medium_value = 6, hard_value = 6, custom_value = 6, step_size = 1,
			flags = CONFIG_INGAME,
		});
		AddSetting({
			name = "rail_grow_max",
			description = "Growing stations (growth.nut): a rail line that earns, runs trains as long as its platforms and still leaves a full trainload waiting gets its platforms one tile longer at both ends, once a game month, up to this many tiles (0: off).",
			min_value = 0, max_value = 7,
			easy_value = 0, medium_value = 0, hard_value = 0, custom_value = 0, step_size = 1,
			flags = CONFIG_INGAME,
		});
		AddSetting({
			name = "rail_feed",
			description = "Multi-source stations (growth.nut FeedStations): a rail line that earns joins up to this many more producers of its cargo, unserved and within the station spread of its source station, into that station's catchment with one station tile each, once a game month (0: off).",
			min_value = 0, max_value = 6,
			easy_value = 0, medium_value = 0, hard_value = 0, custom_value = 0, step_size = 1,
			flags = CONFIG_INGAME,
		});
		AddSetting({
			name = "rail_double",
			description = "1: the bot's own new freight lines are double track (after AAAHogEx): one one-way track each way with path signals, so a line runs many trains and the in-bot steward adds one while half a trainload waits and no train loads at the source (rail.nut BuildDoubleRailLine); 0 (default): single track, two trains a line. Default 1 since 2026-11-19: in the battle, 20 years, paired on 74 common seeds, +14.8% mean value and better in 56 of 74; 0: single track.",
			min_value = 0, max_value = 1,
			easy_value = 1, medium_value = 1, hard_value = 1, custom_value = 1,
			flags = CONFIG_INGAME | CONFIG_BOOLEAN,
		});
		AddSetting({
			name = "autopilot",
			description = "1 (default): the bot grows on its own, with or without a plan from the panel: in-bot steward, own freight lines (at least 10), intercity roads, up to 30 routes and at most a 5000 reserve; a plan's lines are built on top, its passenger pair in a heartbeat that starts no own freight line, or after two years of waiting. 0: the bot stays as small as its settings say (the duels' rival before 2026-11-19).",
			min_value = 0, max_value = 1,
			easy_value = 1, medium_value = 1, hard_value = 1, custom_value = 1,
			flags = CONFIG_INGAME | CONFIG_BOOLEAN,
		});
		AddSetting({
			name = "self_steward",
			description = "1: with no steward's target for a rail line, the bot applies the Rhai steward's policy itself (lengthen, add a train, shrink, close) and reinvests idle money in more own lines beyond own_lines (steward.nut); 0: lines run what the external steward sets.",
			min_value = 0, max_value = 1,
			easy_value = 1, medium_value = 1, hard_value = 1, custom_value = 1,
			flags = CONFIG_INGAME | CONFIG_BOOLEAN,
		});
		/* The in-bot steward's capital rules (steward.nut, lines.nut): a full-load train takes the
		 * cargo it waits for, so "a trainload waiting" rarely shows that a line is short of trains. */
		AddSetting({
			name = "steward_rating",
			description = "In-bot steward: an earning rail line whose source station rates its cargo below this percent gets one more train (up to trains_max, its newest train at least 90 days old, a source platform free). The rating falls with the days between pickups, which a full-load train hides from the cargo waiting (steward.nut; 0: off, only waiting cargo grows a line).",
			min_value = 0, max_value = 100,
			easy_value = 55, medium_value = 55, hard_value = 55, custom_value = 55, step_size = 5,
			flags = CONFIG_INGAME,
		});
		AddSetting({
			name = "steward_target",
			description = "In-bot steward: 1 (default): a freight rail line that earns gets the trains that carry its monthly production (its producer and the ones feeding its source station) over one round trip, in batches (at most doubling a month, up to trains_max), its newest train at least 90 days old (steward.nut LineTrainTarget); 0: only the waiting-cargo and rating rules grow a line, one train at a time.",
			min_value = 0, max_value = 1,
			easy_value = 1, medium_value = 1, hard_value = 1, custom_value = 1,
			flags = CONFIG_INGAME | CONFIG_BOOLEAN,
		});
		AddSetting({
			name = "steward_borrow",
			description = "1 (default): a rail line whose trains earn may borrow for the next train the steward set, while the loan stays at most half its limit and the headroom left holds a line budget and two quarters of running costs (lines.nut FundTrain); 0: a new train only from the bank balance, which RepaySurplus keeps near the reserve.",
			min_value = 0, max_value = 1,
			easy_value = 1, medium_value = 1, hard_value = 1, custom_value = 1,
			flags = CONFIG_INGAME | CONFIG_BOOLEAN,
		});
		AddSetting({
			name = "rich_growth",
			description = "1 (default): once the bank holds rich_budgets line budgets (default ten), the caps that ran out are lifted: max_routes at least 30, air_reuse on, more planes per air route while its airports have room, own freight lines without a cap and from poorly served producers too (rich.nut); 0: money above the caps stays in the bank.",
			min_value = 0, max_value = 1,
			easy_value = 1, medium_value = 1, hard_value = 1, custom_value = 1,
			flags = CONFIG_INGAME | CONFIG_BOOLEAN,
		});
		AddSetting({
			name = "rich_air",
			description = "0 (default): idle money (rich_growth, compete) is not spent on aircraft: air_reuse, planes per air route, air_routes_max and air_add_waiting stay as set; six replays on 2026-11-22 lost 13-55% of company value with 53-79 planes against about 25; 1: rich growth and competitive pressure also lift the air caps (as first built).",
			min_value = 0, max_value = 1,
			easy_value = 0, medium_value = 0, hard_value = 0, custom_value = 0,
			flags = CONFIG_INGAME | CONFIG_BOOLEAN,
		});
		AddSetting({
			name = "modernize",
			description = "1 (default): once a game month the bot sets the game's autoreplace from every engine it runs to the better one its builders would buy now (same rail type with power, same plane class; modernize.nut); 0: engines are only renewed with the same model.",
			min_value = 0, max_value = 1,
			easy_value = 1, medium_value = 1, hard_value = 1, custom_value = 1,
			flags = CONFIG_INGAME | CONFIG_BOOLEAN,
		});
		AddSetting({
			name = "compete",
			description = "1 (default): once a game quarter the bot compares its company value with the strongest other company; while it is behind and money sits idle, its fleet limits loosen by how far behind it is (vehicles per route, the earnings a route needs for one more, air routes, twin lines; pressure.nut); 0: the limits stay as set.",
			min_value = 0, max_value = 1,
			easy_value = 1, medium_value = 1, hard_value = 1, custom_value = 1,
			flags = CONFIG_INGAME | CONFIG_BOOLEAN,
		});
		AddSetting({
			name = "upgrade_track",
			description = "1 (default): once a month, convert one rail network to a better rail type (electrified once an electric locomotive beats the steam and diesel ones) when every train keeps power on it and the bank holds the cost plus track_keep_budgets line budgets (default two); new lines are built in the best type (infra.nut); 0: plain rail all game.",
			min_value = 0, max_value = 1,
			easy_value = 1, medium_value = 1, hard_value = 1, custom_value = 1,
			flags = CONFIG_INGAME | CONFIG_BOOLEAN,
		});
		AddSetting({
			name = "rich_budgets",
			description = "Rich growth: the bot counts as rich once the bank holds this many line budgets (rich.nut Rich). Default 5 (250k at the default line_budget): below the fitness check's hoards floor (500k), so the bot spends before it hoards; at 10 the planned fitness contestant held 9 vehicles while its money rose from 283k to 697k (2026-11-22).",
			min_value = 1, max_value = 100,
			easy_value = 5, medium_value = 5, hard_value = 5, custom_value = 5, step_size = 1,
			flags = CONFIG_INGAME,
		});
		AddSetting({
			name = "rich_air_money",
			description = "Rich growth: one plane more per air route for every this much bank money above the rich threshold, in thousands (rich.nut RichAirCap).",
			min_value = 10, max_value = 5000,
			easy_value = 250, medium_value = 250, hard_value = 250, custom_value = 250, step_size = 10,
			flags = CONFIG_INGAME,
		});
		AddSetting({
			name = "rich_invest_rounds",
			description = "Rich growth: how many times a heartbeat idle money buys one more vehicle for the route that earns most per vehicle (fleet.nut InvestSurplus); 1 when not rich.",
			min_value = 1, max_value = 20,
			easy_value = 4, medium_value = 4, hard_value = 4, custom_value = 4, step_size = 1,
			flags = CONFIG_INGAME,
		});
		AddSetting({
			name = "rich_air_cap_max",
			description = "Rich growth: the most planes per air route it raises invest_air_cap to (rich.nut RichAirCap; never lowers invest_air_cap).",
			min_value = 1, max_value = 30,
			easy_value = 30, medium_value = 30, hard_value = 30, custom_value = 30, step_size = 1,
			flags = CONFIG_INGAME,
		});
		AddSetting({
			name = "rich_served_below",
			description = "Rich growth: an own freight line may also start at a producer others serve while under this share of its production was transported last month, in percent (rich.nut, read by rail.nut NextOwnFreightLine).",
			min_value = 0, max_value = 100,
			easy_value = 60, medium_value = 60, hard_value = 60, custom_value = 60, step_size = 5,
			flags = CONFIG_INGAME,
		});
		AddSetting({
			name = "rich_min_tiles",
			description = "Rich growth: the shortest own freight line, in tiles (the planner's window is 100-260; rich.nut, read by rail.nut NextOwnFreightLine).",
			min_value = 20, max_value = 400,
			easy_value = 80, medium_value = 80, hard_value = 80, custom_value = 80, step_size = 10,
			flags = CONFIG_INGAME,
		});
		AddSetting({
			name = "rich_max_tiles",
			description = "Rich growth: the longest own freight line, in tiles, at least rich_min_tiles (rich.nut, read by rail.nut NextOwnFreightLine).",
			min_value = 100, max_value = 1000,
			easy_value = 320, medium_value = 320, hard_value = 320, custom_value = 320, step_size = 10,
			flags = CONFIG_INGAME,
		});
		AddSetting({
			name = "airport_fill",
			description = "Planes an airport takes before it counts as full, in percent of the planes its type serves (8 small to 24 international; rich.nut AirportPlanesMax, used by AirportHasRoom for every plane added).",
			min_value = 25, max_value = 200,
			easy_value = 100, medium_value = 100, hard_value = 100, custom_value = 100, step_size = 5,
			flags = CONFIG_INGAME,
		});
		AddSetting({
			name = "pressure_low",
			description = "Competitive pressure: the strongest rival's company value against ours, in percent, from which the level is low (pressure.nut PressureLevel).",
			min_value = 100, max_value = 2000,
			easy_value = 125, medium_value = 125, hard_value = 125, custom_value = 125, step_size = 5,
			flags = CONFIG_INGAME,
		});
		AddSetting({
			name = "pressure_mid",
			description = "Competitive pressure: the leader / ours percent from which the level is mid (twin rail lines from here on); below pressure_low counts as pressure_low (pressure.nut).",
			min_value = 100, max_value = 2000,
			easy_value = 200, medium_value = 200, hard_value = 200, custom_value = 200, step_size = 5,
			flags = CONFIG_INGAME,
		});
		AddSetting({
			name = "pressure_high",
			description = "Competitive pressure: the leader / ours percent from which the level is high; below pressure_mid counts as pressure_mid (pressure.nut).",
			min_value = 100, max_value = 2000,
			easy_value = 400, medium_value = 400, hard_value = 400, custom_value = 400, step_size = 5,
			flags = CONFIG_INGAME,
		});
		AddSetting({
			name = "track_keep_budgets",
			description = "Track upgrade: a rail network is converted only while the bank holds its cost plus this many line budgets (infra.nut UpgradeTrack).",
			min_value = 0, max_value = 20,
			easy_value = 2, medium_value = 2, hard_value = 2, custom_value = 2, step_size = 1,
			flags = CONFIG_INGAME,
		});
		/* RESEARCH-ONLY: ships (ships.nut). Off by default: new behaviour is measured before it is on. */
		AddSetting({
			name = "ships_max",
			description = "Ships (ships.nut): most water routes the bot plans itself, from an oil rig or a coastal producer to an acceptor by the sea, or between two coastal towns, one a heartbeat when money plus loan headroom reach line_budget; the fleet grows them like road and air routes (0, the default: no ships).",
			min_value = 0, max_value = 30,
			easy_value = 0, medium_value = 0, hard_value = 0, custom_value = 0, step_size = 1,
			flags = CONFIG_INGAME,
		});
		AddSetting({
			name = "ships_cap",
			description = "Ships: most ships per water route, for the fleet's additions where cargo waits and InvestSurplus (fleet.nut), and for how many a new route is planned with (ships.nut).",
			min_value = 1, max_value = 20,
			easy_value = 6, medium_value = 6, hard_value = 6, custom_value = 6, step_size = 1,
			flags = CONFIG_INGAME,
		});
		AddSetting({
			name = "ships_max_tiles",
			description = "Ships: the longest water route, in tiles between its two ends (the shortest is 20; ships.nut).",
			min_value = 40, max_value = 500,
			easy_value = 150, medium_value = 150, hard_value = 150, custom_value = 150, step_size = 10,
			flags = CONFIG_INGAME,
		});
		AddSetting({
			name = "air_planes",
			description = "Aircraft on the planner's air route.",
			min_value = 1,
			max_value = 8,
			easy_value = 1, medium_value = 1, hard_value = 1, custom_value = 1,
			step_size = 1,
			flags = CONFIG_INGAME,
		});
		/* Yield (fleet.nut RouteLoad, RoadPays; autopilot.nut AirRoutesPay): money goes where it earns. */
		AddSetting({
			name = "load_add_min",
			description = "Yield: no vehicle is added to a road, air or water route (fleet growth, InvestSurplus) whose vehicles ran under this percent full over the last heartbeats, sampled while they run (fleet.nut RouteLoad; 0 (default): off; 50 cost 29.6% of company value over 24 replayed 20-year arena matches, with 332 fewer planes, 2026-11-23).",
			min_value = 0, max_value = 100,
			easy_value = 0, medium_value = 0, hard_value = 0, custom_value = 0, step_size = 5,
			flags = CONFIG_INGAME,
		});
		AddSetting({
			name = "road_yield",
			description = "Yield: a new bus route is built only while our road vehicles a year old earned, last year, at least this percent of their price between them; before any is a year old, only while fewer than road_probes road routes run (fleet.nut RoadPays; 0: off).",
			min_value = 0, max_value = 200,
			easy_value = 20, medium_value = 20, hard_value = 20, custom_value = 20, step_size = 5,
			flags = CONFIG_INGAME,
		});
		AddSetting({
			name = "road_probes",
			description = "Yield: how many bus routes run before any road vehicle is a year old and road_yield can judge them (fleet.nut RoadPays).",
			min_value = 0, max_value = 30,
			easy_value = 2, medium_value = 2, hard_value = 2, custom_value = 2, step_size = 1,
			flags = CONFIG_INGAME,
		});
		AddSetting({
			name = "air_yield_routes",
			description = "Yield: autopilot air routes beyond air_routes_max, one at a time, each once every air route we fly has planes a year old that earned last year at least their price / invest_ppv_div a plane (autopilot.nut AirRoutesPay; 0: off).",
			min_value = 0, max_value = 30,
			easy_value = 8, medium_value = 8, hard_value = 8, custom_value = 8, step_size = 1,
			flags = CONFIG_INGAME,
		});
		/* Capital allocator (capital.nut): where money above the reserve goes, once a heartbeat. */
		AddSetting({
			name = "capital_alloc",
			description = "Capital allocator, once a heartbeat in place of the plain loan repayment (capital.nut). 1: keep one loan interval above the stay-afloat floor, so a few days of costs no longer trigger a rescue and a 90-day construction freeze; 2 (and 1): before repaying, buy the investment with the best measured return whose payback is within invest_ppv_div years: one more vehicle on a road, air or water route (behind the fleet's caps and load guards) or one more train on an earning rail line with cargo waiting or a low source rating, or hold the money for it; 3 (default, and 2): after a quarter with money idle and no loan, one new air route where the air fleet as a whole pays; 0: repay everything above twice the reserve, as before.",
			min_value = 0, max_value = 3,
			easy_value = 3, medium_value = 3, hard_value = 3, custom_value = 3, step_size = 1,
			flags = CONFIG_INGAME,
		});
		/* RESEARCH-ONLY test hook (rail.nut BuildTrain, TestRefuseDepot; lines.nut TestForgetTrainless): drives the paths a crowded map takes
		 * only by chance, so the headless fitness check runs them on every change. */
		AddSetting({
			name = "test_trainless",
			description = "RESEARCH-ONLY test hook. 1: the first train of every new rail line, and every other new line's depot, is refused as if for money, so the line waits trainless for KeepLinesServed to buy it; 2: as 1, and once a game month the bot forgets its lines that still have no train, as a restart would, so AdoptTrainlessLines (monthly in this mode, yearly otherwise) takes them back from the map. 0 (default): off.",
			min_value = 0, max_value = 2,
			easy_value = 0, medium_value = 0, hard_value = 0, custom_value = 0,
			step_size = 1,
			flags = CONFIG_NONE,
		});
		/* RESEARCH-ONLY test hook (lines.nut FundTrain): the borrowing path for a train runs only when the
		 * bank is short of a train's price, which RepaySurplus and income rarely leave it. */
		AddSetting({
			name = "test_borrow",
			description = "RESEARCH-ONLY test hook. 1: when the steward adds a train, FundTrain counts the bank as holding only the cash reserve, so every such train is borrowed for, within the bounds of steward_borrow (the loan at most half its limit, a line budget and two quarters of running costs left), and RepaySurplus pays it back. 0 (default): off.",
			min_value = 0, max_value = 1,
			easy_value = 0, medium_value = 0, hard_value = 0, custom_value = 0,
			flags = CONFIG_BOOLEAN,
		});
		/* RESEARCH-ONLY test hook (main.nut TestFreightSlots): handed freight slots written one
		 * setting at a time, as the panel and the re-planner write them. */
		AddSetting({
			name = "test_freight_slots",
			description = "RESEARCH-ONLY test hook. 1: the handed freight slots (fr_*) are not read from the settings but from main.nut TestFreightSlots, which hands the bot's own best freight line half written, then incoherent (an acceptor that does not take the cargo, a cargo the producer does not make), then flipping between two acceptors, then whole; FreightLines must try it once, only when whole. Run it with planned_freight_only=1. 0 (default): off.",
			min_value = 0, max_value = 1,
			easy_value = 0, medium_value = 0, hard_value = 0, custom_value = 0,
			step_size = 1,
			flags = CONFIG_NONE,
		});
		/* RESEARCH-ONLY test hook (rail.nut TestClaimRace): the producer-claim race that only the second
		 * claim check settles, run by the fitness check's claim-race lineup. */
		AddSetting({
			name = "test_claim_race",
			description = "RESEARCH-ONLY test hook. Days the company waits after the claim check before its first source station, once a game, so a sibling that chose the same producer builds and checks its station first and only the second claim check (rail.nut ClaimBlocks) can settle the race. 0 (default): off.",
			min_value = 0, max_value = 60,
			easy_value = 0, medium_value = 0, hard_value = 0, custom_value = 0,
			step_size = 1,
			flags = CONFIG_NONE,
		});
	}
}

RegisterAI(PrismaticAI());
