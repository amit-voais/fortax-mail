//! Small, defensive vCard 3.0/4.0 projection for the fields the contact UI owns.

use crate::error::Result;
use crate::models::ContactRecord;

const MAX_VCARD_BYTES: usize = 2 * 1024 * 1024;

pub fn ensure_size(input: &str) -> Result<()> {
    if input.len() > MAX_VCARD_BYTES {
        return Err(super::err("vCard exceeds the 2 MiB safety limit"));
    }
    Ok(())
}

fn unescape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut chars = value.chars();
    while let Some(ch) = chars.next() {
        if ch == '\\' {
            match chars.next() {
                Some('n' | 'N') => out.push('\n'),
                Some(ch) => out.push(ch),
                None => out.push('\\'),
            }
        } else {
            out.push(ch);
        }
    }
    out
}

fn escape(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('\n', "\\n")
        .replace(';', "\\;")
        .replace(',', "\\,")
}

fn split_escaped(value: &str, delimiter: char) -> Vec<String> {
    let mut parts = vec![String::new()];
    let mut escaped = false;
    for ch in value.chars() {
        if escaped {
            parts.last_mut().unwrap().push('\\');
            parts.last_mut().unwrap().push(ch);
            escaped = false;
        } else if ch == '\\' {
            escaped = true;
        } else if ch == delimiter {
            parts.push(String::new());
        } else {
            parts.last_mut().unwrap().push(ch);
        }
    }
    if escaped {
        parts.last_mut().unwrap().push('\\');
    }
    parts
}

fn fold_line(line: String) -> Vec<String> {
    const LIMIT: usize = 75;
    let mut lines = Vec::new();
    let mut current = String::new();
    for ch in line.chars() {
        if current.len() + ch.len_utf8() > LIMIT {
            lines.push(current);
            current = String::from(" ");
        }
        current.push(ch);
    }
    lines.push(current);
    lines
}

fn unfold(input: &str) -> Vec<String> {
    let normalized = input.replace("\r\n", "\n").replace('\r', "\n");
    let mut lines: Vec<String> = Vec::new();
    for line in normalized.split('\n') {
        if (line.starts_with(' ') || line.starts_with('\t')) && !lines.is_empty() {
            lines.last_mut().unwrap().push_str(&line[1..]);
        } else {
            lines.push(line.to_owned());
        }
    }
    let mut joined = Vec::<String>::new();
    for line in lines {
        let continues_quoted_printable = joined.last().is_some_and(|previous| {
            previous.ends_with('=')
                && previous
                    .split_once(':')
                    .is_some_and(|(head, _)| head.to_ascii_uppercase().contains("QUOTED-PRINTABLE"))
        });
        if continues_quoted_printable {
            joined.last_mut().unwrap().pop();
            joined.last_mut().unwrap().push_str(&line);
        } else {
            joined.push(line);
        }
    }
    joined
}

fn decoded_value(head: &str, raw: &str) -> String {
    if !head.to_ascii_uppercase().contains("QUOTED-PRINTABLE") {
        return raw.to_owned();
    }
    let bytes = raw.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'='
            && index + 2 < bytes.len()
            && let (Some(high), Some(low)) = (
                (bytes[index + 1] as char).to_digit(16),
                (bytes[index + 2] as char).to_digit(16),
            )
        {
            decoded.push(((high << 4) | low) as u8);
            index += 3;
        } else {
            decoded.push(bytes[index]);
            index += 1;
        }
    }
    let charset = head.split(';').find_map(|parameter| {
        let (name, value) = parameter.split_once('=')?;
        name.eq_ignore_ascii_case("CHARSET")
            .then(|| value.trim_matches('"'))
    });
    charset
        .and_then(|label| encoding_rs::Encoding::for_label(label.as_bytes()))
        .map(|encoding| encoding.decode(&decoded).0.into_owned())
        .unwrap_or_else(|| String::from_utf8_lossy(&decoded).into_owned())
}

fn property_name(line: &str) -> Option<&str> {
    let (head, _) = line.split_once(':')?;
    Some(
        head.split(';')
            .next()
            .unwrap_or(head)
            .rsplit('.')
            .next()
            .unwrap_or(head),
    )
}

fn render(lines: Vec<String>) -> String {
    lines
        .into_iter()
        .flat_map(fold_line)
        .collect::<Vec<_>>()
        .join("\r\n")
        + "\r\n"
}

