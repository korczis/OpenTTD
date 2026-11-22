//! The README's hero: an illustration of the loop the fork exists for, observe, plan, ask, act,
//! verify, over an isometric map. It is drawn here, not painted: the geometry is this file's, the
//! colours are NoirGFX's (`tools/research/noirgfx/style.toml`), and every name on it is one the code
//! uses (`openttd://map`, `pending_approval`, `Command<>::Post`, `VERIFIED_SUCCESS`). It shows no
//! number, so it never needs to change when a result does. It is an illustration, not a screenshot.
//!
//! The motion is CSS inside the SVG, which GitHub plays in an `<img>`: the planned line draws, the
//! approval step lights, a train runs the line, the verdict appears. Every element's resting style
//! is its final state and the keyframes only lead into it, so `prefers-reduced-motion` (and any
//! viewer that plays no animation) shows the whole loop at once.

pub struct Palette {
    pub ink: String,
    pub paper: String,
    pub signal: String,
    pub signal_dark: String,
}

/// The `[palette]` table of NoirGFX's style.toml: four `key = "#rrggbb"` lines.
pub fn palette(style_toml: &str) -> Result<Palette, String> {
    let mut section = "";
    let mut get = std::collections::BTreeMap::new();
    for line in style_toml.lines() {
        let t = line.trim();
        if t.starts_with('[') {
            section = t;
            continue;
        }
        if section != "[palette]" {
            continue;
        }
        if let Some((k, v)) = t.split_once('=') {
            let v = v.split('#').nth(1).map(|h| format!("#{}", h.trim().trim_end_matches('"').split('"').next().unwrap_or("")));
            if let Some(v) = v {
                get.insert(k.trim().to_string(), v);
            }
        }
    }
    let take = |k: &str| get.get(k).cloned().ok_or_else(|| format!("style.toml [palette] has no {k}"));
    Ok(Palette { ink: take("ink")?, paper: take("paper")?, signal: take("signal")?, signal_dark: take("signal_dark")? })
}

const TW: f64 = 56.0; // tile width on screen
const TH: f64 = 28.0; // tile height on screen
const OX: f64 = 330.0;
const OY: f64 = 70.0;
const N: i32 = 10;

fn at(i: f64, j: f64) -> (f64, f64) {
    (OX + (i - j) * TW / 2.0, OY + (i + j) * TH / 2.0)
}

fn centre(i: i32, j: i32) -> (f64, f64) {
    at(i as f64 + 0.5, j as f64 + 0.5)
}

fn tile(i: i32, j: i32) -> String {
    let (a, b, c, d) = (at(i as f64, j as f64), at(i as f64 + 1.0, j as f64), at(i as f64 + 1.0, j as f64 + 1.0), at(i as f64, j as f64 + 1.0));
    format!("M{:.1} {:.1}L{:.1} {:.1}L{:.1} {:.1}L{:.1} {:.1}Z", a.0, a.1, b.0, b.1, c.0, c.1, d.0, d.1)
}

fn path(cells: &[(i32, i32)]) -> String {
    cells.iter().enumerate().map(|(k, &(i, j))| {
        let (x, y) = centre(i, j);
        format!("{}{x:.1} {y:.1}", if k == 0 { "M" } else { "L" })
    }).collect::<Vec<_>>().join("")
}

/// An isometric block (a town building or a mill) standing on tile (i, j), `h` pixels tall.
fn block(i: i32, j: i32, h: f64, inset: f64) -> String {
    let (fi, fj) = (i as f64 + inset, j as f64 + inset);
    let s = 1.0 - 2.0 * inset;
    let top = [at(fi, fj), at(fi + s, fj), at(fi + s, fj + s), at(fi, fj + s)];
    let up = |p: (f64, f64)| (p.0, p.1 - h);
    let t: Vec<(f64, f64)> = top.iter().map(|&p| up(p)).collect();
    let poly = |pts: &[(f64, f64)]| pts.iter().map(|p| format!("{:.1},{:.1}", p.0, p.1)).collect::<Vec<_>>().join(" ");
    format!(
        r#"<polygon class="face-l" points="{}"/><polygon class="face-r" points="{}"/><polygon class="face-t" points="{}"/>"#,
        poly(&[top[3], top[2], t[2], t[3]]),
        poly(&[top[2], top[1], t[1], t[2]]),
        poly(&t)
    )
}

