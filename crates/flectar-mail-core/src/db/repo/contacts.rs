use crate::error::Result;
use crate::models::{
    Address, ContactRecord, ContactRecordCursor, ContactRecordPage, ContactSuggestion,
};
use crate::search::fold;
use rusqlite::{Connection, OptionalExtension, Row, params};

fn record_from_row(row: &Row<'_>) -> rusqlite::Result<ContactRecord> {
    let account_ids = row
        .get::<_, String>(15)?
        .split(',')
        .filter_map(|value| value.parse::<i64>().ok())
        .collect();
    Ok(ContactRecord {
        id: row.get(0)?,
        name: row.get(1)?,
        email: row.get(2)?,
        phone: row.get(3)?,
        company: row.get(4)?,
        job_title: row.get(5)?,
        website: row.get(6)?,
        birthday: row.get(7)?,
        postal_address: row.get(8)?,
        notes: row.get(9)?,
        tags: row.get(10)?,
        is_favorite: row.get(11)?,
        interactions: row.get(12)?,
        last_interacted: row.get(13)?,
        is_managed: row.get::<_, i64>(14)? != 0,
        account_ids,
    })
}

/// Record an address seen in mail headers on `account_id`'s mail. `sent` = we
/// sent to them. Updates both the global `contacts` row (identity + global
/// affinity used by search and sender_known) and the per-account
/// `contact_accounts` row that scopes compose autocomplete to the sending
/// account.
pub fn harvest(
    conn: &Connection,
    account_id: i64,
    addr: &Address,
    sent: bool,
    when_ms: i64,
) -> Result<()> {
    if addr.email.is_empty() || !addr.email.contains('@') {
        return Ok(());
    }
    let email = addr.email.to_lowercase();
    let name = addr.name.as_deref().unwrap_or("");
    let folded = fold(&format!("{} {}", name, addr.email));
    conn.execute(
        "INSERT INTO contacts (email, name, folded, send_count, recv_count, last_interacted)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT(email) DO UPDATE SET
            name = CASE WHEN contacts.is_managed = 1 THEN contacts.name
                   ELSE COALESCE(NULLIF(excluded.name, ''), contacts.name) END,
            folded = CASE
                WHEN contacts.is_managed = 0 AND NULLIF(excluded.name, '') IS NOT NULL
                OR contacts.folded IS NULL
                THEN excluded.folded ELSE contacts.folded END,
            send_count = contacts.send_count + ?4,
            recv_count = contacts.recv_count + ?5,
            last_interacted = MAX(COALESCE(contacts.last_interacted, 0), ?6)",
        params![email, name, folded, sent as i64, (!sent) as i64, when_ms],
    )?;
    conn.execute(
        "INSERT INTO contact_accounts (contact_id, account_id, send_count, recv_count, last_interacted)
         SELECT id, ?2, ?3, ?4, ?5 FROM contacts WHERE email = ?1
         ON CONFLICT(contact_id, account_id) DO UPDATE SET
            send_count = contact_accounts.send_count + ?3,
            recv_count = contact_accounts.recv_count + ?4,
            last_interacted = MAX(COALESCE(contact_accounts.last_interacted, 0), ?5)",
        params![email, account_id, sent as i64, (!sent) as i64, when_ms],
    )?;
    Ok(())
}

/// One-time fill of `contacts.folded` for rows harvested before the column
/// existed. Cheap no-op once every row is folded.
pub fn backfill_folded(conn: &Connection) -> Result<()> {
    loop {
        let rows = {
            let mut stmt = conn.prepare(
                "SELECT id, COALESCE(name,''), email FROM contacts WHERE folded IS NULL LIMIT 256",
            )?;
            stmt.query_map([], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?
        };
        if rows.is_empty() {
            break;
        }
        let tx = conn.unchecked_transaction()?;
        for (id, name, email) in rows {
            tx.execute(
                "UPDATE contacts SET folded = ?1 WHERE id = ?2",
                params![fold(&format!("{name} {email}")), id],
            )?;
        }
        tx.commit()?;
    }
    Ok(())
}

/// Build the WHERE fragment requiring every folded query token to appear in
/// `contacts.folded`, pushing one `%tok%` bind per token. Returns None for
/// queries with no usable tokens.
fn folded_clauses(query: &str, bind: &mut Vec<Box<dyn rusqlite::types::ToSql>>) -> Option<String> {
    let folded = fold(query);
    let tokens: Vec<&str> = folded.split_whitespace().collect();
    if tokens.is_empty() {
        return None;
    }
    let mut clauses = Vec::with_capacity(tokens.len());
    for tok in tokens {
        // Escape LIKE wildcards so a literal % or _ in the query can't scan-match.
        let esc = tok
            .replace('\\', "\\\\")
            .replace('%', "\\%")
            .replace('_', "\\_");
        bind.push(Box::new(format!("%{esc}%")));
        clauses.push(format!(
            "LOWER(COALESCE(folded, email) || ' ' || COALESCE(job_title, '') || ' ' ||
                   COALESCE(website, '') || ' ' || COALESCE(postal_address, ''))
             LIKE ?{} ESCAPE '\\'",
            bind.len()
        ));
    }
    Some(clauses.join(" AND "))
}

