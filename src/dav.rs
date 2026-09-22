//! A small CardDAV client sufficient to talk to Baikal: list resources in
//! an addressbook collection, fetch/create/update/delete individual vCard
//! resources.

use anyhow::{anyhow, Context, Result};
use quick_xml::events::Event;
use quick_xml::reader::Reader;
use reqwest::{Method, StatusCode};

use crate::config::BaikalConfig;

#[derive(Debug, Clone)]
pub struct DavCard {
    pub uid: String,
    pub etag: Option<String>,
    pub vcard_text: String,
}

pub struct CardDavClient {
    http: reqwest::Client,
    base_url: String,
    addressbook_path: String,
    username: String,
    password: String,
}

fn join_url(base: &str, path: &str) -> String {
    format!("{}/{}", base.trim_end_matches('/'), path.trim_start_matches('/'))
}

fn local_name(qname: &[u8]) -> String {
    let s = String::from_utf8_lossy(qname);
    match s.split_once(':') {
        Some((_, n)) => n.to_ascii_lowercase(),
        None => s.to_ascii_lowercase(),
    }
}

fn uid_from_href(href: &str) -> Option<String> {
    let file = href.rsplit('/').next()?;
    if !file.to_ascii_lowercase().ends_with(".vcf") {
        return None;
    }
    Some(file[..file.len() - 4].to_string())
}

#[derive(Default, Debug)]
struct MultistatusEntry {
    href: Option<String>,
    etag: Option<String>,
    address_data: Option<String>,
    status_ok: bool,
}

