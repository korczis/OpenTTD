//! RESEARCH-ONLY: one long-lived AlgoGlyph `flow serve` for every kernel call of a process (I0086).
//!
//! `npm run flow -- kernel <id> -` boots Vite and AlgoGlyph's registry for every call; a planned bot
//! start (mcp-plan --track 16) makes one track-planning call per line and then knapsack-dp, each its
//! own boot. `flow serve` (AlgoGlyph scripts/flow.mjs) answers one JSON request per stdin line with
//! one JSON line, for as long as stdin stays open; mcp-human's `Flow` (src/algoglyph.rs there)
//! already drives it. This is the same client for mcp-rhai, generic over the kernel: it knows the
//! serve protocol and nothing of OpenTTD.
//!
//! Switch: `MCP_ALGOGLYPH_SERVE=1` (or `on`, `true`). Off by default, so every caller keeps the
//! per-call runner until the switch is chosen. A serve that cannot start is dropped for the rest of
//! the process. One request waits at most `DEADLINE` for its answer (I0160): two threads of the
//! serve's own hold its pipes, so no caller blocks on a pipe while it holds the shared lock, and a
//! second caller waits for the lock at most the first one's deadline. A serve that misses its
//! deadline, ends, or answers garbage is killed with its process group, and a later call starts a
//! new one after a backoff that doubles with each failure in a row; after `RESTART_CAP` restarts in
//! a row with no answer the serve is dropped. Whenever the serve refuses, the caller falls back to
//! the per-call runner; the fallbacks, the deadlines missed and the restarts are counted and
//! reported by `summary()`, never silent.
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

/// The longest one request waits for its answer, the hub's `KERNEL_TIMEOUT` (src/hub_algoglyph.rs).
/// The first request of a serve includes its boot.
pub const DEADLINE: Duration = Duration::from_secs(120);
/// Restarts in a row, with no answer between them, before the serve is dropped for the process.
pub const RESTART_CAP: u32 = 3;
/// The wait before the first restart after a failure; doubled for each failure in a row.
pub const BACKOFF: Duration = Duration::from_millis(500);
/// The longest wait before a restart.
pub const BACKOFF_MAX: Duration = Duration::from_secs(8);

/// Whether the switch asks for one long-lived serve.
pub fn wanted() -> bool {
    PREFERRED.load(std::sync::atomic::Ordering::SeqCst)
        || std::env::var("MCP_ALGOGLYPH_SERVE").is_ok_and(|v| matches!(v.trim(), "1" | "on" | "true" | "yes"))
}

/// Whether the switch is explicitly off (`0`, `off`, `false`, `no`), not merely unset.
#[allow(dead_code)] // only the panel's strategist distinguishes unset from off
pub fn refused() -> bool {
    std::env::var("MCP_ALGOGLYPH_SERVE").is_ok_and(|v| matches!(v.trim(), "0" | "off" | "false" | "no"))
}

/// A process that keeps calling kernels (the panel's strategist, src/panel_strategy.rs) asks for
/// the one serve without the variable; `wanted()` is then true for the rest of the process.
static PREFERRED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

#[allow(dead_code)] // only the panel's strategist prefers the serve
pub fn prefer() {
    PREFERRED.store(true, std::sync::atomic::Ordering::SeqCst);
}

/// Forget why the serve went away, so the next call starts a new one. A long-lived caller does this
/// at most once per cycle; a one-shot process never needs it. Answers the reason forgotten.
#[allow(dead_code)]
pub fn retry() -> Option<String> {
    shared().lock().unwrap_or_else(|p| p.into_inner()).gone.take()
}

/// Stop the serve (its process is killed and reaped; node sees its stdin close), at a panel's stop.
#[allow(dead_code)]
pub fn shutdown() {
    let mut s = shared().lock().unwrap_or_else(|p| p.into_inner());
    s.serve = None;
    s.pid = None;
    s.gone = Some("stopped with the process".into());
}

