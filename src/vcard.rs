//! Minimal vCard 3.0/4.0 parsing and serialization, covering just the
//! properties this server cares about for contact management (FN, N,
//! EMAIL, TEL, ORG, NOTE, UID).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone)]
pub struct Property {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone, Default)]
pub struct Vcard {
    pub properties: Vec<Property>,
}

/// Un-fold CRLF/LF-wrapped vCard content into logical lines. Continuation
/// lines start with a space or tab per RFC 6350 section 3.2.
fn unfold(input: &str) -> Vec<String> {
    let normalized = input.replace("\r\n", "\n");
    let mut lines: Vec<String> = Vec::new();
    for raw_line in normalized.split('\n') {
        if (raw_line.starts_with(' ') || raw_line.starts_with('\t')) && !lines.is_empty() {
            let last = lines.last_mut().unwrap();
            last.push_str(&raw_line[1..]);
        } else if !raw_line.is_empty() {
            lines.push(raw_line.to_string());
        }
    }
    lines
}

fn unescape_value(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut chars = value.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('n') | Some('N') => out.push('\n'),
                Some(',') => out.push(','),
                Some(';') => out.push(';'),
                Some('\\') => out.push('\\'),
                Some(other) => out.push(other),
                None => {}
            }
        } else {
            out.push(c);
        }
    }
    out
}

fn escape_value(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('\n', "\\n")
        .replace(',', "\\,")
        .replace(';', "\\;")
}

/// Split a raw property line into "name+params" and "value", respecting
/// that a ':' inside a quoted parameter value doesn't end the name part.
fn split_name_and_value(line: &str) -> Option<(&str, &str)> {
    let mut in_quotes = false;
    for (i, c) in line.char_indices() {
        match c {
            '"' => in_quotes = !in_quotes,
            ':' if !in_quotes => return Some((&line[..i], &line[i + 1..])),
            _ => {}
        }
    }
    None
}

pub fn parse(text: &str) -> anyhow::Result<Vcard> {
    let mut properties = Vec::new();
    for line in unfold(text) {
        let upper_trim = line.trim();
        if upper_trim.eq_ignore_ascii_case("BEGIN:VCARD")
            || upper_trim.eq_ignore_ascii_case("END:VCARD")
        {
            continue;
        }
        let Some((head, raw_value)) = split_name_and_value(&line) else {
            continue;
        };
        let name_part = head.split(';').next().unwrap_or_default();
        // Strip a "GROUP." prefix if present.
        let name = match name_part.rsplit_once('.') {
            Some((_, n)) => n.to_ascii_uppercase(),
            None => name_part.to_ascii_uppercase(),
        };
        properties.push(Property {
            name,
            value: unescape_value(raw_value),
        });
    }
    Ok(Vcard { properties })
}

impl Vcard {
    pub fn get(&self, name: &str) -> Option<&Property> {
        self.properties.iter().find(|p| p.name == name)
    }

    pub fn get_all(&self, name: &str) -> Vec<&Property> {
        self.properties.iter().filter(|p| p.name == name).collect()
    }
}

/// A contact as exposed by the API, with a flattened structure that's
/// convenient for JSON clients (rather than raw vCard properties).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Contact {
    pub uid: String,
    pub full_name: String,
    #[serde(default)]
    pub first_name: Option<String>,
    #[serde(default)]
    pub last_name: Option<String>,
    #[serde(default)]
    pub emails: Vec<String>,
    #[serde(default)]
    pub phones: Vec<String>,
    #[serde(default)]
    pub org: Option<String>,
    #[serde(default)]
    pub note: Option<String>,
}

/// Fields accepted when creating or updating a contact. The uid is either
/// generated server-side (create) or taken from the URL path (update).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContactInput {
    pub full_name: String,
    #[serde(default)]
    pub first_name: Option<String>,
    #[serde(default)]
    pub last_name: Option<String>,
    #[serde(default)]
    pub emails: Vec<String>,
    #[serde(default)]
    pub phones: Vec<String>,
    #[serde(default)]
    pub org: Option<String>,
    #[serde(default)]
    pub note: Option<String>,
}

pub fn contact_from_vcard(uid: &str, v: &Vcard) -> Contact {
    let full_name = v
        .get("FN")
        .map(|p| p.value.clone())
        .unwrap_or_else(|| uid.to_string());

    let (first_name, last_name) = match v.get("N") {
        Some(p) => {
            // N is Family;Given;Additional;Prefix;Suffix
            let fields: Vec<&str> = p.value.split(';').collect();
            let last = fields.first().filter(|s| !s.is_empty()).map(|s| s.to_string());
            let first = fields.get(1).filter(|s| !s.is_empty()).map(|s| s.to_string());
            (first, last)
        }
        None => (None, None),
    };

    let emails = v.get_all("EMAIL").into_iter().map(|p| p.value.clone()).collect();
    let phones = v.get_all("TEL").into_iter().map(|p| p.value.clone()).collect();
    let org = v.get("ORG").map(|p| p.value.clone());
    let note = v.get("NOTE").map(|p| p.value.clone());

    Contact {
        uid: uid.to_string(),
        full_name,
        first_name,
        last_name,
        emails,
        phones,
        org,
        note,
    }
}

pub fn vcard_from_input(uid: &str, input: &ContactInput) -> String {
    let mut out = String::new();
    out.push_str("BEGIN:VCARD\r\n");
    out.push_str("VERSION:3.0\r\n");
    out.push_str(&format!("UID:{}\r\n", escape_value(uid)));
    out.push_str(&format!("FN:{}\r\n", escape_value(&input.full_name)));

    let last = input.last_name.clone().unwrap_or_default();
    let first = input.first_name.clone().unwrap_or_default();
    if !last.is_empty() || !first.is_empty() {
        out.push_str(&format!(
            "N:{};{};;;\r\n",
            escape_value(&last),
            escape_value(&first)
        ));
    }

    for email in &input.emails {
        out.push_str(&format!("EMAIL;TYPE=INTERNET:{}\r\n", escape_value(email)));
    }
    for phone in &input.phones {
        out.push_str(&format!("TEL;TYPE=CELL:{}\r\n", escape_value(phone)));
    }
    if let Some(org) = &input.org {
        if !org.is_empty() {
            out.push_str(&format!("ORG:{}\r\n", escape_value(org)));
        }
    }
    if let Some(note) = &input.note {
        if !note.is_empty() {
            out.push_str(&format!("NOTE:{}\r\n", escape_value(note)));
        }
    }
    out.push_str("END:VCARD\r\n");
    out
}
