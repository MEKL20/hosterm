// ============================================================
//  drive.rs — Google Drive REST (drive.file) (BE-DRIVE owns)
// ============================================================
//
// A minimal Google Drive v3 client scoped to `drive.file` (the app only ever
// sees files it created). It uploads a `.htbk` backup via a multipart
// create, lists backups by their cleartext `appProperties` (so the UI can show
// a backup list WITHOUT decrypting anything), downloads by file id, and
// overwrites (updates) an existing backup's media.
//
// SECURITY / INVARIANTS (TSD §1,§4,§6, PRD §B):
//   - Scope is `drive.file` ONLY. Nothing here requests or depends on a broader
//     scope; the only metadata.q filter we issue is scoped to our own files via
//     appProperties (which only exist on files we created).
//   - Cleartext Drive `appProperties` carry EXACTLY {htbk_schema, created_at,
//     host} — NO filenames, NO host aliases, NO key hints, NO passphrase
//     material. `build_app_properties` is the single constructor and the leak
//     test asserts the key set is exactly those three.
//   - Access tokens are passed in per call and never logged. On HTTP 401 we
//     call the injected refresh hook ONCE, then retry ONCE with the new token;
//     if the refresh itself fails (e.g. refresh_token revoked) we surface a
//     re-auth error so the UI can prompt sign-in again.
//   - All HTTP goes through the `DriveHttp` trait seam so unit tests inject a
//     fake responder — NO real network in tests (NFR-3: zero idle network; the
//     real client is backend-only + user-initiated).

use serde::Deserialize;
use std::time::Duration;

// ---- Endpoints -----------------------------------------------------------

/// Multipart upload (create): metadata + media in one request.
const UPLOAD_CREATE_ENDPOINT: &str =
    "https://www.googleapis.com/upload/drive/v3/files?uploadType=multipart";
/// Media update (overwrite an existing file's bytes): PATCH .../files/{id}.
const UPLOAD_UPDATE_BASE: &str = "https://www.googleapis.com/upload/drive/v3/files/";
/// Metadata list.
const FILES_LIST_ENDPOINT: &str = "https://www.googleapis.com/drive/v3/files";
/// Media download: GET .../files/{id}?alt=media.
const FILES_GET_BASE: &str = "https://www.googleapis.com/drive/v3/files/";
/// Human-visible Drive folder used for all newly-created backups.
pub const BACKUP_FOLDER_NAME: &str = "HostermBackups";
const FOLDER_MIME_TYPE: &str = "application/vnd.google-apps.folder";

/// Fields we ask Drive to echo back on create/list so we can populate metadata
/// without a second round-trip. Kept tight: no fields that would leak.
const FILE_FIELDS: &str = "id,name,size,createdTime,appProperties";
const LIST_FIELDS: &str = "files(id,name,size,createdTime,appProperties)";

/// Boundary for the multipart create body. A fixed constant is fine (Google
/// just needs it to not collide with body content; metadata+base64url media
/// never contain this token).
const MULTIPART_BOUNDARY: &str = "hosterm_htbk_boundary_7c1f";

// ---- HTTP seam -----------------------------------------------------------
//
// One trait so tests can inject a fake. Each method returns the raw
// (status, body) so the 401→refresh→retry policy lives ABOVE the seam in
// `DriveClient`, keeping the seam dumb and the retry logic unit-testable.

/// A completed HTTP response: numeric status + byte-exact body. Metadata
/// callers parse JSON from the bytes; media downloads return them unchanged.
#[derive(Debug, Clone)]
pub struct HttpResponse {
    pub status: u16,
    pub body: Vec<u8>,
}

impl HttpResponse {
    fn is_success(&self) -> bool {
        (200..300).contains(&self.status)
    }
    fn is_unauthorized(&self) -> bool {
        self.status == 401
    }
}

/// Outbound Drive HTTP. All methods take the bearer `access_token` explicitly
/// so the retry wrapper can swap it after a refresh. Implementations MUST NOT
/// log tokens or bodies.
pub trait DriveHttp {
    /// POST a JSON metadata body (used to create the backup folder).
    fn post_json(
        &self,
        url: &str,
        access_token: &str,
        body: Vec<u8>,
    ) -> Result<HttpResponse, String>;

    /// POST a multipart/related body (metadata + media) to `url`.
    fn post_multipart(
        &self,
        url: &str,
        access_token: &str,
        boundary: &str,
        body: Vec<u8>,
    ) -> Result<HttpResponse, String>;

    /// PATCH raw media bytes to `url` (overwrite an existing file).
    fn patch_media(
        &self,
        url: &str,
        access_token: &str,
        content_type: &str,
        body: Vec<u8>,
    ) -> Result<HttpResponse, String>;

