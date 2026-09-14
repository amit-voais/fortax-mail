//! CalDAV server config (one row per account) and discovered calendar
//! collections. Credentials never live here: generic-server app passwords go
//! to the keyring, Google rides the account's OAuth tokens.

use crate::error::Result;
use crate::models::Calendar;
use rusqlite::{Connection, OptionalExtension, Row, params};
use std::collections::HashSet;

macro_rules! config_select {
    ($tail:literal) => {
        concat!(
            "SELECT account_id, kind, base_url, username, principal_url, ",
            "home_set_url, enabled, last_error FROM caldav_config ",
            $tail
        )
    };
}

macro_rules! calendar_select {
    ($tail:literal) => {
        concat!(
            "SELECT id, account_id, url, display_name, color, read_only, ",
            "enabled, is_default, last_synced_at FROM calendars ",
            $tail
        )
    };
}

#[derive(Debug, Clone)]
pub struct CaldavConfig {
    pub account_id: i64,
    /// "google" | "generic" | "microsoft" (Graph, not CalDAV)
    pub kind: String,
    pub base_url: String,
    pub username: Option<String>,
    pub principal_url: Option<String>,
    pub home_set_url: Option<String>,
    pub enabled: bool,
    pub last_error: Option<String>,
}

fn config_row(row: &Row) -> rusqlite::Result<CaldavConfig> {
    Ok(CaldavConfig {
        account_id: row.get("account_id")?,
        kind: row.get("kind")?,
        base_url: row.get("base_url")?,
        username: row.get("username")?,
        principal_url: row.get("principal_url")?,
        home_set_url: row.get("home_set_url")?,
        enabled: row.get::<_, i64>("enabled")? != 0,
        last_error: row.get("last_error")?,
    })
}

fn calendar_row(row: &Row) -> rusqlite::Result<Calendar> {
    Ok(Calendar {
        id: row.get("id")?,
        account_id: row.get("account_id")?,
        url: row.get("url")?,
        display_name: row.get("display_name")?,
        color: row.get("color")?,
        read_only: row.get::<_, i64>("read_only")? != 0,
        enabled: row.get::<_, i64>("enabled")? != 0,
        is_default: row.get::<_, i64>("is_default")? != 0,
        last_synced_at: row.get("last_synced_at")?,
    })
}

pub fn upsert_config(conn: &Connection, cfg: &CaldavConfig) -> Result<()> {
    conn.execute(
        "INSERT INTO caldav_config
            (account_id, kind, base_url, username, principal_url, home_set_url, enabled, last_error)
         VALUES (?1,?2,?3,?4,?5,?6,?7,NULL)
         ON CONFLICT(account_id) DO UPDATE SET
            kind = excluded.kind, base_url = excluded.base_url,
            username = excluded.username, principal_url = excluded.principal_url,
            home_set_url = excluded.home_set_url, enabled = excluded.enabled,
            last_error = NULL",
        params![
            cfg.account_id,
            cfg.kind,
            cfg.base_url,
            cfg.username,
            cfg.principal_url,
            cfg.home_set_url,
            cfg.enabled as i64,
        ],
    )?;
    Ok(())
}

pub fn get_config(conn: &Connection, account_id: i64) -> Result<Option<CaldavConfig>> {
    conn.query_row(
        config_select!("WHERE account_id = ?1"),
        params![account_id],
        config_row,
    )
    .optional()
    .map_err(Into::into)
}

