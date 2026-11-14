//! Sitemap, robots.txt, canonical URLs and conditional responses.
//!
//! Static files already answer `If-None-Match` in `fexu-static`. These helpers cover
//! generated documents. Origins come from application configuration, never from `Host`.

use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use sha2::{Digest, Sha256};

use crate::views::ViewError;

const ORIGIN_LIMIT: usize = 200;
const URL_LIMIT: usize = 1000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CachePolicy {
    /// Browser may revalidate. Shared caches must not store the document.
    Private,
    /// A public document such as `robots.txt`. `seconds` is capped at one day.
    Public { seconds: u64 },
}

pub fn canonical_url(origin: &str, path: &str) -> Result<String, ViewError> {
    require_origin(origin)?;
    let path = crate::views::normalized_path(path).ok_or_else(|| {
        ViewError::new(
            StatusCode::BAD_REQUEST,
            "web.url",
            "A canonical path must be local.",
        )
    })?;
    let mut url = String::with_capacity(origin.len() + path.len());
    url.push_str(origin.trim_end_matches('/'));
    url.push_str(&path);
    Ok(url)
}

pub fn sitemap_xml(origin: &str, paths: &[&str]) -> Result<String, ViewError> {
    if paths.len() > URL_LIMIT {
        return Err(ViewError::new(
            StatusCode::BAD_REQUEST,
            "web.sitemap",
            "A sitemap lists at most 1000 paths.",
        ));
    }
    let mut xml = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<urlset xmlns=\"http://www.sitemaps.org/schemas/sitemap/0.9\">\n",
    );
    for path in paths {
        let loc = canonical_url(origin, path)?;
        xml.push_str("  <url><loc>");
        xml.push_str(&xml_escape(&loc));
        xml.push_str("</loc></url>\n");
    }
    xml.push_str("</urlset>\n");
    Ok(xml)
}

/// `User-agent: *` only. Allow and disallow values are local paths.
pub fn robots_txt(
    origin: &str,
    allow: &[&str],
    disallow: &[&str],
    sitemap_path: Option<&str>,
) -> Result<String, ViewError> {
    if allow.len() + disallow.len() > 100 {
        return Err(ViewError::new(
            StatusCode::BAD_REQUEST,
            "web.robots",
            "robots.txt accepts at most 100 paths.",
        ));
    }
    let mut text = String::from("User-agent: *\n");
    for path in allow {
        push_rule(&mut text, "Allow", path)?;
    }
    for path in disallow {
        push_rule(&mut text, "Disallow", path)?;
    }
    if let Some(path) = sitemap_path {
        let loc = canonical_url(origin, path)?;
        text.push_str("Sitemap: ");
        text.push_str(&loc);
        text.push('\n');
    }
    Ok(text)
}

/// Sets an ETag. A matching `If-None-Match` on a 200 response becomes 304.
///
/// The response ETag is always the hash of this body. The request header is not copied back.
pub fn conditional_response(
    headers: &HeaderMap,
    status: StatusCode,
    content_type: &'static str,
    body: String,
    cache: CachePolicy,
) -> Response {
    match conditional_document(headers, status, content_type, body, cache) {
        Ok(response) => response,
        Err(error) => error.into_response(),
    }
}

fn conditional_document(
    headers: &HeaderMap,
    status: StatusCode,
    content_type: &'static str,
    body: String,
    cache: CachePolicy,
) -> Result<Response, ViewError> {
    if !content_type_ok(content_type) {
        return Err(ViewError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "web.header",
            "The content type was rejected.",
        ));
    }
    let etag_text = strong_etag(body.as_bytes());
    let cache_text = cache_control(cache);
    let content_type = header_value(content_type)?;
    let etag = header_value(&etag_text)?;
    let cache_header = header_value(&cache_text)?;
    let vary = header_value("Cookie")?;
    if status == StatusCode::OK && if_none_match(headers, &etag_text) {
        let mut response = StatusCode::NOT_MODIFIED.into_response();
        let headers = response.headers_mut();
        headers.insert(header::ETAG, etag);
        headers.insert(header::CACHE_CONTROL, cache_header);
        headers.insert(header::VARY, vary);
        return Ok(response);
    }
    let mut response = (status, body).into_response();
    let headers = response.headers_mut();
    headers.insert(header::CONTENT_TYPE, content_type);
    headers.insert(header::ETAG, etag);
    headers.insert(header::CACHE_CONTROL, cache_header);
    headers.insert(header::VARY, vary);
    Ok(response)
}

