//! RESEARCH-ONLY. The research daemons never outlive what they serve (src/daemon.rs): the real
//! `mcp-bus-blackboard` and `mcp-hub serve` binaries, started as a person or a script starts them,
//! stop on their own when their Majordomus server, parent, working directory, registry or panel is
//! gone, and a pidfile refuses a second copy. Found 2026-11-22: a bus bridge that ran 13.5 h after
//! its server exited, and a hub that ran 7 h after its worktree and panel were deleted. The same for
//! the `mcp-rhai` helpers (director, narrator, steward, ask relay), found 2026-11-23 running 6.5 h
//! under pid 1 after their stack had stopped.

#[path = "../src/fake_mcp.rs"]
#[allow(dead_code)]
mod fake_mcp;

use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

const BUS: &str = env!("CARGO_BIN_EXE_mcp-bus-blackboard");
const HUB: &str = env!("CARGO_BIN_EXE_mcp-hub");
const RHAI: &str = env!("CARGO_BIN_EXE_mcp-rhai");

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("mcp-rhai-daemons-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// A command whose stderr goes to `log`, with no Majordomus server named from outside.
fn cmd(bin: &str, log: &Path) -> Command {
    let mut c = Command::new(bin);
    c.env_remove("MAJORDOMUS_URL").stdout(Stdio::null()).stderr(std::fs::File::create(log).unwrap());
    c
}

fn wait_exit(child: &mut Child, within: Duration) -> ExitStatus {
    let t = Instant::now();
    loop {
        if let Some(s) = child.try_wait().unwrap() { return s; }
        if t.elapsed() > within {
            let _ = child.kill();
            panic!("pid {} still running after {within:?}", child.id());
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn until(what: &str, within: Duration, mut ok: impl FnMut() -> bool) {
    let t = Instant::now();
    while !ok() {
        assert!(t.elapsed() < within, "{what}: not within {within:?}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn log_of(p: &Path) -> String {
    std::fs::read_to_string(p).unwrap_or_default()
}

fn alive(pid: i32) -> bool {
    // SAFETY: signal 0 only checks that the process exists.
    unsafe { libc::kill(pid, 0) == 0 }
}

/// Start `bin args` from a shell that exits at once, as `nohup ... &` from a script does, and return
/// the daemon's pid: its parent is gone the moment it starts.
fn orphan(bin: &str, args: &str, log: &Path, registry: &Path) -> i32 {
    let out = Command::new("sh").arg("-c").arg(format!("'{bin}' {args} >/dev/null 2>'{}' & echo $!", log.display()))
        .env("PRISMATIC_MCP_REGISTRY", registry).env_remove("MAJORDOMUS_URL").output().unwrap();
    String::from_utf8(out.stdout).unwrap().trim().parse().unwrap()
}

#[test]
fn the_bus_gives_up_on_a_server_that_stopped_and_its_pidfile_refuses_a_second_copy() {
    let d = tmp("bus-give-up");
    let (pid, log) = (d.join("bus.pid"), d.join("bus.log"));
    let fake = fake_mcp::FakeMcp::start();
    let args = ["--majordomus", &fake.url, "--endpoint", "tcp://127.0.0.1:1", "--give-up", "2", "--pidfile", pid.to_str().unwrap()];
    let mut bus = cmd(BUS, &log).args(args).env("MCP_BOARD_PING_MS", "100").spawn().unwrap();
    until("the bus announces itself", Duration::from_secs(20), || log_of(&log).contains("posting to"));
    until("its pidfile names it", Duration::from_secs(5), || std::fs::read_to_string(&pid).ok().map(|t| t.trim().to_string()) == Some(bus.id().to_string()));

    let log2 = d.join("second.log");
    let mut second = cmd(BUS, &log2).args(args).spawn().unwrap();
    assert_eq!(wait_exit(&mut second, Duration::from_secs(10)).code(), Some(5));
    assert!(log_of(&log2).contains("already running as pid"), "{}", log_of(&log2));

    fake.stop();
    assert_eq!(wait_exit(&mut bus, Duration::from_secs(30)).code(), Some(4), "{}", log_of(&log));
    assert!(log_of(&log).contains("has not answered for"), "{}", log_of(&log));
    assert!(!pid.exists(), "the pidfile goes with it");
}

#[test]
fn the_bus_looks_its_server_up_again_and_stops_cleanly_on_sigterm() {
    let d = tmp("bus-retarget");
    let (bin, url_file, log) = (d.join("bin"), d.join("url.json"), d.join("bus.log"));
    std::fs::create_dir_all(&bin).unwrap();
    // a `majordomus` that answers `serve ensure` with whatever the test writes
    let fake_mj = bin.join("majordomus");
    std::fs::write(&fake_mj, format!("#!/bin/sh\ncat '{}'\n", url_file.display())).unwrap();
    std::fs::set_permissions(&fake_mj, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    let (first, second) = (fake_mcp::FakeMcp::start(), fake_mcp::FakeMcp::start());
    let say = |url: &str| std::fs::write(&url_file, format!("{{\"url\": \"{url}\"}}")).unwrap();
    say(&first.url);
    let first_url = first.url.clone();
    let path = format!("{}:{}", bin.display(), std::env::var("PATH").unwrap_or_default());
    let mut bus = cmd(BUS, &log).args(["--endpoint", "tcp://127.0.0.1:1", "--give-up", "120"])
        .env("PATH", path).env("MCP_BOARD_PING_MS", "100").spawn().unwrap();
    until("the bus posts to the first server", Duration::from_secs(20), || log_of(&log).contains("posting to"));
    first.stop();
    std::fs::write(&url_file, "not json").unwrap();
    until("a lookup that fails is reported", Duration::from_secs(30), || log_of(&log).contains("majordomus serve ensure"));
    // the same dead server named again changes nothing; then a new one is named
    std::thread::sleep(Duration::from_millis(500));
    say(&format!("{}/", first_url));
    std::thread::sleep(Duration::from_secs(11));
    say(&second.url);
    until("the bus moves to the new server and says its claims there", Duration::from_secs(30),
          || second.bodies().iter().any(|b| b.contains("OpenTTD bus digest")));
    assert!(log_of(&log).contains("posting to "), "{}", log_of(&log));
    // SAFETY: SIGTERM to our own child.
    unsafe { libc::kill(bus.id() as i32, libc::SIGTERM) };
    assert_eq!(wait_exit(&mut bus, Duration::from_secs(20)).code(), Some(0), "{}", log_of(&log));
    assert!(second.bodies().iter().any(|b| b.contains("stopped at")), "its last word goes to the server it ended on");
}

#[test]
fn the_bus_stops_when_idle_orphaned_or_its_directory_is_deleted() {
    let d = tmp("bus-watch");
    let registry = d.join("registry");
    std::fs::create_dir_all(&registry).unwrap();

    let log = d.join("idle.log");
    let mut idle = cmd(BUS, &log).args(["--dry-run", "--idle-exit", "1"]).env("PRISMATIC_MCP_REGISTRY", &registry).spawn().unwrap();
    assert_eq!(wait_exit(&mut idle, Duration::from_secs(20)).code(), Some(0), "{}", log_of(&log));
    assert!(log_of(&log).contains("no panel to follow"), "{}", log_of(&log));

    let log = d.join("orphan.log");
    let pid = orphan(BUS, "--dry-run --exit-with-parent", &log, &registry);
    until("an orphaned bus stops", Duration::from_secs(20), || !alive(pid));
    assert!(log_of(&log).contains("its parent"), "{}", log_of(&log));

    let (gone, log) = (d.join("deleted-first"), d.join("deleted-first.log"));
    std::fs::create_dir_all(&gone).unwrap();
    let st = Command::new("sh").arg("-c").arg(format!("cd '{0}' && rm -rf '{0}' && exec '{BUS}' --dry-run 2>'{1}'", gone.display(), log.display()))
        .env("PRISMATIC_MCP_REGISTRY", &registry).env_remove("MAJORDOMUS_URL").status().unwrap();
    assert_eq!(st.code(), Some(4), "{}", log_of(&log));
    assert!(log_of(&log).contains("its working directory is gone"), "{}", log_of(&log));

    let (cwd, log) = (d.join("worktree"), d.join("cwd.log"));
    std::fs::create_dir_all(&cwd).unwrap();
    let mut bus = cmd(BUS, &log).args(["--dry-run"]).current_dir(&cwd).env("PRISMATIC_MCP_REGISTRY", &registry).spawn().unwrap();
    until("the bus runs", Duration::from_secs(20), || log_of(&log).contains("posting to"));
    std::fs::remove_dir_all(&cwd).unwrap();
    assert_eq!(wait_exit(&mut bus, Duration::from_secs(20)).code(), Some(4), "{}", log_of(&log));
    assert!(log_of(&log).contains("working directory"), "{}", log_of(&log));
}

#[test]
fn the_bus_unsubscribes_a_panel_whose_record_left_the_registry() {
    let d = tmp("bus-registry");
    let (registry, log) = (d.join("registry"), d.join("bus.log"));
    std::fs::create_dir_all(&registry).unwrap();
    let record = registry.join("panel-x.json");
    let me = std::process::id();
    std::fs::write(&record, format!(r#"{{"kind": "panel", "name": "panel-x", "owner_pid": {me}, "bus": "tcp://127.0.0.1:1"}}"#)).unwrap();
    std::fs::write(registry.join("dead.json"), r#"{"kind": "panel", "name": "panel-dead", "owner_pid": 2147483646, "bus": "tcp://127.0.0.1:2"}"#).unwrap();
    std::fs::write(registry.join("notes.txt"), "not a record").unwrap();
    let mut bus = cmd(BUS, &log).args(["--dry-run"]).env("PRISMATIC_MCP_REGISTRY", &registry)
        .env("MCP_BUS_RESCAN_MS", "200").spawn().unwrap();
    until("the bus follows the live panel", Duration::from_secs(20), || log_of(&log).contains("following panel-x"));
    assert!(!log_of(&log).contains("panel-dead"), "a record whose owner is dead is not followed");
    std::fs::remove_file(&record).unwrap();
    until("its subscription is dropped", Duration::from_secs(20), || log_of(&log).contains("tcp://127.0.0.1:1 left the registry; unsubscribed"));
    std::fs::write(&record, format!(r#"{{"kind": "panel", "name": "panel-x", "owner_pid": {me}, "bus": "tcp://127.0.0.1:3"}}"#)).unwrap();
    until("a panel that comes back is followed again", Duration::from_secs(20), || log_of(&log).contains("following panel-x at tcp://127.0.0.1:3"));
    // SAFETY: SIGTERM to our own child.
    unsafe { libc::kill(bus.id() as i32, libc::SIGTERM) };
    assert_eq!(wait_exit(&mut bus, Duration::from_secs(20)).code(), Some(0), "{}", log_of(&log));
}

#[test]
fn the_hub_stops_when_its_registry_its_panel_or_its_parent_is_gone() {
    let d = tmp("hub");
    let registry = d.join("registry");
    std::fs::create_dir_all(&registry).unwrap();
    let (pid, log) = (d.join("hub.pid"), d.join("hub.log"));
    let base = ["serve", "--port", "0", "--no-store", "--audit-every", "0", "--registry", registry.to_str().unwrap()];
    let mut hub = cmd(HUB, &log).args(base).args(["--pidfile", pid.to_str().unwrap()]).spawn().unwrap();
    until("the hub claims its pidfile", Duration::from_secs(20), || pid.exists());
    let log2 = d.join("second.log");
    let mut second = cmd(HUB, &log2).args(base).args(["--pidfile", pid.to_str().unwrap()]).spawn().unwrap();
    assert_eq!(wait_exit(&mut second, Duration::from_secs(10)).code(), Some(5), "{}", log_of(&log2));
    std::fs::remove_dir_all(&registry).unwrap();
    assert_eq!(wait_exit(&mut hub, Duration::from_secs(20)).code(), Some(4), "{}", log_of(&log));
    assert!(log_of(&log).contains("its registry"), "{}", log_of(&log));
    assert!(!pid.exists());

    std::fs::create_dir_all(&registry).unwrap();
    let dead = { let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap(); format!("http://{}", l.local_addr().unwrap()) };
    let log = d.join("panel.log");
    let mut hub = cmd(HUB, &log).args(base).args(["--panel", &dead, "--panel-deadline", "1"]).spawn().unwrap();
    assert_eq!(wait_exit(&mut hub, Duration::from_secs(30)).code(), Some(4), "{}", log_of(&log));
    assert!(log_of(&log).contains("no --panel"), "{}", log_of(&log));

    let log = d.join("orphan.log");
    let hub = orphan(HUB, &format!("serve --port 0 --no-store --audit-every 0 --registry '{}' --exit-with-parent", registry.display()), &log, &registry);
    until("an orphaned hub stops", Duration::from_secs(20), || !alive(hub));
    assert!(log_of(&log).contains("its parent"), "{}", log_of(&log));
}

/// A stand-in panel that answers every request with `{}` until `stop` is set, then refuses
/// connections (its listener is closed): a panel that stopped.
fn fake_panel() -> (String, std::sync::Arc<std::sync::atomic::AtomicBool>) {
    use std::io::{Read, Write};
    use std::sync::atomic::{AtomicBool, Ordering};
    let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", l.local_addr().unwrap());
    l.set_nonblocking(true).unwrap();
    let stop = std::sync::Arc::new(AtomicBool::new(false));
    let s = stop.clone();
    std::thread::spawn(move || {
        while !s.load(Ordering::SeqCst) {
            match l.accept() {
                Ok((mut c, _)) => {
                    let _ = c.set_nonblocking(false);
                    let _ = c.set_read_timeout(Some(Duration::from_secs(2)));
                    let mut buf = [0u8; 8192];
                    let _ = c.read(&mut buf);
                    let _ = c.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}");
                }
                Err(_) => std::thread::sleep(Duration::from_millis(20)),
            }
        }
    });
    (url, stop)
}

/// A script that never ends on its own: only its guard stops it.
const FOR_EVER: &str = "loop { sleep(200); }";

#[test]
fn a_rhai_helper_stops_when_its_panel_stops_and_its_pidfile_refuses_a_second_copy() {
    let d = tmp("rhai");
    let (url, stop) = fake_panel();
    let (pid, log) = (d.join("helper.pid"), d.join("helper.log"));
    let guard = ["--give-up", "1", "--pidfile", pid.to_str().unwrap()];
    let mut helper = cmd(RHAI, &log).args(["--panel", &url]).args(guard).args(["-e", FOR_EVER]).spawn().unwrap();
    until("the helper claims its pidfile", Duration::from_secs(20), || pid.exists());
    let log2 = d.join("second.log");
    let mut second = cmd(RHAI, &log2).args(["--panel", &url]).args(guard).args(["-e", FOR_EVER]).spawn().unwrap();
    assert_eq!(wait_exit(&mut second, Duration::from_secs(10)).code(), Some(5), "{}", log_of(&log2));
    assert!(log_of(&log2).contains("already running as pid"), "{}", log_of(&log2));
    std::thread::sleep(Duration::from_secs(2));
    assert!(helper.try_wait().unwrap().is_none(), "a panel that answers keeps the helper: {}", log_of(&log));
    stop.store(true, std::sync::atomic::Ordering::SeqCst);
    assert_eq!(wait_exit(&mut helper, Duration::from_secs(10)).code(), Some(4), "{}", log_of(&log));
    assert!(log_of(&log).contains("has not answered"), "{}", log_of(&log));
    assert!(!pid.exists(), "the pidfile goes with the helper");

    let bad = d.join("bad.log");
    let mut b = cmd(RHAI, &bad).args(["--give-up", "0", "-e", "1"]).spawn().unwrap();
    assert_eq!(wait_exit(&mut b, Duration::from_secs(10)).code(), Some(2), "{}", log_of(&bad));
    let fine = d.join("fine.log");
    let mut f = cmd(RHAI, &fine).args(["--panel", "http://127.0.0.1:1", "--pidfile", d.join("one.pid").to_str().unwrap(), "-e", "40 + 2"])
        .stdout(Stdio::piped()).spawn().unwrap();
    assert_eq!(wait_exit(&mut f, Duration::from_secs(20)).code(), Some(0), "{}", log_of(&fine));
    assert!(!d.join("one.pid").exists(), "a script that ends releases its pidfile");
    let err = d.join("err.log");
    let mut e = cmd(RHAI, &err).args(["--panel", "http://127.0.0.1:1", "--pidfile", d.join("two.pid").to_str().unwrap(), "-e", "throw \"boom\""]).spawn().unwrap();
    assert_eq!(wait_exit(&mut e, Duration::from_secs(20)).code(), Some(1), "{}", log_of(&err));
    assert!(log_of(&err).contains("boom") && !d.join("two.pid").exists(), "{}", log_of(&err));
}

#[test]
fn a_rhai_helper_stops_when_its_parent_is_gone() {
    let d = tmp("rhai-parent");
    let (url, _stop) = fake_panel();
    let log = d.join("orphan.log");
    let helper = orphan(RHAI, &format!("--panel {url} --exit-with-parent -e '{FOR_EVER}'"), &log, &d);
    until("an orphaned helper stops", Duration::from_secs(20), || !alive(helper));
    assert!(log_of(&log).contains("its parent"), "{}", log_of(&log));
}
