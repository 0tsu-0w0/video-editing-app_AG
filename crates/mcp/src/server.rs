use crate::session::SharedEditorSession;
use crate::tools::{call_tool, tool_definitions};
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::thread::{self, JoinHandle};
use std::time::Duration;
use thiserror::Error;
use uuid::Uuid;

const MAX_CONCURRENT_CONNECTIONS: usize = 16;
const IO_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_HEADER_COUNT: usize = 64;
const MAX_HEADER_LINE_BYTES: usize = 8 * 1024;
const MAX_BODY_BYTES: usize = 4 * 1024 * 1024;

#[derive(Debug, Error)]
pub enum McpServerError {
    #[error("Failed to bind MCP server socket: {0}")]
    Bind(#[from] std::io::Error),
}

struct ConnectionGuard {
    active: Arc<AtomicUsize>,
}

impl Drop for ConnectionGuard {
    fn drop(&mut self) {
        self.active.fetch_sub(1, Ordering::AcqRel);
    }
}

/// タイミング攻撃を防ぐ定数時間バイト列比較
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    let mut diff = a.len() ^ b.len();
    let max_len = a.len().max(b.len());
    for i in 0..max_len {
        let ba = a.get(i).copied().unwrap_or(0);
        let bb = b.get(i).copied().unwrap_or(0);
        diff |= usize::from(ba ^ bb);
    }
    diff == 0
}

/// DNS リバインディング対策: `Host` ヘッダーがループバックアドレスであることを検証する
fn is_loopback_host(host: &str) -> bool {
    let h = host.trim().to_ascii_lowercase();
    if let Some(rest) = h.strip_prefix("[::1]") {
        return rest.is_empty()
            || rest
                .strip_prefix(':')
                .is_some_and(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()));
    }
    let (name, port_opt) = match h.split_once(':') {
        Some((n, p)) => (n, Some(p)),
        None => (h.as_str(), None),
    };
    let host_ok = name == "127.0.0.1" || name == "localhost";
    let port_ok = match port_opt {
        None => true,
        Some(p) => !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()),
    };
    host_ok && port_ok
}

/// ブラウザからのクロスサイト CSRF 対策: `Origin` ヘッダーが存在する場合はループバックのみ許可する
fn is_loopback_origin(origin: &str) -> bool {
    let o = origin.trim();
    let rest = if let Some(r) = o.strip_prefix("http://") {
        r
    } else if let Some(r) = o.strip_prefix("https://") {
        r
    } else {
        return false;
    };
    let host_part = rest.split('/').next().unwrap_or("");
    is_loopback_host(host_part)
}

/// 1行あたりの最大バイト数を制限して読み込み、無制限改行なしヘッダーによる OOM を防ぐ
fn read_bounded_line<R: BufRead>(
    reader: &mut R,
    max_bytes: usize,
) -> Result<Option<String>, &'static str> {
    let mut buf = Vec::new();
    let read = reader
        .take((max_bytes + 1) as u64)
        .read_until(b'\n', &mut buf)
        .map_err(|_| "read error")?;
    if read == 0 {
        return Ok(None);
    }
    if buf.len() > max_bytes {
        return Err("header line too long");
    }
    String::from_utf8(buf).map(Some).map_err(|_| "invalid utf8")
}

/// ローカル専用 Streamable HTTP（`127.0.0.1` + 起動時生成ベアラートークン認証）内蔵 MCP サーバー
pub struct EmbeddedMcpServer {
    addr: SocketAddr,
    token: String,
    shutdown: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl EmbeddedMcpServer {
    pub fn start(session: SharedEditorSession) -> Result<Self, McpServerError> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let addr = listener.local_addr()?;
        listener.set_nonblocking(true)?;

        let token = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
        let shutdown = Arc::new(AtomicBool::new(false));
        let active_conns = Arc::new(AtomicUsize::new(0));

        let thread_token = token.clone();
        let thread_shutdown = Arc::clone(&shutdown);
        let handle = thread::spawn(move || {
            while !thread_shutdown.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        let _ = stream.set_nonblocking(false);
                        let _ = stream.set_read_timeout(Some(IO_TIMEOUT));
                        let _ = stream.set_write_timeout(Some(IO_TIMEOUT));

                        let prev = active_conns.fetch_add(1, Ordering::AcqRel);
                        if prev >= MAX_CONCURRENT_CONNECTIONS {
                            active_conns.fetch_sub(1, Ordering::AcqRel);
                            write_http_json(
                                &mut stream,
                                "503 Service Unavailable",
                                &json!({"error": "too many concurrent MCP connections"}),
                            );
                            continue;
                        }

                        let guard = ConnectionGuard {
                            active: Arc::clone(&active_conns),
                        };
                        let sess = Arc::clone(&session);
                        let expected_token = thread_token.clone();
                        thread::spawn(move || {
                            let _guard = guard;
                            handle_connection(stream, &sess, &expected_token);
                        });
                    }
                    Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(15));
                    }
                    Err(_) => break,
                }
            }
        });

        Ok(Self {
            addr,
            token,
            shutdown,
            handle: Some(handle),
        })
    }

    pub fn endpoint_url(&self) -> String {
        format!("http://{}/mcp", self.addr)
    }

    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    pub fn bearer_token(&self) -> &str {
        &self.token
    }

    /// Claude Desktop / Cursor / Codex 等へそのまま貼り付けできる MCP クライアント設定 JSON を生成する
    pub fn client_config_json(&self) -> String {
        serde_json::to_string_pretty(&json!({
            "mcpServers": {
                "next-gen-video-editor": {
                    "type": "streamable-http",
                    "url": self.endpoint_url(),
                    "headers": {
                        "Authorization": format!("Bearer {}", self.token)
                    }
                }
            }
        }))
        .unwrap_or_default()
    }
}

