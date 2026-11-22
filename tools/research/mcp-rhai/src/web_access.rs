//! RESEARCH-ONLY. Who may use the web surface of the panel and the hub: one policy, one function
//! (ADR 0038). The panel's POST guard, its game WebSocket bridge, the hub's HTTP and the hub's
//! WebSocket all call [`judge`]; nothing else decides it. A second, narrower copy of the origin
//! check once refused every button of a panel opened as `localhost` (owner, 2026-11-22).
//!
//! - **Who:** a request from a loopback peer naming a loopback `Host` needs nothing. Any other
//!   (the LAN, the web, a proxy, a foreign `Host`, which is also how DNS rebinding arrives) needs
//!   the token: a Bearer header, the cookie that `?token=` sets, or `?token=` itself.
//! - **From where:** a request that changes something (POST and the other unsafe methods, and a
//!   WebSocket upgrade) is accepted without an `Origin`, from its own origin under whatever host
//!   name the page was opened by (the `Origin` authority equals `Host`), or from an origin listed
//!   with `--allow-origin`. A listed origin gets CORS: preflight, credentials, `Vary: Origin`.
//! - **Where it listens:** `--bind` (default 127.0.0.1). A non-loopback bind without `--token`
//!   (or `MCP_WEB_TOKEN`) gets a random token, printed at start.

use std::cell::RefCell;
use std::net::IpAddr;
use std::sync::OnceLock;

/// The policy one process serves under.
#[derive(Clone, Debug, PartialEq)]
pub struct Policy {
    pub bind: IpAddr,
    pub allow_origins: Vec<String>,
    pub token: Option<String>,
    /// The cookie that carries the token: one per listening port, since cookies ignore ports.
    pub cookie: String,
}

impl Default for Policy {
    fn default() -> Self {
        Policy { bind: IpAddr::from([127, 0, 0, 1]), allow_origins: vec![], token: None, cookie: "mcp_token".into() }
    }
}

impl Policy {
    /// Takes `--bind`, `--allow-origin` (repeatable) and `--token`: Ok(true) when `name` was one.
    pub fn flag(&mut self, name: &str, value: &str) -> Result<bool, String> {
        match name {
            "--bind" => self.bind = value.parse().map_err(|_| format!("argument --bind: not an IP address: '{value}'"))?,
            "--allow-origin" => self.allow_origins.push(origin_of(value).ok_or_else(|| format!("argument --allow-origin: not an origin (scheme://host[:port]): '{value}'"))?),
            "--token" => self.token = Some(value.to_string()).filter(|t| !t.is_empty()),
            _ => return Ok(false),
        }
        Ok(true)
    }

    /// The environment beside the flags (`MCP_WEB_TOKEN`, `MCP_WEB_ALLOW_ORIGIN` comma-separated),
    /// the cookie named for `port`, and a generated token for a bind that is not loopback.
    /// Returns the policy and, when a token was generated, a line to print.
    pub fn finish(mut self, port: u16) -> (Policy, Option<String>) {
        if self.token.is_none() {
            self.token = std::env::var("MCP_WEB_TOKEN").ok().filter(|t| !t.is_empty());
        }
        for o in std::env::var("MCP_WEB_ALLOW_ORIGIN").unwrap_or_default().split(',').filter_map(origin_of) {
            if !self.allow_origins.contains(&o) { self.allow_origins.push(o); }
        }
        self.cookie = format!("mcp_token_{port}");
        let mut note = None;
        if !self.bind.is_loopback() && self.token.is_none() {
            let t = random_token();
            note = Some(format!("listening on {} beyond loopback: open it once as http://<host>:{port}/?token={t}", self.bind));
            self.token = Some(t);
        }
        (self, note)
    }
}

static POLICY: OnceLock<Policy> = OnceLock::new();

/// Install the process's policy (once, at start).
pub fn install(p: Policy) {
    let _ = POLICY.set(p);
}

/// The installed policy, or the loopback default.
pub fn policy() -> Policy {
    POLICY.get().cloned().unwrap_or_default()
}

/// What to do with a request.
#[derive(Debug, PartialEq)]
pub enum Verdict {
    /// Serve it; `headers` go on its answer (CORS, the token cookie).
    Pass { headers: String },
    /// A CORS preflight of a listed origin: answer 204 with `headers`.
    Preflight { headers: String },
    /// Refuse with this status and reason.
    Refuse(u16, String),
}

/// One request as the policy sees it.
pub struct Seen<'a> {
    pub method: &'a str,
    pub header: &'a dyn Fn(&str) -> Option<String>,
    /// The peer's address, None when unknown (then it counts as remote).
    pub peer: Option<IpAddr>,
    /// `?token=` of the request target.
    pub query_token: Option<&'a str>,
}