/// Why a request has no answer.
enum Fault {
    /// No answer within the deadline: the serve is hung, or the kernel is slower than allowed.
    Missed(Duration),
    /// The serve ended, could not be written to, or answered something else.
    Broken(String),
}

impl std::fmt::Display for Fault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Fault::Missed(d) => write!(f, "flow serve: no answer within its deadline of {} ms", d.as_millis()),
            Fault::Broken(e) => f.write_str(e),
        }
    }
}

/// A running `flow serve`. Two threads of its own hold its pipes, one writing the request lines and
/// one reading the answer lines, so a request never blocks on a pipe and waits for its answer with
/// a deadline.
pub struct Serve {
    child: Child,
    /// To the writer thread; None once the serve is being stopped.
    lines_in: Option<Sender<String>>,
    /// From the reader thread; disconnected once the serve's stdout closes.
    lines_out: Receiver<std::io::Result<String>>,
    next: u64,
}

impl Serve {
    /// Start `npm run --silent flow -- serve` in the AlgoGlyph checkout `dir`.
    pub fn start(dir: &Path) -> Result<Serve, String> {
        if !dir.join("package.json").is_file() {
            return Err(format!("no AlgoGlyph checkout at {}", dir.display()));
        }
        Self::spawn(Command::new("npm").args(["run", "--silent", "flow", "--", "serve"]).current_dir(dir))
    }

    /// Any process that speaks the serve protocol (a test stands one in). It leads a process group
    /// of its own, so that killing it also kills what it started (npm's node).
    pub fn spawn(cmd: &mut Command) -> Result<Serve, String> {
        let mut child = cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).process_group(0)
            .spawn().map_err(|e| format!("flow serve: {e}"))?;
        let mut stdin = child.stdin.take().ok_or("flow serve: no stdin")?;
        let stdout = BufReader::new(child.stdout.take().ok_or("flow serve: no stdout")?);
        let (lines_in, to_write) = mpsc::channel::<String>();
        std::thread::spawn(move || {
            for line in to_write {
                if writeln!(stdin, "{line}").and_then(|_| stdin.flush()).is_err() { break; }
            }
        });
        let (read, lines_out) = mpsc::channel();
        std::thread::spawn(move || {
            for line in stdout.lines() {
                let end = line.is_err();
                if read.send(line).is_err() || end { break; }
            }
        });
        Ok(Serve { child, lines_in: Some(lines_in), lines_out, next: 1 })
    }

    pub fn pid(&self) -> u32 { self.child.id() }

    /// One request {id, cmd, ...}: a line in, the line with the same id out, within `deadline`.
    fn ask(&mut self, mut q: Value, deadline: Duration) -> Result<Value, Fault> {
        let id = self.next;
        self.next += 1;
        q["id"] = json!(id);
        let until = Instant::now() + deadline;
        if !self.lines_in.as_ref().is_some_and(|w| w.send(q.to_string()).is_ok()) {
            return Err(Fault::Broken("flow serve: its stdin is closed".into()));
        }
        loop {
            let line = match self.lines_out.recv_timeout(until.saturating_duration_since(Instant::now())) {
                Ok(Ok(line)) => line,
                Ok(Err(e)) => return Err(Fault::Broken(format!("flow serve: {e}"))),
                Err(RecvTimeoutError::Timeout) => return Err(Fault::Missed(deadline)),
                Err(RecvTimeoutError::Disconnected) => return Err(Fault::Broken("flow serve: the server ended".into())),
            };
            if line.trim().is_empty() { continue; }
            let mut v: Value = serde_json::from_str(&line).map_err(|e| Fault::Broken(format!("flow serve: not JSON ({e})")))?;
            if v.get("id") != Some(&json!(id)) {
                let got = v.get("id").unwrap_or(&Value::Null);
                return Err(Fault::Broken(format!("flow serve: answer for id {got} while waiting for {id}")));
            }
            if let Some(m) = v.as_object_mut() { m.remove("id"); }
            return Ok(v);
        }
    }

    /// One kernel on an input, within `DEADLINE`; the answer exactly as `flow -- kernel <id> -`
    /// prints it.
    #[allow(dead_code)] // the process's serve is called through `kernel()`; tests drive one directly
    pub fn kernel(&mut self, kernel: &str, input: &Value) -> Result<Value, String> {
        self.ask(kernel_request(kernel, input), DEADLINE).map_err(|f| f.to_string())
    }
}

