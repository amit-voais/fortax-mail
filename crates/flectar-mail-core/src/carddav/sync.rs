//! ETag-safe CardDAV push followed by incremental or full collection pull.

use super::{discovery::resolve, http::Transport, vcard, xml};
use crate::{
    db::{Db, repo},
    error::Result,
    events::{CoreEvent, EventBus},
    models::now_ms,
};
use std::collections::{HashMap, HashSet};

const MULTIGET_BATCH: usize = 50;

fn required_etag(href: &str, etag: Option<&str>) -> Result<String> {
    etag.map(str::trim)
        .filter(|etag| !etag.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| super::err(format!("CardDAV resource {href} has no required ETag")))
}

fn href_key(collection_url: &str, href: &str) -> Result<String> {
    let resolved = resolve(collection_url, href)?;
    let url = url::Url::parse(&resolved)
        .map_err(|error| super::err(format!("invalid CardDAV href {href}: {error}")))?;
    let mut key = url.path().to_owned();
    if let Some(query) = url.query() {
        key.push('?');
        key.push_str(query);
    }
    Ok(key)
}

async fn accept_server_version(
    db: &Db,
    transport: &dyn Transport,
    account_id: i64,
    book_id: i64,
    book_url: &str,
    href: &str,
) -> Result<()> {
    let target = resolve(book_url, href)?;
    let response = transport
        .request(
            "GET",
            &target,
            None,
            &[("Accept", "text/vcard")],
            None,
            None,
        )
        .await?;
    if response.status == 404 {
        let href = href.to_owned();
        db.write(move |conn| repo::carddav::remove_remote(conn, book_id, &href))
            .await?;
        return Ok(());
    }
    if !response.ok() {
        return Err(super::err(format!(
            "GET {target} after an ETag conflict: HTTP {}",
            response.status
        )));
    }
    let etag = required_etag(&target, response.etag.as_deref())?;
    let href = href.to_owned();
    let card = response.body;
    vcard::ensure_size(&card)?;
    let parsed = vcard::parse(&card);
    db.write(move |conn| {
        let tx = conn.transaction()?;
        match parsed {
            Ok(record) => repo::carddav::upsert_remote(
                &tx,
                account_id,
                book_id,
                &href,
                Some(&etag),
                &card,
                &record,
            )?,
            Err(error) => {
                tracing::warn!(%href, %error, "retaining unsupported CardDAV conflict version");
                repo::carddav::upsert_unmapped_remote(&tx, book_id, &href, Some(&etag), &card)?;
            }
        }
        tx.commit()?;
        Ok(())
    })
    .await
}

pub async fn sync_account(
    db: &Db,
    bus: &EventBus,
    transport: &dyn Transport,
    account_id: i64,
) -> Result<bool> {
    let mut changed = push_dirty(db, transport, account_id).await?;
    let books = db
        .read(move |conn| repo::carddav::list_addressbooks(conn, Some(account_id)))
        .await?;
    for book in books.into_iter().filter(|book| book.enabled) {
        changed |= sync_addressbook(db, transport, account_id, book.id, &book.url).await?;
    }
    if changed {
        bus.emit(CoreEvent::ContactsUpdated { account_id });
    }
    Ok(changed)
}