impl Drop for EmbeddedMcpServer {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

fn handle_connection(mut stream: TcpStream, session: &SharedEditorSession, expected_token: &str) {
    let mut reader = BufReader::new(match stream.try_clone() {
        Ok(s) => s,
        Err(_) => return,
    });

    let request_line = match read_bounded_line(&mut reader, MAX_HEADER_LINE_BYTES) {
        Ok(Some(line)) if !line.trim().is_empty() => line,
        Ok(_) => return,
        Err(_) => {
            write_http_json(
                &mut stream,
                "431 Request Header Fields Too Large",
                &json!({"error": "request line too long or invalid"}),
            );
            return;
        }
    };

    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or("");
    let path = parts.next().unwrap_or("");

    let mut content_length: usize = 0;
    let mut auth_header: Option<String> = None;
    let mut host_header: Option<String> = None;
    let mut origin_header: Option<String> = None;
    let mut header_count: usize = 0;

    loop {
        let line = match read_bounded_line(&mut reader, MAX_HEADER_LINE_BYTES) {
            Ok(Some(l)) => l,
            Ok(None) => break,
            Err(_) => {
                write_http_json(
                    &mut stream,
                    "431 Request Header Fields Too Large",
                    &json!({"error": "header line exceeds limit"}),
                );
                return;
            }
        };
        let trimmed = line.trim();
        if trimmed.is_empty() {
            break;
        }
        header_count += 1;
        if header_count > MAX_HEADER_COUNT {
            write_http_json(
                &mut stream,
                "431 Request Header Fields Too Large",
                &json!({"error": "too many headers"}),
            );
            return;
        }
        if let Some((k, v)) = trimmed.split_once(':') {
            let k_lower = k.trim().to_ascii_lowercase();
            let v_trim = v.trim();
            match k_lower.as_str() {
                "content-length" => match v_trim.parse::<usize>() {
                    Ok(n) => content_length = n,
                    Err(_) => {
                        write_http_json(
                            &mut stream,
                            "400 Bad Request",
                            &json!({"error": "invalid Content-Length header"}),
                        );
                        return;
                    }
                },
                "authorization" => auth_header = Some(v_trim.to_string()),
                "host" => host_header = Some(v_trim.to_string()),
                "origin" => origin_header = Some(v_trim.to_string()),
                _ => {}
            }
        }
    }

    // DNS リバインディング＆ブラウザ CSRF 防御
    if let Some(ref host) = host_header
        && !is_loopback_host(host)
    {
        write_http_json(
            &mut stream,
            "403 Forbidden",
            &json!({"error": "non-loopback Host header rejected"}),
        );
        return;
    }
    if let Some(ref origin) = origin_header
        && !is_loopback_origin(origin)
    {
        write_http_json(
            &mut stream,
            "403 Forbidden",
            &json!({"error": "cross-origin request rejected"}),
        );
        return;
    }

    // 定数時間 Bearer トークン検証
    let expected_header = format!("Bearer {expected_token}");
    let auth_ok = auth_header
        .as_deref()
        .is_some_and(|h| constant_time_eq(h.as_bytes(), expected_header.as_bytes()));
    if !auth_ok {
        write_http_json(
            &mut stream,
            "401 Unauthorized",
            &json!({"error": "invalid or missing bearer token"}),
        );
        return;
    }

    if method != "POST" || path != "/mcp" {
        write_http_json(
            &mut stream,
            "404 Not Found",
            &json!({"error": "use POST /mcp"}),
        );
        return;
    }

    if content_length > MAX_BODY_BYTES {
        write_http_json(
            &mut stream,
            "413 Payload Too Large",
            &json!({"error": format!("request body exceeds {} bytes limit", MAX_BODY_BYTES)}),
        );
        return;
    }

    let mut body = vec![0u8; content_length];
    if reader.read_exact(&mut body).is_err() {
        write_http_json(
            &mut stream,
            "400 Bad Request",
            &json!({"error": "short body"}),
        );
        return;
    }

    let req: Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(e) => {
            write_http_json(
                &mut stream,
                "400 Bad Request",
                &json!({"jsonrpc": "2.0", "id": null, "error": {"code": -32700, "message": e.to_string()}}),
            );
            return;
        }
    };