fn record_where_clause(
    query: &str,
    account_id: Option<i64>,
    favorites_only: bool,
    bind: &mut Vec<Box<dyn rusqlite::types::ToSql>>,
) -> String {
    let mut clauses = Vec::new();
    if let Some(query_clause) = folded_clauses(query, bind) {
        clauses.push(format!("({query_clause})"));
    }
    if favorites_only {
        clauses.push("is_favorite = 1".to_owned());
    }
    if let Some(account_id) = account_id {
        bind.push(Box::new(account_id));
        clauses.push(format!(
            "(is_managed = 1 OR EXISTS (
                SELECT 1 FROM contact_accounts ca
                WHERE ca.contact_id = contacts.id AND ca.account_id = ?{}
            ))",
            bind.len()
        ));
    }
    if clauses.is_empty() {
        "1 = 1".to_owned()
    } else {
        clauses.join(" AND ")
    }
}

fn count_records(
    conn: &Connection,
    query: &str,
    account_id: Option<i64>,
    favorites_only: bool,
) -> Result<usize> {
    let mut bind: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
    let where_sql = record_where_clause(query, account_id, favorites_only, &mut bind);
    let sql = format!("SELECT COUNT(*) FROM contacts WHERE {where_sql}");
    let params_ref = bind
        .iter()
        .map(|value| value.as_ref())
        .collect::<Vec<&dyn rusqlite::types::ToSql>>();
    let count = conn.query_row(&sql, params_ref.as_slice(), |row| row.get::<_, i64>(0))?;
    Ok(count.max(0) as usize)
}

