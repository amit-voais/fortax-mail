//! The hiSAI bridge — Fortax Mail's local, opt-in interface for the hiSAI desktop agent.
//!
//! Three rules shape it:
//!
//!  1. **It never leaves the machine.** The listener binds 127.0.0.1 on a port the operating system
//!     picks, and nothing is published anywhere else.
//!  2. **It is off until asked for.** No listener exists unless the user switches the bridge on
//!     (or `FORTAX_MAIL_BRIDGE=1` for a developer run), and the token file is owner-only.
//!  3. **It reads; it does not send.** Search, read, contacts and calendar are here. Sending is not:
//!     a person presses Send in the app. Drafts are the seam between the two.
//!
//! The token lives in `<data dir>/bridge.json` — the same directory the agent can read as the user,
//! and no one else can.

use std::io::Write as _;
use std::net::Ipv4Addr;
use std::path::{Path, PathBuf};

use rand::RngCore;
use rusqlite::Connection;
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use crate::db::repo::{accounts, calendar as cal_repo, contacts, search as search_repo};
use crate::db::Db;
use crate::error::Result;
use crate::search;

/// What a running bridge tells the rest of the app about itself.
#[derive(Debug, Clone)]
pub struct BridgeHandle {
    pub port: u16,
    pub token: String,
    pub state_file: PathBuf,
}

/// True when the user (or a developer run) asked for the bridge.
pub fn enabled(conn: &Connection) -> bool {
    if matches!(std::env::var("FORTAX_MAIL_BRIDGE").as_deref(), Ok("1") | Ok("true")) {
        return true;
    }
    conn.query_row(
        "SELECT value FROM app_settings WHERE key = 'bridge_enabled'",
        [],
        |r| r.get::<_, String>(0),
    )
    .map(|v| v == "1" || v == "true")
    .unwrap_or(false)
}

/// Switch the bridge on or off in the app's own settings. The listener itself starts with the app.
pub fn set_enabled(db_path: &Path, on: bool) -> Result<()> {
    let conn = Connection::open(db_path)
        .map_err(|e| crate::error::CoreError::Other(format!("opening {}: {e}", db_path.display())))?;
    conn.execute(
        "INSERT INTO app_settings (key, value) VALUES ('bridge_enabled', ?1)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        [if on { "1" } else { "0" }],
    )
    .map_err(|e| crate::error::CoreError::Other(format!("writing the setting: {e}")))?;
    Ok(())
}

/// Whether the bridge is switched on, read straight from the settings table.
pub fn is_enabled(db_path: &Path) -> Result<bool> {
    let conn = Connection::open(db_path)
        .map_err(|e| crate::error::CoreError::Other(format!("opening {}: {e}", db_path.display())))?;
    Ok(enabled(&conn))
}