/// The verdict on one request under `p`.
pub fn judge(p: &Policy, r: &Seen) -> Verdict {
    let host = (r.header)("Host");
    // "null" (a sandboxed frame, a file:// page) is an Origin like any other: never its own, never listed.
    let origin = (r.header)("Origin");
    let local = r.peer.is_some_and(|a| a.is_loopback()) && host.as_deref().map_or(true, host_is_loopback);
    let presented = bearer(r.header).or_else(|| cookie(r.header, &p.cookie)).or(r.query_token.map(str::to_string));
    let token_ok = matches!((&p.token, &presented), (Some(t), Some(g)) if same(t, g));
    if !local && !token_ok {
        return Verdict::Refuse(401, format!(
            "this address needs the token: open the page once with ?token=<token> (printed when the server started), or send Authorization: Bearer <token>{}",
            if p.token.is_none() { "; this server has none, so start it with --token or MCP_WEB_TOKEN" } else { "" }));
    }
    let listed = origin.as_deref().is_some_and(|o| p.allow_origins.iter().any(|a| a == o));
    let mut headers = String::new();
    if listed {
        let o = origin.as_deref().unwrap_or("");
        headers.push_str(&format!("Access-Control-Allow-Origin: {o}\r\nAccess-Control-Allow-Credentials: true\r\nVary: Origin\r\n"));
    }
    if let (Some(t), Some(q)) = (&p.token, r.query_token) {
        if same(t, q) {
            let secure = (r.header)("X-Forwarded-Proto").is_some_and(|v| v.eq_ignore_ascii_case("https"));
            headers.push_str(&format!("Set-Cookie: {}={t}; HttpOnly; SameSite=Lax; Path=/{}\r\n", p.cookie, if secure { "; Secure" } else { "" }));
        }
    }
    if r.method.eq_ignore_ascii_case("OPTIONS") {
        return if listed {
            headers.push_str("Access-Control-Allow-Methods: GET, POST, OPTIONS\r\nAccess-Control-Allow-Headers: Content-Type, Authorization\r\nAccess-Control-Max-Age: 600\r\n");
            Verdict::Preflight { headers }
        } else {
            Verdict::Refuse(403, foreign(origin.as_deref()))
        };
    }
    let upgrade = (r.header)("Upgrade").is_some_and(|v| v.eq_ignore_ascii_case("websocket"));
    let unsafe_method = !matches!(r.method.to_ascii_uppercase().as_str(), "GET" | "HEAD");
    if (unsafe_method || upgrade) && origin.is_some() && !listed && !same_origin(origin.as_deref(), host.as_deref()) {
        return Verdict::Refuse(403, foreign(origin.as_deref()));
    }
    Verdict::Pass { headers }
}

fn foreign(origin: Option<&str>) -> String {
    let o = origin.unwrap_or("?");
    format!("foreign origin {o}: open the page from this server's own address, or start the server with --allow-origin {o}")
}

/// `scheme://authority` of an origin or URL, lower-cased, without a path; None when it is none.
pub fn origin_of(s: &str) -> Option<String> {
    let s = s.trim().trim_end_matches('/');
    let (scheme, rest) = s.split_once("://")?;
    let authority = rest.split('/').next()?;
    (matches!(scheme, "http" | "https") && !authority.is_empty()).then(|| format!("{scheme}://{}", authority.to_ascii_lowercase()))
}

/// Whether `origin` is the page's own: its authority is the request's `Host`, or both name this
/// machine's loopback on the same port (a page from 127.0.0.1 calling localhost).
fn same_origin(origin: Option<&str>, host: Option<&str>) -> bool {
    let port = |a: &str| a.rsplit_once(':').filter(|(_, p)| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit())).map(|(_, p)| p.to_string());
    match (origin.and_then(origin_of), host) {
        (Some(o), Some(h)) => o.split_once("://").is_some_and(|(_, a)| {
            a == h.to_ascii_lowercase() || (host_is_loopback(a) && host_is_loopback(h) && port(a).is_some() && port(a) == port(h))
        }),
        _ => false,
    }
}

/// Whether a `Host` header names this machine's loopback.
fn host_is_loopback(host: &str) -> bool {
    let name = if let Some(rest) = host.strip_prefix('[') { rest.split(']').next().unwrap_or("") } else { host.rsplit_once(':').map_or(host, |(h, _)| h) };
    name.eq_ignore_ascii_case("localhost") || name.parse::<IpAddr>().is_ok_and(|a| a.is_loopback())
}

