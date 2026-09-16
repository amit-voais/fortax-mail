//! Canonical baseline for the standalone calendar SQLite database.

use crate::error::{CoreError, Result};
use rusqlite::Connection;

const MIGRATIONS: &[&str] = &[r#"
CREATE TABLE calendars (
  id INTEGER PRIMARY KEY,
  account_id INTEGER NOT NULL,
  url TEXT NOT NULL,
  display_name TEXT,
  color TEXT,
  ctag TEXT,
  sync_token TEXT,
  read_only INTEGER NOT NULL DEFAULT 0 CHECK (read_only IN (0, 1)),
  enabled INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0, 1)),
  is_default INTEGER NOT NULL DEFAULT 0 CHECK (is_default IN (0, 1)),
  last_synced_at INTEGER,
  UNIQUE(account_id, url)
);
CREATE UNIQUE INDEX idx_calendars_one_default
  ON calendars(account_id) WHERE is_default = 1;

CREATE TABLE calendar_events (
  id INTEGER PRIMARY KEY,
  account_id INTEGER NOT NULL,
  message_id INTEGER,
  ical_uid TEXT NOT NULL,
  method TEXT,
  summary TEXT,
  location TEXT,
  organizer TEXT,
  starts_at INTEGER NOT NULL,
  ends_at INTEGER,
  all_day INTEGER NOT NULL DEFAULT 0 CHECK (all_day IN (0, 1)),
  status TEXT,
  description TEXT,
  attendees_json TEXT,
  join_url TEXT,
  rsvp_status TEXT,
  is_local INTEGER NOT NULL DEFAULT 0 CHECK (is_local IN (0, 1)),
  sequence INTEGER NOT NULL DEFAULT 0,
  calendar_id INTEGER REFERENCES calendars(id) ON DELETE SET NULL,
  caldav_href TEXT,
  etag TEXT,
  ical_raw TEXT,
  rrule TEXT,
  tzid TEXT,
  dirty INTEGER NOT NULL DEFAULT 0 CHECK (dirty IN (0, 1)),
  deleted INTEGER NOT NULL DEFAULT 0 CHECK (deleted IN (0, 1)),
  notified_at INTEGER,
  series_master_id TEXT
);
CREATE INDEX idx_calendar_starts ON calendar_events(starts_at);
CREATE UNIQUE INDEX idx_events_caldav
  ON calendar_events(calendar_id, caldav_href)
  WHERE calendar_id IS NOT NULL AND caldav_href IS NOT NULL;
-- A UID identifies one resource inside a provider collection, but the same
-- event may legitimately be copied into multiple calendars. Graph recurrence
-- instances use their unique Graph ids as local UIDs before reaching here.
CREATE UNIQUE INDEX idx_events_calendar_uid
  ON calendar_events(calendar_id, ical_uid) WHERE calendar_id IS NOT NULL;
-- One event row owns the mail-invite link. Provider disconnect may detach
-- several same-UID copies, so calendar_id IS NULL cannot itself be unique.
CREATE UNIQUE INDEX idx_events_mail_uid
  ON calendar_events(account_id, ical_uid) WHERE message_id IS NOT NULL;
CREATE INDEX idx_events_dirty ON calendar_events(dirty, deleted)
  WHERE dirty = 1 OR deleted = 1;
CREATE INDEX idx_events_series_master ON calendar_events(calendar_id, series_master_id);
CREATE INDEX idx_events_message ON calendar_events(message_id);

-- calendar_id is a local foreign key while account_id identifies the owner in
-- the separate mail store. Keep those two pieces of identity consistent.
CREATE TRIGGER calendar_event_account_insert
BEFORE INSERT ON calendar_events
WHEN NEW.calendar_id IS NOT NULL AND NOT EXISTS (
  SELECT 1 FROM calendars
  WHERE id = NEW.calendar_id AND account_id = NEW.account_id
)
BEGIN
  SELECT RAISE(ABORT, 'calendar event account mismatch');
END;
CREATE TRIGGER calendar_event_account_update
BEFORE UPDATE OF calendar_id, account_id ON calendar_events
WHEN NEW.calendar_id IS NOT NULL AND NOT EXISTS (
  SELECT 1 FROM calendars
  WHERE id = NEW.calendar_id AND account_id = NEW.account_id
)
BEGIN
  SELECT RAISE(ABORT, 'calendar event account mismatch');
END;
CREATE TRIGGER calendar_owner_immutable
BEFORE UPDATE OF account_id ON calendars
WHEN NEW.account_id != OLD.account_id AND EXISTS (
  SELECT 1 FROM calendar_events WHERE calendar_id = OLD.id
)
BEGIN
  SELECT RAISE(ABORT, 'calendar owner has events');
END;

CREATE TABLE caldav_config (
  account_id INTEGER PRIMARY KEY,
  kind TEXT NOT NULL CHECK (kind IN ('google','generic','microsoft')),
  base_url TEXT NOT NULL,
  username TEXT,
  principal_url TEXT,
  home_set_url TEXT,
  enabled INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0, 1)),
  last_error TEXT
);