fn token() -> String {
    let mut bytes = [0u8; 24];
    rand::rng().fill_bytes(&mut bytes);
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Same-length comparison, so a wrong token cannot be found one character at a time.
fn same_token(a: &str, b: &str) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.bytes().zip(b.bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn write_state(path: &Path, port: u16, token: &str) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let body = json!({
        "port": port,
        "token": token,
        "url": format!("http://127.0.0.1:{port}"),
        "api": "v1",
        "app": "Fortax Mail",
        "started_at": chrono::Utc::now().to_rfc3339(),
    });
    let mut f = std::fs::File::create(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        f.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    }
    f.write_all(serde_json::to_string_pretty(&body).unwrap_or_default().as_bytes())?;
    Ok(())
}

pub fn state_path(data_dir: &Path) -> PathBuf {
    data_dir.join("bridge.json")
}

/// Stop advertising a bridge that is no longer listening.
pub fn clear_state(data_dir: &Path) {
    let _ = std::fs::remove_file(state_path(data_dir));
}

/// Start the bridge. The caller decides whether it should run at all (see [`enabled`]).
pub async fn start(db: Db, calendar_db: Option<Db>, data_dir: PathBuf) -> Result<BridgeHandle> {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .await
        .map_err(|e| crate::error::CoreError::Other(format!("the hiSAI bridge could not listen: {e}")))?;
    let port = listener
        .local_addr()
        .map_err(|e| crate::error::CoreError::Other(format!("the hiSAI bridge has no address: {e}")))?
        .port();
    let token = token();
    let file = state_path(&data_dir);
    if let Err(e) = write_state(&file, port, &token) {
        tracing::warn!("hiSAI bridge: could not write {}: {e}", file.display());
    }
    let handle = BridgeHandle { port, token: token.clone(), state_file: file };
    tokio::spawn(async move {
        loop {
            match listener.accept().await {
                Ok((stream, _)) => {
                    let db = db.clone();
                    let cal = calendar_db.clone();
                    let token = token.clone();
                    tokio::spawn(async move {
                        if let Err(e) = serve(stream, db, cal, token).await {
                            tracing::debug!("hiSAI bridge request ended: {e}");
                        }
                    });
                }
                Err(e) => {
                    tracing::warn!("hiSAI bridge stopped accepting: {e}");
                    return;
                }
            }
        }
    });
    tracing::info!("hiSAI bridge listening on 127.0.0.1:{port}");
    Ok(handle)
}

async fn serve(mut stream: TcpStream, db: Db, calendar_db: Option<Db>, token: String) -> std::io::Result<()> {
    let mut buf = Vec::with_capacity(2048);
    let mut chunk = [0u8; 2048];
    let head_end = loop {
        let n = stream.read(&mut chunk).await?;
        if n == 0 {
            return Ok(());
        }
        buf.extend_from_slice(&chunk[..n]);
        if let Some(i) = find(&buf, b"\r\n\r\n") {
            break i + 4;
        }
        if buf.len() > 64 * 1024 {
            return reply(&mut stream, 431, &json!({"error": "request header too large"})).await;
        }
    };
    let head = String::from_utf8_lossy(&buf[..head_end]).to_string();
    let mut lines = head.lines();
    let request_line = lines.next().unwrap_or_default().to_string();
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or("").to_string();
    let target = parts.next().unwrap_or("/").to_string();

    let mut auth = String::new();
    for line in lines {
        if let Some(v) = line.strip_prefix("Authorization:").or_else(|| line.strip_prefix("authorization:")) {
            auth = v.trim().to_string();
        }
    }
    let given = auth.strip_prefix("Bearer ").unwrap_or("").trim();
    if !same_token(given, &token) {
        return reply(&mut stream, 401, &json!({"error": "bad or missing token"})).await;
    }
    if method != "GET" {
        return reply(&mut stream, 405, &json!({"error": "this bridge answers GET only"})).await;
    }

    let (path, query) = match target.split_once('?') {
        Some((p, q)) => (p.to_string(), q.to_string()),
        None => (target.clone(), String::new()),
    };
    let q = Query::parse(&query);

    let out = match path.as_str() {
        "/v1/ping" => Ok(json!({"ok": true, "app": "Fortax Mail", "api": "v1"})),
        "/v1/accounts" => list_accounts(&db).await,
        "/v1/search" => search_threads(&db, &q).await,
        "/v1/thread" => thread_messages(&db, &q).await,
        "/v1/message" => message(&db, &q).await,
        "/v1/contacts" => contact_list(&db, &q).await,
        "/v1/calendar" => events(calendar_db.as_ref().unwrap_or(&db), &q).await,
        _ => return reply(&mut stream, 404, &json!({"error": "no such endpoint"})).await,
    };
    match out {
        Ok(v) => reply(&mut stream, 200, &v).await,
        Err(e) => reply(&mut stream, 500, &json!({"error": e.to_string()})).await,
    }
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

async fn reply(stream: &mut TcpStream, code: u16, body: &Value) -> std::io::Result<()> {
    let text = serde_json::to_string(body).unwrap_or_else(|_| "{}".into());
    let head = format!(
        "HTTP/1.1 {code} {}\r\nContent-Type: application/json; charset=utf-8\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n",
        match code { 200 => "OK", 401 => "Unauthorized", 404 => "Not Found", 405 => "Method Not Allowed", 431 => "Request Header Fields Too Large", _ => "Internal Server Error" },
        text.as_bytes().len()
    );
    stream.write_all(head.as_bytes()).await?;
    stream.write_all(text.as_bytes()).await?;
    stream.flush().await
}

/// The query string, decoded.
struct Query(Vec<(String, String)>);

impl Query {
    fn parse(raw: &str) -> Self {
        let mut out = Vec::new();
        for pair in raw.split('&').filter(|p| !p.is_empty()) {
            let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
            out.push((decode(k), decode(v)));
        }
        Self(out)
    }
    fn get(&self, key: &str) -> Option<&str> {
        self.0.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
    }
    fn num(&self, key: &str, default: i64, max: i64) -> i64 {
        self.get(key).and_then(|v| v.parse::<i64>().ok()).unwrap_or(default).clamp(1, max)
    }
}

fn decode(s: &str) -> String {
    let b = s.replace('+', " ");
    let bytes = b.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(v) = u8::from_str_radix(&b[i + 1..i + 3], 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).to_string()
}

// ── the answers ───────────────────────────────────────────────────────────────

async fn list_accounts(db: &Db) -> Result<Value> {
    let rows = db.read(|conn| accounts::list(conn)).await?;
    Ok(json!({
        "accounts": rows.iter().map(|a| json!({
            "id": a.id, "email": a.email, "name": a.display_name,
            "provider": format!("{:?}", a.provider), "sync_state": a.sync_state,
        })).collect::<Vec<_>>()
    }))
}

async fn search_threads(db: &Db, q: &Query) -> Result<Value> {
    let text = q.get("q").unwrap_or_default().to_string();
    let limit = q.num("limit", 20, 100);
    let rows = db
        .read(move |conn| {
            let parsed = search::parse(&text);
            search_repo::search(conn, &parsed, limit)
        })
        .await?;
    Ok(json!({
        "threads": rows.iter().map(|t| json!({
            "thread_id": t.id, "account_id": t.account_id, "account": t.account_email,
            "subject": t.subject, "snippet": t.snippet,
            "participants": t.participants.iter().map(|p| json!({"name": p.name, "email": p.email})).collect::<Vec<_>>(),
            "last_message_at": t.last_message_at, "messages": t.message_count,
            "unread": t.unread_count, "starred": t.is_starred, "has_attachments": t.has_attachments,
        })).collect::<Vec<_>>()
    }))
}

async fn thread_messages(db: &Db, q: &Query) -> Result<Value> {
    let id: i64 = q.get("id").and_then(|v| v.parse().ok()).unwrap_or(0);
    let rows = db
        .read(move |conn| {
            let mut st = conn.prepare(
                "SELECT id, subject, from_name, from_addr, to_json, date, is_read, has_attachments, snippet
                   FROM messages WHERE thread_id = ?1 ORDER BY date ASC LIMIT 200",
            )?;
            let out = st
                .query_map([id], |r| {
                    Ok(json!({
                        "id": r.get::<_, i64>(0)?,
                        "subject": r.get::<_, String>(1)?,
                        "from": {"name": r.get::<_, Option<String>>(2)?, "email": r.get::<_, Option<String>>(3)?},
                        "to": serde_json::from_str::<Value>(&r.get::<_, String>(4)?).unwrap_or(Value::Null),
                        "date": r.get::<_, i64>(5)?,
                        "read": r.get::<_, i64>(6)? != 0,
                        "has_attachments": r.get::<_, i64>(7)? != 0,
                        "snippet": r.get::<_, String>(8)?,
                    }))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(out)
        })
        .await?;
    Ok(json!({"thread_id": id, "messages": rows}))
}

async fn message(db: &Db, q: &Query) -> Result<Value> {
    let id: i64 = q.get("id").and_then(|v| v.parse().ok()).unwrap_or(0);
    let row = db
        .read(move |conn| {
            let head = conn.query_row(
                "SELECT m.id, m.account_id, m.thread_id, m.subject, m.from_name, m.from_addr, m.to_json,
                        m.cc_json, m.date, m.is_read, m.has_attachments, m.snippet,
                        b.text_body, b.html_body
                   FROM messages m LEFT JOIN message_bodies b ON b.message_id = m.id
                  WHERE m.id = ?1",
                [id],
                |r| {
                    Ok(json!({
                        "id": r.get::<_, i64>(0)?,
                        "account_id": r.get::<_, i64>(1)?,
                        "thread_id": r.get::<_, Option<i64>>(2)?,
                        "subject": r.get::<_, String>(3)?,
                        "from": {"name": r.get::<_, Option<String>>(4)?, "email": r.get::<_, Option<String>>(5)?},
                        "to": serde_json::from_str::<Value>(&r.get::<_, String>(6)?).unwrap_or(Value::Null),
                        "cc": serde_json::from_str::<Value>(&r.get::<_, String>(7)?).unwrap_or(Value::Null),
                        "date": r.get::<_, i64>(8)?,
                        "read": r.get::<_, i64>(9)? != 0,
                        "has_attachments": r.get::<_, i64>(10)? != 0,
                        "snippet": r.get::<_, String>(11)?,
                        // The text part is what an agent should read; the HTML is only said to exist.
                        "text": r.get::<_, Option<String>>(12)?,
                        "html_available": r.get::<_, Option<String>>(13)?.is_some(),
                    }))
                },
            );
            match head {
                Ok(v) => Ok(v),
                Err(rusqlite::Error::QueryReturnedNoRows) => Ok(Value::Null),
                Err(e) => Err(e.into()),
            }
        })
        .await?;
    Ok(json!({"message": row}))
}

async fn contact_list(db: &Db, q: &Query) -> Result<Value> {
    let text = q.get("q").unwrap_or_default().to_string();
    let limit = q.num("limit", 25, 200);
    let rows = db.read(move |conn| contacts::list_records(conn, &text, limit)).await?;
    Ok(json!({
        "contacts": rows.iter().map(|c| json!({
            "id": c.id, "name": c.name, "email": c.email, "phone": c.phone,
            "company": c.company, "job_title": c.job_title, "favorite": c.is_favorite,
        })).collect::<Vec<_>>()
    }))
}

async fn events(db: &Db, q: &Query) -> Result<Value> {
    let now = chrono::Utc::now().timestamp_millis();
    let from = q.get("from").and_then(|v| v.parse::<i64>().ok()).unwrap_or(now);
    let to = q
        .get("to")
        .and_then(|v| v.parse::<i64>().ok())
        .unwrap_or(now + 14 * 24 * 60 * 60 * 1000);
    let rows = db.read(move |conn| cal_repo::list_range(conn, from, to)).await?;
    Ok(json!({
        "from": from, "to": to,
        "events": rows.iter().map(|e| json!({
            "id": e.id, "title": e.summary, "location": e.location, "organizer": e.organizer,
            "starts_at": e.starts_at, "ends_at": e.ends_at, "all_day": e.all_day,
            "status": e.status, "join_url": e.join_url,
            "attendees": e.attendees.len(),
        })).collect::<Vec<_>>()
    }))
}