    /// GET `url` with a bearer token, return status + text body.
    fn get(&self, url: &str, access_token: &str) -> Result<HttpResponse, String>;
}

/// Production HTTP client: reqwest blocking + rustls. One client per call;
/// zero network at idle (NFR-3) — every call here is user-initiated (a backup
/// or restore the user asked for).
pub struct ReqwestDriveHttp;

impl DriveHttp for ReqwestDriveHttp {
    fn post_json(
        &self,
        url: &str,
        access_token: &str,
        body: Vec<u8>,
    ) -> Result<HttpResponse, String> {
        let client = build_client()?;
        let resp = client
            .post(url)
            .bearer_auth(access_token)
            .header(reqwest::header::CONTENT_TYPE, "application/json; charset=UTF-8")
            .body(body)
            .send()
            .map_err(|e| format!("drive metadata request failed: {e}"))?;
        into_response(resp)
    }

    fn post_multipart(
        &self,
        url: &str,
        access_token: &str,
        boundary: &str,
        body: Vec<u8>,
    ) -> Result<HttpResponse, String> {
        let client = build_client()?;
        let ctype = format!("multipart/related; boundary={boundary}");
        let resp = client
            .post(url)
            .bearer_auth(access_token)
            .header(reqwest::header::CONTENT_TYPE, ctype)
            .body(body)
            .send()
            .map_err(|e| format!("drive upload request failed: {e}"))?;
        into_response(resp)
    }

    fn patch_media(
        &self,
        url: &str,
        access_token: &str,
        content_type: &str,
        body: Vec<u8>,
    ) -> Result<HttpResponse, String> {
        let client = build_client()?;
        let resp = client
            .patch(url)
            .bearer_auth(access_token)
            .header(reqwest::header::CONTENT_TYPE, content_type.to_string())
            .body(body)
            .send()
            .map_err(|e| format!("drive update request failed: {e}"))?;
        into_response(resp)
    }

    fn get(&self, url: &str, access_token: &str) -> Result<HttpResponse, String> {
        let client = build_client()?;
        let resp = client
            .get(url)
            .bearer_auth(access_token)
            .send()
            .map_err(|e| format!("drive request failed: {e}"))?;
        into_response(resp)
    }
}

fn build_client() -> Result<reqwest::blocking::Client, String> {
    reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(60))
        .build()
        .map_err(|e| format!("http client build failed: {e}"))
}

fn into_response(resp: reqwest::blocking::Response) -> Result<HttpResponse, String> {
    let status = resp.status().as_u16();
    let body = resp
        .bytes()
        .map_err(|e| format!("drive response read failed: {e}"))?;
    Ok(HttpResponse {
        status,
        body: body.to_vec(),
    })
}

// ---- appProperties (cleartext — must leak NOTHING) -----------------------

/// The ONLY sanctioned constructor for the cleartext Drive `appProperties`.
/// Exactly three keys, all non-sensitive (TSD §4). NO filenames, NO host
/// aliases, NO key hints. `htbk_schema` lets us filter the list to our own
/// backups without decrypting anything.
///
/// Returns an ordered `Vec<(key, value)>` so the serialized JSON is
/// deterministic (nice for the multipart golden test).
pub fn build_app_properties(schema: u32, created_at: &str, host: &str) -> Vec<(String, String)> {
    vec![
        ("htbk_schema".to_string(), schema.to_string()),
        ("created_at".to_string(), created_at.to_string()),
        ("host".to_string(), host.to_string()),
    ]
}

/// Serialize the file-create metadata JSON. `name` is the backup filename
/// (`hosterm-backup-<host>-<UTC-iso>.htbk`), which is NOT secret (it is the
/// Drive file name, visible in the user's own Drive). appProperties are the
/// only *searchable* cleartext facets and are leak-checked above.
fn metadata_json(
    name: &str,
    app_properties: &[(String, String)],
    parent_id: Option<&str>,
) -> String {
    let props = app_properties
        .iter()
        .map(|(k, v)| format!("{}:{}", json_str(k), json_str(v)))
        .collect::<Vec<_>>()
        .join(",");
    let parent = parent_id
        .map(|id| format!(",\"parents\":[{}]", json_str(id)))
        .unwrap_or_default();
    format!(
        "{{\"name\":{},\"appProperties\":{{{}}}{}}}",
        json_str(name), props, parent
    )
}