fn kernel_request(kernel: &str, input: &Value) -> Value {
    json!({"cmd": "kernel", "kernel": kernel, "input": input})
}

impl Drop for Serve {
    fn drop(&mut self) {
        self.lines_in = None;
        // The whole group (its id is the serve's pid, process_group(0)): npm's node would outlive
        // npm, and a hung one never reads its stdin closing. A pid always fits a pid_t.
        unsafe { libc::kill(-(self.child.id() as libc::pid_t), libc::SIGKILL); }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// How long a request may wait, and how a failed serve is restarted.
#[derive(Clone, Copy)]
struct Limits {
    deadline: Duration,
    backoff: Duration,
    backoff_max: Duration,
    cap: u32,
}

const LIMITS: Limits = Limits { deadline: DEADLINE, backoff: BACKOFF, backoff_max: BACKOFF_MAX, cap: RESTART_CAP };

impl Limits {
    /// The wait before the restart after the `failures`-th failure in a row (1 is the first).
    fn backoff_after(&self, failures: u32) -> Duration {
        self.backoff.saturating_mul(1u32 << failures.saturating_sub(1).min(16)).min(self.backoff_max)
    }
}

/// The process's one serve, and what it did.
struct Shared {
    serve: Option<Serve>,
    /// Why the serve is gone (it is then not restarted), or None while it runs, before it started,
    /// or while it waits for a restart.
    gone: Option<String>,
    pid: Option<u32>,
    calls: u64,
    fallbacks: u64,
    /// Requests that had no answer within the deadline.
    missed: u64,
    /// Serves started after a failure.
    restarts: u64,
    /// Failures since the last answer.
    failures: u32,
    /// No restart before this instant, and why the last serve failed.
    not_before: Option<(Instant, String)>,
    limits: Limits,
}

impl Shared {
    fn new(limits: Limits) -> Shared {
        Shared { serve: None, gone: None, pid: None, calls: 0, fallbacks: 0, missed: 0, restarts: 0, failures: 0,
                 not_before: None, limits }
    }

    /// One kernel call through this serve, started by `start` when there is none. Err(reason) when
    /// the serve is unavailable: it cannot start, waits out a backoff, has failed past the cap, or
    /// failed on this call (it is then killed, and a later call starts a new one).
    fn call(&mut self, start: impl FnOnce() -> Result<Serve, String>, kernel: &str, input: &Value) -> Result<Value, String> {
        if let Some(why) = &self.gone { return Err(why.clone()); }
        if self.serve.is_none() {
            if let Some((t, why)) = &self.not_before {
                let left = t.saturating_duration_since(Instant::now());
                if !left.is_zero() {
                    return Err(format!("flow serve: restarts in {} ms, after: {why}", left.as_millis()));
                }
            }
            match start() {
                Ok(v) => {
                    if self.failures > 0 { self.restarts += 1; }
                    self.pid = Some(v.pid());
                    self.serve = Some(v);
                }
                Err(e) => { self.gone = Some(e.clone()); return Err(e); }
            }
        }
        let deadline = self.limits.deadline;
        let r = self.serve.as_mut().map(|v| v.ask(kernel_request(kernel, input), deadline))
            .unwrap_or_else(|| Err(Fault::Broken("flow serve: none".into())));
        match r {
            Ok(v) => {
                self.calls += 1;
                self.failures = 0;
                self.not_before = None;
                Ok(v)
            }
            Err(f) => {
                if matches!(f, Fault::Missed(_)) { self.missed += 1; }
                self.serve = None;
                self.failures += 1;
                let e = f.to_string();
                if self.failures > self.limits.cap {
                    self.gone = Some(format!("flow serve: failed {} times in a row, the cap of {} restarts; last: {e}",
                                             self.failures, self.limits.cap));
                } else {
                    self.not_before = Some((Instant::now() + self.limits.backoff_after(self.failures), e.clone()));
                }
                Err(e)
            }
        }
    }

    fn summary(&self, wanted: bool) -> Value {
        let runner = match (self.pid, wanted) {
            (Some(_), _) => "flow serve, one process for every call",
            (None, true) => "flow serve asked for (MCP_ALGOGLYPH_SERVE), not started",
            (None, false) => "npm run flow -- kernel, one process per call",
        };
        json!({"runner": runner,
               "pid": self.pid, "calls": self.calls, "fallbacks": self.fallbacks,
               "deadline_ms": self.limits.deadline.as_millis() as u64, "deadlines_missed": self.missed,
               "restarts": self.restarts, "restart_cap": self.limits.cap, "gone": self.gone})
    }
}

fn shared() -> &'static Mutex<Shared> {
    static S: OnceLock<Mutex<Shared>> = OnceLock::new();
    S.get_or_init(|| Mutex::new(Shared::new(LIMITS)))
}

/// One kernel call through the process's serve, started on first use in `dir`. Err(reason) when the
/// serve is unavailable; the caller then runs the kernel its own way and says so (`fell_back`).
/// A caller holds the lock for at most one request's deadline.
pub fn kernel(dir: &Path, kernel: &str, input: &Value) -> Result<Value, String> {
    shared().lock().unwrap_or_else(|p| p.into_inner()).call(|| Serve::start(dir), kernel, input)
}

/// Count a call the caller ran with the per-call runner after `kernel` refused.
#[allow(dead_code)] // not every binary that includes this module has a per-call runner
pub fn fell_back() {
    shared().lock().unwrap_or_else(|p| p.into_inner()).fallbacks += 1;
}

/// What answered this process's kernel calls: {runner, pid, calls, fallbacks, deadline_ms,
/// deadlines_missed, restarts, restart_cap, gone}.
#[allow(dead_code)] // mcp-intent includes cargo.rs and never reports its runner
pub fn summary() -> Value {
    shared().lock().unwrap_or_else(|p| p.into_inner()).summary(wanted())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Barrier};

