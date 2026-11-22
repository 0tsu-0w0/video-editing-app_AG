//! v3.0 #32: リアルタイム P2P / LAN ＆ CRDT 同時共同編集（クラウド不要ローカルチーム同期）
//! Lamport 論理時計＋Vector Clock＋決定論的全順序（`(lamport, peer_id, op_id)`）による
//! 可換・結合・冪等な Op-Based CRDT と、Pure-Rust 標準 TCP JSON-Lines ピア同期トランスポートを提供する。

use crate::project::{
    ItemId, ItemKind, Project, SceneColorGrade, TimelineItem, TimelineMarker, Transform2D,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use uuid::Uuid;

pub type PeerId = String;

/// ピアごとのライブカーソル／再生ヘッド在席状態（プレゼンス）
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PeerCursorState {
    pub peer_id: PeerId,
    pub display_name: String,
    pub color_hex: String,
    pub playhead_sec: f64,
    pub selected_item: Option<ItemId>,
    pub updated_at_ms: u64,
}

/// CRDT で同期される編集オペレーション種別（LWW / Add-Remove セマンティクス）
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum CrdtOpKind {
    UpsertItem {
        layer_index: usize,
        item: Box<TimelineItem>,
    },
    DeleteItem {
        item_id: ItemId,
    },
    SetTransform {
        item_id: ItemId,
        transform: Transform2D,
    },
    SetTextContent {
        item_id: ItemId,
        content: String,
    },
    AddMarker {
        marker: TimelineMarker,
    },
    SetSceneColorGrade {
        grade: SceneColorGrade,
    },
    CursorPresence {
        cursor: PeerCursorState,
    },
}

/// Lamport 論理タイムスタンプ付き CRDT オペレーション（全ピア間で同一順序に収束）
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CrdtOperation {
    pub op_id: Uuid,
    pub peer_id: PeerId,
    pub lamport: u64,
    pub created_ms: u64,
    pub kind: CrdtOpKind,
}

/// Op-Based CRDT 同期ログ（可換・結合・冪等マージ保証）
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CrdtSyncLog {
    pub local_peer_id: PeerId,
    pub display_name: String,
    pub color_hex: String,
    pub lamport_clock: u64,
    pub vector_clock: BTreeMap<PeerId, u64>,
    pub operations: Vec<CrdtOperation>,
    #[serde(default)]
    pub seen_ids: BTreeSet<Uuid>,
    #[serde(default)]
    pub peers: BTreeMap<PeerId, PeerCursorState>,
}

fn now_epoch_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

impl CrdtSyncLog {
    pub fn new(
        peer_id: impl Into<String>,
        display_name: impl Into<String>,
        color_hex: impl Into<String>,
    ) -> Self {
        Self {
            local_peer_id: peer_id.into(),
            display_name: display_name.into(),
            color_hex: color_hex.into(),
            lamport_clock: 0,
            vector_clock: BTreeMap::new(),
            operations: Vec::new(),
            seen_ids: BTreeSet::new(),
            peers: BTreeMap::new(),
        }
    }

    /// ローカル編集を Lamport 時計を 1 進めて記録する
    pub fn record_local(&mut self, kind: CrdtOpKind) -> CrdtOperation {
        self.lamport_clock += 1;
        let op = CrdtOperation {
            op_id: Uuid::new_v4(),
            peer_id: self.local_peer_id.clone(),
            lamport: self.lamport_clock,
            created_ms: now_epoch_ms(),
            kind,
        };
        self.apply_bookkeeping(&op);
        self.operations.push(op.clone());
        op
    }

    /// ライブカーソル／再生ヘッド位置を更新してプレゼンス操作を発行する
    pub fn update_local_cursor(
        &mut self,
        playhead_sec: f64,
        selected_item: Option<ItemId>,
    ) -> CrdtOperation {
        let cursor = PeerCursorState {
            peer_id: self.local_peer_id.clone(),
            display_name: self.display_name.clone(),
            color_hex: self.color_hex.clone(),
            playhead_sec: playhead_sec.max(0.0),
            selected_item,
            updated_at_ms: now_epoch_ms(),
        };
        self.record_local(CrdtOpKind::CursorPresence { cursor })
    }

    /// リモートピアからのオペレーション列を重複排除しつつマージする（順序非依存・冪等）
    pub fn merge_remote_ops(&mut self, remote_ops: &[CrdtOperation]) -> usize {
        let mut added = 0usize;
        for op in remote_ops {
            if self.seen_ids.contains(&op.op_id) {
                continue;
            }
            self.lamport_clock = self.lamport_clock.max(op.lamport) + 1;
            self.apply_bookkeeping(op);
            self.operations.push(op.clone());
            added += 1;
        }
        added
    }

    fn apply_bookkeeping(&mut self, op: &CrdtOperation) {
        self.seen_ids.insert(op.op_id);
        let entry = self.vector_clock.entry(op.peer_id.clone()).or_insert(0);
        if op.lamport > *entry {
            *entry = op.lamport;
        }
        if let CrdtOpKind::CursorPresence { cursor } = &op.kind {
            let replace = self
                .peers
                .get(&cursor.peer_id)
                .map(|prev| cursor.updated_at_ms >= prev.updated_at_ms)
                .unwrap_or(true);
            if replace {
                self.peers.insert(cursor.peer_id.clone(), cursor.clone());
            }
        }
    }

    /// 全ピア間で一意に定まる決定論的全順序 `(lamport, peer_id, op_id)` で操作列を返す
    pub fn canonical_ops(&self) -> Vec<&CrdtOperation> {
        let mut sorted: Vec<&CrdtOperation> = self.operations.iter().collect();
        sorted.sort_by(|a, b| {
            a.lamport
                .cmp(&b.lamport)
                .then_with(|| a.peer_id.cmp(&b.peer_id))
                .then_with(|| a.op_id.cmp(&b.op_id))
        });
        sorted
    }

    /// ベースプロジェクトへ正規順序の CRDT 操作列を決定論的に適用し、収束済み `Project` を生成する
    pub fn apply_to_project(&self, base: &Project) -> Project {
        let mut proj = base.clone();
        let mut deleted_items: BTreeSet<ItemId> = BTreeSet::new();

        for op in self.canonical_ops() {
            let Some(scene) = proj.active_scene_mut() else {
                break;
            };
            match &op.kind {
                CrdtOpKind::UpsertItem { layer_index, item } => {
                    deleted_items.remove(&item.id);
                    // 既存位置があれば置換、無ければ指定レイヤーへ挿入
                    let mut updated = false;
                    for layer in &mut scene.layers {
                        if let Some(existing) = layer.items.iter_mut().find(|i| i.id == item.id) {
                            *existing = (**item).clone();
                            updated = true;
                            break;
                        }
                    }
                    if !updated {
                        let li = (*layer_index).min(scene.layers.len().saturating_sub(1));
                        if let Some(layer) = scene.layers.get_mut(li) {
                            layer.items.push((**item).clone());
                            layer.items.sort_by_key(|i| i.start);
                        }
                    }
                }
                CrdtOpKind::DeleteItem { item_id } => {
                    deleted_items.insert(*item_id);
                    for layer in &mut scene.layers {
                        layer.items.retain(|i| i.id != *item_id);
                    }
                }
                CrdtOpKind::SetTransform { item_id, transform } => {
                    if deleted_items.contains(item_id) {
                        continue;
                    }
                    for layer in &mut scene.layers {
                        if let Some(it) = layer.items.iter_mut().find(|i| i.id == *item_id) {
                            it.transform = *transform;
                            break;
                        }
                    }
                }
                CrdtOpKind::SetTextContent { item_id, content } => {
                    if deleted_items.contains(item_id) {
                        continue;
                    }
                    for layer in &mut scene.layers {
                        if let Some(it) = layer.items.iter_mut().find(|i| i.id == *item_id)
                            && let ItemKind::Text {
                                content: ref mut c, ..
                            } = it.kind
                        {
                            *c = content.clone();
                            break;
                        }
                    }
                }
                CrdtOpKind::AddMarker { marker } => {
                    if let Some(existing) = scene.markers.iter_mut().find(|m| m.id == marker.id) {
                        *existing = marker.clone();
                    } else {
                        scene.markers.push(marker.clone());
                        scene.markers.sort_by_key(|m| m.time);
                    }
                }
                CrdtOpKind::SetSceneColorGrade { grade } => {
                    scene.color_grade = grade.clone();
                }
                CrdtOpKind::CursorPresence { .. } => {}
            }
        }
        proj
    }

    /// 相手ピアの Vector Clock より新しいデルタ操作のみを JSON 文字列として書き出す
    pub fn export_delta_json(&self, remote_vector: Option<&BTreeMap<PeerId, u64>>) -> String {
        let delta: Vec<&CrdtOperation> = self
            .operations
            .iter()
            .filter(|op| {
                let known = remote_vector
                    .and_then(|vc| vc.get(&op.peer_id).copied())
                    .unwrap_or(0);
                op.lamport > known
            })
            .collect();
        serde_json::to_string_pretty(&delta).unwrap_or_else(|_| "[]".into())
    }

    /// JSON デルタ文字列をパースする
    pub fn parse_delta_json(json_str: &str) -> Result<Vec<CrdtOperation>, String> {
        serde_json::from_str::<Vec<CrdtOperation>>(json_str)
            .map_err(|e| format!("invalid CRDT delta JSON: {e}"))
    }
}

/// Pure-Rust LAN / Localhost TCP JSON-Lines ピア同期サーバー（クラウド不要・セッショントークン認証付）
#[derive(Debug, Clone, Serialize, Deserialize)]
struct LanSyncPacket {
    token: String,
    ops: Vec<CrdtOperation>,
}

pub struct LanCollabHost {
    pub bound_addr: String,
    stop: Arc<AtomicBool>,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl LanCollabHost {
    /// 指定アドレス（例: `"127.0.0.1:0"` または `"0.0.0.0:17890"`）で LAN CRDT 同期ホストを起動する
    pub fn start(
        bind_addr: &str,
        session_token: String,
        shared_log: Arc<Mutex<CrdtSyncLog>>,
    ) -> Result<Self, String> {
        let listener =
            TcpListener::bind(bind_addr).map_err(|e| format!("LAN host bind failed: {e}"))?;
        let bound_addr = listener
            .local_addr()
            .map_err(|e| format!("local_addr failed: {e}"))?
            .to_string();
        listener
            .set_nonblocking(true)
            .map_err(|e| format!("set_nonblocking failed: {e}"))?;

        let stop = Arc::new(AtomicBool::new(false));
        let stop_bg = Arc::clone(&stop);

        let handle = std::thread::spawn(move || {
            while !stop_bg.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        let _ = handle_lan_client(stream, &session_token, &shared_log);
                    }
                    Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(20));
                    }
                    Err(_) => break,
                }
            }
        });

        Ok(Self {
            bound_addr,
            stop,
            handle: Some(handle),
        })
    }

    pub fn stop(mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

impl Drop for LanCollabHost {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

fn handle_lan_client(
    stream: TcpStream,
    expected_token: &str,
    shared_log: &Arc<Mutex<CrdtSyncLog>>,
) -> Result<(), String> {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(3)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(3)));
    let mut reader = BufReader::new(stream.try_clone().map_err(|e| e.to_string())?);
    let mut line = String::new();
    reader.read_line(&mut line).map_err(|e| e.to_string())?;
    let req: LanSyncPacket = serde_json::from_str(line.trim()).map_err(|e| e.to_string())?;
    if req.token != expected_token {
        return Err("unauthorized LAN sync token".into());
    }
    let reply_ops = {
        let mut log = shared_log.lock().map_err(|_| "lock poisoned".to_string())?;
        log.merge_remote_ops(&req.ops);
        log.operations.clone()
    };
    let resp = LanSyncPacket {
        token: expected_token.to_string(),
        ops: reply_ops,
    };
    let mut writer = stream;
    let payload = serde_json::to_string(&resp).map_err(|e| e.to_string())?;
    writeln!(writer, "{payload}").map_err(|e| e.to_string())?;
    let _ = writer.flush();
    Ok(())
}

