//! One keep-alive HTTP/1.1 client. A dropped connection fails the run.

use std::net::SocketAddr;
use std::time::Instant;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use super::report::Phase;
use super::PageProbe;

const ORIGIN: &str = "http://127.0.0.1:3000";

pub struct Session {
    cookie: String,
    csrf: String,
}

pub struct Client {
    stream: TcpStream,
    pending: Vec<u8>,
}

struct Reply {
    status: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl Client {
    pub async fn connect(address: SocketAddr) -> Result<Self, String> {
        let stream = TcpStream::connect(address)
            .await
            .map_err(|error| error.to_string())?;
        stream
            .set_nodelay(true)
            .map_err(|error| error.to_string())?;
        Ok(Self {
            stream,
            pending: Vec::new(),
        })
    }

    pub async fn login(&mut self, email: &str, password: &str) -> Result<Session, String> {
        let body = serde_json::json!({"email": email, "password": password}).to_string();
        let reply = self
            .exchange("POST", "/auth/login", None, Some(body.as_bytes()))
            .await?;
        if reply.status != 200 {
            return Err(format!("login returned {}", reply.status));
        }
        let json: serde_json::Value =
            serde_json::from_slice(&reply.body).map_err(|error| error.to_string())?;
        let csrf = json["csrf_token"]
            .as_str()
            .filter(|value| !value.is_empty())
            .ok_or("login response has no csrf_token")?
            .to_owned();
        let cookie = reply
            .headers
            .iter()
            .filter(|(name, _)| name == "set-cookie")
            .filter_map(|(_, value)| value.split(';').next())
            .find(|value| value.trim_start().starts_with("fexu_session="))
            .map(str::trim)
            .ok_or("login response has no fexu_session cookie")?
            .to_owned();
        Ok(Session { cookie, csrf })
    }

    pub async fn discover(&mut self, session: &Session, limit: u64) -> Result<PageProbe, String> {
        let path = format!("/api/v1/bench_items?limit={limit}");
        let reply = self.exchange("GET", &path, Some(session), None).await?;
        if reply.status != 200 {
            return Err(format!("discover returned {}", reply.status));
        }
        let json: serde_json::Value =
            serde_json::from_slice(&reply.body).map_err(|error| error.to_string())?;
        let rows = json["data"]
            .as_array()
            .ok_or("discover response has no data array")?;
        if rows.len() != limit as usize {
            return Err(format!(
                "discover page has {} rows, expected {limit}",
                rows.len()
            ));
        }
        if json["has_more"] != true {
            return Err("discover page did not set has_more".into());
        }
        let next = json["next"]
            .as_str()
            .filter(|value| value.starts_with("k1."))
            .ok_or("discover page has no keyset cursor")?
            .to_owned();
        Ok(PageProbe {
            next,
            has_more: true,
        })
    }

    pub async fn get(&mut self, session: &Session, path: &str) -> Result<u16, String> {
        Ok(self
            .exchange("GET", path, Some(session), None)
            .await?
            .status)
    }

    pub async fn timed<F>(
        &mut self,
        session: &Session,
        count: usize,
        operation: &'static str,
        mut path_at: F,
    ) -> Result<Phase, String>
    where
        F: FnMut(usize) -> String,
    {
        let started = Instant::now();
        let mut samples = Vec::with_capacity(count);
        let mut status = None;
        for index in 0..count {
            let path = path_at(index);
            let one = Instant::now();
            let reply = self.exchange("GET", &path, Some(session), None).await?;
            samples.push(one.elapsed().as_nanos() as u64);
            match status {
                None => status = Some(reply.status),
                Some(expected) if expected != reply.status => {
                    return Err(format!(
                        "{operation} status changed from {expected} to {}",
                        reply.status
                    ));
                }
                Some(_) => {}
            }
        }
        Phase::new(
            operation,
            status.ok_or("timed phase has no status")?,
            started.elapsed().as_nanos() as u64,
            &samples,
        )
    }

    pub async fn timed_write(
        &mut self,
        session: &Session,
        count: usize,
        path: &str,
        body: &[u8],
    ) -> Result<Phase, String> {
        let started = Instant::now();
        let mut samples = Vec::with_capacity(count);
        let mut status = None;
        for _ in 0..count {
            let one = Instant::now();
            let reply = self
                .exchange("PATCH", path, Some(session), Some(body))
                .await?;
            samples.push(one.elapsed().as_nanos() as u64);
            match status {
                None => status = Some(reply.status),
                Some(expected) if expected != reply.status => {
                    return Err(format!(
                        "write status changed from {expected} to {}",
                        reply.status
                    ));
                }
                Some(_) => {}
            }
        }
        Phase::new(
            "authenticated patch",
            status.ok_or("write phase has no status")?,
            started.elapsed().as_nanos() as u64,
            &samples,
        )
    }

    pub async fn timed_status(
        &mut self,
        count: usize,
        path: &str,
        session: Option<&Session>,
        operation: &'static str,
    ) -> Result<Phase, String> {
        let started = Instant::now();
        let mut samples = Vec::with_capacity(count);
        let mut status = None;
        for _ in 0..count {
            let one = Instant::now();
            let reply = self.exchange("GET", path, session, None).await?;
            samples.push(one.elapsed().as_nanos() as u64);
            match status {
                None => status = Some(reply.status),
                Some(expected) if expected != reply.status => {
                    return Err(format!(
                        "{operation} status changed from {expected} to {}",
                        reply.status
                    ));
                }
                Some(_) => {}
            }
        }
        Phase::new(
            operation,
            status.ok_or("error phase has no status")?,
            started.elapsed().as_nanos() as u64,
            &samples,
        )
    }

    async fn exchange(
        &mut self,
        method: &str,
        path: &str,
        session: Option<&Session>,
        body: Option<&[u8]>,
    ) -> Result<Reply, String> {
        let request = build_request(method, path, session, body);
        self.stream
            .write_all(&request)
            .await
            .map_err(|error| format!("{method} {path} write failed: {error}"))?;
        read_reply(&mut self.stream, &mut self.pending)
            .await
            .map_err(|error| format!("{method} {path}: {error}"))
    }
}

pub async fn concurrent_reads(
    address: SocketAddr,
    session: &Session,
    path: &str,
    workers: usize,
    reads_per_worker: usize,
) -> Result<Phase, String> {
    let started = Instant::now();
    let cookie = session.cookie.clone();
    let csrf = session.csrf.clone();
    let mut tasks = Vec::with_capacity(workers);
    for _ in 0..workers {
        let cookie = cookie.clone();
        let csrf = csrf.clone();
        let path = path.to_owned();
        tasks.push(tokio::spawn(async move {
            let session = Session { cookie, csrf };
            let mut client = Client::connect(address).await?;
            let mut samples = Vec::with_capacity(reads_per_worker);
            for _ in 0..reads_per_worker {
                let one = Instant::now();
                let status = client.get(&session, &path).await?;
                samples.push((status, one.elapsed().as_nanos() as u64));
            }
            Ok::<_, String>(samples)
        }));
    }
    let mut samples = Vec::new();
    let mut status = None;
    for task in tasks {
        for (code, latency) in task.await.map_err(|error| error.to_string())?? {
            match status {
                None => status = Some(code),
                Some(expected) if expected != code => {
                    return Err(format!(
                        "concurrent status changed from {expected} to {code}"
                    ));
                }
                Some(_) => {}
            }
            samples.push(latency);
        }
    }
    Phase::new(
        "concurrent authenticated list",
        status.ok_or("concurrent phase has no status")?,
        started.elapsed().as_nanos() as u64,
        &samples,
    )
}

fn build_request(
    method: &str,
    path: &str,
    session: Option<&Session>,
    body: Option<&[u8]>,
) -> Vec<u8> {
    let mut text = format!(
        "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: keep-alive\r\nAccept: application/json\r\n"
    );
    if let Some(session) = session {
        text.push_str(&format!("Cookie: {}\r\n", session.cookie));
        if method != "GET" && method != "HEAD" {
            text.push_str(&format!("x-csrf-token: {}\r\n", session.csrf));
            text.push_str(&format!("Origin: {ORIGIN}\r\n"));
        }
    }
    if let Some(body) = body {
        text.push_str("Content-Type: application/json\r\n");
        text.push_str(&format!("Content-Length: {}\r\n", body.len()));
    }
    text.push_str("\r\n");
    let mut bytes = text.into_bytes();
    if let Some(body) = body {
        bytes.extend_from_slice(body);
    }
    bytes
}

async fn read_reply(stream: &mut TcpStream, pending: &mut Vec<u8>) -> Result<Reply, String> {
    let header_end = loop {
        if let Some(end) = find_header_end(pending) {
            break end;
        }
        if !read_more(stream, pending).await? {
            return Err("connection closed before headers".into());
        }
    };
    let header_bytes = pending[..header_end].to_vec();
    pending.drain(..header_end + 4);
    let header_text = String::from_utf8(header_bytes).map_err(|error| error.to_string())?;
    let mut lines = header_text.split("\r\n");
    let status_line = lines.next().ok_or("missing status line")?;
    let status = status_line
        .split_whitespace()
        .nth(1)
        .ok_or("status line has no code")?
        .parse::<u16>()
        .map_err(|error| error.to_string())?;
    let mut headers = Vec::new();
    for line in lines {
        if let Some((name, value)) = line.split_once(':') {
            headers.push((name.trim().to_ascii_lowercase(), value.trim().to_owned()));
        }
    }
    let body = if header_value(&headers, "transfer-encoding")
        .is_some_and(|value| value.to_ascii_lowercase().contains("chunked"))
    {
        read_chunked(stream, pending).await?
    } else if let Some(length) = header_value(&headers, "content-length") {
        let length = length.parse::<usize>().map_err(|error| error.to_string())?;
        while pending.len() < length {
            if !read_more(stream, pending).await? {
                return Err("connection closed before the body".into());
            }
        }
        let body = pending[..length].to_vec();
        pending.drain(..length);
        body
    } else {
        return Err("response has neither content-length nor chunked encoding".into());
    };
    Ok(Reply {
        status,
        headers,
        body,
    })
}

fn header_value<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value.as_str())
}