async fn push_dirty(db: &Db, transport: &dyn Transport, account_id: i64) -> Result<bool> {
    let objects = db
        .read(move |conn| repo::carddav::dirty_objects(conn, account_id))
        .await?;
    let changed = !objects.is_empty();
    for object in objects {
        let book_id = object.addressbook_id;
        let book_url = db
            .read(move |conn| repo::carddav::addressbook_url(conn, book_id))
            .await?
            .ok_or_else(|| super::err("address book no longer exists"))?;
        let target = resolve(&book_url, &object.href)?;
        if object.deleted {
            if !object.remote_exists {
                let id = object.id;
                db.write(move |conn| repo::carddav::delete_object(conn, id))
                    .await?;
                continue;
            }
            let etag = object.etag.as_deref().ok_or_else(|| {
                super::err(format!(
                    "cannot safely delete {} because the server supplied no ETag",
                    object.href
                ))
            })?;
            let headers = [("If-Match", etag)];
            let response = transport
                .request("DELETE", &target, None, &headers, None, None)
                .await?;
            match response.status {
                200..=299 | 404 => {
                    let id = object.id;
                    db.write(move |conn| repo::carddav::delete_object(conn, id))
                        .await?;
                }
                412 => {
                    // The resource changed after the local deletion. Fetch it
                    // immediately so the server's version wins deterministically.
                    accept_server_version(
                        db,
                        transport,
                        account_id,
                        book_id,
                        &book_url,
                        &object.href,
                    )
                    .await?;
                }
                403 => {
                    db.write(move |conn| repo::carddav::mark_addressbook_read_only(conn, book_id))
                        .await?;
                    tracing::warn!(
                        book_id,
                        "CardDAV address book rejected DELETE; marking it read-only"
                    );
                }
                status => return Err(super::err(format!("DELETE {target}: HTTP {status}"))),
            }
            continue;
        }
        let Some(contact_id) = object.contact_id else {
            continue;
        };
        let record = db
            .read(move |conn| repo::carddav::contact(conn, contact_id))
            .await?
            .ok_or_else(|| super::err("contact no longer exists"))?;
        let body = vcard::update(&object.vcard_raw, &record, &format!("flectar-{contact_id}"));
        vcard::ensure_size(&body)?;
        let headers = if object.remote_exists {
            let etag = object.etag.as_deref().ok_or_else(|| {
                super::err(format!(
                    "cannot safely update {} because the server supplied no ETag",
                    object.href
                ))
            })?;
            vec![("If-Match", etag)]
        } else {
            vec![("If-None-Match", "*")]
        };
        let mut response = transport
            .request(
                "PUT",
                &target,
                None,
                &headers,
                Some("text/vcard; charset=utf-8"),
                Some(body.clone()),
            )
            .await?;
        if response.status == 403 {
            db.write(move |conn| repo::carddav::mark_addressbook_read_only(conn, book_id))
                .await?;
            tracing::warn!(
                book_id,
                "CardDAV address book rejected PUT; marking it read-only"
            );
            continue;
        }
        if response.status == 412 && object.remote_exists {
            // Server wins a concurrent edit. Import it now rather than
            // relying on a later ctag or sync-token response.
            accept_server_version(db, transport, account_id, book_id, &book_url, &object.href)
                .await?;
            continue;
        }
        if response.status == 412 {
            return Err(super::err(format!(
                "server rejected the new contact URI {target}: HTTP {}",
                response.status
            )));
        }
        if !response.ok() {
            return Err(super::err(format!(
                "PUT {target}: HTTP {}",
                response.status
            )));
        }
        let mut saved_body = body;
        if response
            .etag
            .as_deref()
            .is_none_or(|etag| etag.trim().is_empty())
            && let Ok(get) = transport
                .request(
                    "GET",
                    &target,
                    None,
                    &[("Accept", "text/vcard")],
                    None,
                    None,
                )
                .await
        {
            if get.ok() && !get.body.trim().is_empty() {
                saved_body = get.body;
            }
            response.etag = get.etag;
        }
        let id = object.id;
        let etag = response.etag.filter(|etag| !etag.trim().is_empty());
        let missing_etag = etag.is_none();
        db.write(move |conn| repo::carddav::clear_dirty(conn, id, etag.as_deref(), &saved_body))
            .await?;
        if missing_etag {
            return Err(super::err(format!(
                "server stored {target} but did not return its required ETag"
            )));
        }
    }
    Ok(changed)
}