    let response = dispatch_jsonrpc(session, &req);
    write_http_json(&mut stream, "200 OK", &response);
}

fn dispatch_jsonrpc(session: &SharedEditorSession, req: &Value) -> Value {
    let id = req.get("id").cloned().unwrap_or(Value::Null);
    let method = req.get("method").and_then(Value::as_str).unwrap_or("");
    let params = req.get("params").cloned().unwrap_or(json!({}));

    match method {
        "initialize" => json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "protocolVersion": "2025-03-26",
                "capabilities": { "tools": {} },
                "serverInfo": {
                    "name": "ve-embedded-mcp",
                    "version": env!("CARGO_PKG_VERSION")
                }
            }
        }),
        "tools/list" => json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": { "tools": tool_definitions() }
        }),
        "tools/call" => {
            let tool_name = params.get("name").and_then(Value::as_str).unwrap_or("");
            let args = params.get("arguments").cloned().unwrap_or(json!({}));
            match call_tool(session, tool_name, &args) {
                Ok(res) => json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": res
                }),
                Err(msg) => json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "error": { "code": -32000, "message": msg }
                }),
            }
        }
        _ => json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": { "code": -32601, "message": format!("method not found: {method}") }
        }),
    }
}

fn write_http_json(stream: &mut TcpStream, status: &str, body: &Value) {
    let payload = serde_json::to_vec(body).unwrap_or_default();
    let header = format!(
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        payload.len()
    );
    let _ = stream.write_all(header.as_bytes());
    let _ = stream.write_all(&payload);
    let _ = stream.flush();
    // RFC 7230 §6.6 graceful close: 送信側のみ先に FIN を送り、未読の受信ボディ残存により
    // Windows Winsock が RST (WSAECONNRESET 10054) を返してレスポンス読取を中断するのを防ぐ
    let _ = stream.shutdown(std::net::Shutdown::Write);
    let _ = stream.set_read_timeout(Some(Duration::from_millis(50)));
    let mut drain = [0u8; 512];
    let mut drained = 0usize;
    while drained < 64 * 1024 {
        match stream.read(&mut drain) {
            Ok(0) | Err(_) => break,
            Ok(n) => drained += n,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::EditorSession;
    use std::sync::Mutex;
    use ve_core::{Project, ProjectSettings};

    fn http_post(addr: SocketAddr, token: Option<&str>, body: &Value) -> (String, Value) {
        let mut stream = TcpStream::connect(addr).unwrap();
        let bytes = serde_json::to_vec(body).unwrap();
        let auth = token
            .map(|t| format!("Authorization: Bearer {t}\r\n"))
            .unwrap_or_default();
        let req = format!(
            "POST /mcp HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\n{auth}Content-Length: {}\r\nConnection: close\r\n\r\n",
            bytes.len()
        );
        stream.write_all(req.as_bytes()).unwrap();
        stream.write_all(&bytes).unwrap();
        stream.flush().unwrap();

        let mut resp = String::new();
        stream.read_to_string(&mut resp).unwrap();
        let (head, body_str) = resp.split_once("\r\n\r\n").unwrap();
        let status_line = head.lines().next().unwrap().to_string();
        (status_line, serde_json::from_str(body_str).unwrap())
    }

    #[test]
    fn mcp_server_auth_add_object_render_preview_and_undo() {
        let settings = ProjectSettings {
            width: 64,
            height: 36,
            ..Default::default()
        };
        let project = Project::new("McpTest", settings);
        let session = Arc::new(Mutex::new(EditorSession::new(project)));
        let server = EmbeddedMcpServer::start(Arc::clone(&session)).unwrap();

        // 認証トークン無しは 401
        let (status, _) = http_post(
            server.addr(),
            None,
            &json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"}),
        );
        assert!(status.contains("401"));

        // トークン付きで tools/list
        let (status, list_res) = http_post(
            server.addr(),
            Some(server.bearer_token()),
            &json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}),
        );
        assert!(status.contains("200"));
        assert!(list_res["result"]["tools"].as_array().unwrap().len() >= 10);

        // AI から add_object 実行
        let (status, add_res) = http_post(
            server.addr(),
            Some(server.bearer_token()),
            &json!({
                "jsonrpc": "2.0",
                "id": 3,
                "method": "tools/call",
                "params": {
                    "name": "add_object",
                    "arguments": {
                        "layer_index": 0,
                        "name": "AI Rect",
                        "kind": "shape",
                        "width": 32,
                        "height": 18
                    }
                }
            }),
        );
        assert!(status.contains("200"));
        assert!(add_res.get("error").is_none());

        // render_frame_preview が PNG (base64) を返すこと
        let (status, preview_res) = http_post(
            server.addr(),
            Some(server.bearer_token()),
            &json!({
                "jsonrpc": "2.0",
                "id": 4,
                "method": "tools/call",
                "params": {
                    "name": "render_frame_preview",
                    "arguments": { "frame": 0, "scale": 1.0 }
                }
            }),
        );
        assert!(status.contains("200"));
        assert_eq!(preview_res["result"]["content"][0]["mimeType"], "image/png");
        assert!(
            !preview_res["result"]["content"][0]["data"]
                .as_str()
                .unwrap()
                .is_empty()
        );

        // AI が追加したオブジェクトは GUI と同じく Ctrl+Z (Undo) で取り消せる
        {
            let mut guard = session.lock().unwrap();
            assert_eq!(
                guard.project.active_scene().unwrap().layers[0].items.len(),
                1
            );
            let sess = &mut *guard;
            assert!(sess.bus.undo(&mut sess.project));
            assert_eq!(
                sess.project.active_scene().unwrap().layers[0].items.len(),
                0
            );
        }
    }

    fn send_raw_http(addr: SocketAddr, raw_req: &str) -> String {
        let mut stream = TcpStream::connect(addr).unwrap();
        stream.write_all(raw_req.as_bytes()).unwrap();
        stream.flush().unwrap();
        let mut resp = String::new();
        stream.read_to_string(&mut resp).unwrap();
        resp.lines().next().unwrap_or("").to_string()
    }

    #[test]
    fn mcp_server_blocks_dns_rebinding_csrf_and_oversized_payloads() {
        assert!(constant_time_eq(b"secret", b"secret"));
        assert!(!constant_time_eq(b"secret", b"secreT"));
        assert!(!constant_time_eq(b"secret", b"secret1"));

        assert!(is_loopback_host("127.0.0.1:8080"));
        assert!(is_loopback_host("localhost:3000"));
        assert!(is_loopback_host("[::1]:8080"));
        assert!(!is_loopback_host("evil.example.com"));
        assert!(!is_loopback_host("localhost.evil.com"));
        assert!(!is_loopback_origin("https://evil.example.com"));
        assert!(!is_loopback_origin("http://localhost.evil.com"));
        assert!(is_loopback_origin("http://127.0.0.1:5173"));

        let project = Project::new("SecMcp", ProjectSettings::default());
        let session = Arc::new(Mutex::new(EditorSession::new(project)));
        let server = EmbeddedMcpServer::start(Arc::clone(&session)).unwrap();
        let token = server.bearer_token();

        // 1) DNS リバインディング: 非ループバック Host は 403
        let bad_host_req = format!(
            "POST /mcp HTTP/1.1\r\nHost: evil.example.com\r\nAuthorization: Bearer {token}\r\nContent-Length: 2\r\n\r\n{{}}"
        );
        assert!(send_raw_http(server.addr(), &bad_host_req).contains("403"));

        // 2) ブラウザ CSRF: クロスオリジン Origin は 403
        let bad_origin_req = format!(
            "POST /mcp HTTP/1.1\r\nHost: 127.0.0.1\r\nOrigin: https://attacker.example\r\nAuthorization: Bearer {token}\r\nContent-Length: 2\r\n\r\n{{}}"
        );
        assert!(send_raw_http(server.addr(), &bad_origin_req).contains("403"));

        // 3) 4 MiB 超過ペイロードはサイレント切り詰めせず 413
        let big_len = MAX_BODY_BYTES + 1;
        let big_req = format!(
            "POST /mcp HTTP/1.1\r\nHost: 127.0.0.1\r\nAuthorization: Bearer {token}\r\nContent-Length: {big_len}\r\n\r\n"
        );
        assert!(send_raw_http(server.addr(), &big_req).contains("413"));
    }
}