fn tree(i: i32, j: i32, dx: f64, dy: f64) -> String {
    let (x, y) = centre(i, j);
    let (x, y) = (x + dx, y + dy);
    format!(r#"<path class="tree" d="M{:.1} {:.1}l7 14h-14z M{:.1} {:.1}l9 16h-18z"/>"#, x, y - 22.0, x, y - 14.0)
}

const STEPS: [(&str, &str, &str); 5] = [
    ("OBSERVE", "openttd://map · towns · industries", "the game's live state, read as MCP resources"),
    ("PLAN", "AlgoGlyph: track priced tile by tile", "a knapsack picks the lines the money allows"),
    ("ASK", "pending_approval", "held on its exact arguments until the owner says yes"),
    ("ACT", "Command<>::Post", "the game's own command system, in lockstep"),
    ("VERIFY", "VERIFIED_SUCCESS · EXECUTED_UNVERIFIED", "what the game shows afterwards, not what was hoped"),
];

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

/// The hero for a light page (`dark == false`: paper ground, ink lines) or a dark one.
pub fn svg(p: &Palette, dark: bool) -> String {
    let (ground, line, faint) = if dark { (&p.ink, &p.paper, "#8a8a8a") } else { (&p.paper, &p.ink, "#8a8a8a") };
    let signal = &p.signal;
    let mut s = String::new();
    s.push_str(&format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1200 420" width="1200" height="420" role="img" aria-labelledby="t d">
<title id="t">OpenTTD research fork: observe, plan, ask, act, verify</title>
<desc id="d">An illustration, not a screenshot. On an isometric map a planner weighs candidate rail lines between a forest and a sawmill and picks one; the owner approves it at the approval boundary; a train runs the line through the game's own command system; the result is observed and recorded.</desc>
<style>
.bg{{fill:{ground}}}
.grid{{fill:none;stroke:{line};stroke-opacity:.16;stroke-width:1}}
.hill{{fill:url(#hatch);stroke:none}}
.water{{fill:none;stroke:{line};stroke-opacity:.45;stroke-width:1.2}}
.face-t{{fill:{ground};stroke:{line};stroke-width:1.2}}
.face-l{{fill:{line};stroke:{line};stroke-width:1.2}}
.face-r{{fill:url(#hatch);stroke:{line};stroke-width:1.2}}
.tree{{fill:{line}}}
.cand{{fill:none;stroke:{faint};stroke-width:2;stroke-dasharray:4 6}}
.route{{fill:none;stroke:{signal};stroke-width:5;stroke-linejoin:round;stroke-linecap:round;stroke-dasharray:640;stroke-dashoffset:0}}
.stn{{fill:{ground};stroke:{signal};stroke-width:3}}
.train{{fill:{signal}}}
.mono{{font-family:ui-monospace,SFMono-Regular,Menlo,Consolas,monospace}}
.sans{{font-family:Inter,ui-sans-serif,system-ui,-apple-system,"Segoe UI",sans-serif}}
.kicker{{fill:{line};font-size:13px;letter-spacing:.18em}}
.num{{fill:{ground};font-size:13px;font-weight:700}}
.numbox{{fill:{line}}}
.numbox.hot{{fill:{signal}}}
.name{{fill:{line};font-size:17px;font-weight:700;letter-spacing:.12em}}
.code{{fill:{line};font-size:14px}}
.code.hot{{fill:{signal}}}
.note{{fill:{faint};font-size:12.5px}}
.rule{{stroke:{line};stroke-opacity:.25;stroke-width:1}}
.boundary{{stroke:{signal};stroke-width:1.5;stroke-dasharray:6 5}}
.blabel{{fill:{signal};font-size:11px;letter-spacing:.16em}}
.foot{{fill:{faint};font-size:11px}}
.step{{opacity:1}}
.done{{opacity:1}}
.mover{{opacity:0}}
@media (prefers-reduced-motion:no-preference){{
.route{{animation:draw 12s ease-in-out infinite}}
.s1{{animation:s1 12s infinite}}.s2{{animation:s2 12s infinite}}.s3{{animation:s3 12s infinite}}.s4{{animation:s4 12s infinite}}.s5{{animation:s5 12s infinite}}
.mover{{animation:mover 12s infinite}}
.parked{{animation:parked 12s infinite}}
.done{{animation:done 12s infinite}}
}}
@keyframes draw{{0%,12%{{stroke-dashoffset:640}}34%,100%{{stroke-dashoffset:0}}}}
@keyframes s1{{0%{{opacity:.3}}4%,100%{{opacity:1}}}}
@keyframes s2{{0%,12%{{opacity:.3}}16%,100%{{opacity:1}}}}
@keyframes s3{{0%,34%{{opacity:.3}}38%,100%{{opacity:1}}}}
@keyframes s4{{0%,48%{{opacity:.3}}52%,100%{{opacity:1}}}}
@keyframes s5{{0%,80%{{opacity:.3}}84%,100%{{opacity:1}}}}
@keyframes mover{{0%,50%{{opacity:0}}52%,80%{{opacity:1}}82%,100%{{opacity:0}}}}
@keyframes parked{{0%,80%{{opacity:0}}82%,100%{{opacity:1}}}}
@keyframes done{{0%,80%{{opacity:0}}86%,100%{{opacity:1}}}}
</style>
<defs><pattern id="hatch" width="6" height="6" patternUnits="userSpaceOnUse" patternTransform="rotate(-30)"><rect width="6" height="6" fill="{ground}"/><line x1="0" y1="0" x2="0" y2="6" stroke="{line}" stroke-width="1.6"/></pattern></defs>
<rect class="bg" width="1200" height="420"/>
"#
    ));
    // The map: a grid, hills, water, two towns, a forest and a sawmill.
    s.push_str(r#"<g class="map">"#);
    for i in 0..N {
        for j in 0..N {
            s.push_str(&format!(r#"<path class="grid" d="{}"/>"#, tile(i, j)));
        }
    }
    for (i, j) in [(4, 0), (5, 0), (5, 1), (4, 1), (6, 0), (0, 3), (0, 4), (1, 4)] {
        s.push_str(&format!(r#"<path class="hill" d="{}"/>"#, tile(i, j)));
    }
    for (i, j) in [(9, 6), (9, 7), (8, 7), (9, 8)] {
        let (x, y) = centre(i, j);
        s.push_str(&format!(r#"<path class="water" d="M{:.1} {:.1}q5 -4 10 0t10 0 M{:.1} {:.1}q5 -4 10 0t10 0"/>"#, x - 10.0, y - 3.0, x - 14.0, y + 4.0));
    }
    for &(i, j, h) in &[(2, 2, 20.0), (3, 2, 30.0), (2, 3, 14.0), (6, 8, 18.0), (7, 8, 26.0)] {
        s.push_str(&block(i, j, h, 0.18));
    }
    for &(i, j, dx, dy) in &[(0, 7, -6.0, 2.0), (0, 8, 4.0, 0.0), (1, 8, -2.0, 4.0), (0, 6, 6.0, 4.0), (1, 9, 2.0, 0.0)] {
        s.push_str(&tree(i, j, dx, dy));
    }
    s.push_str(&block(8, 2, 26.0, 0.12));
    s.push_str("</g>");
    // Candidates the planner priced and did not pick, then the line it did.
    let line_cells = [(1, 7), (3, 7), (6, 4), (8, 4), (8, 3)];
    let cand_a = [(1, 7), (1, 5), (4, 2), (7, 2), (8, 3)];
    let cand_b = [(1, 7), (5, 7), (8, 4), (8, 3)];
    s.push_str(&format!(r#"<path class="cand step s2" d="{}"/><path class="cand step s2" d="{}"/>"#, path(&cand_a), path(&cand_b)));
    let route = path(&line_cells);
    s.push_str(&format!(r#"<path id="line" class="route" d="{route}"/>"#));
    for &(i, j) in &[line_cells[0], line_cells[line_cells.len() - 1]] {
        let (x, y) = centre(i, j);
        s.push_str(&format!(r#"<rect class="stn" x="{:.1}" y="{:.1}" width="14" height="14" transform="rotate(45 {x:.1} {y:.1})"/>"#, x - 7.0, y - 7.0));
    }
    s.push_str(&format!(
        r#"<g class="mover"><rect class="train" x="-11" y="-6" width="22" height="12" rx="2"><animateMotion dur="12s" repeatCount="indefinite" keyPoints="0;0;1;1" keyTimes="0;0.52;0.8;1" calcMode="linear" path="{route}"/></rect></g>"#
    ));
    let (ex, ey) = centre(8, 3);
    s.push_str(&format!(r#"<rect class="train parked" x="{:.1}" y="{:.1}" width="22" height="12" rx="2"/>"#, ex - 11.0 - 26.0, ey - 6.0 + 13.0));
    s.push_str(&format!(
        r#"<text class="mono kicker" x="40" y="44">OPENTTD · RESEARCH FORK</text><text class="mono foot" x="40" y="398">forest → sawmill · grey: candidates priced, red: the line picked</text>"#
    ));
    // The loop, right of the map.
    let (x0, y0, dy) = (700.0, 60.0, 72.0);
    s.push_str(&format!(r#"<line class="rule" x1="{x0}" y1="{:.1}" x2="{x0}" y2="{:.1}"/>"#, y0 - 14.0, y0 + dy * 4.0 + 40.0));
    for (k, (name, code, note)) in STEPS.iter().enumerate() {
        let y = y0 + dy * k as f64;
        let hot = k == 2;
        s.push_str(&format!(
            r#"<g class="step s{}"><rect class="numbox{}" x="{:.1}" y="{:.1}" width="28" height="28" rx="3"/><text class="mono num" x="{:.1}" y="{:.1}" text-anchor="middle">{:02}</text><text class="mono name" x="{:.1}" y="{:.1}">{}</text><text class="mono code{}" x="{:.1}" y="{:.1}">{}</text><text class="sans note" x="{:.1}" y="{:.1}">{}</text></g>"#,
            k + 1, if hot { " hot" } else { "" }, x0 - 14.0, y - 4.0, x0, y + 15.0, k + 1,
            x0 + 32.0, y + 10.0, name, if hot { " hot" } else { "" }, x0 + 150.0, y + 10.0, esc(code),
            x0 + 32.0, y + 32.0, esc(note)
        ));
        if k == 2 {
            let by = y + 48.0;
            s.push_str(&format!(
                r#"<line class="boundary" x1="{:.1}" y1="{by:.1}" x2="1160" y2="{by:.1}"/><text class="mono blabel" x="1160" y="{:.1}" text-anchor="end">APPROVAL BOUNDARY · NO SCRIPT APPROVES ITSELF</text>"#,
                x0 - 30.0, by + 13.0
            ));
        }
    }
    s.push_str(&format!(
        r#"<text class="mono foot done" x="1160" y="398" text-anchor="end">illustration · colours: tools/research/noirgfx/style.toml</text>"#
    ));
    let _ = &p.signal_dark;
    s.push_str("</svg>\n");
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn palette_from_style_toml() {
        let p = palette("[palette]\n# c\nink = \"#0b0b0b\"\npaper = \"#ece4d6\"\nsignal = \"#d11f1a\"\nsignal_dark = \"#5e0b08\"\n[screen]\nink = \"#ffffff\"\n").unwrap();
        assert_eq!((p.ink.as_str(), p.paper.as_str(), p.signal.as_str(), p.signal_dark.as_str()), ("#0b0b0b", "#ece4d6", "#d11f1a", "#5e0b08"));
    }

    #[test]
    fn both_heroes_carry_the_palette_and_reduced_motion() {
        let p = palette("[palette]\nink = \"#000001\"\npaper = \"#fffffe\"\nsignal = \"#ff0002\"\nsignal_dark = \"#110000\"\n").unwrap();
        let light = svg(&p, false);
        let dark = svg(&p, true);
        assert!(light.contains(".bg{fill:#fffffe}") && dark.contains(".bg{fill:#000001}"));
        assert!(light.contains("prefers-reduced-motion:no-preference") && light.contains("pending_approval"));
        assert!(light.contains("Command&lt;&gt;::Post"));
    }
}