CREATE TABLE pending_actions (
  id INTEGER PRIMARY KEY,
  account_id INTEGER NOT NULL,
  kind TEXT NOT NULL,
  message_id INTEGER,
  thread_id INTEGER,
  payload TEXT NOT NULL DEFAULT '{}',
  state TEXT NOT NULL DEFAULT 'pending'
    CHECK (state IN ('pending','inflight','done','failed','cancelled')),
  attempts INTEGER NOT NULL DEFAULT 0,
  not_before INTEGER,
  created_at INTEGER NOT NULL,
  finished_at INTEGER,
  last_error TEXT
);
CREATE INDEX idx_calendar_actions_due
  ON pending_actions(account_id, state, not_before, created_at);
"#];
pub const LATEST_VERSION: i64 = MIGRATIONS.len() as i64;

pub fn run(conn: &mut Connection) -> Result<()> {
    let version: i64 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    let latest = LATEST_VERSION;
    if version > latest {
        return Err(CoreError::Other(format!(
            "unsupported pre-release calendar database version {version}; recreate the development profile"
        )));
    }

    for (index, sql) in MIGRATIONS.iter().enumerate().skip(version as usize) {
        let target = (index + 1) as i64;
        let tx = conn.transaction()?;
        tx.execute_batch(sql)?;
        tx.pragma_update(None, "user_version", target)?;
        tx.commit()?;
        tracing::info!("applied calendar db migration {target}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    fn fresh() -> Connection {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.pragma_update(None, "foreign_keys", "ON").unwrap();
        run(&mut conn).unwrap();
        conn
    }

    #[test]
    fn fresh_schema_has_the_canonical_calendar_contract() {
        let conn = fresh();

        let tables: BTreeSet<String> = conn
            .prepare(
                "SELECT name FROM sqlite_master
                 WHERE type = 'table' AND name NOT LIKE 'sqlite_%'
                 ORDER BY name",
            )
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        let expected = [
            "calendar_events",
            "caldav_config",
            "calendars",
            "pending_actions",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect();
        assert_eq!(tables, expected);
        assert_eq!(
            conn.pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
                .unwrap(),
            1
        );
        let violations = conn
            .prepare("PRAGMA foreign_key_check")
            .unwrap()
            .query([])
            .unwrap()
            .next()
            .unwrap()
            .is_some();
        assert!(!violations);
        let integrity: String = conn
            .query_row("PRAGMA integrity_check", [], |row| row.get(0))
            .unwrap();
        assert_eq!(integrity, "ok");
    }

    #[test]
    fn identity_constraints_preserve_distinct_calendar_copies() {
        let conn = fresh();
        conn.execute(
            "INSERT INTO calendars (id, account_id, url, display_name)
             VALUES (1, 7, 'https://cal.test/a/', 'A'),
                    (2, 7, 'https://cal.test/b/', 'B')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO calendar_events
                (account_id, calendar_id, caldav_href, ical_uid, starts_at)
             VALUES (7, 1, '/a/event.ics', 'shared@test', 1000),
                    (7, 2, '/b/event.ics', 'shared@test', 1000)",
            [],
        )
        .unwrap();

        assert!(
            conn.execute(
                "INSERT INTO calendar_events
                    (account_id, calendar_id, caldav_href, ical_uid, starts_at)
                 VALUES (7, 1, '/a/duplicate.ics', 'shared@test', 1000)",
                [],
            )
            .is_err()
        );
        assert!(
            conn.execute(
                "INSERT INTO calendar_events
                    (account_id, calendar_id, caldav_href, ical_uid, starts_at)
                 VALUES (8, 1, '/a/wrong-account.ics', 'wrong@test', 1000)",
                [],
            )
            .is_err()
        );

        conn.execute(
            "UPDATE calendar_events SET message_id = 10
             WHERE calendar_id = 1 AND ical_uid = 'shared@test'",
            [],
        )
        .unwrap();
        assert!(
            conn.execute(
                "UPDATE calendar_events SET message_id = 11
                 WHERE calendar_id = 2 AND ical_uid = 'shared@test'",
                [],
            )
            .is_err()
        );

        // Disconnecting a provider detaches every copy. This must remain
        // valid even though the detached rows share an iCalendar UID.
        conn.execute(
            "UPDATE calendar_events
             SET calendar_id = NULL, caldav_href = NULL, etag = NULL",
            [],
        )
        .unwrap();
        let copies: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM calendar_events WHERE ical_uid = 'shared@test'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(copies, 2);
    }

    #[test]
    fn pre_release_profiles_are_rejected_without_mutation() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.pragma_update(None, "user_version", 2).unwrap();
        let error = run(&mut conn).unwrap_err().to_string();
        assert!(error.contains("recreate the development profile"));
        assert_eq!(
            conn.pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
                .unwrap(),
            2
        );
    }
}