/// Folder metadata contains only the fixed public display name and Drive MIME
/// type. No host, filename, passphrase, or token material is attached.
fn folder_metadata_json() -> String {
    format!(
        "{{\"name\":{},\"mimeType\":{}}}",
        json_str(BACKUP_FOLDER_NAME),
        json_str(FOLDER_MIME_TYPE)
    )
}

/// Minimal JSON string escaper (quotes + backslash + control chars). The
/// inputs here are a filename, an ISO timestamp, a hostname and small integers;
/// we keep a local escaper to avoid pulling serde_json value-building into a
/// hot, leak-sensitive path and to keep the body byte-deterministic.
fn json_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

// ---- Multipart body ------------------------------------------------------

/// Build a `multipart/related` body: part 1 = JSON metadata, part 2 = the raw
/// `.htbk` media bytes (application/octet-stream). This is exactly what Drive's
/// `uploadType=multipart` create expects.
pub fn build_multipart_body(
    boundary: &str,
    metadata: &str,
    media: &[u8],
) -> Vec<u8> {
    let mut body: Vec<u8> = Vec::with_capacity(media.len() + metadata.len() + 256);
    // Metadata part.
    body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
    body.extend_from_slice(b"Content-Type: application/json; charset=UTF-8\r\n\r\n");
    body.extend_from_slice(metadata.as_bytes());
    body.extend_from_slice(b"\r\n");
    // Media part.
    body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
    body.extend_from_slice(b"Content-Type: application/octet-stream\r\n\r\n");
    body.extend_from_slice(media);
    body.extend_from_slice(b"\r\n");
    // Closing delimiter.
    body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
    body
}

// ---- Response shapes we parse --------------------------------------------

/// A single Drive file as returned by create/list (the tight field set we ask
/// for). appProperties are parsed back so the UI can list without decrypting.
#[derive(Debug, Clone, Deserialize)]
pub struct DriveFile {
    pub id: String,
    #[serde(default)]
    pub name: String,
    /// Drive returns `size` as a string of a number; keep it a string here and
    /// let callers parse — avoids a serde custom deser for a display-only field.
    #[serde(default)]
    pub size: Option<String>,
    #[serde(default, rename = "createdTime")]
    pub created_time: Option<String>,
    #[serde(default, rename = "appProperties")]
    pub app_properties: Option<std::collections::BTreeMap<String, String>>,
}

#[derive(Debug, Deserialize)]
struct FileList {
    #[serde(default)]
    files: Vec<DriveFile>,
}

// ---- The client (401→refresh→retry policy lives here) --------------------

/// Outcome signalled when we cannot recover a valid token — the UI must prompt
/// sign-in again (refresh_token likely revoked/expired).
pub const REAUTH_REQUIRED: &str = "reauth-required: Google sign-in expired, please sign in again";

/// Drive client over the HTTP seam. Holds the current access token and a
/// refresh hook. The refresh hook returns a fresh access token (and lets the
/// caller persist a rotated refresh_token out of band); if it errors we treat
/// the session as unrecoverable and surface `REAUTH_REQUIRED`.
pub struct DriveClient<'a> {
    http: &'a dyn DriveHttp,
    access_token: String,
    /// Called on 401 to obtain a new access token. `Err` ⇒ re-auth required.
    refresh: Box<dyn FnMut() -> Result<String, String> + 'a>,
}

impl<'a> DriveClient<'a> {
    pub fn new(
        http: &'a dyn DriveHttp,
        access_token: String,
        refresh: Box<dyn FnMut() -> Result<String, String> + 'a>,
    ) -> Self {
        DriveClient {
            http,
            access_token,
            refresh,
        }
    }

    /// Run `op` with the current token; on HTTP 401 refresh ONCE and retry
    /// ONCE. Any non-401 error or a second 401 is returned as-is. A failed
    /// refresh surfaces `REAUTH_REQUIRED`.
    fn with_retry<F>(&mut self, mut op: F) -> Result<HttpResponse, String>
    where
        F: FnMut(&dyn DriveHttp, &str) -> Result<HttpResponse, String>,
    {
        let resp = op(self.http, &self.access_token)?;
        if !resp.is_unauthorized() {
            return Ok(resp);
        }
        // Token likely expired — refresh once.
        let new_token = (self.refresh)().map_err(|_| REAUTH_REQUIRED.to_string())?;
        self.access_token = new_token;
        // Retry exactly once with the fresh token.
        op(self.http, &self.access_token)
    }