async fn sync_addressbook(
    db: &Db,
    transport: &dyn Transport,
    account_id: i64,
    book_id: i64,
    book_url: &str,
) -> Result<bool> {
    let (stored_ctag, stored_token) = db
        .read(move |conn| repo::carddav::sync_state(conn, book_id))
        .await?;
    let state = transport
        .request(
            "PROPFIND",
            book_url,
            Some("0"),
            &[],
            Some("application/xml; charset=utf-8"),
            Some(xml::state()),
        )
        .await?;
    let (ctag, advertised_token) = if state.status == 207 {
        let parsed = xml::parse(&state.body)?;
        (
            parsed.items.first().and_then(|item| item.ctag.clone()),
            parsed
                .items
                .first()
                .and_then(|item| item.sync_token.clone()),
        )
    } else {
        (None, None)
    };
    if ctag.is_some() && ctag == stored_ctag {
        return Ok(false);
    }

    let collection_href = href_key(book_url, book_url)?;
    let mut removed = Vec::new();
    let mut remote = Vec::<(String, Option<String>)>::new();
    let mut token = None;
    let mut incremental = false;
    if stored_token.is_some() || advertised_token.is_some() {
        // An empty token requests the initial state. RFC 6578 requires Depth
        // 0; sync-level controls the collection scope.
        let requested_token = stored_token.as_deref().unwrap_or("");
        let response = transport
            .request(
                "REPORT",
                book_url,
                Some("0"),
                &[],
                Some("application/xml; charset=utf-8"),
                Some(xml::sync_collection(requested_token)),
            )
            .await?;
        if response.status == 207 && !response.body.contains("valid-sync-token") {
            let parsed = xml::parse(&response.body)?;
            for item in parsed.items {
                let href = href_key(book_url, &item.href)?;
                if href.is_empty() || href == collection_href {
                    continue;
                }
                if item.status == 404 {
                    removed.push(href);
                } else if !(200..300).contains(&item.status) {
                    return Err(super::err(format!(
                        "sync-collection {href}: HTTP {}",
                        item.status
                    )));
                } else {
                    let etag = required_etag(&href, item.etag.as_deref())?;
                    remote.push((href, Some(etag)));
                }
            }
            token = parsed.sync_token.or(advertised_token.clone());
            incremental = true;
        }
    }
    if !incremental {
        let response = transport
            .request(
                "REPORT",
                book_url,
                Some("1"),
                &[],
                Some("application/xml; charset=utf-8"),
                Some(xml::addressbook_query()),
            )
            .await?;
        if response.status != 207 {
            return Err(super::err(format!(
                "addressbook-query: HTTP {}",
                response.status
            )));
        }
        let parsed = xml::parse(&response.body)?;
        for item in parsed.items {
            if item.href.is_empty() {
                return Err(super::err("addressbook-query returned an empty href"));
            }
            let href = href_key(book_url, &item.href)?;
            if href == collection_href {
                continue;
            }
            if !(200..300).contains(&item.status) {
                return Err(super::err(format!(
                    "addressbook-query {href}: HTTP {}",
                    item.status
                )));
            }
            let etag = required_etag(&href, item.etag.as_deref())?;
            remote.push((href, Some(etag)));
        }
        let remote_set = remote
            .iter()
            .map(|(href, _)| href.as_str())
            .collect::<HashSet<_>>();
        let local = db
            .read(move |conn| repo::carddav::object_etags(conn, book_id))
            .await?;
        removed.extend(
            local
                .into_iter()
                .map(|(href, _)| href)
                .filter(|href| !remote_set.contains(href.as_str())),
        );
        token = advertised_token;
    }

    let local = db
        .read(move |conn| repo::carddav::object_etags(conn, book_id))
        .await?;
    let etags = local.into_iter().collect::<HashMap<_, _>>();
    remote.retain(|(href, etag)| {
        etags
            .get(href)
            .is_none_or(|stored| etag.is_none() || stored != etag)
    });
    let mut changed = !removed.is_empty();
    for batch in remote.chunks(MULTIGET_BATCH) {
        let hrefs = batch
            .iter()
            .map(|(href, _)| href.clone())
            .collect::<Vec<_>>();
        let response = transport
            .request(
                "REPORT",
                book_url,
                Some("0"),
                &[],
                Some("application/xml; charset=utf-8"),
                Some(xml::multiget(&hrefs)),
            )
            .await?;
        if response.status != 207 {
            return Err(super::err(format!(
                "addressbook-multiget: HTTP {}",
                response.status
            )));
        }
        let items = xml::parse(&response.body)?
            .items
            .into_iter()
            .map(|item| Ok((href_key(book_url, &item.href)?, item)))
            .collect::<Result<Vec<_>>>()?;
        let expected = hrefs.iter().cloned().collect::<HashSet<_>>();
        let mut seen = HashSet::new();
        for (href, item) in &items {
            if !expected.contains(href) {
                continue;
            }
            seen.insert(href.clone());
            if item.status == 404 {
                removed.push(href.clone());
            } else if !(200..300).contains(&item.status) {
                return Err(super::err(format!(
                    "addressbook-multiget {href}: HTTP {}",
                    item.status
                )));
            } else if item.address_data.is_none() {
                return Err(super::err(format!(
                    "addressbook-multiget returned no vCard for {href}"
                )));
            } else if item
                .etag
                .as_deref()
                .is_none_or(|etag| etag.trim().is_empty())
            {
                return Err(super::err(format!(
                    "addressbook-multiget returned no ETag for {href}"
                )));
            } else if let Some(card) = item.address_data.as_deref() {
                vcard::ensure_size(card)?;
            }
        }
        if let Some(missing) = expected.difference(&seen).next() {
            return Err(super::err(format!(
                "addressbook-multiget omitted {missing}"
            )));
        }
        if !seen.is_empty() {
            changed = true;
        }
        db.write(move |conn| {
            let tx = conn.transaction()?;
            for (href, item) in items
                .into_iter()
                .filter(|(_, item)| (200..300).contains(&item.status))
            {
                let Some(card) = item.address_data else {
                    continue;
                };
                match vcard::parse(&card) {
                    Ok(record) => repo::carddav::upsert_remote(
                        &tx,
                        account_id,
                        book_id,
                        &href,
                        item.etag.as_deref(),
                        &card,
                        &record,
                    )?,
                    Err(error) => {
                        tracing::warn!(href=%item.href, %error, "retaining unsupported CardDAV resource");
                        repo::carddav::upsert_unmapped_remote(
                            &tx,
                            book_id,
                            &href,
                            item.etag.as_deref(),
                            &card,
                        )?;
                    }
                }
            }
            tx.commit()?;
            Ok(())
        })
        .await?;
    }
    if !removed.is_empty() {
        db.write(move |conn| {
            let tx = conn.transaction()?;
            for href in removed {
                repo::carddav::remove_remote(&tx, book_id, &href)?;
            }
            tx.commit()?;
            Ok(())
        })
        .await?;
    }
    db.write(move |conn| {
        repo::carddav::set_sync_state(conn, book_id, ctag.as_deref(), token.as_deref(), now_ms())
    })
    .await?;
    Ok(changed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::carddav::http::{DavResponse, MockTransport};
    fn dav(body: &str) -> DavResponse {
        DavResponse {
            status: 207,
            body: body.into(),
            etag: None,
        }
    }

    #[tokio::test]
    async fn full_pull_imports_a_vcard() {
        let dir = tempfile::tempdir().unwrap();
        let db = Db::open(&dir.path().join("mail.db")).unwrap();
        db.write(|conn| {
            crate::db::testutil::seed_account(conn);
            repo::carddav::upsert_config(
                conn,
                &repo::carddav::CardDavConfig {
                    account_id: 1,
                    base_url: "https://dav.test/".into(),
                    username: "me".into(),
                    principal_url: None,
                    home_set_url: "https://dav.test/books/me/".into(),
                    enabled: true,
                    last_error: None,
                },
            )?;
            repo::carddav::upsert_addressbook(
                conn,
                1,
                "https://dav.test/books/me/default/",
                Some("Contacts"),
                false,
            )?;
            Ok(())
        })
        .await
        .unwrap();
        let state = r#"<d:multistatus xmlns:d="DAV:"><d:response><d:href>/books/me/default/</d:href><d:propstat><d:prop/><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response></d:multistatus>"#;
        let query = r#"<d:multistatus xmlns:d="DAV:"><d:response><d:href>/books/me/default/ada.vcf</d:href><d:propstat><d:prop><d:getetag>"1"</d:getetag></d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response></d:multistatus>"#;
        let card = r#"<d:multistatus xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:carddav"><d:response><d:href>/books/me/default/ada.vcf</d:href><d:propstat><d:prop><d:getetag>"1"</d:getetag><c:address-data>BEGIN:VCARD
VERSION:4.0
UID:ada
FN:Ada Lovelace
EMAIL:ada@example.test
END:VCARD</c:address-data></d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response></d:multistatus>"#;
        let transport = MockTransport::new(vec![dav(state), dav(query), dav(card)]);
        assert!(
            sync_account(&db, &EventBus::new(), &transport, 1)
                .await
                .unwrap()
        );
        let contacts = db
            .read(|conn| repo::contacts::list_records(conn, "", 10))
            .await
            .unwrap();
        assert_eq!(contacts[0].name, "Ada Lovelace");
        assert_eq!(contacts[0].account_ids, vec![1]);
        let seen = transport.seen.lock().unwrap();
        assert_eq!(seen[1].5.as_deref(), Some("1"));
        assert_eq!(seen[2].5.as_deref(), Some("0"));
    }

    #[tokio::test]
    async fn initial_sync_uses_an_empty_sync_token_when_supported() {
        let dir = tempfile::tempdir().unwrap();
        let db = Db::open(&dir.path().join("mail.db")).unwrap();
        db.write(|conn| {
            crate::db::testutil::seed_account(conn);
            repo::carddav::upsert_config(
                conn,
                &repo::carddav::CardDavConfig {
                    account_id: 1,
                    base_url: "https://dav.test/".into(),
                    username: "me".into(),
                    principal_url: None,
                    home_set_url: "https://dav.test/books/me/".into(),
                    enabled: true,
                    last_error: None,
                },
            )?;
            repo::carddav::upsert_addressbook(
                conn,
                1,
                "https://dav.test/books/me/default/",
                Some("Contacts"),
                false,
            )?;
            Ok(())
        })
        .await
        .unwrap();
        let state = r#"<d:multistatus xmlns:d="DAV:"><d:response><d:href>/books/me/default/</d:href><d:propstat><d:prop><d:sync-token>urn:sync:before</d:sync-token></d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response></d:multistatus>"#;
        let changes = r#"<d:multistatus xmlns:d="DAV:"><d:response><d:href>/books/me/default/ada.vcf</d:href><d:propstat><d:prop><d:getetag>&quot;1&quot;</d:getetag></d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response><d:sync-token>urn:sync:after</d:sync-token></d:multistatus>"#;
        let card = r#"<d:multistatus xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:carddav"><d:response><d:href>/books/me/default/ada.vcf</d:href><d:propstat><d:prop><d:getetag>&quot;1&quot;</d:getetag><c:address-data>BEGIN:VCARD
VERSION:3.0
UID:ada
N:;;;;
FN:Ada
EMAIL:ada@example.test
END:VCARD</c:address-data></d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response></d:multistatus>"#;
        let transport = MockTransport::new(vec![dav(state), dav(changes), dav(card)]);
        sync_account(&db, &EventBus::new(), &transport, 1)
            .await
            .unwrap();
        let seen = transport.seen.lock().unwrap();
        assert_eq!(seen[1].5.as_deref(), Some("0"));
        assert!(
            seen[1]
                .4
                .as_deref()
                .unwrap()
                .contains("<d:sync-token></d:sync-token>")
        );
        assert_eq!(seen[2].5.as_deref(), Some("0"));
        drop(seen);
        let (_, token) = db
            .read(|conn| repo::carddav::sync_state(conn, 1))
            .await
            .unwrap();
        assert_eq!(token.as_deref(), Some("urn:sync:after"));
    }

    #[tokio::test]
    async fn new_contact_uses_if_none_match_vcard3_and_uuid_uid() {
        let dir = tempfile::tempdir().unwrap();
        let db = Db::open(&dir.path().join("mail.db")).unwrap();
        db.write(|conn| {
            crate::db::testutil::seed_account(conn);
            repo::carddav::upsert_config(
                conn,
                &repo::carddav::CardDavConfig {
                    account_id: 1,
                    base_url: "https://dav.test/".into(),
                    username: "me".into(),
                    principal_url: None,
                    home_set_url: "https://dav.test/books/me/".into(),
                    enabled: true,
                    last_error: None,
                },
            )?;
            repo::carddav::upsert_addressbook(
                conn,
                1,
                "https://dav.test/books/me/default/",
                Some("Contacts"),
                false,
            )?;
            let record = vcard::parse(
                "BEGIN:VCARD\nVERSION:3.0\nUID:temporary\nN:;;;;\nFN:Ada\nEMAIL:ada@example.test\nEND:VCARD",
            )?;
            let saved = repo::contacts::save_record(conn, &record, now_ms())?;
            repo::carddav::attach_new_contact(conn, 1, saved.id)?;
            Ok(())
        })
        .await
        .unwrap();
        let transport = MockTransport::new(vec![DavResponse {
            status: 201,
            etag: Some("\"created\"".into()),
            body: String::new(),
        }]);
        push_dirty(&db, &transport, 1).await.unwrap();
        let seen = transport.seen.lock().unwrap();
        assert_eq!(seen[0].2, vec![("If-None-Match".into(), "*".into())]);
        let body = seen[0].4.as_deref().unwrap();
        assert!(body.contains("VERSION:3.0"));
        assert!(body.contains("UID:urn:uuid:"));
        assert!(body.contains("N:;;;;"));
    }

    #[tokio::test]
    async fn local_edit_uses_vcard_content_type_and_if_match() {
        let dir = tempfile::tempdir().unwrap();
        let db = Db::open(&dir.path().join("mail.db")).unwrap();
        db.write(|conn| {
            crate::db::testutil::seed_account(conn);
            repo::carddav::upsert_config(
                conn,
                &repo::carddav::CardDavConfig {
                    account_id: 1,
                    base_url: "https://dav.test/".into(),
                    username: "me".into(),
                    principal_url: None,
                    home_set_url: "https://dav.test/books/me/".into(),
                    enabled: true,
                    last_error: None,
                },
            )?;
            let book = repo::carddav::upsert_addressbook(
                conn,
                1,
                "https://dav.test/books/me/default/",
                Some("Contacts"),
                false,
            )?;
            let record = vcard::parse(
                "BEGIN:VCARD\nVERSION:4.0\nUID:ada\nFN:Ada\nEMAIL:ada@example.test\nEND:VCARD",
            )?;
            repo::carddav::upsert_remote(
                conn,
                1,
                book,
                "/books/me/default/ada.vcf",
                Some("\"old\""),
                "old",
                &record,
            )?;
            let id = repo::contacts::list_records(conn, "", 10)?[0].id;
            repo::carddav::mark_saved_contact_dirty(conn, id)?;
            Ok(())
        })
        .await
        .unwrap();
        let transport = MockTransport::new(vec![DavResponse {
            status: 204,
            etag: Some("\"new\"".into()),
            body: String::new(),
        }]);
        assert!(push_dirty(&db, &transport, 1).await.unwrap());
        let seen = transport.seen.lock().unwrap();
        assert_eq!(seen[0].0, "PUT");
        assert_eq!(seen[0].2, vec![("If-Match".into(), "\"old\"".into())]);
        assert_eq!(seen[0].3.as_deref(), Some("text/vcard; charset=utf-8"));
        assert!(
            seen[0]
                .4
                .as_deref()
                .unwrap()
                .contains("EMAIL;TYPE=PREF:ada@example.test")
        );
    }

    #[tokio::test]
    async fn etag_conflict_imports_the_server_version_immediately() {
        let dir = tempfile::tempdir().unwrap();
        let db = Db::open(&dir.path().join("mail.db")).unwrap();
        db.write(|conn| {
            crate::db::testutil::seed_account(conn);
            repo::carddav::upsert_config(
                conn,
                &repo::carddav::CardDavConfig {
                    account_id: 1,
                    base_url: "https://dav.test/".into(),
                    username: "me".into(),
                    principal_url: None,
                    home_set_url: "https://dav.test/books/me/".into(),
                    enabled: true,
                    last_error: None,
                },
            )?;
            let book = repo::carddav::upsert_addressbook(
                conn,
                1,
                "https://dav.test/books/me/default/",
                Some("Contacts"),
                false,
            )?;
            let record = vcard::parse(
                "BEGIN:VCARD\nVERSION:3.0\nUID:server-id\nN:;;;;\nFN:Local edit\nEMAIL:ada@example.test\nEND:VCARD",
            )?;
            repo::carddav::upsert_remote(
                conn,
                1,
                book,
                "/books/me/default/ada.vcf",
                Some("\"old\""),
                "BEGIN:VCARD\nVERSION:3.0\nUID:server-id\nN:;;;;\nFN:Before\nEMAIL:ada@example.test\nEND:VCARD",
                &record,
            )?;
            let id = repo::contacts::list_records(conn, "", 10)?[0].id;
            repo::carddav::mark_saved_contact_dirty(conn, id)?;
            Ok(())
        })
        .await
        .unwrap();
        let transport = MockTransport::new(vec![
            DavResponse {
                status: 412,
                ..Default::default()
            },
            DavResponse {
                status: 200,
                etag: Some("\"server\"".into()),
                body: "BEGIN:VCARD\r\nVERSION:3.0\r\nUID:server-id\r\nN:;;;;\r\nFN:Server version\r\nEMAIL:ada@example.test\r\nEND:VCARD\r\n".into(),
            },
        ]);
        assert!(push_dirty(&db, &transport, 1).await.unwrap());
        let contacts = db
            .read(|conn| repo::contacts::list_records(conn, "", 10))
            .await
            .unwrap();
        assert_eq!(contacts[0].name, "Server version");
        let seen = transport.seen.lock().unwrap();
        assert_eq!(seen[0].0, "PUT");
        assert_eq!(seen[1].0, "GET");
    }

    #[tokio::test]
    async fn failed_query_item_never_deletes_the_local_contact() {
        let dir = tempfile::tempdir().unwrap();
        let db = Db::open(&dir.path().join("mail.db")).unwrap();
        db.write(|conn| {
            crate::db::testutil::seed_account(conn);
            repo::carddav::upsert_config(
                conn,
                &repo::carddav::CardDavConfig {
                    account_id: 1,
                    base_url: "https://dav.test/".into(),
                    username: "me".into(),
                    principal_url: None,
                    home_set_url: "https://dav.test/books/me/".into(),
                    enabled: true,
                    last_error: None,
                },
            )?;
            let book = repo::carddav::upsert_addressbook(
                conn,
                1,
                "https://dav.test/books/me/default/",
                Some("Contacts"),
                false,
            )?;
            let card = "BEGIN:VCARD\nVERSION:3.0\nUID:ada\nN:;;;;\nFN:Ada\nEMAIL:ada@example.test\nEND:VCARD";
            let record = vcard::parse(card)?;
            repo::carddav::upsert_remote(
                conn,
                1,
                book,
                "/books/me/default/ada.vcf",
                Some("\"old\""),
                card,
                &record,
            )
        })
        .await
        .unwrap();
        let state = r#"<d:multistatus xmlns:d="DAV:"><d:response><d:href>/books/me/default/</d:href><d:propstat><d:prop/><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response></d:multistatus>"#;
        let failed = r#"<d:multistatus xmlns:d="DAV:"><d:response><d:href>/books/me/default/ada.vcf</d:href><d:status>HTTP/1.1 500 Server Error</d:status></d:response></d:multistatus>"#;
        let transport = MockTransport::new(vec![dav(state), dav(failed)]);
        assert!(
            sync_account(&db, &EventBus::new(), &transport, 1)
                .await
                .is_err()
        );
        let contacts = db
            .read(|conn| repo::contacts::list_records(conn, "", 10))
            .await
            .unwrap();
        assert_eq!(contacts.len(), 1);
        assert_eq!(contacts[0].email, "ada@example.test");
    }
}
