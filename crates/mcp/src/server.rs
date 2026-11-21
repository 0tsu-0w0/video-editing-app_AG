use crate::session::SharedEditorSession;
use crate::tools::{call_tool, tool_definitions};
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum McpServerError {
    #[error("Failed to bind MCP server socket: {0}")]
    Bind(#[from] std::io::Error),
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

        let thread_token = token.clone();
        let thread_shutdown = Arc::clone(&shutdown);
        let handle = thread::spawn(move || {
            while !thread_shutdown.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        let _ = stream.set_nonblocking(false);
                        let sess = Arc::clone(&session);
                        let expected_token = thread_token.clone();
                        thread::spawn(move || {
                            handle_connection(stream, &sess, &expected_token);
                        });
                    }
                    Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(std::time::Duration::from_millis(15));
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

    let mut request_line = String::new();
    if reader.read_line(&mut request_line).is_err() || request_line.is_empty() {
        return;
    }

    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or("");
    let path = parts.next().unwrap_or("");

    let mut content_length: usize = 0;
    let mut auth_header: Option<String> = None;

    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).is_err() {
            return;
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            break;
        }
        if let Some((k, v)) = trimmed.split_once(':') {
            let k_lower = k.trim().to_ascii_lowercase();
            let v_trim = v.trim();
            if k_lower == "content-length" {
                content_length = v_trim.parse().unwrap_or(0);
            } else if k_lower == "authorization" {
                auth_header = Some(v_trim.to_string());
            }
        }
    }

    // Bearer トークン検証
    let expected_header = format!("Bearer {expected_token}");
    if auth_header.as_deref() != Some(expected_header.as_str()) {
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

    let mut body = vec![0u8; content_length.min(16 * 1024 * 1024)];
    if reader.read_exact(&mut body).is_err() {
        write_http_json(&mut stream, "400 Bad Request", &json!({"error": "short body"}));
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
        let mut settings = ProjectSettings::default();
        settings.width = 64;
        settings.height = 36;
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
        assert!(!preview_res["result"]["content"][0]["data"].as_str().unwrap().is_empty());

        // AI が追加したオブジェクトは GUI と同じく Ctrl+Z (Undo) で取り消せる
        {
            let mut guard = session.lock().unwrap();
            assert_eq!(guard.project.active_scene().unwrap().layers[0].items.len(), 1);
            let sess = &mut *guard;
            assert!(sess.bus.undo(&mut sess.project));
            assert_eq!(sess.project.active_scene().unwrap().layers[0].items.len(), 0);
        }
    }
}