pub(crate) fn require_origin(origin: &str) -> Result<(), ViewError> {
    let host = origin
        .strip_prefix("https://")
        .or_else(|| origin.strip_prefix("http://"))
        .ok_or_else(|| {
            ViewError::new(
                StatusCode::BAD_REQUEST,
                "web.origin",
                "The origin must start with http:// or https://.",
            )
        })?;
    if host.is_empty()
        || host.len() > ORIGIN_LIMIT
        || host.contains('/')
        || host.contains('?')
        || host.contains('#')
        || host.contains('@')
        || host.contains('\\')
        || host
            .chars()
            .any(|char| char.is_control() || char.is_whitespace())
    {
        return Err(ViewError::new(
            StatusCode::BAD_REQUEST,
            "web.origin",
            "The origin must be a host without a path.",
        ));
    }
    let (name, port) = host
        .rsplit_once(':')
        .map(|(name, port)| (name, Some(port)))
        .unwrap_or((host, None));
    if name.is_empty()
        || name.starts_with('.')
        || name.ends_with('.')
        || name.starts_with('-')
        || name.contains("..")
        || !name
            .chars()
            .all(|char| char.is_ascii_alphanumeric() || char == '.' || char == '-')
    {
        return Err(ViewError::new(
            StatusCode::BAD_REQUEST,
            "web.origin",
            "The origin host is invalid.",
        ));
    }
    if let Some(port) = port {
        if port.is_empty()
            || port.len() > 5
            || !port.chars().all(|char| char.is_ascii_digit())
            || port.parse::<u16>().ok().filter(|port| *port > 0).is_none()
        {
            return Err(ViewError::new(
                StatusCode::BAD_REQUEST,
                "web.origin",
                "The origin port is invalid.",
            ));
        }
    }
    Ok(())
}

fn push_rule(text: &mut String, label: &str, path: &str) -> Result<(), ViewError> {
    let path = crate::views::normalized_path(path).ok_or_else(|| {
        ViewError::new(
            StatusCode::BAD_REQUEST,
            "web.robots",
            "robots.txt paths must be local.",
        )
    })?;
    text.push_str(label);
    text.push_str(": ");
    text.push_str(&path);
    text.push('\n');
    Ok(())
}

fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn strong_etag(body: &[u8]) -> String {
    format!("\"{}\"", hex_lower(Sha256::digest(body).as_ref()))
}

fn hex_lower(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(HEX[(byte >> 4) as usize] as char);
        encoded.push(HEX[(byte & 0x0f) as usize] as char);
    }
    encoded
}

fn if_none_match(headers: &HeaderMap, etag: &str) -> bool {
    let Some(value) = headers
        .get(header::IF_NONE_MATCH)
        .and_then(|value| value.to_str().ok())
    else {
        return false;
    };
    if value.len() > 1024 || value.chars().any(|char| char.is_control()) {
        return false;
    }
    value.split(',').any(|part| {
        let part = part.trim();
        part == "*" || part == etag
    })
}

fn cache_control(cache: CachePolicy) -> String {
    match cache {
        CachePolicy::Private => "private, no-cache".to_owned(),
        CachePolicy::Public { seconds } => format!("public, max-age={}", seconds.min(86_400)),
    }
}