/// Contacts matching every query token (accent- and case-insensitive), ranked
/// by interaction affinity - people you actually email float to the top. When
/// `account_id` is Some, only contacts that account has corresponded with are
/// returned, ranked by that account's affinity; None searches all contacts.
pub fn suggest(
    conn: &Connection,
    query: &str,
    account_id: Option<i64>,
    limit: i64,
) -> Result<Vec<ContactSuggestion>> {
    let mut bind: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
    let Some(where_sql) = folded_clauses(query, &mut bind) else {
        return Ok(Vec::new());
    };
    // `contact_accounts` has no name/email/folded columns, so the folded WHERE
    // clause stays unambiguous; only the affinity columns get an alias.
    let sql = if let Some(aid) = account_id {
        bind.push(Box::new(aid));
        let aid_ix = bind.len();
        bind.push(Box::new(limit));
        format!(
            "SELECT c.name, c.email,
                    COALESCE(ca.send_count * 3 + ca.recv_count,
                             c.send_count * 3 + c.recv_count)
             FROM contacts c
             LEFT JOIN contact_accounts ca
               ON ca.contact_id = c.id AND ca.account_id = ?{aid_ix}
             WHERE ({where_sql}) AND (ca.account_id IS NOT NULL OR c.is_managed = 1)
             ORDER BY COALESCE(ca.send_count * 3 + ca.recv_count,
                               c.send_count * 3 + c.recv_count) DESC,
                      COALESCE(ca.last_interacted, c.last_interacted) DESC
             LIMIT ?{}",
            bind.len()
        )
    } else {
        bind.push(Box::new(limit));
        format!(
            "SELECT name, email, send_count * 3 + recv_count FROM contacts
             WHERE {where_sql}
             ORDER BY (send_count * 3 + recv_count) DESC, last_interacted DESC
             LIMIT ?{}",
            bind.len()
        )
    };
    let mut stmt = conn.prepare(&sql)?;
    let params_ref: Vec<&dyn rusqlite::types::ToSql> = bind.iter().map(|b| b.as_ref()).collect();
    let rows = stmt
        .query_map(params_ref.as_slice(), |r| {
            Ok(ContactSuggestion {
                name: r.get(0)?,
                email: r.get(1)?,
                interactions: r.get(2)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

pub fn autocomplete(
    conn: &Connection,
    prefix: &str,
    account_id: Option<i64>,
    limit: i64,
) -> Result<Vec<Address>> {
    Ok(suggest(conn, prefix, account_id, limit)?
        .into_iter()
        .map(|c| Address {
            name: c.name,
            email: c.email,
        })
        .collect())
}

/// List address-book records for the dedicated contacts workspace. An empty
/// query returns the full directory; otherwise every folded token must match.
pub fn list_records(conn: &Connection, query: &str, limit: i64) -> Result<Vec<ContactRecord>> {
    let mut bind: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
    let where_sql = folded_clauses(query, &mut bind).unwrap_or_else(|| "1 = 1".to_owned());
    bind.push(Box::new(limit.clamp(1, 500)));
    let sql = format!(
        "SELECT id, COALESCE(name, ''), email, phone, company, job_title,
                website, birthday, postal_address, notes, tags, is_favorite,
                send_count * 3 + recv_count, last_interacted, is_managed,
                COALESCE((SELECT GROUP_CONCAT(ca.account_id)
                          FROM contact_accounts ca
                          WHERE ca.contact_id = contacts.id), '')
         FROM contacts
         WHERE {where_sql}
         ORDER BY is_favorite DESC,
                  CASE WHEN name IS NULL OR name = '' THEN email ELSE name END COLLATE NOCASE,
                  email COLLATE NOCASE,
                  id ASC
         LIMIT ?{}",
        bind.len()
    );
    let params_ref: Vec<&dyn rusqlite::types::ToSql> = bind.iter().map(|b| b.as_ref()).collect();
    let mut stmt = conn.prepare(&sql)?;
    Ok(stmt
        .query_map(params_ref.as_slice(), record_from_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?)
}

/// Query one strict directory page and the scalar counts needed by the
/// sidebar/status line. The keyset cursor follows the complete sort tuple, so
/// contacts inserted before the cursor cannot shift or duplicate later pages.
pub fn list_record_page(
    conn: &Connection,
    query: &str,
    account_id: Option<i64>,
    favorites_only: bool,
    cursor: Option<&ContactRecordCursor>,
    limit: i64,
) -> Result<ContactRecordPage> {
    let limit = limit.clamp(1, 100);
    let matching_count = count_records(conn, query, account_id, favorites_only)?;

    let mut bind: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
    let mut where_sql = record_where_clause(query, account_id, favorites_only, &mut bind);
    if let Some(cursor) = cursor {
        bind.push(Box::new(i64::from(cursor.is_favorite)));
        let favorite_index = bind.len();
        bind.push(Box::new(cursor.sort_name.clone()));
        let name_index = bind.len();
        bind.push(Box::new(cursor.email.clone()));
        let email_index = bind.len();
        bind.push(Box::new(cursor.id));
        let id_index = bind.len();
        let sort_name = "CASE WHEN name IS NULL OR name = '' THEN email ELSE name END";
        where_sql.push_str(&format!(
            " AND (
                is_favorite < ?{favorite_index}
                OR (is_favorite = ?{favorite_index} AND (
                    {sort_name} COLLATE NOCASE > ?{name_index} COLLATE NOCASE
                    OR ({sort_name} COLLATE NOCASE = ?{name_index} COLLATE NOCASE AND (
                        email COLLATE NOCASE > ?{email_index} COLLATE NOCASE
                        OR (email COLLATE NOCASE = ?{email_index} COLLATE NOCASE
                            AND id > ?{id_index})
                    ))
                ))
            )"
        ));
    }
    bind.push(Box::new(limit + 1));
    let limit_index = bind.len();
    let sql = format!(
        "SELECT id, COALESCE(name, ''), email, phone, company, job_title,
                website, birthday, postal_address, notes, tags, is_favorite,
                send_count * 3 + recv_count, last_interacted, is_managed,
                COALESCE((SELECT GROUP_CONCAT(ca.account_id)
                          FROM contact_accounts ca
                          WHERE ca.contact_id = contacts.id), '')
         FROM contacts
         WHERE {where_sql}
         ORDER BY is_favorite DESC,
                  CASE WHEN name IS NULL OR name = '' THEN email ELSE name END COLLATE NOCASE,
                  email COLLATE NOCASE,
                  id ASC
         LIMIT ?{limit_index}"
    );
    let params_ref = bind
        .iter()
        .map(|value| value.as_ref())
        .collect::<Vec<&dyn rusqlite::types::ToSql>>();
    let mut stmt = conn.prepare(&sql)?;
    let mut records = stmt
        .query_map(params_ref.as_slice(), record_from_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let has_more = records.len() > limit as usize;
    if has_more {
        records.truncate(limit as usize);
    }

    let (total_count, favorite_count) = conn.query_row(
        "SELECT COUNT(*), COALESCE(SUM(is_favorite), 0) FROM contacts",
        [],
        |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
    )?;
    let mut account_stmt = conn.prepare(
        "WITH managed(value) AS (
             SELECT COUNT(*) FROM contacts WHERE is_managed = 1
         ), discovered(account_id, value) AS (
             SELECT ca.account_id, COUNT(DISTINCT ca.contact_id)
             FROM contact_accounts ca
             JOIN contacts c ON c.id = ca.contact_id
             WHERE c.is_managed = 0
             GROUP BY ca.account_id
         )
         SELECT a.id, managed.value + COALESCE(discovered.value, 0)
         FROM accounts a
         CROSS JOIN managed
         LEFT JOIN discovered ON discovered.account_id = a.id",
    )?;
    let account_counts = account_stmt
        .query_map([], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?.max(0) as usize))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let next_cursor = has_more.then(|| {
        let last = records
            .last()
            .expect("a page with a look-ahead row has a retained row");
        ContactRecordCursor {
            is_favorite: last.is_favorite,
            sort_name: if last.name.is_empty() {
                last.email.clone()
            } else {
                last.name.clone()
            },
            email: last.email.clone(),
            id: last.id,
        }
    });

    Ok(ContactRecordPage {
        next_cursor,
        records,
        matching_count,
        total_count: total_count.max(0) as usize,
        favorite_count: favorite_count.max(0) as usize,
        account_counts,
    })
}

/// Insert or update a user-managed contact and return the canonical stored row.
pub fn save_record(
    conn: &Connection,
    record: &ContactRecord,
    now_ms: i64,
) -> Result<ContactRecord> {
    let email = record.email.trim().to_lowercase();
    let name = record.name.trim();
    if email.is_empty() || !email.contains('@') {
        return Err(crate::error::CoreError::Other(
            "contact email address is invalid".into(),
        ));
    }
    let folded = fold(&format!(
        "{name} {email} {} {} {} {} {} {}",
        record.company,
        record.phone,
        record.job_title,
        record.website,
        record.tags,
        record.postal_address,
    ));
    if record.id > 0 {
        let changed = conn.execute(
            "UPDATE contacts SET name = ?1, email = ?2, folded = ?3, phone = ?4,
                    company = ?5, job_title = ?6, website = ?7, birthday = ?8,
                    postal_address = ?9, notes = ?10, tags = ?11, is_favorite = ?12,
                    is_managed = 1, updated_at = ?13
             WHERE id = ?14",
            params![
                name,
                email,
                folded,
                record.phone.trim(),
                record.company.trim(),
                record.job_title.trim(),
                record.website.trim(),
                record.birthday.trim(),
                record.postal_address.trim(),
                record.notes.trim(),
                record.tags.trim(),
                record.is_favorite,
                now_ms,
                record.id
            ],
        )?;
        if changed == 0 {
            return Err(crate::error::CoreError::Other("contact not found".into()));
        }
    } else {
        conn.execute(
            "INSERT INTO contacts (
                email, name, folded, phone, company, job_title, website, birthday,
                postal_address, notes, tags, is_favorite, is_managed, updated_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, 1, ?13)
             ON CONFLICT(email) DO UPDATE SET
                name = excluded.name,
                folded = excluded.folded,
                phone = excluded.phone,
                company = excluded.company,
                job_title = excluded.job_title,
                website = excluded.website,
                birthday = excluded.birthday,
                postal_address = excluded.postal_address,
                notes = excluded.notes,
                tags = excluded.tags,
                is_favorite = excluded.is_favorite,
                is_managed = 1,
                updated_at = excluded.updated_at",
            params![
                email,
                name,
                folded,
                record.phone.trim(),
                record.company.trim(),
                record.job_title.trim(),
                record.website.trim(),
                record.birthday.trim(),
                record.postal_address.trim(),
                record.notes.trim(),
                record.tags.trim(),
                record.is_favorite,
                now_ms
            ],
        )?;
    }
    let id = if record.id > 0 {
        record.id
    } else {
        conn.query_row(
            "SELECT id FROM contacts WHERE email = ?1 COLLATE NOCASE",
            params![email],
            |row| row.get(0),
        )?
    };
    let mut saved = conn.query_row(
        "SELECT id, COALESCE(name, ''), email, phone, company, job_title,
                website, birthday, postal_address, notes, tags, is_favorite,
                send_count * 3 + recv_count, last_interacted, is_managed,
                COALESCE((SELECT GROUP_CONCAT(ca.account_id)
                          FROM contact_accounts ca
                          WHERE ca.contact_id = contacts.id), '')
         FROM contacts WHERE id = ?1",
        [id],
        record_from_row,
    )?;
    saved.email = email;
    Ok(saved)
}

pub fn delete_record(conn: &Connection, id: i64) -> Result<()> {
    if conn.execute("DELETE FROM contacts WHERE id = ?1", params![id])? == 0 {
        return Err(crate::error::CoreError::Other("contact not found".into()));
    }
    Ok(())
}

pub fn get_record(conn: &Connection, id: i64) -> Result<Option<ContactRecord>> {
    conn.query_row(
        "SELECT id, COALESCE(name, ''), email, phone, company, job_title,
                website, birthday, postal_address, notes, tags, is_favorite,
                send_count * 3 + recv_count, last_interacted, is_managed,
                COALESCE((SELECT GROUP_CONCAT(ca.account_id)
                          FROM contact_accounts ca
                          WHERE ca.contact_id = contacts.id), '')
         FROM contacts WHERE id = ?1",
        [id],
        record_from_row,
    )
    .optional()
    .map_err(Into::into)
}

/// Affinity score (send_count*3 + recv_count) per email, for the given
/// lowercase addresses. Used to personalize search ranking.
pub fn affinity_for(
    conn: &Connection,
    emails: &[String],
) -> Result<std::collections::HashMap<String, i64>> {
    let mut out = std::collections::HashMap::new();
    if emails.is_empty() {
        return Ok(out);
    }
    let placeholders = (1..=emails.len())
        .map(|i| format!("?{i}"))
        .collect::<Vec<_>>()
        .join(",");
    let sql = format!(
        "SELECT email, send_count * 3 + recv_count FROM contacts WHERE email IN ({placeholders})"
    );
    let mut stmt = conn.prepare(&sql)?;
    let params_ref: Vec<&dyn rusqlite::types::ToSql> = emails
        .iter()
        .map(|e| e as &dyn rusqlite::types::ToSql)
        .collect();
    let rows = stmt.query_map(params_ref.as_slice(), |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
    })?;
    for row in rows {
        let (email, score) = row?;
        out.insert(email, score);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::testutil;
    use crate::models::Address;

    fn addr(email: &str, name: Option<&str>) -> Address {
        Address {
            name: name.map(str::to_string),
            email: email.into(),
        }
    }

    #[test]
    fn harvest_counts_and_autocomplete() {
        let c = testutil::conn();
        testutil::seed_account(&c);
        harvest(&c, 1, &addr("alice@acme.com", Some("Alice")), true, 100).unwrap();
        harvest(&c, 1, &addr("alice@acme.com", None), true, 200).unwrap();
        harvest(&c, 1, &addr("bob@other.org", Some("Bob")), false, 150).unwrap();

        let (send, recv): (i64, i64) = c
            .query_row(
                "SELECT send_count, recv_count FROM contacts WHERE email = 'alice@acme.com'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!((send, recv), (2, 0));

        let hits = autocomplete(&c, "ali", None, 10).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].email, "alice@acme.com");
        // harvested name survives even when a later sighting had none
        assert_eq!(hits[0].name.as_deref(), Some("Alice"));

        assert!(autocomplete(&c, "zzz", None, 10).unwrap().is_empty());
    }

    #[test]
    fn autocomplete_scopes_to_account() {
        let c = testutil::conn();
        testutil::seed_account(&c);
        c.execute(
            "INSERT INTO accounts (id, email, provider, auth_kind, username,
             imap_host, imap_port, smtp_host, smtp_port, created_at)
             VALUES (2,'two@test.dev','imap','password','two','h',993,'h',587,0)",
            [],
        )
        .unwrap();
        // alice belongs to account 1, carol to account 2.
        harvest(&c, 1, &addr("alice@acme.com", Some("Alice")), true, 100).unwrap();
        harvest(&c, 2, &addr("carol@acme.com", Some("Carol")), true, 100).unwrap();

        // Account-scoped: each account only sees its own contact.
        let a1 = autocomplete(&c, "a", Some(1), 10).unwrap();
        assert_eq!(
            a1.iter().map(|h| h.email.as_str()).collect::<Vec<_>>(),
            ["alice@acme.com"]
        );
        let a2 = autocomplete(&c, "a", Some(2), 10).unwrap();
        assert_eq!(
            a2.iter().map(|h| h.email.as_str()).collect::<Vec<_>>(),
            ["carol@acme.com"]
        );

        // Global (view-all): both surface.
        let all = autocomplete(&c, "a", None, 10).unwrap();
        assert_eq!(all.len(), 2);
    }

    #[test]
    fn managed_contact_round_trip_and_harvest_preserves_edited_name() {
        let c = testutil::conn();
        testutil::seed_account(&c);
        let created = save_record(
            &c,
            &ContactRecord {
                id: 0,
                name: "Ada Lovelace".into(),
                email: "ADA@EXAMPLE.COM".into(),
                phone: "+44 20 0000 0000".into(),
                company: "Analytical Engines".into(),
                job_title: "Programmer".into(),
                website: "https://example.com/ada".into(),
                birthday: "1815-12-10".into(),
                postal_address: "London".into(),
                notes: "Prefers written updates.".into(),
                tags: "vip, history".into(),
                is_favorite: true,
                interactions: 0,
                last_interacted: None,
                account_ids: Vec::new(),
                is_managed: true,
            },
            100,
        )
        .unwrap();
        assert!(created.id > 0);
        assert_eq!(created.email, "ada@example.com");
        assert!(created.is_favorite);
        assert_eq!(
            autocomplete(&c, "ada", Some(1), 20).unwrap()[0].email,
            "ada@example.com",
            "manually managed contacts remain available to account-scoped compose"
        );

        harvest(
            &c,
            1,
            &addr("ada@example.com", Some("Automated Header Name")),
            false,
            200,
        )
        .unwrap();
        let listed = list_records(&c, "analytical vip", 20).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].name, "Ada Lovelace");
        assert_eq!(listed[0].interactions, 1);
        assert_eq!(listed[0].account_ids, vec![1]);
        assert!(listed[0].is_managed);

        delete_record(&c, created.id).unwrap();
        assert!(list_records(&c, "", 20).unwrap().is_empty());
    }

    #[test]
    fn directory_pages_are_strict_non_overlapping_batches() {
        let c = testutil::conn();
        testutil::seed_account(&c);
        for index in 0..61 {
            harvest(
                &c,
                1,
                &addr(
                    &format!("person-{index:02}@example.com"),
                    Some(&format!("Person {index:02}")),
                ),
                false,
                index,
            )
            .unwrap();
        }

        let first = list_record_page(&c, "", None, false, None, 25).unwrap();
        harvest(
            &c,
            1,
            &addr("aardvark@example.com", Some("Aardvark")),
            false,
            100,
        )
        .unwrap();
        let second = list_record_page(&c, "", None, false, first.next_cursor.as_ref(), 25).unwrap();
        let third = list_record_page(&c, "", None, false, second.next_cursor.as_ref(), 25).unwrap();
        assert_eq!(first.records.len(), 25);
        assert_eq!(second.records.len(), 25);
        assert_eq!(third.records.len(), 11);
        assert!(first.next_cursor.is_some());
        assert!(second.next_cursor.is_some());
        assert!(third.next_cursor.is_none());
        assert_eq!(first.matching_count, 61);

        let ids = first
            .records
            .iter()
            .chain(&second.records)
            .chain(&third.records)
            .map(|contact| contact.id)
            .collect::<std::collections::HashSet<_>>();
        assert_eq!(ids.len(), 61);
        assert!(
            first
                .records
                .iter()
                .chain(&second.records)
                .chain(&third.records)
                .all(|contact| contact.email != "aardvark@example.com"),
            "the newly inserted head belongs before the cursor"
        );
        assert_eq!(first.account_counts, vec![(1, 61)]);
    }
}
