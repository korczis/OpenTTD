//! RESEARCH-ONLY: policy freight-margin@2 of the Battlebrain freight-line decision (I0096,
//! research/battlebrain/README.md). A pure function: the planner's output, one company's money and
//! loan, and the parameters in; the chosen freight rail line or HOLD, every rejected line with its
//! reason, and the planner's own choice beside it out. No I/O, no clock, no game.
//!
//! The rule. A planner candidate is one freight line (cargo, producer, acceptor) with the option
//! the planner rates best. Only freight_rail options are eligible, because the decision fills one
//! `fr_*` slot. A line is adopted when all of these hold:
//! - it is profitable: monthly profit > 0 and investment > 0;
//! - track-planning priced it: its `track` answer has status `found` (with `require_priced`, the
//!   default since @2); with `require_priced` off, a line track-planning never priced is judged on
//!   the planner's estimate alone, as @1 did, and one it could not route is still refused;
//! - its annual return 12 x monthly_profit / investment is at least `margin`;
//! - the company can pay for it and keep `reserve`: investment <= money + (max_loan - loan) - reserve.
//!
//! Among the adoptable lines the highest annual return wins. Ties are broken deterministically:
//! the lower investment, then the lower (cargo_id, producer, acceptor). With no adoptable line the
//! decision is HOLD.
//!
//! The planner's choice is the baseline: the highest-scored freight_rail line of its portfolio
//! (`portfolio.chosen`). Its slot order in the bot (panel_bot.rs settings_from_plan) may differ
//! when a line is unbuildable; the comparison names that.

use serde_json::{json, Value};

pub const POLICY_ID: &str = "freight-margin";
/// 2: `require_priced` (I0096 finding: the shadow run on a real capture adopted an unpriced line).
/// With `require_priced: false` the rule is exactly @1's.
pub const POLICY_VERSION: &str = "2";

#[derive(Clone, Debug, PartialEq)]
pub struct Params {
    /// The minimum annual return (12 x monthly profit / investment) to adopt a line.
    pub margin: f64,
    /// The money that must remain after paying for the line, loan headroom included.
    pub reserve: i64,
    /// Adopt only a line track-planning priced on the real map (status `found`).
    pub require_priced: bool,
}

impl Params {
    pub fn to_json(&self) -> Value {
        json!({"margin": self.margin, "reserve": self.reserve, "require_priced": self.require_priced})
    }
}

/// One freight line as the policy sees it.
#[derive(Clone, Debug, PartialEq)]
pub struct Line {
    pub cargo_id: i64,
    pub producer: i64,
    pub acceptor: i64,
    pub mode: String,
    pub investment: f64,
    pub monthly_profit: f64,
    pub planner_score: f64,
    /// The track-planning status the planner recorded for the option, if it priced it.
    pub track: Option<String>,
}

impl Line {
    pub fn key(&self) -> (i64, i64, i64) {
        (self.cargo_id, self.producer, self.acceptor)
    }
    pub fn annual_return(&self) -> f64 {
        if self.investment > 0.0 { 12.0 * self.monthly_profit / self.investment } else { f64::NEG_INFINITY }
    }
    pub fn payback_months(&self) -> Option<f64> {
        (self.monthly_profit > 0.0).then(|| self.investment / self.monthly_profit)
    }
    pub fn to_json(&self) -> Value {
        json!({"cargo_id": self.cargo_id, "producer": self.producer, "acceptor": self.acceptor, "mode": self.mode,
               "investment": self.investment, "monthly_profit": self.monthly_profit, "planner_score": self.planner_score,
               "annual_return": round4(self.annual_return()), "payback_months": self.payback_months().map(round4), "track": self.track})
    }
}

fn round4(x: f64) -> f64 {
    if x.is_finite() { (x * 10_000.0).round() / 10_000.0 } else { x }
}

fn num(v: &Value, k: &str) -> Option<f64> {
    v.get(k).and_then(Value::as_f64)
}