fn content_type_ok(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 80
        && !value
            .chars()
            .any(|char| char.is_control() || char == '"' || char == '\\')
}

fn header_error() -> ViewError {
    ViewError::new(
        StatusCode::INTERNAL_SERVER_ERROR,
        "web.header",
        "A response header was rejected.",
    )
}

fn header_value(value: &str) -> Result<axum::http::HeaderValue, ViewError> {
    axum::http::HeaderValue::from_str(value).map_err(|_| header_error())
}

#[cfg(test)]
mod tests {
    use super::{canonical_url, conditional_response, robots_txt, sitemap_xml, CachePolicy};
    use axum::http::{HeaderMap, StatusCode};
    use http_body_util::BodyExt;

    #[test]
    fn canonical_and_sitemap_reject_foreign_targets() {
        assert_eq!(
            canonical_url("http://127.0.0.1:3000", "/board/items").unwrap(),
            "http://127.0.0.1:3000/board/items"
        );
        assert!(canonical_url("http://127.0.0.1:3000", "https://evil.example").is_err());
        assert!(canonical_url("http://user:pass@127.0.0.1:3000", "/board").is_err());
        assert!(canonical_url("http://127.0.0.1:3000/app", "/board").is_err());
        assert!(canonical_url("https://evil.example", "//evil.example").is_err());
        let xml = sitemap_xml("http://127.0.0.1:3000", &["/board/items"]).unwrap();
        assert!(xml.contains("http://127.0.0.1:3000/board/items"));
        assert!(xml.contains("&amp;") || !xml.contains("<script"));
        assert!(sitemap_xml("http://127.0.0.1:3000", &["https://evil.example"]).is_err());
        let robots = robots_txt(
            "http://127.0.0.1:3000",
            &["/board/items"],
            &["/board/items/new"],
            Some("/board/sitemap.xml"),
        )
        .unwrap();
        assert!(robots.contains("User-agent: *\n"));
        assert!(robots.contains("Disallow: /board/items/new\n"));
        assert!(robots.contains("Sitemap: http://127.0.0.1:3000/board/sitemap.xml\n"));
        assert!(robots_txt("http://127.0.0.1:3000", &["//evil.example"], &[], None).is_err());
    }

    #[tokio::test]
    async fn conditional_response_uses_its_own_etag() {
        let mut headers = HeaderMap::new();
        let first = conditional_response(
            &headers,
            StatusCode::OK,
            "text/plain; charset=utf-8",
            "notes".to_owned(),
            CachePolicy::Public { seconds: 60 },
        );
        let etag = first
            .headers()
            .get("etag")
            .unwrap()
            .to_str()
            .unwrap()
            .to_owned();
        assert!(first
            .headers()
            .get("cache-control")
            .unwrap()
            .to_str()
            .unwrap()
            .contains("public"));
        headers.insert("if-none-match", etag.parse().unwrap());
        let again = conditional_response(
            &headers,
            StatusCode::OK,
            "text/plain; charset=utf-8",
            "notes".to_owned(),
            CachePolicy::Public { seconds: 9_999_999 },
        );
        assert_eq!(again.status(), StatusCode::NOT_MODIFIED);
        let bytes = again.into_body().collect().await.unwrap().to_bytes();
        assert!(bytes.is_empty());
        headers.insert("if-none-match", "\"forged\"".parse().unwrap());
        let mismatch = conditional_response(
            &headers,
            StatusCode::OK,
            "text/plain; charset=utf-8",
            "notes".to_owned(),
            CachePolicy::Private,
        );
        assert_eq!(mismatch.status(), StatusCode::OK);
        assert_ne!(
            mismatch.headers().get("etag").unwrap().to_str().unwrap(),
            "\"forged\""
        );
        assert!(mismatch
            .headers()
            .get("cache-control")
            .unwrap()
            .to_str()
            .unwrap()
            .contains("private"));
    }
}