    /// Find the app-created HostermBackups folder, or create it once when
    /// absent. `drive.file` limits both operations to files this app created.
    pub fn ensure_backup_folder(&mut self) -> Result<String, String> {
        let q = format!(
            "name='{}' and mimeType='{}' and trashed=false",
            BACKUP_FOLDER_NAME, FOLDER_MIME_TYPE
        );
        let url = format!(
            "{FILES_LIST_ENDPOINT}?q={}&fields={}&spaces=drive&orderBy=createdTime asc",
            urlencode(&q),
            urlencode("files(id,name)")
        );
        let resp = self.with_retry(|http, tok| http.get(&url, tok))?;
        if !resp.is_success() {
            return Err(format!("drive folder lookup failed: HTTP {}", resp.status));
        }
        let list: FileList = serde_json::from_slice(&resp.body)
            .map_err(|_| "malformed drive folder lookup response".to_string())?;
        if let Some(folder) = list.files.into_iter().next() {
            return Ok(folder.id);
        }

        let create_url = format!("{FILES_LIST_ENDPOINT}?fields={}", urlencode("id,name"));
        let body = folder_metadata_json().into_bytes();
        let resp = self.with_retry(|http, tok| http.post_json(&create_url, tok, body.clone()))?;
        if !resp.is_success() {
            return Err(format!("drive folder create failed: HTTP {}", resp.status));
        }
        let folder: DriveFile = serde_json::from_slice(&resp.body)
            .map_err(|_| "malformed drive folder create response".to_string())?;
        if folder.id.is_empty() {
            return Err("drive folder create response missing id".to_string());
        }
        Ok(folder.id)
    }

    /// Multipart create: upload a new `.htbk` with cleartext appProperties.
    /// `name` = backup filename; `media` = the encrypted `.htbk` bytes.
    pub fn upload(
        &mut self,
        name: &str,
        media: &[u8],
        app_properties: &[(String, String)],
    ) -> Result<DriveFile, String> {
        self.upload_with_parent(name, media, app_properties, None)
    }

    /// Multipart create inside a specific Drive folder.
    pub fn upload_in_folder(
        &mut self,
        name: &str,
        media: &[u8],
        app_properties: &[(String, String)],
        folder_id: &str,
    ) -> Result<DriveFile, String> {
        self.upload_with_parent(name, media, app_properties, Some(folder_id))
    }

    fn upload_with_parent(
        &mut self,
        name: &str,
        media: &[u8],
        app_properties: &[(String, String)],
        parent_id: Option<&str>,
    ) -> Result<DriveFile, String> {
        let metadata = metadata_json(name, app_properties, parent_id);
        let body = build_multipart_body(MULTIPART_BOUNDARY, &metadata, media);
        let url = format!("{UPLOAD_CREATE_ENDPOINT}&fields={FILE_FIELDS}");
        let resp = self.with_retry(|http, tok| {
            http.post_multipart(&url, tok, MULTIPART_BOUNDARY, body.clone())
        })?;
        if !resp.is_success() {
            return Err(format!("drive upload failed: HTTP {}", resp.status));
        }
        serde_json::from_slice::<DriveFile>(&resp.body)
            .map_err(|_| "malformed drive create response".to_string())
    }

    /// Overwrite an existing backup's media (PATCH). Metadata/appProperties are
    /// left as-is; this replaces the bytes only.
    pub fn update(&mut self, file_id: &str, media: &[u8]) -> Result<DriveFile, String> {
        let url = format!(
            "{UPLOAD_UPDATE_BASE}{}?uploadType=media&fields={FILE_FIELDS}",
            file_id
        );
        let resp = self.with_retry(|http, tok| {
            http.patch_media(&url, tok, "application/octet-stream", media.to_vec())
        })?;
        if !resp.is_success() {
            return Err(format!("drive update failed: HTTP {}", resp.status));
        }
        serde_json::from_slice::<DriveFile>(&resp.body)
            .map_err(|_| "malformed drive update response".to_string())
    }

    /// List `.htbk` backups the app created, filtered by our cleartext
    /// `htbk_schema` appProperty. Because the scope is `drive.file`, the app
    /// only ever sees its own files; the appProperties filter narrows to
    /// hosterm backups of the given schema. NO decryption needed.
    pub fn list_backups(&mut self, schema: u32) -> Result<Vec<DriveFile>, String> {
        let q = format!(
            "appProperties has {{ key='htbk_schema' and value='{schema}' }} and trashed=false"
        );
        let url = format!(
            "{FILES_LIST_ENDPOINT}?q={}&fields={}&spaces=drive&orderBy=createdTime desc",
            urlencode(&q),
            urlencode(LIST_FIELDS)
        );
        let resp = self.with_retry(|http, tok| http.get(&url, tok))?;
        if !resp.is_success() {
            return Err(format!("drive list failed: HTTP {}", resp.status));
        }
        let list: FileList = serde_json::from_slice(&resp.body)
            .map_err(|_| "malformed drive list response".to_string())?;
        Ok(list.files)
    }