/// The freight lines of a planner candidate list (`freight` of mcp-plan's answer, or a portfolio's
/// `chosen`): the freight_rail option of each, if it has one.
pub fn lines(candidates: &Value) -> Vec<Line> {
    let mut out = Vec::new();
    for c in candidates.as_array().into_iter().flatten() {
        let id = |p: &str| c.pointer(p).and_then(crate::battlebrain_contract::graph_number);
        let (Some(cargo_id), Some(producer), Some(acceptor)) = (id("/cargo_id"), id("/producer/id"), id("/acceptor/id"))
        else {
            continue;
        };
        let Some(o) = c["options"].as_array().into_iter().flatten().find(|o| o["mode"] == "freight_rail") else { continue };
        out.push(Line {
            cargo_id,
            producer,
            acceptor,
            mode: "freight_rail".into(),
            investment: num(o, "investment").unwrap_or(0.0),
            monthly_profit: num(o, "monthly_profit").unwrap_or(0.0),
            planner_score: num(o, "score").unwrap_or(f64::MIN),
            track: o.pointer("/track/status").and_then(Value::as_str).map(str::to_string),
        });
    }
    out
}

/// What the company can spend: money plus loan headroom, minus the reserve.
pub fn budget(company: &Value, p: &Params) -> Option<i64> {
    let money = company["money"].as_i64()?;
    let headroom = company["max_loan"].as_i64()? - company["loan"].as_i64()?;
    Some(money + headroom.max(0) - p.reserve)
}

/// Why a line cannot be adopted, or None when it can.
pub fn refusal(l: &Line, budget: i64, p: &Params) -> Option<String> {
    if l.monthly_profit <= 0.0 || l.investment <= 0.0 {
        return Some(format!("not profitable (monthly profit {}, investment {})", l.monthly_profit, l.investment));
    }
    match &l.track {
        Some(t) if t != "found" => return Some(format!("track-planning did not find a route ({t})")),
        None if p.require_priced => return Some("track-planning did not price this line on the map".to_string()),
        _ => {}
    }
    if l.annual_return() < p.margin {
        return Some(format!("annual return {:.4} is below the margin {}", l.annual_return(), p.margin));
    }
    if l.investment > budget as f64 {
        return Some(format!("investment {} exceeds what the company can spend and keep the reserve ({budget})", l.investment));
    }
    None
}

/// The deterministic order of adoptable lines: best first.
pub fn better(a: &Line, b: &Line) -> std::cmp::Ordering {
    b.annual_return().total_cmp(&a.annual_return()).then(a.investment.total_cmp(&b.investment)).then(a.key().cmp(&b.key()))
}

/// The outcome of the policy on one snapshot.
#[derive(Clone, Debug, PartialEq)]
pub struct Choice {
    pub selected: Option<Line>,
    pub rejected: Vec<(Line, String)>,
    pub budget: i64,
}

/// Apply the policy (POLICY_ID@POLICY_VERSION) to the planner's freight candidates for one company.
pub fn decide(freight: &Value, company: &Value, p: &Params) -> Result<Choice, String> {
    let budget = budget(company, p).ok_or("the company record has no money, loan or max_loan")?;
    let mut adoptable = Vec::new();
    let mut rejected = Vec::new();
    for l in lines(freight) {
        match refusal(&l, budget, p) {
            Some(why) => rejected.push((l, why)),
            None => adoptable.push(l),
        }
    }
    adoptable.sort_by(better);
    let mut it = adoptable.into_iter();
    let selected = it.next();
    for l in it {
        let why = match &selected {
            Some(s) if (l.annual_return() - s.annual_return()).abs() < 1e-12 => "ties on annual return; the tie-break (lower investment, then lower cargo/producer/acceptor) prefers the selected line".to_string(),
            _ => "lower annual return than the selected line".to_string(),
        };
        rejected.push((l, why));
    }
    rejected.sort_by(|a, b| a.0.key().cmp(&b.0.key()));
    Ok(Choice { selected, rejected, budget })
}

/// The planner's own choice: the highest-scored freight_rail line of its portfolio.
pub fn baseline(plan: &Value) -> Option<Line> {
    let mut ls = lines(&plan["portfolio"]["chosen"]);
    ls.sort_by(|a, b| b.planner_score.total_cmp(&a.planner_score).then(a.key().cmp(&b.key())));
    ls.into_iter().next()
}

