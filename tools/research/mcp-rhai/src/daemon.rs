//! RESEARCH-ONLY. What a long-running research daemon (`mcp-bus-blackboard`, `mcp-hub serve`) needs
//! so that it never outlives what it serves: a pidfile that refuses a second copy and reclaims a dead
//! one's, a watch that names the first thing gone (the parent it was started by, a path it reads), an
//! idle clock, and a bounded retry with backoff for an upstream that stopped answering.
//!
//! Found 2026-11-22: an `mcp-bus-blackboard` ran 13.5 h under launchd's pid 1, re-trying a
//! Majordomus server that had exited, and an `mcp-hub serve` ran 7 h after the worktree it ran in
//! and the panel it relayed were gone. Both now stop on their own (exit status 4).

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// Whether a process with this pid exists (signal 0 only checks).
pub fn pid_alive(pid: i64) -> bool {
    // SAFETY: kill with signal 0 sends nothing; it only checks that the process exists.
    pid > 0 && pid <= i64::from(i32::MAX) && unsafe { libc::kill(pid as libc::pid_t, 0) } == 0
}

/// This process's parent now.
pub fn parent_pid() -> i64 {
    // SAFETY: getppid has no preconditions.
    i64::from(unsafe { libc::getppid() })
}

/// A pidfile this process holds: written on claim, removed on drop while it still names this process.
#[derive(Debug)]
pub struct Pidfile {
    path: PathBuf,
    pid: u32,
}

impl Pidfile {
    /// Write this process's pid to `path`. Refused while the pid already there is another live
    /// process; a dead one's file (a crash, a kill -9, a reboot) is taken over.
    pub fn claim(path: &Path) -> Result<Pidfile, String> {
        let pid = std::process::id();
        if let Ok(text) = std::fs::read_to_string(path) {
            if let Ok(other) = text.trim().parse::<i64>() {
                if other != i64::from(pid) && pid_alive(other) {
                    return Err(format!("{}: already running as pid {other}", path.display()));
                }
            }
        }
        if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) { std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?; }
        std::fs::write(path, format!("{pid}\n")).map_err(|e| format!("{}: {e}", path.display()))?;
        Ok(Pidfile { path: path.to_path_buf(), pid })
    }
}

