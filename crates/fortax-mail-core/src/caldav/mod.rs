//! Two-way CalDAV sync: discovery (RFC 6764/4791), incremental pull via
//! sync-collection (RFC 6578) with a calendar-query fallback, and offline-safe
//! push of local edits (PUT If-Match / DELETE). Google is CalDAV-with-OAuth;
//! Microsoft Graph is a future second `kind` behind the same seams.

pub mod discovery;
pub mod http;
pub mod push;
pub mod rrule;
pub mod sync;
pub mod task;
pub mod xml;

pub use http::{DavAuth, DavResponse, HttpTransport, Transport};

use crate::error::CoreError;

/// Google's CalDAV endpoint prefix. Google does not expose a discoverable
/// service root here: clients must start at `/{calendar_id}/events` when they
/// already know the calendar collection. The primary calendar id is the
/// account email address.
pub const GOOGLE_CALDAV_BASE: &str = "https://apidata.googleusercontent.com/caldav/v2/";

/// Collection URL for a Google calendar id, encoded as one URL path segment.
///
/// Using `Url::path_segments_mut` matters for secondary calendar ids, which
/// can contain characters such as `#` that must not be interpreted as URL
/// syntax.
pub fn google_calendar_url(calendar_id: &str) -> Result<String, CoreError> {
    let mut url = url::Url::parse(GOOGLE_CALDAV_BASE)
        .map_err(|error| err(format!("invalid built-in Google CalDAV base URL: {error}")))?;
    url.path_segments_mut()
        .map_err(|_| err("built-in Google CalDAV URL cannot contain path segments"))?
        .pop_if_empty()
        .extend([calendar_id, "events"]);
    Ok(url.to_string())
}

pub fn err(msg: impl Into<String>) -> CoreError {
    CoreError::CalDav(msg.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn google_collection_starts_at_the_primary_calendar() {
        assert_eq!(
            google_calendar_url("person@example.com").unwrap(),
            "https://apidata.googleusercontent.com/caldav/v2/person@example.com/events"
        );
    }

    #[test]
    fn google_collection_encodes_calendar_id_as_one_path_segment() {
        assert_eq!(
            google_calendar_url("team/calendar#group.v.calendar.google.com").unwrap(),
            "https://apidata.googleusercontent.com/caldav/v2/team%2Fcalendar%23group.v.calendar.google.com/events"
        );
    }
}