pub fn all_configs(conn: &Connection) -> Result<Vec<CaldavConfig>> {
    let mut stmt = conn.prepare(config_select!("WHERE enabled = 1"))?;
    let rows = stmt
        .query_map([], config_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

pub fn list_configs(conn: &Connection) -> Result<Vec<CaldavConfig>> {
    let mut stmt = conn.prepare(config_select!("ORDER BY account_id"))?;
    let rows = stmt
        .query_map([], config_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

pub fn set_config_enabled(conn: &Connection, account_id: i64, enabled: bool) -> Result<bool> {
    Ok(conn.execute(
        "UPDATE caldav_config SET enabled = ?2, last_error = NULL WHERE account_id = ?1",
        params![account_id, enabled as i64],
    )? > 0)
}

pub fn set_config_error(conn: &Connection, account_id: i64, error: Option<&str>) -> Result<()> {
    conn.execute(
        "UPDATE caldav_config SET last_error = ?2 WHERE account_id = ?1",
        params![account_id, error],
    )?;
    Ok(())
}

/// Drop the config and detach its events (they stay local; sync bookkeeping
/// is cleared so a later reconnect re-adopts them by UID).
pub fn delete_config(conn: &Connection, account_id: i64) -> Result<()> {
    conn.execute(
        "UPDATE calendar_events SET calendar_id = NULL, caldav_href = NULL, etag = NULL,
                dirty = 0 WHERE account_id = ?1",
        params![account_id],
    )?;
    conn.execute(
        "DELETE FROM calendar_events WHERE account_id = ?1 AND deleted = 1",
        params![account_id],
    )?;
    conn.execute(
        "DELETE FROM calendars WHERE account_id = ?1",
        params![account_id],
    )?;
    conn.execute(
        "DELETE FROM caldav_config WHERE account_id = ?1",
        params![account_id],
    )?;
    Ok(())
}

/// Upsert a discovered collection; returns its id. Existing sync state
/// (ctag/sync_token/enabled) survives re-discovery.
pub fn upsert_calendar(
    conn: &Connection,
    account_id: i64,
    url: &str,
    display_name: Option<&str>,
    color: Option<&str>,
    read_only: bool,
) -> Result<i64> {
    upsert_calendar_with_initial_enabled(
        conn,
        account_id,
        url,
        display_name,
        color,
        read_only,
        true,
    )
}

/// Insert a newly discovered collection with the provider's initial
/// visibility. Existing local visibility and sync state always survive
/// re-discovery.
pub fn upsert_calendar_with_initial_enabled(
    conn: &Connection,
    account_id: i64,
    url: &str,
    display_name: Option<&str>,
    color: Option<&str>,
    read_only: bool,
    enabled: bool,
) -> Result<i64> {
    conn.execute(
        "INSERT INTO calendars (account_id, url, display_name, color, read_only, enabled)
         VALUES (?1,?2,?3,?4,?5,?6)
         ON CONFLICT(account_id, url) DO UPDATE SET
            display_name = excluded.display_name,
            color = COALESCE(calendars.color, excluded.color),
            read_only = excluded.read_only",
        params![
            account_id,
            url,
            display_name,
            color,
            read_only as i64,
            enabled as i64
        ],
    )?;
    conn.query_row(
        "SELECT id FROM calendars WHERE account_id = ?1 AND url = ?2",
        params![account_id, url],
        |r| r.get(0),
    )
    .map_err(Into::into)
}

/// Remove collections no longer advertised during an explicit reconnect.
/// Their events stay available as local events and can be adopted again by
/// UID if that collection is connected later.
pub fn retain_calendars(conn: &Connection, account_id: i64, urls: &HashSet<String>) -> Result<()> {
    let mut statement = conn.prepare("SELECT id, url FROM calendars WHERE account_id = ?1")?;
    let calendars = statement
        .query_map(params![account_id], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    drop(statement);
    for (id, url) in calendars {
        if !urls.contains(&url) {
            conn.execute(
                "UPDATE calendar_events
                 SET calendar_id = NULL, caldav_href = NULL, etag = NULL, dirty = 0
                 WHERE calendar_id = ?1",
                params![id],
            )?;
            conn.execute(
                "DELETE FROM calendar_events
                 WHERE account_id = ?1 AND calendar_id IS NULL AND deleted = 1",
                params![account_id],
            )?;
            conn.execute("DELETE FROM calendars WHERE id = ?1", params![id])?;
        }
    }
    Ok(())
}

pub fn list_calendars(conn: &Connection, account_id: Option<i64>) -> Result<Vec<Calendar>> {
    let mut out = Vec::new();
    match account_id {
        Some(id) => {
            let mut stmt = conn.prepare(calendar_select!(
                "WHERE account_id = ?1
                 ORDER BY is_default DESC, display_name COLLATE NOCASE, id"
            ))?;
            for row in stmt.query_map(params![id], calendar_row)? {
                out.push(row?);
            }
        }
        None => {
            let mut stmt = conn.prepare(calendar_select!(
                "ORDER BY account_id, is_default DESC, display_name COLLATE NOCASE, id"
            ))?;
            for row in stmt.query_map([], calendar_row)? {
                out.push(row?);
            }
        }
    }
    Ok(out)
}

pub fn get_calendar(conn: &Connection, calendar_id: i64) -> Result<Option<Calendar>> {
    conn.query_row(
        calendar_select!("WHERE id = ?1"),
        params![calendar_id],
        calendar_row,
    )
    .optional()
    .map_err(Into::into)
}

pub fn set_calendar_enabled(conn: &Connection, calendar_id: i64, enabled: bool) -> Result<()> {
    conn.execute(
        "UPDATE calendars SET enabled = ?2 WHERE id = ?1",
        params![calendar_id, enabled as i64],
    )?;
    Ok(())
}

pub fn set_calendar_color(conn: &Connection, calendar_id: i64, color: Option<&str>) -> Result<()> {
    conn.execute(
        "UPDATE calendars SET color = ?2 WHERE id = ?1",
        params![calendar_id, color],
    )?;
    Ok(())
}

/// Exactly one default (new-event target) per account.
pub fn set_default_calendar(conn: &Connection, account_id: i64, calendar_id: i64) -> Result<()> {
    conn.execute(
        "UPDATE calendars SET is_default = (id = ?2) WHERE account_id = ?1",
        params![account_id, calendar_id],
    )?;
    Ok(())
}

pub fn default_calendar(conn: &Connection, account_id: i64) -> Result<Option<Calendar>> {
    conn.query_row(
        calendar_select!(
            "WHERE account_id = ?1 AND enabled = 1
             ORDER BY is_default DESC, id ASC LIMIT 1"
        ),
        params![account_id],
        calendar_row,
    )
    .optional()
    .map_err(Into::into)
}

pub fn set_sync_state(
    conn: &Connection,
    calendar_id: i64,
    ctag: Option<&str>,
    sync_token: Option<&str>,
    synced_at: i64,
) -> Result<()> {
    conn.execute(
        "UPDATE calendars SET ctag = ?2, sync_token = ?3, last_synced_at = ?4 WHERE id = ?1",
        params![calendar_id, ctag, sync_token, synced_at],
    )?;
    Ok(())
}

/// (ctag, sync_token) as last stored.
pub fn sync_state(conn: &Connection, calendar_id: i64) -> Result<(Option<String>, Option<String>)> {
    conn.query_row(
        "SELECT ctag, sync_token FROM calendars WHERE id = ?1",
        params![calendar_id],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )
    .map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::testutil;

    #[test]
    fn config_and_calendar_lifecycle() {
        let c = testutil::calendar_conn();
        upsert_config(
            &c,
            &CaldavConfig {
                account_id: 1,
                kind: "generic".into(),
                base_url: "https://dav.example.com/".into(),
                username: Some("me".into()),
                principal_url: None,
                home_set_url: None,
                enabled: true,
                last_error: None,
            },
        )
        .unwrap();
        assert_eq!(all_configs(&c).unwrap().len(), 1);

        let initially_hidden = upsert_calendar_with_initial_enabled(
            &c,
            1,
            "https://dav.example.com/cal/hidden/",
            Some("Hidden"),
            None,
            true,
            false,
        )
        .unwrap();
        assert!(!get_calendar(&c, initially_hidden).unwrap().unwrap().enabled);
        // Provider re-discovery never overwrites the user's local choice.
        set_calendar_enabled(&c, initially_hidden, true).unwrap();
        upsert_calendar_with_initial_enabled(
            &c,
            1,
            "https://dav.example.com/cal/hidden/",
            Some("Hidden renamed"),
            None,
            true,
            false,
        )
        .unwrap();
        assert!(get_calendar(&c, initially_hidden).unwrap().unwrap().enabled);
        set_calendar_enabled(&c, initially_hidden, false).unwrap();

        let a = upsert_calendar(
            &c,
            1,
            "https://dav.example.com/cal/a/",
            Some("A"),
            None,
            false,
        )
        .unwrap();
        let b = upsert_calendar(
            &c,
            1,
            "https://dav.example.com/cal/b/",
            Some("B"),
            None,
            true,
        )
        .unwrap();
        c.execute(
            "INSERT INTO calendar_events
             (account_id, ical_uid, starts_at, calendar_id, caldav_href, etag, dirty)
             VALUES (1, 'removed@test', 1000, ?1, '/cal/b/removed.ics', 'etag', 1)",
            params![b],
        )
        .unwrap();
        // Re-discovery keeps ids and sync state.
        set_sync_state(&c, a, Some("c1"), Some("t1"), 42).unwrap();
        let a2 = upsert_calendar(
            &c,
            1,
            "https://dav.example.com/cal/a/",
            Some("A2"),
            None,
            false,
        )
        .unwrap();
        assert_eq!(a, a2);
        assert_eq!(
            sync_state(&c, a).unwrap(),
            (Some("c1".into()), Some("t1".into()))
        );

        // User-set color survives re-discovery.
        set_calendar_color(&c, a, Some("#dc2626")).unwrap();
        upsert_calendar(
            &c,
            1,
            "https://dav.example.com/cal/a/",
            Some("A3"),
            Some("#123456"),
            false,
        )
        .unwrap();
        assert_eq!(
            get_calendar(&c, a).unwrap().unwrap().color.as_deref(),
            Some("#dc2626")
        );

        set_default_calendar(&c, 1, b).unwrap();
        assert_eq!(default_calendar(&c, 1).unwrap().unwrap().id, b);
        set_calendar_enabled(&c, b, false).unwrap();
        // Default falls back to an enabled calendar.
        assert_eq!(default_calendar(&c, 1).unwrap().unwrap().id, a);

        retain_calendars(
            &c,
            1,
            &HashSet::from(["https://dav.example.com/cal/a/".into()]),
        )
        .unwrap();
        assert_eq!(list_calendars(&c, Some(1)).unwrap().len(), 1);
        let detached: (Option<i64>, Option<String>, Option<String>, bool) = c
            .query_row(
                "SELECT calendar_id, caldav_href, etag, dirty
                 FROM calendar_events WHERE ical_uid = 'removed@test'",
                [],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get::<_, i64>(3)? != 0,
                    ))
                },
            )
            .unwrap();
        assert_eq!(detached, (None, None, None, false));

        delete_config(&c, 1).unwrap();
        assert!(all_configs(&c).unwrap().is_empty());
        assert!(list_calendars(&c, Some(1)).unwrap().is_empty());
    }
}