fn bearer(header: &dyn Fn(&str) -> Option<String>) -> Option<String> {
    header("Authorization").and_then(|v| v.strip_prefix("Bearer ").map(|t| t.trim().to_string()))
}

fn cookie(header: &dyn Fn(&str) -> Option<String>, name: &str) -> Option<String> {
    header("Cookie")?.split(';').find_map(|c| c.trim().strip_prefix(name)?.strip_prefix('=').map(str::to_string))
}

/// Equal without an early exit on the first differing byte.
pub fn same(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn random_token() -> String {
    let mut b = [0u8; 16];
    if let Ok(mut f) = std::fs::File::open("/dev/urandom") {
        let _ = std::io::Read::read_exact(&mut f, &mut b);
    }
    b.iter().map(|x| format!("{x:02x}")).collect()
}

thread_local! {
    /// The headers [`judge`] asked for on this connection's answer (one thread per connection).
    static HEADERS: RefCell<String> = const { RefCell::new(String::new()) };
}

/// Remember the headers this connection's answer carries; `hub::respond` and the event stream
/// head append them.
pub fn set_headers(h: &str) {
    HEADERS.with(|c| *c.borrow_mut() = h.to_string());
}

/// The headers of this connection's answer ("" when none).
pub fn headers() -> String {
    HEADERS.with(|c| c.borrow().clone())
}

/// The verdict on an HTTP request already parsed into headers, installed for its answer: Ok(())
/// to serve it, Err((status, extra headers, body)) to answer at once (a preflight or a refusal).
pub fn gate(method: &str, headers: &[(String, String)], peer: Option<IpAddr>, query_token: Option<&str>) -> Result<(), (u16, String)> {
    let header = |n: &str| headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(n)).map(|(_, v)| v.clone());
    match judge(&policy(), &Seen { method, header: &header, peer, query_token }) {
        Verdict::Pass { headers } => { set_headers(&headers); Ok(()) }
        Verdict::Preflight { headers } => { set_headers(&headers); Err((204, String::new())) }
        Verdict::Refuse(code, why) => { set_headers(""); Err((code, why)) }
    }
}

