// stall: 600
// Service self-evaluation (RESEARCH-ONLY; research/mechanisms/service-self-evaluation.md): the
// panel's services are judged by what followed their actions, every game month, and a service that
// keeps being judged poor is said, never silenced.
//
// Run it as a service on the Scripts page (/scripts/service_eval) or:
//   curl -X POST 127.0.0.1:8800/api/services/start -d '{"name":"service_eval"}'
//
// Once a game month it reads GET /api/services/scores (src/service_eval.rs): per service and action
// (replan proposal and hold, watchdog stall_alert, build_audit defect_flag, rail_monitor advice_flag,
// gap_watch gap_alert, all derived by the store from the writes those services already make) the
// outcomes HORIZON game months later, the verdict (insufficient, poor, unclear, helps), and how many
// deterioration episodes each service reacted to. Then:
//   1. eval.<service>.<action> on the blackboard: the score, the verdict and `streak`, the
//      evaluations in a row judged poor. The streak advances only when new outcomes matured since
//      the last evaluation (n grew): the same evidence read again is one finding, not several. A
//      verdict that is not poor resets it. Every write is a rhai.shared event, so the history is in
//      the store (view service_eval).
//   2. eval.poor: every score whose streak reached PERSIST, and eval.coverage: per service the
//      episodes of the newest run and how many it reacted to.
//   3. One log line per persistently poor score, starting "[service_eval] POOR", each game month it
//      stays so: GET /api/services/log?name=service_eval, which the strategy digest shows.
//   4. derived.<consumer>.<name>: the thresholds the store gives (derive_lib.rhai), each with its
//      source, n and why, for the services that read them: gap_watch's share drop over its WINDOW,
//      which gap_watch reads (derived.gap_watch.share_drop: a value in 0..1 is its flag threshold;
//      while the store holds too few drops the value is null and gap_watch keeps its SHARE_DROP,
//      labelled as the cold start), and the coverage threshold of the scores route. The watchdog
//      derives its own cold start (watchdog.rhai), so none is written for it here.
// One line a look (the heartbeat its `// stall:` limit needs; #308 fixed the same in the watchdog)
// and the evaluation lines once a game month.
// It disables, restarts and changes nothing: no service, setting or approval is touched. The scores
// are associative, not causal (the route's caveat). SERVICE_EVAL_ROUNDS stops it after that many
// looks (0 or unset: never).
import "derive_lib" as dl;

const ROUND_S = 30;
// Game months between an action's state and its outcome; the route's default.
const HORIZON = 3;
// Evaluations with fresh outcomes in a row a score must be poor before it is said.
const PERSIST = 3;
// gap_watch.rhai's WINDOW, the months its share drop is measured over (a test keeps the two equal,
// and src/service_eval.rs GAP_WINDOW).
const GAP_WINDOW = 6;
const DERIVE_MIN_N = 30;

fn rounds() { let v = env("SERVICE_EVAL_ROUNDS"); if v == "" { 0 } else { parse_int(v) } }