fn serialized_lines(record: &ContactRecord, uid: &str, version: &str) -> Vec<String> {
    let email_property = if version == "3.0" {
        "EMAIL;TYPE=PREF"
    } else {
        "EMAIL;PREF=1"
    };
    let mut lines = vec![
        "BEGIN:VCARD".into(),
        format!("VERSION:{version}"),
        format!("UID:{}", escape(uid)),
        format!(
            "FN:{}",
            escape(if record.name.is_empty() {
                &record.email
            } else {
                &record.name
            })
        ),
        // N is mandatory in vCard 3.0. The app currently models one display
        // name rather than its structured components, so keep N empty and
        // preserve the full value in FN.
        "N:;;;;".into(),
        format!("{email_property}:{}", escape(&record.email)),
    ];
    for (name, value) in [
        ("TEL", &record.phone),
        ("ORG", &record.company),
        ("TITLE", &record.job_title),
        ("URL", &record.website),
        ("BDAY", &record.birthday),
        ("NOTE", &record.notes),
        ("CATEGORIES", &record.tags),
    ] {
        if !value.trim().is_empty() {
            lines.push(format!("{name}:{}", escape(value.trim())));
        }
    }
    if !record.postal_address.trim().is_empty() {
        lines.push(format!(
            "ADR:;;{};;;;",
            escape(record.postal_address.trim())
        ));
    }
    lines.push("END:VCARD".into());
    lines
}

pub fn parse(input: &str) -> Result<ContactRecord> {
    ensure_size(input)?;
    let lines = unfold(input);
    if lines
        .iter()
        .filter(|line| line.eq_ignore_ascii_case("BEGIN:VCARD"))
        .count()
        != 1
        || lines
            .iter()
            .filter(|line| line.eq_ignore_ascii_case("END:VCARD"))
            .count()
            != 1
    {
        return Err(super::err("resource is not a vCard"));
    }
    let version = lines.iter().find_map(|line| {
        let (head, value) = line.split_once(':')?;
        head.eq_ignore_ascii_case("VERSION").then(|| value.trim())
    });
    if !matches!(version, Some("3.0" | "4.0")) {
        return Err(super::err("vCard has no supported VERSION"));
    }
    let has_uid = lines.iter().any(|line| {
        line.split_once(':').is_some_and(|(head, value)| {
            head.split(';')
                .next()
                .is_some_and(|name| name.eq_ignore_ascii_case("UID"))
                && !value.trim().is_empty()
        })
    });
    let has_fn = lines.iter().any(|line| {
        line.split_once(':').is_some_and(|(head, value)| {
            head.split(';')
                .next()
                .is_some_and(|name| name.eq_ignore_ascii_case("FN"))
                && !value.trim().is_empty()
        })
    });
    if !has_uid || !has_fn {
        return Err(super::err("vCard is missing its required UID or FN"));
    }
    let mut record = ContactRecord {
        id: 0,
        name: String::new(),
        email: String::new(),
        phone: String::new(),
        company: String::new(),
        job_title: String::new(),
        website: String::new(),
        birthday: String::new(),
        postal_address: String::new(),
        notes: String::new(),
        tags: String::new(),
        is_favorite: false,
        interactions: 0,
        last_interacted: None,
        account_ids: Vec::new(),
        is_managed: true,
    };
    let mut emails: Vec<(bool, String)> = Vec::new();
    for line in lines {
        let Some((head, raw)) = line.split_once(':') else {
            continue;
        };
        let name = head
            .split(';')
            .next()
            .unwrap_or(head)
            .rsplit('.')
            .next()
            .unwrap_or(head)
            .to_ascii_uppercase();
        let decoded = decoded_value(head, raw);
        let value = unescape(&decoded).trim().to_owned();
        match name.as_str() {
            "FN" => record.name = value,
            "N" if record.name.is_empty() => {
                let parts = split_escaped(decoded.trim(), ';');
                record.name = [
                    parts.get(1).map(String::as_str).unwrap_or(""),
                    parts.first().map(String::as_str).unwrap_or(""),
                ]
                .into_iter()
                .filter(|v| !v.is_empty())
                .map(unescape)
                .collect::<Vec<_>>()
                .join(" ");
            }
            "EMAIL" if !value.is_empty() => {
                let preferred = head.to_ascii_uppercase().contains("PREF=1")
                    || head.to_ascii_uppercase().contains("TYPE=PREF");
                emails.push((preferred, value));
            }
            "TEL" if record.phone.is_empty() => record.phone = value,
            "ORG" => {
                record.company = split_escaped(decoded.trim(), ';')
                    .first()
                    .map(|value| unescape(value).trim().to_owned())
                    .unwrap_or_default()
            }
            "TITLE" => record.job_title = value,
            "URL" if record.website.is_empty() => record.website = value,
            "BDAY" => record.birthday = value,
            "ADR" => {
                record.postal_address = split_escaped(decoded.trim(), ';')
                    .into_iter()
                    .map(|value| unescape(&value).trim().to_owned())
                    .filter(|v| !v.is_empty())
                    .collect::<Vec<_>>()
                    .join(", ")
            }
            "NOTE" => record.notes = value,
            "CATEGORIES" => record.tags = value,
            _ => {}
        }
    }
    record.email = emails
        .iter()
        .find(|(preferred, _)| *preferred)
        .or_else(|| emails.first())
        .map(|(_, email)| email.trim().to_lowercase())
        .unwrap_or_default();
    if record.email.is_empty() || !record.email.contains('@') {
        return Err(super::err("vCard has no valid email address"));
    }
    if record.name.is_empty() {
        record.name = record.email.clone();
    }
    Ok(record)
}