    /// Download a backup blob by file id (`alt=media`). Returns the raw bytes,
    /// which the caller hands to `crypto::decode`.
    pub fn download(&mut self, file_id: &str) -> Result<Vec<u8>, String> {
        let url = format!("{FILES_GET_BASE}{}?alt=media", file_id);
        let resp = self.with_retry(|http, tok| http.get(&url, tok))?;
        if !resp.is_success() {
            return Err(format!("drive download failed: HTTP {}", resp.status));
        }
        Ok(resp.body)
    }
}

/// Percent-encode a query component (RFC 3986 unreserved kept; everything else
/// %XX). Small and dependency-free; the `q`/`fields` strings here are short.
fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for &b in s.as_bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

// ==========================================================================
//  TESTS — mock HTTP seam, NO real network.
// ==========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    /// Records every request and returns canned responses in order. Can be set
    /// to return a 401 on the first call to exercise the refresh→retry path.
    struct MockHttp {
        /// Queued responses, popped front-to-back per outbound call.
        responses: RefCell<Vec<HttpResponse>>,
        /// (method, url, access_token) seen, in order.
        seen: RefCell<Vec<(String, String, String)>>,
        /// Last multipart body captured (for body-shape assertions).
        last_multipart: RefCell<Option<(String, Vec<u8>)>>,
        /// Last JSON metadata body captured (folder creation assertions).
        last_json: RefCell<Option<Vec<u8>>>,
    }

    impl MockHttp {
        fn new(responses: Vec<HttpResponse>) -> Self {
            MockHttp {
                responses: RefCell::new(responses),
                seen: RefCell::new(Vec::new()),
                last_multipart: RefCell::new(None),
                last_json: RefCell::new(None),
            }
        }
        fn next(&self) -> HttpResponse {
            let mut r = self.responses.borrow_mut();
            assert!(!r.is_empty(), "mock ran out of canned responses");
            r.remove(0)
        }
    }

    impl DriveHttp for MockHttp {
        fn post_json(
            &self,
            url: &str,
            access_token: &str,
            body: Vec<u8>,
        ) -> Result<HttpResponse, String> {
            self.seen.borrow_mut().push((
                "POST_JSON".into(),
                url.into(),
                access_token.into(),
            ));
            *self.last_json.borrow_mut() = Some(body);
            Ok(self.next())
        }

        fn post_multipart(
            &self,
            url: &str,
            access_token: &str,
            boundary: &str,
            body: Vec<u8>,
        ) -> Result<HttpResponse, String> {
            self.seen.borrow_mut().push((
                "POST_MULTIPART".into(),
                url.into(),
                access_token.into(),
            ));
            *self.last_multipart.borrow_mut() = Some((boundary.to_string(), body));
            Ok(self.next())
        }
        fn patch_media(
            &self,
            url: &str,
            access_token: &str,
            _content_type: &str,
            _body: Vec<u8>,
        ) -> Result<HttpResponse, String> {
            self.seen
                .borrow_mut()
                .push(("PATCH".into(), url.into(), access_token.into()));
            Ok(self.next())
        }
        fn get(&self, url: &str, access_token: &str) -> Result<HttpResponse, String> {
            self.seen
                .borrow_mut()
                .push(("GET".into(), url.into(), access_token.into()));
            Ok(self.next())
        }
    }

    fn ok(body: &str) -> HttpResponse {
        HttpResponse {
            status: 200,
            body: body.as_bytes().to_vec(),
        }
    }
    fn ok_bytes(body: &[u8]) -> HttpResponse {
        HttpResponse {
            status: 200,
            body: body.to_vec(),
        }
    }
    fn unauth() -> HttpResponse {
        HttpResponse {
            status: 401,
            body: br#"{"error":{"code":401}}"#.to_vec(),
        }
    }

    // ---- appProperties leak check ---------------------------------------

    #[test]
    fn app_properties_contain_only_the_three_allowed_keys() {
        let props = build_app_properties(1, "2026-11-03T18:00:00Z", "my-laptop");
        let keys: std::collections::BTreeSet<&str> =
            props.iter().map(|(k, _)| k.as_str()).collect();
        let expected: std::collections::BTreeSet<&str> =
            ["htbk_schema", "created_at", "host"].into_iter().collect();
        assert_eq!(keys, expected, "appProperties key set must be EXACTLY the three allowed");
        // Defensive: none of the keys hints at secrets/filenames/aliases.
        for (k, _) in &props {
            let lk = k.to_lowercase();
            for banned in ["file", "name", "alias", "key", "pass", "secret", "salt", "nonce", "path"] {
                assert!(
                    !lk.contains(banned),
                    "appProperty key '{k}' looks like it could leak ('{banned}')"
                );
            }
        }
    }

    #[test]
    fn metadata_json_only_emits_name_and_appproperties() {
        let props = build_app_properties(1, "2026-11-03T18:00:00Z", "host-x");
        let md = metadata_json(
            "hosterm-backup-host-x-2026-11-03T18:00:00Z.htbk",
            &props,
            None,
        );
        // Exactly the two top-level keys; parse it back and assert the shape.
        let v: serde_json::Value = serde_json::from_str(&md).unwrap();
        let obj = v.as_object().unwrap();
        let keys: std::collections::BTreeSet<&str> = obj.keys().map(|s| s.as_str()).collect();
        assert_eq!(
            keys,
            ["appProperties", "name"].into_iter().collect::<std::collections::BTreeSet<_>>()
        );
        let ap = obj["appProperties"].as_object().unwrap();
        let ap_keys: std::collections::BTreeSet<&str> = ap.keys().map(|s| s.as_str()).collect();
        assert_eq!(
            ap_keys,
            ["created_at", "host", "htbk_schema"]
                .into_iter()
                .collect::<std::collections::BTreeSet<_>>()
        );
    }

    #[test]
    fn metadata_json_adds_only_the_requested_parent() {
        let props = build_app_properties(1, "t", "h");
        let md = metadata_json("b.htbk", &props, Some("FOLDER1"));
        let v: serde_json::Value = serde_json::from_str(&md).unwrap();
        let keys: std::collections::BTreeSet<&str> = v
            .as_object()
            .unwrap()
            .keys()
            .map(|s| s.as_str())
            .collect();
        assert_eq!(
            keys,
            ["appProperties", "name", "parents"]
                .into_iter()
                .collect::<std::collections::BTreeSet<_>>()
        );
        assert_eq!(v["parents"], serde_json::json!(["FOLDER1"]));
    }

    #[test]
    fn ensure_backup_folder_reuses_existing_app_created_folder() {
        let mock = MockHttp::new(vec![ok(
            r#"{"files":[{"id":"DIR1","name":"HostermBackups"}]}"#,
        )]);
        let mut client =
            DriveClient::new(&mock, "AT".to_string(), Box::new(|| Ok("x".to_string())));
        assert_eq!(client.ensure_backup_folder().unwrap(), "DIR1");
        let seen = mock.seen.borrow();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].0, "GET");
        assert!(seen[0].1.contains("HostermBackups"));
        assert!(seen[0].1.contains("application%2Fvnd.google-apps.folder"));
    }

    #[test]
    fn ensure_backup_folder_creates_it_without_secret_metadata_when_missing() {
        let mock = MockHttp::new(vec![
            ok(r#"{"files":[]}"#),
            ok(r#"{"id":"DIR2","name":"HostermBackups"}"#),
        ]);
        let mut client =
            DriveClient::new(&mock, "AT".to_string(), Box::new(|| Ok("x".to_string())));
        assert_eq!(client.ensure_backup_folder().unwrap(), "DIR2");
        let seen = mock.seen.borrow();
        assert_eq!(seen.len(), 2);
        assert_eq!(seen[0].0, "GET");
        assert_eq!(seen[1].0, "POST_JSON");
        let body = mock.last_json.borrow().clone().unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["name"], BACKUP_FOLDER_NAME);
        assert_eq!(v["mimeType"], FOLDER_MIME_TYPE);
        assert_eq!(v.as_object().unwrap().len(), 2);
    }

    // ---- multipart body shape -------------------------------------------

    #[test]
    fn multipart_body_has_metadata_and_media_parts_with_boundary() {
        let media = b"\x00\x01\x02HTBKbytes";
        let md = r#"{"name":"x.htbk"}"#;
        let body = build_multipart_body("BND", md, media);
        let text = String::from_utf8_lossy(&body);
        // Opening + closing boundary present.
        assert!(text.contains("--BND\r\n"), "opening boundary missing");
        assert!(text.contains("--BND--\r\n"), "closing boundary missing");
        // Metadata part carries the JSON content-type and the metadata JSON.
        assert!(text.contains("Content-Type: application/json; charset=UTF-8"));
        assert!(text.contains(md), "metadata JSON not embedded");
        // Media part is octet-stream; raw media bytes present verbatim.
        assert!(text.contains("Content-Type: application/octet-stream"));
        let idx = body
            .windows(media.len())
            .position(|w| w == media)
            .expect("raw media bytes must appear verbatim in the body");
        assert!(idx > 0);
        // Exactly two parts => the boundary token "--BND\r\n" appears twice,
        // plus the closing "--BND--".
        let opens = text.matches("--BND\r\n").count();
        assert_eq!(opens, 2, "expected exactly two multipart parts");
    }

    // ---- upload happy path ----------------------------------------------

    #[test]
    fn upload_posts_multipart_and_parses_file() {
        let resp = r#"{"id":"FILE1","name":"b.htbk","size":"123","createdTime":"2026-11-03T18:00:00Z","appProperties":{"htbk_schema":"1","created_at":"2026-11-03T18:00:00Z","host":"h"}}"#;
        let mock = MockHttp::new(vec![ok(resp)]);
        let mut client = DriveClient::new(
            &mock,
            "AT".to_string(),
            Box::new(|| panic!("refresh must not be called on 200")),
        );
        let props = build_app_properties(1, "2026-11-03T18:00:00Z", "h");
        let file = client.upload("b.htbk", b"ciphertext", &props).unwrap();
        assert_eq!(file.id, "FILE1");
        assert_eq!(file.size.as_deref(), Some("123"));
        // One multipart POST to the create endpoint with the bearer token.
        let seen = mock.seen.borrow();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].0, "POST_MULTIPART");
        assert!(seen[0].1.contains("uploadType=multipart"));
        assert_eq!(seen[0].2, "AT");
        // Captured body uses our boundary and contains the ciphertext media.
        let (bnd, body) = mock.last_multipart.borrow().clone().unwrap();
        assert_eq!(bnd, MULTIPART_BOUNDARY);
        assert!(body.windows(10).any(|w| w == b"ciphertext"));
    }

    #[test]
    fn upload_in_folder_sets_drive_parent() {
        let mock = MockHttp::new(vec![ok(r#"{"id":"FILEP"}"#)]);
        let mut client =
            DriveClient::new(&mock, "AT".to_string(), Box::new(|| Ok("x".to_string())));
        let props = build_app_properties(1, "t", "h");
        client
            .upload_in_folder("b.htbk", b"ciphertext", &props, "DIR9")
            .unwrap();
        let (_, body) = mock.last_multipart.borrow().clone().unwrap();
        let text = String::from_utf8_lossy(&body);
        assert!(text.contains(r#""parents":["DIR9"]"#));
    }

    #[test]
    fn repeated_uploads_create_distinct_drive_files() {
        let mock = MockHttp::new(vec![
            ok(r#"{"id":"FILE-A"}"#),
            ok(r#"{"id":"FILE-B"}"#),
        ]);
        let mut client =
            DriveClient::new(&mock, "AT".to_string(), Box::new(|| Ok("x".to_string())));
        let props = build_app_properties(1, "t", "h");
        let first = client
            .upload_in_folder("b-1.htbk", b"one", &props, "DIR9")
            .unwrap();
        let second = client
            .upload_in_folder("b-2.htbk", b"two", &props, "DIR9")
            .unwrap();
        assert_ne!(first.id, second.id);
        let seen = mock.seen.borrow();
        assert_eq!(seen.len(), 2);
        assert!(seen.iter().all(|entry| entry.0 == "POST_MULTIPART"));
        assert!(seen.iter().all(|entry| entry.1.contains("uploadType=multipart")));
    }

    // ---- 401 → refresh → retry ------------------------------------------

    #[test]
    fn upload_on_401_refreshes_once_then_retries_with_new_token() {
        let good = r#"{"id":"FILE2"}"#;
        let mock = MockHttp::new(vec![unauth(), ok(good)]);
        let refreshed = RefCell::new(0u32);
        let mut client = DriveClient::new(
            &mock,
            "OLD".to_string(),
            Box::new(|| {
                *refreshed.borrow_mut() += 1;
                Ok("NEW".to_string())
            }),
        );
        let props = build_app_properties(1, "t", "h");
        let file = client.upload("b.htbk", b"data", &props).unwrap();
        assert_eq!(file.id, "FILE2");
        assert_eq!(*refreshed.borrow(), 1, "refresh must run exactly once");
        let seen = mock.seen.borrow();
        assert_eq!(seen.len(), 2, "one 401 + one retry");
        assert_eq!(seen[0].2, "OLD", "first attempt uses the old token");
        assert_eq!(seen[1].2, "NEW", "retry uses the refreshed token");
    }

    #[test]
    fn list_on_401_refreshes_and_retries() {
        let good = r#"{"files":[{"id":"F1","name":"a.htbk","appProperties":{"htbk_schema":"1"}}]}"#;
        let mock = MockHttp::new(vec![unauth(), ok(good)]);
        let mut client = DriveClient::new(
            &mock,
            "OLD".to_string(),
            Box::new(|| Ok("NEW".to_string())),
        );
        let files = client.list_backups(1).unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].id, "F1");
        let seen = mock.seen.borrow();
        assert_eq!(seen.len(), 2);
        assert_eq!(seen[1].2, "NEW");
    }

    #[test]
    fn refresh_failure_surfaces_reauth_required() {
        let mock = MockHttp::new(vec![unauth()]);
        let mut client = DriveClient::new(
            &mock,
            "OLD".to_string(),
            Box::new(|| Err("refresh_token revoked".to_string())),
        );
        let err = client.download("FID").unwrap_err();
        assert_eq!(err, REAUTH_REQUIRED, "revoked refresh must surface re-auth");
        // Only the first GET happened; no retry after a failed refresh.
        assert_eq!(mock.seen.borrow().len(), 1);
    }

    #[test]
    fn second_401_is_not_retried_again() {
        // 401 then (after refresh) still 401 → surfaced as an auth HTTP error,
        // NOT an infinite refresh loop.
        let mock = MockHttp::new(vec![unauth(), unauth()]);
        let refreshed = RefCell::new(0u32);
        let mut client = DriveClient::new(
            &mock,
            "OLD".to_string(),
            Box::new(|| {
                *refreshed.borrow_mut() += 1;
                Ok("NEW".to_string())
            }),
        );
        let err = client.download("FID").unwrap_err();
        assert!(err.contains("401"), "second 401 surfaces as an HTTP error: {err}");
        assert_eq!(*refreshed.borrow(), 1, "refresh attempted exactly once");
        assert_eq!(mock.seen.borrow().len(), 2, "no third attempt");
    }

    // ---- list query filter ----------------------------------------------

    #[test]
    fn list_query_filters_by_htbk_schema_appproperty() {
        let mock = MockHttp::new(vec![ok(r#"{"files":[]}"#)]);
        let mut client =
            DriveClient::new(&mock, "AT".to_string(), Box::new(|| Ok("x".to_string())));
        client.list_backups(1).unwrap();
        let seen = mock.seen.borrow();
        let url = &seen[0].1;
        // The appProperties-scoped query must be present (percent-encoded).
        let decoded = url.replace("%20", " ").replace("%27", "'").replace("%3D", "=");
        assert!(
            decoded.contains("appProperties has"),
            "list must filter by appProperties: {url}"
        );
        assert!(
            decoded.contains("htbk_schema") && decoded.contains("value='1'"),
            "list must scope to htbk_schema=1: {url}"
        );
        assert!(
            decoded.contains("trashed=false"),
            "list should exclude trashed files: {url}"
        );
    }

    // ---- download / update ----------------------------------------------

    #[test]
    fn download_returns_non_utf8_raw_bytes_via_alt_media() {
        let raw = b"HTBK\x00\xff\xfe\x80ciphertext\x00";
        let mock = MockHttp::new(vec![ok_bytes(raw)]);
        let mut client =
            DriveClient::new(&mock, "AT".to_string(), Box::new(|| Ok("x".to_string())));
        let bytes = client.download("FID9").unwrap();
        assert_eq!(bytes, raw, "download must not perform UTF-8 decoding");
        let seen = mock.seen.borrow();
        assert_eq!(seen[0].0, "GET");
        assert!(seen[0].1.contains("FID9"));
        assert!(seen[0].1.contains("alt=media"));
    }

    #[test]
    fn update_patches_media_by_file_id() {
        let mock = MockHttp::new(vec![ok(r#"{"id":"FID7","size":"9"}"#)]);
        let mut client =
            DriveClient::new(&mock, "AT".to_string(), Box::new(|| Ok("x".to_string())));
        let file = client.update("FID7", b"newbytes").unwrap();
        assert_eq!(file.id, "FID7");
        let seen = mock.seen.borrow();
        assert_eq!(seen[0].0, "PATCH");
        assert!(seen[0].1.contains("FID7"));
        assert!(seen[0].1.contains("uploadType=media"));
    }

    // ---- url encoder -----------------------------------------------------

    #[test]
    fn urlencode_escapes_reserved_but_keeps_unreserved() {
        assert_eq!(urlencode("abcXYZ09-_.~"), "abcXYZ09-_.~");
        assert_eq!(urlencode("a b"), "a%20b");
        assert_eq!(urlencode("'"), "%27");
        assert_eq!(urlencode("="), "%3D");
    }
}