/// クライアント側: リモート LAN ホストへローカル CRDT 操作を送信し、双方向マージ結果を受信する
pub fn sync_crdt_with_lan_peer(
    host_addr: &str,
    session_token: &str,
    local_log: &mut CrdtSyncLog,
) -> Result<usize, String> {
    let mut stream =
        TcpStream::connect(host_addr).map_err(|e| format!("LAN peer connect failed: {e}"))?;
    let _ = stream.set_read_timeout(Some(Duration::from_secs(3)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(3)));
    let pkt = LanSyncPacket {
        token: session_token.to_string(),
        ops: local_log.operations.clone(),
    };
    let payload = serde_json::to_string(&pkt).map_err(|e| e.to_string())?;
    writeln!(stream, "{payload}").map_err(|e| e.to_string())?;
    stream.flush().map_err(|e| e.to_string())?;

    let mut reader = BufReader::new(stream);
    let mut resp_line = String::new();
    reader
        .read_line(&mut resp_line)
        .map_err(|e| format!("LAN peer read failed: {e}"))?;
    let resp: LanSyncPacket =
        serde_json::from_str(resp_line.trim()).map_err(|e| format!("invalid peer reply: {e}"))?;
    Ok(local_log.merge_remote_ops(&resp.ops))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::{ProjectSettings, RgbaColor, TextAnimator};
    use crate::time::RationalTime;

    #[test]
    fn crdt_converges_deterministically_under_concurrent_out_of_order_and_duplicate_ops() {
        let base = Project::new("CrdtBase", ProjectSettings::default());
        let shared_sub = TimelineItem::new(
            "Sub1",
            RationalTime::ZERO,
            RationalTime::from_seconds(3.0),
            ItemKind::Text {
                content: "初期テキスト".into(),
                font_family: "Sans".into(),
                font_size: 40.0,
                color: RgbaColor::WHITE,
                animator: TextAnimator::default(),
            },
        );
        let sub_id = shared_sub.id;

        let mut peer_a = CrdtSyncLog::new("peer_a", "Alice", "#44ccff");
        let mut peer_b = CrdtSyncLog::new("peer_b", "Bob", "#ff6688");

        let op_seed = peer_a.record_local(CrdtOpKind::UpsertItem {
            layer_index: 1,
            item: Box::new(shared_sub),
        });
        peer_b.merge_remote_ops(&[op_seed]);

        // 同時並列編集（A: テキスト更新＋カーソル移動, B: 座標更新＋マーカー追加）
        let a1 = peer_a.record_local(CrdtOpKind::SetTextContent {
            item_id: sub_id,
            content: "Alice が編集した字幕".into(),
        });
        let a2 = peer_a.update_local_cursor(1.25, Some(sub_id));

        let moved_tf = Transform2D {
            x: 120.0,
            y: 480.0,
            ..Transform2D::default()
        };
        let b1 = peer_b.record_local(CrdtOpKind::SetTransform {
            item_id: sub_id,
            transform: moved_tf,
        });
        let b2 = peer_b.record_local(CrdtOpKind::AddMarker {
            marker: TimelineMarker::new(RationalTime::from_seconds(2.0), "Bob Checkpoint"),
        });

        // 逆順かつ重複パケットで交換しても、両ピアは完全に同一状態へ収束する
        assert_eq!(peer_a.merge_remote_ops(&[b2.clone(), b1.clone(), b2]), 2);
        assert_eq!(peer_b.merge_remote_ops(&[a2.clone(), a1.clone(), a1]), 2);

        let proj_a = peer_a.apply_to_project(&base);
        let proj_b = peer_b.apply_to_project(&base);
        assert_eq!(proj_a, proj_b, "CRDT peers must converge identically");
        assert_eq!(proj_a.active_scene().unwrap().markers.len(), 1);
        assert!(peer_b.peers.contains_key("peer_a"));
    }

    #[test]
    fn lan_tcp_peer_sync_exchanges_crdt_operations_with_token_auth() {
        let host_log = Arc::new(Mutex::new(CrdtSyncLog::new("host", "Host", "#22dd88")));
        host_log.lock().unwrap().update_local_cursor(4.5, None);

        let host = LanCollabHost::start("127.0.0.1:0", "secret-v30".into(), Arc::clone(&host_log))
            .expect("start LAN host");
        let addr = host.bound_addr.clone();

        let mut client_log = CrdtSyncLog::new("client", "Client", "#ffaa22");
        client_log.update_local_cursor(1.0, None);

        // 不正トークンは拒否される
        assert!(sync_crdt_with_lan_peer(&addr, "wrong-token", &mut client_log).is_err());

        // 正規トークンで双方向同期
        let received = sync_crdt_with_lan_peer(&addr, "secret-v30", &mut client_log)
            .expect("LAN sync should succeed");
        assert_eq!(received, 1);
        assert!(client_log.peers.contains_key("host"));
        assert!(host_log.lock().unwrap().peers.contains_key("client"));

        host.stop();
    }
}