impl Drop for Pidfile {
    fn drop(&mut self) {
        let ours = std::fs::read_to_string(&self.path).ok().and_then(|t| t.trim().parse::<u32>().ok()) == Some(self.pid);
        if ours {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

/// What a daemon watches: the parent it must not outlive, and the paths it must not outlive.
#[derive(Debug, Default)]
pub struct Watch {
    /// The parent at start, when the daemon exits with it (`--exit-with-parent`).
    pub parent: Option<i64>,
    /// (what, path): each must keep existing (the working directory, an explicit registry).
    pub paths: Vec<(String, PathBuf)>,
    /// The working directory could not be read at start: it was deleted before the daemon looked.
    pub cwd_lost: bool,
}

impl Watch {
    /// Watch the parent this process has now.
    pub fn with_parent(mut self) -> Self {
        self.parent = Some(parent_pid());
        self
    }

    /// Watch one path, named `what` in the reason.
    pub fn path(mut self, what: &str, path: PathBuf) -> Self {
        self.paths.push((what.to_string(), path));
        self
    }

    /// The working directory (a deleted worktree is the case to catch); one that cannot be read
    /// now is gone already.
    pub fn cwd(self) -> Self {
        match std::env::current_dir() {
            Ok(d) => self.path("working directory", d),
            Err(_) => Watch { cwd_lost: true, ..self },
        }
    }

    /// The first thing gone, as a reason; None while everything is there. The parent is gone when
    /// this process was re-parented (to launchd or init), whatever pid it got, or when it already
    /// had been at start (pid 1: the shell that started it exited first).
    pub fn gone(&self) -> Option<String> {
        self.gone_with(parent_pid())
    }

    fn gone_with(&self, parent_now: i64) -> Option<String> {
        if self.cwd_lost {
            return Some("its working directory is gone".to_string());
        }
        if let Some(p) = self.parent.filter(|p| *p != parent_now || *p == 1) {
            return Some(format!("its parent (pid {p}) is gone"));
        }
        self.paths.iter().find(|(_, p)| !p.exists()).map(|(what, p)| format!("its {what} {} is gone", p.display()))
    }
}

/// How long something has had nothing to do; expired past its limit.
#[derive(Debug)]
pub struct Idle {
    limit: Option<Duration>,
    since: Option<Instant>,
}

impl Idle {
    /// An idle clock with this limit (None or zero: never expires).
    pub fn new(limit: Option<Duration>) -> Self {
        Idle { limit: limit.filter(|d| !d.is_zero()), since: None }
    }

    /// Record whether there is work at `now`; true when idle past the limit.
    pub fn tick(&mut self, busy: bool, now: Instant) -> bool {
        if busy {
            self.since = None;
            return false;
        }
        let since = *self.since.get_or_insert(now);
        self.limit.map_or(false, |l| now.duration_since(since) >= l)
    }
}

/// A bounded retry: after each failure the next attempt waits twice as long (from `base` up to
/// `cap`), and the failures since the last success are counted from the first of them.
#[derive(Debug)]
pub struct Backoff {
    base: Duration,
    cap: Duration,
    wait: Duration,
    next_at: Option<Instant>,
    first_failure: Option<Instant>,
    failures: u32,
}

impl Backoff {
    pub fn new(base: Duration, cap: Duration) -> Self {
        Backoff { base, cap, wait: base, next_at: None, first_failure: None, failures: 0 }
    }

    /// Whether an attempt may go out at `now`.
    pub fn ready(&self, now: Instant) -> bool {
        self.next_at.map_or(true, |t| now >= t)
    }

    /// One attempt failed at `now`: the next one waits longer.
    pub fn failed(&mut self, now: Instant) {
        self.first_failure.get_or_insert(now);
        self.failures = self.failures.saturating_add(1);
        self.next_at = Some(now + self.wait);
        self.wait = (self.wait * 2).min(self.cap);
    }

    /// One attempt succeeded: the next failure starts from `base` again.
    pub fn succeeded(&mut self) {
        *self = Backoff::new(self.base, self.cap);
    }

    /// How long the upstream has failed without a success in between, at `now`.
    pub fn down_for(&self, now: Instant) -> Option<Duration> {
        self.first_failure.map(|t| now.duration_since(t))
    }

    pub fn failures(&self) -> u32 {
        self.failures
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("mcp-rhai-daemon-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// A pid that is certainly dead: a child that has exited and been reaped.
    fn dead_pid() -> u32 {
        let mut c = std::process::Command::new("true").spawn().unwrap();
        let pid = c.id();
        c.wait().unwrap();
        pid
    }

    #[test]
    fn a_pidfile_refuses_a_live_owner_and_reclaims_a_dead_one() {
        let d = tmp("pidfile");
        let path = d.join("nested/hub.pid");
        // a dead owner's file is taken over
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, format!("{}\n", dead_pid())).unwrap();
        let held = Pidfile::claim(&path).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap().trim(), std::process::id().to_string());
        // claiming again from this process is not a second copy
        drop(Pidfile::claim(&path).unwrap());
        // a live other owner (our parent, the test harness) is refused
        std::fs::write(&path, format!("{}\n", parent_pid())).unwrap();
        let e = Pidfile::claim(&path).unwrap_err();
        assert!(e.contains("already running as pid"), "{e}");
        // dropping a pidfile someone else now names leaves it alone
        drop(held);
        assert!(path.exists());
        // a file that names no pid is taken over, and dropped with its holder
        std::fs::write(&path, "garbage").unwrap();
        drop(Pidfile::claim(&path).unwrap());
        assert!(!path.exists());
        assert!(Pidfile::claim(Path::new("/dev/null/x.pid")).is_err(), "a directory that cannot exist");
        assert!(Pidfile::claim(&d).is_err(), "a directory is not a pidfile");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_watch_names_the_first_thing_gone() {
        let d = tmp("watch");
        let w = Watch::default().cwd().path("registry", d.clone());
        assert_eq!(w.gone(), None);
        std::fs::remove_dir_all(&d).unwrap();
        assert_eq!(w.gone(), Some(format!("its registry {} is gone", d.display())));
        let p = Watch::default().with_parent();
        assert_eq!(p.gone(), None, "the parent that started the test is alive");
        assert_eq!(p.gone_with(1), Some(format!("its parent (pid {}) is gone", parent_pid())), "re-parented to launchd");
        let late = Watch { parent: Some(1), ..Default::default() };
        assert_eq!(late.gone_with(1), Some("its parent (pid 1) is gone".to_string()), "orphaned before it looked");
        assert!(pid_alive(i64::from(std::process::id())));
        assert!(!pid_alive(i64::from(dead_pid())));
        assert!(!pid_alive(0) && !pid_alive(-1) && !pid_alive(i64::MAX));
    }

    #[test]
    fn idle_expires_only_past_its_limit_and_work_resets_it() {
        let t0 = Instant::now();
        let mut i = Idle::new(Some(Duration::from_secs(10)));
        assert!(!i.tick(false, t0));
        assert!(!i.tick(false, t0 + Duration::from_secs(9)));
        assert!(i.tick(false, t0 + Duration::from_secs(10)));
        assert!(!i.tick(true, t0 + Duration::from_secs(11)));
        assert!(!i.tick(false, t0 + Duration::from_secs(12)), "the clock starts again after work");
        let mut never = Idle::new(Some(Duration::ZERO));
        assert!(!never.tick(false, t0) && !never.tick(false, t0 + Duration::from_secs(1_000_000)));
    }

    #[test]
    fn backoff_doubles_to_its_cap_and_a_success_resets_it() {
        let t0 = Instant::now();
        let mut b = Backoff::new(Duration::from_secs(1), Duration::from_secs(4));
        assert!(b.ready(t0));
        assert_eq!(b.down_for(t0), None);
        b.failed(t0);
        assert!(!b.ready(t0) && b.ready(t0 + Duration::from_secs(1)));
        b.failed(t0 + Duration::from_secs(1));
        assert!(!b.ready(t0 + Duration::from_secs(2)) && b.ready(t0 + Duration::from_secs(3)));
        b.failed(t0 + Duration::from_secs(3));
        b.failed(t0 + Duration::from_secs(7));
        assert!(b.ready(t0 + Duration::from_secs(11)), "capped at 4 s");
        assert_eq!(b.failures(), 4);
        assert_eq!(b.down_for(t0 + Duration::from_secs(11)), Some(Duration::from_secs(11)), "counted from the first failure");
        b.succeeded();
        assert!(b.ready(t0) && b.failures() == 0 && b.down_for(t0).is_none());
    }
}