/// The same verdict on a WebSocket handshake (tungstenite's callback): Err refuses it with the
/// verdict's status and reason.
pub fn ws_gate(req: &tungstenite::handshake::server::Request, peer: Option<IpAddr>) -> Result<(), tungstenite::handshake::server::ErrorResponse> {
    let headers: Vec<(String, String)> = req.headers().iter()
        .filter_map(|(k, v)| v.to_str().ok().map(|v| (k.as_str().to_string(), v.to_string()))).collect();
    let token = req.uri().query().and_then(|q| q.split('&').find_map(|kv| kv.strip_prefix("token=")).map(str::to_string));
    match gate("GET", &headers, peer, token.as_deref()) {
        Ok(()) | Err((204, _)) => Ok(()),
        Err((code, why)) => {
            let mut e = tungstenite::handshake::server::ErrorResponse::new(Some(why));
            *e.status_mut() = tungstenite::http::StatusCode::from_u16(code).unwrap_or(tungstenite::http::StatusCode::FORBIDDEN);
            Err(e)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seen(p: &Policy, method: &str, peer: &str, hs: &[(&str, &str)], q: Option<&str>) -> Verdict {
        let owned: Vec<(String, String)> = hs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        let header = |n: &str| owned.iter().find(|(k, _)| k.eq_ignore_ascii_case(n)).map(|(_, v)| v.clone());
        judge(p, &Seen { method, header: &header, peer: peer.parse().ok(), query_token: q })
    }
    fn pass(v: &Verdict) -> bool { matches!(v, Verdict::Pass { .. }) }

    #[test]
    fn a_page_posts_to_itself_under_any_loopback_name() {
        let p = Policy::default();
        for (host, origin) in [("127.0.0.1:8800", "http://127.0.0.1:8800"), ("localhost:8800", "http://localhost:8800"), ("[::1]:8800", "http://[::1]:8800")] {
            assert!(pass(&seen(&p, "POST", "127.0.0.1", &[("Host", host), ("Origin", origin)], None)), "{host}");
        }
        // A non-browser client on loopback sends no Origin.
        assert!(pass(&seen(&p, "POST", "127.0.0.1", &[("Host", "127.0.0.1:8800")], None)));
    }

    #[test]
    fn a_foreign_page_cannot_act_but_can_be_listed() {
        let mut p = Policy::default();
        let evil = [("Host", "127.0.0.1:8800"), ("Origin", "https://evil.example")];
        assert_eq!(seen(&p, "POST", "127.0.0.1", &evil, None).clone_code(), Some(403));
        assert_eq!(seen(&p, "GET", "127.0.0.1", &[("Host", "127.0.0.1:8800"), ("Origin", "https://evil.example"), ("Upgrade", "websocket")], None).clone_code(), Some(403));
        assert!(pass(&seen(&p, "GET", "127.0.0.1", &evil, None)), "a read gets no CORS header, so the browser keeps it from the page");
        p.flag("--allow-origin", "https://dash.example/").unwrap();
        let ok = [("Host", "127.0.0.1:8800"), ("Origin", "https://dash.example")];
        match seen(&p, "POST", "127.0.0.1", &ok, None) {
            Verdict::Pass { headers } => assert!(headers.contains("Access-Control-Allow-Origin: https://dash.example") && headers.contains("Vary: Origin")),
            v => panic!("{v:?}"),
        }
        assert!(matches!(seen(&p, "OPTIONS", "127.0.0.1", &ok, None), Verdict::Preflight { .. }));
        assert_eq!(seen(&p, "OPTIONS", "127.0.0.1", &evil, None).clone_code(), Some(403));
    }

    #[test]
    fn beyond_loopback_the_token_is_needed_and_dns_rebinding_is_refused() {
        let mut p = Policy::default();
        p.flag("--token", "s3cret").unwrap();
        let lan = [("Host", "192.168.1.5:8800"), ("Origin", "http://192.168.1.5:8800")];
        assert_eq!(seen(&p, "POST", "192.168.1.9", &lan, None).clone_code(), Some(401));
        assert!(pass(&seen(&p, "POST", "192.168.1.9", &[lan[0], lan[1], ("Authorization", "Bearer s3cret")], None)));
        assert!(pass(&seen(&p, "POST", "192.168.1.9", &[lan[0], lan[1], ("Cookie", "a=b; mcp_token=s3cret")], None)));
        match seen(&p, "GET", "192.168.1.9", &[lan[0]], Some("s3cret")) {
            Verdict::Pass { headers } => assert!(headers.contains("Set-Cookie: mcp_token=s3cret; HttpOnly")),
            v => panic!("{v:?}"),
        }
        assert_eq!(seen(&p, "GET", "192.168.1.9", &[lan[0]], Some("wrong")).clone_code(), Some(401));
        // A rebinding page reaches loopback under its own host name: remote, so it needs the token.
        assert_eq!(seen(&p, "GET", "127.0.0.1", &[("Host", "rebind.evil.example:8800")], None).clone_code(), Some(401));
        // A proxy on this machine forwards the public host name: remote too.
        assert_eq!(seen(&p, "GET", "127.0.0.1", &[("Host", "panel.example.org")], None).clone_code(), Some(401));
    }

    #[test]
    fn a_wide_bind_without_a_token_gets_one() {
        let mut p = Policy::default();
        p.flag("--bind", "0.0.0.0").unwrap();
        std::env::remove_var("MCP_WEB_TOKEN");
        let (p, note) = p.finish(8800);
        assert_eq!(p.token.as_ref().map(String::len), Some(32));
        assert!(note.unwrap().contains("?token="));
        assert_eq!(p.cookie, "mcp_token_8800");
        assert!(Policy::default().flag("--bind", "nope").is_err());
        assert!(Policy::default().flag("--allow-origin", "dash.example").is_err());
    }

    #[test]
    fn origins_and_hosts_parse_strictly() {
        assert_eq!(origin_of("HTTP://x"), None);
        assert_eq!(origin_of("https://Dash.Example:8443/x").as_deref(), Some("https://dash.example:8443"));
        assert!(host_is_loopback("localhost:1") && host_is_loopback("[::1]:8800") && host_is_loopback("127.0.0.1"));
        assert!(!host_is_loopback("localhost.evil.example:8800") && !host_is_loopback("10.0.0.1:8800"));
        assert!(!same_origin(Some("http://127.0.0.1:8800.evil.example"), Some("127.0.0.1:8800")));
        assert!(same_origin(Some("http://127.0.0.1:8800"), Some("localhost:8800")), "loopback names, one port");
        assert!(!same_origin(Some("http://127.0.0.1:9999"), Some("localhost:8800")), "another port is another origin");
        assert!(!same_origin(Some("http://localhost"), Some("localhost:8800")));
    }

    impl Verdict {
        fn clone_code(&self) -> Option<u16> { if let Verdict::Refuse(c, _) = self { Some(*c) } else { None } }
    }
}