fn find_header_end(pending: &[u8]) -> Option<usize> {
    pending.windows(4).position(|bytes| bytes == b"\r\n\r\n")
}

async fn read_more(stream: &mut TcpStream, pending: &mut Vec<u8>) -> Result<bool, String> {
    let mut buffer = [0_u8; 8192];
    let count = stream
        .read(&mut buffer)
        .await
        .map_err(|error| error.to_string())?;
    if count == 0 {
        return Ok(false);
    }
    pending.extend_from_slice(&buffer[..count]);
    Ok(true)
}

async fn read_chunked(stream: &mut TcpStream, pending: &mut Vec<u8>) -> Result<Vec<u8>, String> {
    let mut decoded = Vec::new();
    loop {
        let line_end = loop {
            if let Some(end) = pending.windows(2).position(|bytes| bytes == b"\r\n") {
                break end;
            }
            if !read_more(stream, pending).await? {
                return Err("connection closed during a chunk header".into());
            }
        };
        let line = std::str::from_utf8(&pending[..line_end]).map_err(|error| error.to_string())?;
        let length = usize::from_str_radix(line.split(';').next().unwrap_or(line).trim(), 16)
            .map_err(|error| error.to_string())?;
        pending.drain(..line_end + 2);
        if length == 0 {
            while pending.len() < 2 {
                if !read_more(stream, pending).await? {
                    return Err("connection closed after the last chunk".into());
                }
            }
            if pending.starts_with(b"\r\n") {
                pending.drain(..2);
            }
            return Ok(decoded);
        }
        while pending.len() < length + 2 {
            if !read_more(stream, pending).await? {
                return Err("connection closed during a chunk".into());
            }
        }
        decoded.extend_from_slice(&pending[..length]);
        pending.drain(..length + 2);
    }
}

pub fn encode_query(value: &str) -> String {
    let mut out = String::new();
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char);
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}