/// How battlebrain's choice relates to the planner's, with the reason.
pub fn divergence(base: Option<&Line>, ours: Option<&Line>, budget: i64, p: &Params) -> (&'static str, String) {
    match (base, ours) {
        (None, None) => ("same", "neither the planner nor battlebrain picks a freight rail line".into()),
        (Some(b), Some(o)) if b.key() == o.key() => ("same", "both pick the same line".into()),
        (Some(b), _) if refusal(b, budget, p).is_some() => (
            "input_model_discrepancy",
            format!("the planner's line fails {POLICY_ID}@{POLICY_VERSION} on the same snapshot: {}", refusal(b, budget, p).unwrap_or_default()),
        ),
        (Some(_), None) => ("intended_policy_difference", "battlebrain holds: no line clears the margin and the reserve".into()),
        (None, Some(_)) => ("intended_policy_difference", "the planner's portfolio holds no freight rail line; battlebrain adopts one".into()),
        (Some(b), Some(o)) if (b.annual_return() - o.annual_return()).abs() < 1e-12 => {
            ("tie_order", "equal annual return; the two orders differ (planner score against the documented tie-break)".into())
        }
        (Some(_), Some(_)) => (
            "intended_policy_difference",
            "the planner ranks by its payback score within a knapsack, battlebrain by annual return within money and reserve".into(),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cand(cargo: i64, prod: i64, acc: i64, inv: f64, profit: f64, score: f64, track: Option<&str>) -> Value {
        let mut o = json!({"mode": "freight_rail", "investment": inv, "monthly_profit": profit, "score": score});
        if let Some(t) = track {
            o["track"] = json!({"status": t});
        }
        // The planner carries world-graph ids ("industry:41"), as a real mcp-plan answer does.
        json!({"kind": "freight", "cargo_id": cargo, "producer": {"id": format!("industry:{prod}")}, "acceptor": {"id": format!("industry:{acc}")}, "mode": "freight_rail", "score": score,
               "options": [{"mode": "truck", "investment": 1.0, "monthly_profit": 1000.0, "score": 1e9}, o]})
    }
    fn company(money: i64, loan: i64, max_loan: i64) -> Value {
        json!({"id": 1, "money": money, "loan": loan, "max_loan": max_loan})
    }
    const P: Params = Params { margin: 0.2, reserve: 25_000, require_priced: false };
    const PRICED: Params = Params { margin: 0.2, reserve: 25_000, require_priced: true };

    #[test]
    fn with_require_priced_an_unpriced_line_is_never_adopted() {
        let f = json!([cand(1, 10, 20, 50_000.0, 5_000.0, 9.0, None), cand(1, 11, 21, 60_000.0, 3_000.0, 1.0, Some("found"))]);
        let off = decide(&f, &company(500_000, 0, 0), &P).unwrap();
        assert_eq!(off.selected.unwrap().key(), (1, 10, 20), "@1 behaviour: the better unpriced line");
        let on = decide(&f, &company(500_000, 0, 0), &PRICED).unwrap();
        assert_eq!(on.selected.unwrap().key(), (1, 11, 21), "only the priced line");
        assert!(on.rejected.iter().any(|(l, r)| l.key() == (1, 10, 20) && r.contains("did not price")));
        let none = decide(&json!([cand(1, 10, 20, 50_000.0, 5_000.0, 9.0, None)]), &company(500_000, 0, 0), &PRICED).unwrap();
        assert!(none.selected.is_none(), "nothing priced: HOLD");
    }

    #[test]
    fn the_best_annual_return_within_budget_is_adopted() {
        let f = json!([cand(1, 10, 20, 100_000.0, 2_000.0, 5.0, None), cand(1, 11, 21, 60_000.0, 3_000.0, 1.0, Some("found"))]);
        let c = decide(&f, &company(50_000, 0, 100_000), &P).unwrap();
        assert_eq!(c.budget, 125_000);
        assert_eq!(c.selected.as_ref().unwrap().key(), (1, 11, 21), "0.6 a year beats 0.24");
        assert_eq!(c.rejected.len(), 1);
        assert!(c.rejected[0].1.contains("lower annual return"));
    }

    #[test]
    fn the_truck_option_is_never_the_line() {
        let f = json!([cand(1, 10, 20, 100_000.0, 2_000.0, 5.0, None)]);
        assert_eq!(decide(&f, &company(500_000, 0, 0), &P).unwrap().selected.unwrap().mode, "freight_rail");
    }

    #[test]
    fn hold_when_nothing_clears_the_margin() {
        let f = json!([cand(1, 10, 20, 100_000.0, 1_000.0, 5.0, None)]); // 0.12 a year
        let c = decide(&f, &company(500_000, 0, 0), &P).unwrap();
        assert!(c.selected.is_none());
        assert!(c.rejected[0].1.contains("below the margin"));
    }

    #[test]
    fn hold_when_the_reserve_would_be_spent() {
        let f = json!([cand(1, 10, 20, 100_000.0, 5_000.0, 5.0, None)]);
        let c = decide(&f, &company(100_000, 300_000, 300_000), &P).unwrap(); // no headroom, 75k spendable
        assert!(c.selected.is_none());
        assert!(c.rejected[0].1.contains("keep the reserve"));
    }

    #[test]
    fn a_line_track_planning_could_not_route_is_refused() {
        for status in ["unreachable", "budget"] {
            let f = json!([cand(1, 10, 20, 50_000.0, 5_000.0, 5.0, Some(status))]);
            let c = decide(&f, &company(500_000, 0, 0), &P).unwrap();
            assert!(c.selected.is_none(), "{status}");
            assert!(c.rejected[0].1.contains("did not find a route"));
        }
    }

    #[test]
    fn ties_break_on_investment_then_key_whatever_the_input_order() {
        let a = cand(2, 30, 40, 50_000.0, 2_500.0, 1.0, None); // 0.6
        let b = cand(1, 31, 41, 50_000.0, 2_500.0, 9.0, None); // 0.6, lower cargo id
        let c = cand(3, 32, 42, 40_000.0, 2_000.0, 2.0, None); // 0.6, lower investment
        for order in [json!([a.clone(), b.clone(), c.clone()]), json!([c.clone(), b.clone(), a.clone()]), json!([b.clone(), a.clone(), c.clone()])] {
            let ch = decide(&order, &company(500_000, 0, 0), &P).unwrap();
            assert_eq!(ch.selected.unwrap().key(), (3, 32, 42));
            assert_eq!(ch.rejected.iter().map(|r| r.0.key()).collect::<Vec<_>>(), vec![(1, 31, 41), (2, 30, 40)]);
            assert!(ch.rejected.iter().all(|r| r.1.starts_with("ties on annual return")));
        }
        let ch = decide(&json!([a.clone(), b.clone()]), &company(500_000, 0, 0), &P).unwrap();
        assert_eq!(ch.selected.unwrap().key(), (1, 31, 41), "equal investment: the lower key");
    }

    #[test]
    fn a_company_without_money_fields_is_an_error_not_a_guess() {
        assert!(decide(&json!([]), &json!({"id": 1}), &P).is_err());
    }

    #[test]
    fn the_baseline_is_the_portfolios_best_scored_rail_line_and_divergence_is_classified() {
        let plan = json!({"portfolio": {"chosen": [cand(1, 10, 20, 100_000.0, 1_500.0, 50.0, None), cand(1, 11, 21, 60_000.0, 3_000.0, 9.0, None)]}});
        let base = baseline(&plan).unwrap();
        assert_eq!(base.key(), (1, 10, 20));
        let ours = decide(&plan["portfolio"]["chosen"], &company(500_000, 0, 0), &P).unwrap();
        let (class, why) = divergence(Some(&base), ours.selected.as_ref(), ours.budget, &P);
        assert_eq!(class, "input_model_discrepancy", "{why}"); // 0.18 a year fails the 0.2 margin
        let (class, _) = divergence(Some(&base), Some(&base), ours.budget, &P);
        assert_eq!(class, "same");
        let (class, _) = divergence(None, None, 0, &P);
        assert_eq!(class, "same");
    }
}