// The streak after this evaluation, pure: the previous key (() at first) and the score now.
fn streak_of(prev, s) {
    if s.verdict != "poor" { return 0; }
    let before = if type_of(prev) == "map" { prev } else { #{} };
    let streak = before.streak ?? 0;
    if s.n > (before.n ?? 0) { streak + 1 } else { streak }
}

// The log line of a persistently poor score.
fn line_of(s, streak) {
    let what = if s.endpoint == "resolution" { "followed by a resolution" } else { "above the company's own trend" };
    `[service_eval] POOR ${s.service}.${s.action}: ${s.successes} of ${s.n} outcome(s) ${what} ` +
    `(P(rate < 1/2) ${s.posterior.p_below_half}, ${s.runs} run(s), median relative outcome ${s.median_relative_outcome}); ` +
    `poor ${streak} evaluation(s) in a row. Expected: ${s.expected}. Never disabled: look at it`
}

// One evaluation, pure: the route's answer, the previous eval.<service>.<action> keys by key, the
// game date. Answers the writes, the persistently poor keys and the log lines.
fn evaluate(answer, prev, date) {
    let writes = [];
    let poor = [];
    let lines = [];
    for s in answer.scores ?? [] {
        let key = `eval.${s.service}.${s.action}`;
        let streak = streak_of(prev[key], s);
        let v = s;
        v.streak = streak;
        v.persistent = streak >= global::PERSIST;
        v.date = date;
        v.horizon = answer.horizon;
        writes.push(#{ key: key, value: v });
        if v.persistent {
            poor.push(#{ key: key, service: s.service, action: s.action, streak: streak, n: s.n, rate: s.rate });
            lines.push(line_of(s, streak));
        }
    }
    let coverage = #{};
    let newest = 0;
    for c in (answer.coverage ?? #{}).runs ?? [] { if c.run_ms > newest { newest = c.run_ms; } }
    for c in (answer.coverage ?? #{}).runs ?? [] {
        if c.run_ms == newest {
            coverage[c.service] = #{ episodes: c.episodes, covered: c.covered, coverage: c.coverage,
                                     median_latency_months: c.median_latency_months, actions: c.actions,
                                     outside_episodes: c.outside_episodes };
        }
    }
    #{ writes: writes, poor: poor, lines: lines, coverage: coverage }
}

// The thresholds the store gives, keyed by their blackboard key. A threshold the store cannot give
// yet has the value () and says so: its consumer keeps its own cold start, under that name.
fn thresholds(rows, answer) {
    let out = #{};
    let g = dl::pick(rows, "leader_ratio_drop", "p75", 1.0, (), global::DERIVE_MIN_N, 0.0, 1.0);
    if g.source == "fallback" { g.why = `${g.why}: gap_watch keeps its own SHARE_DROP`; }
    out["derived.gap_watch.share_drop"] = g;
    out["derived.service_eval.coverage_drop"] = (answer.coverage ?? #{}).drop;
    out
}

let looks = 0;
let last_month = "";
print(`service_eval: a look every ${ROUND_S} s, an evaluation every game month (horizon ${HORIZON} months, poor said after ${PERSIST} evaluations)`);
loop {
    try {
        let date = read("openttd://game/state").game_date;
        let month = `${date}`.sub_string(0, 7);
        if month != last_month {
            let answer = api_serviceScores(#{ horizon: HORIZON });
            if "error" in answer || "http_error" in answer { throw `scores: ${answer.error ?? answer.body}`; }
            let prev = #{};
            for s in answer.scores ?? [] { let k = `eval.${s.service}.${s.action}`; prev[k] = shared_get(k); }
            let r = evaluate(answer, prev, date);
            for w in r.writes { shared_set(w.key, w.value); }
            shared_set("eval.poor", #{ date: date, poor: r.poor, rule: answer.rule, caveat: answer.caveat });
            shared_set("eval.coverage", #{ date: date, services: r.coverage, drop: (answer.coverage ?? #{}).drop });
            // The month is evaluated: a failure below costs the thresholds, never a second evaluation.
            last_month = month;
            let verdicts = [];
            for w in r.writes { verdicts.push(`${w.key.sub_string(5)} ${w.value.verdict} (${w.value.successes}/${w.value.n})`); }
            print(`service_eval ${date}: ${answer.actions} action(s); ${verdicts}`);
            for l in r.lines { print(l); }
            try {
                let t = thresholds(dl::baselines(GAP_WINDOW), answer);
                for k in t.keys() { shared_set(k, t[k]); }
            } catch (err) {
                print(`service_eval: thresholds: ${err}`);
            }
        }
    } catch (err) {
        print(`service_eval: ${err}`);
    }
    print(`service_eval: look ${looks + 1}, game month ${if last_month == "" { "not evaluated yet" } else { last_month }}`);
    looks += 1;
    if rounds() > 0 && looks >= rounds() { break; }
    sleep(ROUND_S * 1000);
}