pub fn serialize(record: &ContactRecord, uid: &str) -> String {
    // vCard 3.0 is CardDAV's mandatory baseline. A collection may advertise
    // v4 support, but new objects must remain interoperable when it does not.
    render(serialized_lines(record, uid, "3.0"))
}

pub fn new_uid() -> String {
    let mut bytes = rand::random::<[u8; 16]>();
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    format!(
        "urn:uuid:{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        bytes[0],
        bytes[1],
        bytes[2],
        bytes[3],
        bytes[4],
        bytes[5],
        bytes[6],
        bytes[7],
        bytes[8],
        bytes[9],
        bytes[10],
        bytes[11],
        bytes[12],
        bytes[13],
        bytes[14],
        bytes[15]
    )
}

/// Update the fields represented by `ContactRecord` while retaining properties
/// such as PHOTO, IMPP, custom extensions, and additional email/phone values.
pub fn update(existing: &str, record: &ContactRecord, fallback_uid: &str) -> String {
    if existing.len() > MAX_VCARD_BYTES {
        return serialize(record, fallback_uid);
    }
    let old = unfold(existing);
    if !old
        .iter()
        .any(|line| line.eq_ignore_ascii_case("BEGIN:VCARD"))
        || !old
            .iter()
            .any(|line| line.eq_ignore_ascii_case("END:VCARD"))
    {
        return serialize(record, fallback_uid);
    }

    let version = old
        .iter()
        .find_map(|line| {
            let (head, value) = line.split_once(':')?;
            head.eq_ignore_ascii_case("VERSION").then_some(value)
        })
        .filter(|version| matches!(*version, "3.0" | "4.0"))
        .unwrap_or("4.0");
    let uid = old
        .iter()
        .find_map(|line| {
            line.split_once(':')
                .filter(|(head, _)| head.eq_ignore_ascii_case("UID"))
        })
        .map(|(_, value)| value)
        .filter(|value| !value.is_empty())
        .unwrap_or(fallback_uid);

    let email_indexes = old
        .iter()
        .enumerate()
        .filter(|(_, line)| {
            property_name(line).is_some_and(|name| name.eq_ignore_ascii_case("EMAIL"))
        })
        .map(|(index, line)| (index, line.to_ascii_uppercase().contains("PREF")))
        .collect::<Vec<_>>();
    let primary_email = email_indexes
        .iter()
        .find(|(_, preferred)| *preferred)
        .or_else(|| email_indexes.first())
        .map(|(index, _)| *index);
    let first_tel = old
        .iter()
        .position(|line| property_name(line).is_some_and(|name| name.eq_ignore_ascii_case("TEL")));
    let first_url = old
        .iter()
        .position(|line| property_name(line).is_some_and(|name| name.eq_ignore_ascii_case("URL")));

    let mut lines = serialized_lines(record, uid, version);
    let end = lines.len() - 1;
    for (index, line) in old.into_iter().enumerate() {
        let Some(name) = property_name(&line) else {
            continue;
        };
        let preserve = match name.to_ascii_uppercase().as_str() {
            "BEGIN" | "END" | "VERSION" | "UID" | "FN" | "N" | "ORG" | "TITLE" | "BDAY" | "ADR"
            | "NOTE" | "CATEGORIES" => false,
            "EMAIL" => Some(index) != primary_email,
            "TEL" => Some(index) != first_tel,
            "URL" => Some(index) != first_url,
            _ => true,
        };
        if preserve {
            lines.insert(end, line);
        }
    }
    render(lines)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_folded_vcard_and_prefers_preferred_email() {
        let card = "BEGIN:VCARD\r\nVERSION:3.0\r\nUID:ada\r\nFN:Ada \r\n Lovelace\r\nEMAIL;TYPE=WORK:other@example.test\r\nEMAIL;PREF=1:ADA@example.test\r\nORG:Analytical Engines;Research\r\nADR:;;12 St James Sq;London;;;UK\r\nNOTE:Line one\\nLine two\r\nEND:VCARD\r\n";
        let parsed = parse(card).unwrap();
        assert_eq!(parsed.name, "Ada Lovelace");
        assert_eq!(parsed.email, "ada@example.test");
        assert_eq!(parsed.company, "Analytical Engines");
        assert!(parsed.postal_address.contains("London"));
    }

    #[test]
    fn serialized_card_round_trips_supported_fields() {
        let mut record =
            parse("BEGIN:VCARD\nVERSION:4.0\nUID:alice\nFN:Alice\nEMAIL:a@example.test\nEND:VCARD")
                .unwrap();
        record.notes = "one, two\nthree".into();
        let reparsed = parse(&serialize(&record, "contact-1@example.test")).unwrap();
        assert!(serialize(&record, "contact-1@example.test").contains("VERSION:3.0"));
        assert_eq!(reparsed.email, record.email);
        assert_eq!(reparsed.notes, record.notes);
    }

    #[test]
    fn structured_escapes_and_long_utf8_lines_round_trip() {
        let mut record =
            parse("BEGIN:VCARD\nVERSION:4.0\nUID:alice\nFN:Alice\nEMAIL:a@example.test\nEND:VCARD")
                .unwrap();
        record.company = "Research; Development".into();
        record.notes = "Résumé notes, ".repeat(12);
        let encoded = serialize(&record, "contact-2@example.test");
        assert!(encoded.contains("\r\n "));
        let reparsed = parse(&encoded).unwrap();
        assert_eq!(reparsed.company, record.company);
        assert_eq!(reparsed.notes, record.notes.trim());
    }

    #[test]
    fn update_preserves_unmodeled_and_additional_properties() {
        let original = "BEGIN:VCARD\r\nVERSION:3.0\r\nUID:server-id\r\nFN:Alice\r\nEMAIL;TYPE=PREF:old@example.test\r\nEMAIL;TYPE=HOME:other@example.test\r\nTEL;TYPE=CELL:111\r\nTEL;TYPE=HOME:222\r\nPHOTO;ENCODING=b:AAAB\r\nX-AB-LABEL:Friend\r\nEND:VCARD\r\n";
        let mut record = parse(original).unwrap();
        record.email = "new@example.test".into();
        record.phone = "333".into();
        let encoded = update(original, &record, "fallback");
        assert!(encoded.contains("VERSION:3.0"));
        assert!(encoded.contains("UID:server-id"));
        assert!(encoded.contains("EMAIL;TYPE=PREF:new@example.test"));
        assert!(encoded.contains("EMAIL;TYPE=HOME:other@example.test"));
        assert!(encoded.contains("TEL;TYPE=HOME:222"));
        assert!(encoded.contains("PHOTO;ENCODING=b:AAAB"));
        assert!(encoded.contains("X-AB-LABEL:Friend"));
    }

    #[test]
    fn generated_uid_is_an_rfc_4122_version_4_urn() {
        let uid = new_uid();
        assert_eq!(uid.len(), 45);
        assert!(uid.starts_with("urn:uuid:"));
        assert_eq!(&uid[23..24], "4");
        assert!(matches!(&uid[28..29], "8" | "9" | "a" | "b"));
    }

    #[test]
    fn parses_vcard3_quoted_printable_and_escaped_structured_name() {
        let card = "BEGIN:VCARD\r\nVERSION:3.0\r\nUID:andre\r\nFN;CHARSET=ISO-8859-1;ENCODING=QUOTED-PRINTABLE:Andr=E9 Doe\\;Sr.\r\nN;CHARSET=ISO-8859-1;ENCODING=QUOTED-PRINTABLE:Doe\\;Sr.;Andr=E9\r\nEMAIL:andre@example.test\r\nNOTE;ENCODING=QUOTED-PRINTABLE:Line=20one=\r\n=0ALine=20two\r\nEND:VCARD\r\n";
        let parsed = parse(card).unwrap();
        assert_eq!(parsed.name, "André Doe;Sr.");
        assert_eq!(parsed.notes, "Line one\nLine two");
    }

    #[test]
    fn rejects_multiple_cards_and_missing_required_properties() {
        assert!(parse("BEGIN:VCARD\nVERSION:3.0\nFN:A\nEMAIL:a@example.test\nEND:VCARD").is_err());
        assert!(
            parse("BEGIN:VCARD\nVERSION:2.1\nUID:a\nFN:A\nEMAIL:a@example.test\nEND:VCARD")
                .is_err()
        );
        assert!(parse("BEGIN:VCARD\nVERSION:3.0\nUID:a\nFN:A\nEMAIL:a@example.test\nEND:VCARD\nBEGIN:VCARD\nVERSION:3.0\nUID:b\nFN:B\nEMAIL:b@example.test\nEND:VCARD").is_err());
    }
}