    /// A stand-in for `flow serve`: echoes each request's id with the kernel name, line by line.
    fn fake() -> Serve {
        let script = r#"while IFS= read -r l; do id=$(printf '%s' "$l" | sed -E 's/.*"id":([0-9]+).*/\1/'); k=$(printf '%s' "$l" | sed -E 's/.*"kernel":"([^"]*)".*/\1/'); printf '{"id":%s,"ok":true,"kernel":{"id":"%s"}}\n' "$id" "$k"; done"#;
        sh(script)
    }

    fn sh(script: &str) -> Serve {
        Serve::spawn(Command::new("sh").args(["-c", script])).expect("sh")
    }

    /// A serve that reads every request and never answers, its stdout held open: a hung kernel.
    fn hung() -> Result<Serve, String> { Ok(sh("while IFS= read -r l; do :; done")) }

    /// Deadlines in the tens of milliseconds, margins far wider than a loaded machine's jitter.
    fn limits(deadline_ms: u64, backoff_ms: u64, cap: u32) -> Limits {
        Limits { deadline: Duration::from_millis(deadline_ms), backoff: Duration::from_millis(backoff_ms),
                 backoff_max: Duration::from_secs(5), cap }
    }

    const MARGIN: Duration = Duration::from_secs(2);

    #[test]
    fn one_process_answers_many_calls_each_by_its_own_id() {
        let mut s = fake();
        let pid = s.pid();
        for k in ["knapsack-dp", "track-planning", "transport-investment"] {
            let v = s.kernel(k, &json!({"weights": [1]})).unwrap();
            assert_eq!(v["kernel"]["id"], k);
            assert!(v.get("id").is_none(), "the serve's id is not part of the answer");
        }
        assert_eq!(s.pid(), pid);
    }