/// Parses a WebDAV multistatus (207) response body into per-<response>
/// entries, extracting href / getetag / address-data / status text.
fn parse_multistatus(body: &str) -> Result<Vec<MultistatusEntry>> {
    let mut reader = Reader::from_str(body);
    reader.config_mut().trim_text(true);

    let mut entries = Vec::new();
    let mut current: Option<MultistatusEntry> = None;
    let mut field: Option<String> = None;

    loop {
        match reader.read_event().context("parsing multistatus XML")? {
            Event::Start(e) => {
                let name = local_name(e.name().as_ref());
                match name.as_str() {
                    "response" => current = Some(MultistatusEntry::default()),
                    "href" | "getetag" | "address-data" | "status" => field = Some(name),
                    _ => {}
                }
            }
            Event::Empty(e) => {
                let _ = local_name(e.name().as_ref());
            }
            Event::Text(t) => {
                if let (Some(f), Some(entry)) = (&field, current.as_mut()) {
                    let text = t.unescape().unwrap_or_default().into_owned();
                    match f.as_str() {
                        "href" => entry.href = Some(text),
                        "getetag" => entry.etag = Some(text.trim_matches('"').to_string()),
                        "address-data" => entry.address_data = Some(text),
                        "status" => entry.status_ok = text.contains("200"),
                        _ => {}
                    }
                }
            }
            Event::End(e) => {
                let name = local_name(e.name().as_ref());
                if field.as_deref() == Some(name.as_str()) {
                    field = None;
                }
                if name == "response" {
                    if let Some(entry) = current.take() {
                        entries.push(entry);
                    }
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }

    Ok(entries)
}

impl CardDavClient {
    pub fn new(cfg: &BaikalConfig) -> Result<Self> {
        Ok(Self {
            http: reqwest::Client::builder().build()?,
            base_url: cfg.base_url.clone(),
            addressbook_path: cfg.addressbook_path.clone(),
            username: cfg.username.clone(),
            password: cfg.password.clone(),
        })
    }

    fn addressbook_url(&self) -> String {
        let joined = join_url(&self.base_url, &self.addressbook_path);
        if joined.ends_with('/') {
            joined
        } else {
            format!("{joined}/")
        }
    }

    fn contact_url(&self, uid: &str) -> String {
        format!("{}{}.vcf", self.addressbook_url(), uid)
    }

    /// Lists every vCard resource in the addressbook, including its
    /// current content, in as few requests as possible: a PROPFIND to
    /// discover member hrefs, followed by one addressbook-multiget REPORT
    /// to fetch etags + vCard bodies for all of them.
    pub async fn list(&self) -> Result<Vec<DavCard>> {
        let hrefs = self.list_hrefs().await?;
        if hrefs.is_empty() {
            return Ok(Vec::new());
        }
        self.multiget(&hrefs).await
    }

    async fn list_hrefs(&self) -> Result<Vec<String>> {
        let body = r#"<?xml version="1.0" encoding="utf-8" ?>
<D:propfind xmlns:D="DAV:">
  <D:prop>
    <D:resourcetype/>
    <D:getcontenttype/>
  </D:prop>
</D:propfind>"#;

        let resp = self
            .http
            .request(Method::from_bytes(b"PROPFIND")?, self.addressbook_url())
            .basic_auth(&self.username, Some(&self.password))
            .header("Depth", "1")
            .header("Content-Type", "application/xml; charset=utf-8")
            .body(body)
            .send()
            .await
            .context("PROPFIND request to Baikal failed")?;

        let status = resp.status();
        let text = resp.text().await.context("reading PROPFIND response body")?;
        if !status.is_success() {
            return Err(anyhow!("PROPFIND failed with status {status}: {text}"));
        }

        let entries = parse_multistatus(&text)?;
        Ok(entries
            .into_iter()
            .filter_map(|e| e.href)
            .filter(|h| h.to_ascii_lowercase().ends_with(".vcf"))
            .collect())
    }

    async fn multiget(&self, hrefs: &[String]) -> Result<Vec<DavCard>> {
        let mut href_xml = String::new();
        for href in hrefs {
            href_xml.push_str(&format!("  <D:href>{}</D:href>\n", xml_escape(href)));
        }
        let body = format!(
            r#"<?xml version="1.0" encoding="utf-8" ?>
<C:addressbook-multiget xmlns:D="DAV:" xmlns:C="urn:ietf:params:xml:ns:carddav">
  <D:prop>
    <D:getetag/>
    <C:address-data/>
  </D:prop>
{href_xml}</C:addressbook-multiget>"#
        );

        let resp = self
            .http
            .request(Method::from_bytes(b"REPORT")?, self.addressbook_url())
            .basic_auth(&self.username, Some(&self.password))
            .header("Depth", "0")
            .header("Content-Type", "application/xml; charset=utf-8")
            .body(body)
            .send()
            .await
            .context("REPORT (addressbook-multiget) request to Baikal failed")?;

        let status = resp.status();
        let text = resp.text().await.context("reading REPORT response body")?;
        if !status.is_success() {
            return Err(anyhow!("REPORT failed with status {status}: {text}"));
        }

        let entries = parse_multistatus(&text)?;
        let mut cards = Vec::new();
        for entry in entries {
            let (Some(href), Some(vcard_text)) = (entry.href.clone(), entry.address_data) else {
                continue;
            };
            let Some(uid) = uid_from_href(&href) else {
                continue;
            };
            cards.push(DavCard {
                uid,
                etag: entry.etag,
                vcard_text,
            });
        }
        Ok(cards)
    }

    /// Fetches a single vCard by uid. Returns None if it doesn't exist.
    pub async fn get(&self, uid: &str) -> Result<Option<DavCard>> {
        let resp = self
            .http
            .get(self.contact_url(uid))
            .basic_auth(&self.username, Some(&self.password))
            .send()
            .await
            .context("GET request to Baikal failed")?;

        if resp.status() == StatusCode::NOT_FOUND {
            return Ok(None);
        }
        let status = resp.status();
        let etag = resp
            .headers()
            .get("etag")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.trim_matches('"').to_string());
        let text = resp.text().await.context("reading GET response body")?;
        if !status.is_success() {
            return Err(anyhow!("GET failed with status {status}: {text}"));
        }

        Ok(Some(DavCard {
            uid: uid.to_string(),
            etag,
            vcard_text: text,
        }))
    }

    /// Creates a new vCard resource. Fails if one already exists for this uid.
    pub async fn create(&self, uid: &str, vcard_text: &str) -> Result<Option<String>> {
        let resp = self
            .http
            .put(self.contact_url(uid))
            .basic_auth(&self.username, Some(&self.password))
            .header("Content-Type", "text/vcard; charset=utf-8")
            .header("If-None-Match", "*")
            .body(vcard_text.to_string())
            .send()
            .await
            .context("PUT (create) request to Baikal failed")?;

        let status = resp.status();
        let etag = resp
            .headers()
            .get("etag")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.trim_matches('"').to_string());
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(anyhow!("create failed with status {status}: {text}"));
        }
        Ok(etag)
    }

    /// Updates an existing vCard resource. If `if_match` is provided the
    /// update is conditional on the resource's current etag matching.
    pub async fn update(
        &self,
        uid: &str,
        vcard_text: &str,
        if_match: Option<&str>,
    ) -> Result<Option<String>> {
        let mut req = self
            .http
            .put(self.contact_url(uid))
            .basic_auth(&self.username, Some(&self.password))
            .header("Content-Type", "text/vcard; charset=utf-8")
            .body(vcard_text.to_string());
        if let Some(etag) = if_match {
            req = req.header("If-Match", format!("\"{etag}\""));
        }

        let resp = req.send().await.context("PUT (update) request to Baikal failed")?;
        let status = resp.status();
        let etag = resp
            .headers()
            .get("etag")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.trim_matches('"').to_string());
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(anyhow!("update failed with status {status}: {text}"));
        }
        Ok(etag)
    }

    /// Deletes a vCard resource. Returns Ok(false) if it didn't exist.
    pub async fn delete(&self, uid: &str, if_match: Option<&str>) -> Result<bool> {
        let mut req = self
            .http
            .delete(self.contact_url(uid))
            .basic_auth(&self.username, Some(&self.password));
        if let Some(etag) = if_match {
            req = req.header("If-Match", format!("\"{etag}\""));
        }

        let resp = req.send().await.context("DELETE request to Baikal failed")?;
        let status = resp.status();
        if status == StatusCode::NOT_FOUND {
            return Ok(false);
        }
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(anyhow!("delete failed with status {status}: {text}"));
        }
        Ok(true)
    }
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}