    #[test]
    fn a_serve_that_ended_is_an_error_not_an_answer() {
        let mut s = Serve::spawn(Command::new("sh").args(["-c", "exit 0"])).unwrap();
        let _ = s.child.wait();
        assert!(s.kernel("knapsack-dp", &json!({})).is_err());
    }

    #[test]
    fn garbage_and_a_foreign_id_are_errors_and_blank_lines_are_skipped() {
        let e = sh("read l; echo 'not json'").kernel("k", &json!({})).unwrap_err();
        assert!(e.contains("not JSON"), "{e}");
        let e = sh(r#"read l; echo '{"id":99}'"#).kernel("k", &json!({})).unwrap_err();
        assert!(e.contains("answer for id 99 while waiting for 1"), "{e}");
        let e = sh(r#"read l; printf '\377\n'"#).kernel("k", &json!({})).unwrap_err();
        assert!(e.starts_with("flow serve: ") && !e.contains("not JSON"), "a line that is not UTF-8 is a read error: {e}");
        let v = sh(r#"read l; echo; echo '{"id":1,"ok":true}'"#).kernel("k", &json!({})).unwrap();
        assert_eq!(v, json!({"ok": true}));
    }

    #[test]
    fn a_stopped_writer_is_an_error_not_a_wait() {
        let mut s = fake();
        s.lines_in = None;
        let e = s.kernel("k", &json!({})).unwrap_err();
        assert!(e.contains("stdin is closed"), "{e}");
    }

    #[test]
    fn a_hung_serve_fails_one_call_at_its_deadline_and_a_second_caller_is_not_held_past_it() {
        let deadline = Duration::from_millis(60);
        let s = Arc::new(Mutex::new(Shared::new(limits(60, 5_000, 3))));
        let go = Arc::new(Barrier::new(2));
        let callers: Vec<_> = (0..2).map(|_| {
            let (s, go) = (s.clone(), go.clone());
            std::thread::spawn(move || {
                go.wait();
                let t = Instant::now();
                let r = s.lock().unwrap().call(hung, "track-planning", &json!({}));
                (r.unwrap_err(), t.elapsed())
            })
        }).collect();
        let mut out: Vec<_> = callers.into_iter().map(|c| c.join().unwrap()).collect();
        // Whichever got the lock first is the one whose deadline passed.
        out.sort_by_key(|(e, _)| e.contains("restarts in"));
        let (first, t1) = &out[0];
        let (second, t2) = &out[1];
        assert_eq!(first, "flow serve: no answer within its deadline of 60 ms");
        assert!(*t1 >= deadline && *t1 < deadline + MARGIN, "{t1:?}");
        // The second waited for the lock, then refused on the backoff, with the reason, without a
        // serve of its own: it never waited out a deadline of its own.
        assert!(second.starts_with("flow serve: restarts in ") && second.ends_with(first.as_str()), "{second}");
        assert!(*t2 < deadline + MARGIN, "{t2:?}");
        let g = s.lock().unwrap();
        assert!(g.serve.is_none(), "the hung serve is killed");
        let v = g.summary(true);
        assert_eq!((v["deadlines_missed"].as_u64(), v["calls"].as_u64(), v["restarts"].as_u64()), (Some(1), Some(0), Some(0)));
        assert_eq!(v["deadline_ms"], 60);
    }

    #[test]
    fn a_killed_serve_is_restarted_on_the_next_call_and_counted() {
        let mut s = Shared::new(limits(2_000, 0, 3));
        let start = || Ok(fake());
        assert_eq!(s.call(start, "knapsack-dp", &json!({})).unwrap()["kernel"]["id"], "knapsack-dp");
        let first = s.pid.unwrap();
        // Killed from outside, and reaped, before the next call.
        let child = &mut s.serve.as_mut().unwrap().child;
        child.kill().unwrap();
        child.wait().unwrap();
        let e = s.call(start, "knapsack-dp", &json!({})).unwrap_err();
        assert!(e.contains("the server ended") || e.contains("stdin is closed"), "{e}");
        assert!(s.gone.is_none(), "a failed serve waits for a restart, it is not dropped");
        let v = s.call(start, "track-planning", &json!({})).unwrap();
        assert_eq!(v["kernel"]["id"], "track-planning");
        assert_ne!(s.pid.unwrap(), first, "a new process answered");
        let v = s.summary(false);
        assert_eq!((v["calls"].as_u64(), v["restarts"].as_u64(), v["deadlines_missed"].as_u64()), (Some(2), Some(1), Some(0)));
        assert_eq!(v["runner"], "flow serve, one process for every call");
        assert_eq!(s.failures, 0, "an answer clears the failures in a row");
    }

    #[test]
    fn past_the_restart_cap_calls_fall_back_with_the_reason() {
        let mut s = Shared::new(limits(30, 0, 2));
        let mut started = 0;
        for _ in 0..3 {
            let e = s.call(|| { started += 1; hung() }, "k", &json!({})).unwrap_err();
            assert_eq!(e, "flow serve: no answer within its deadline of 30 ms");
        }
        assert_eq!(started, 3, "the first serve and two restarts");
        let why = s.gone.clone().expect("dropped past the cap");
        assert_eq!(why, "flow serve: failed 3 times in a row, the cap of 2 restarts; last: \
                         flow serve: no answer within its deadline of 30 ms");
        let t = Instant::now();
        assert_eq!(s.call(|| panic!("no start past the cap"), "k", &json!({})).unwrap_err(), why);
        assert!(t.elapsed() < MARGIN);
        let v = s.summary(true);
        assert_eq!((v["deadlines_missed"].as_u64(), v["restarts"].as_u64(), v["restart_cap"].as_u64()), (Some(3), Some(2), Some(2)));
        assert_eq!(v["gone"], json!(why));
    }

    #[test]
    fn a_serve_that_cannot_start_is_dropped_with_the_reason() {
        let mut s = Shared::new(limits(30, 0, 3));
        let dir = std::env::temp_dir().join("no-algoglyph-checkout-here");
        let e = s.call(|| Serve::start(&dir), "k", &json!({})).unwrap_err();
        assert!(e.starts_with("no AlgoGlyph checkout at"), "{e}");
        assert_eq!(s.gone.as_deref(), Some(e.as_str()));
        assert_eq!(s.summary(false)["runner"], "npm run flow -- kernel, one process per call");
        assert_eq!(s.summary(true)["runner"], "flow serve asked for (MCP_ALGOGLYPH_SERVE), not started");
    }

    #[test]
    fn the_backoff_doubles_up_to_its_maximum() {
        let l = Limits { deadline: DEADLINE, backoff: Duration::from_millis(10), backoff_max: Duration::from_millis(50), cap: 3 };
        let waits: Vec<u128> = (1..=5).map(|n| l.backoff_after(n).as_millis()).collect();
        assert_eq!(waits, vec![10, 20, 40, 50, 50]);
        assert_eq!(LIMITS.backoff_after(40), BACKOFF_MAX, "no overflow on a long run of failures");
    }

    #[test]
    fn the_process_serve_reports_its_deadline_and_counters() {
        // The process's serve: a checkout that is not there refuses before any process starts, and
        // the reason is forgotten again at once.
        let _ = kernel(&std::env::temp_dir().join("no-algoglyph-checkout-here"), "k", &json!({}));
        let _ = retry();
        let v = summary();
        assert_eq!(v["deadline_ms"], DEADLINE.as_millis() as u64);
        assert_eq!(v["restart_cap"], RESTART_CAP);
        for k in ["calls", "fallbacks", "deadlines_missed", "restarts"] { assert!(v[k].is_u64(), "{k}: {v}"); }
    }

    #[test]
    fn the_switch_is_off_unless_asked() {
        // Read only: the variable is not set by the test runner.
        if std::env::var_os("MCP_ALGOGLYPH_SERVE").is_none() { assert!(!wanted()); }
    }
}
