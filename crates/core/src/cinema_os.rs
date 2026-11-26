//! v10.0 #60: LAN 分散レンダーファーム ＆ 自律 AI シネマプロデューサー（台本→絵コンテ・A/B 自動編集・視聴維持率予測）
//!
//! Pure Rust・ゼロ `unsafe`・外部追加依存ゼロで以下を完全オフライン／ローカル LAN で実現する:
//! - RFC 2104 準拠 Pure-Rust `hmac_sha256_hex` 認証付き LAN 分散レンダーファームチャンク計画＆検証
//! - ローカル非ブロッキング TCP HTTP/1.1 コーディネータサーバー（`RenderFarmCoordinatorServer`）＋ FFmpeg concat 結合マニフェスト
//! - 自律 AI 台本解析 → プロシージャル絵コンテ（ショットフレーミング・尺・ムード配色）→ プレイアブルアニマティック 1-Undo 生成
//! - タイムライン編集リズム・フック強度・字幕密度・視覚多様性に基づく秒別視聴維持率予測＆離脱リスク診断
//! - 戦略別 A/B 自動編集バリアントシーン生成（`FastPacedShorts` / `CinematicNarrative` / `RetentionBoostHook`）

use crate::command::EditCommand;
use crate::finishing::CinemaFinishingConfig;
use crate::project::{
    ItemKind, Project, RgbaColor, Scene, SceneId, ShapeKind, TextAnimator, TimelineItem,
    TimelineMarker,
};
use crate::registry::sha256_hex;
use crate::time::{FrameRate, RationalTime};
use serde::{Deserialize, Serialize};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use uuid::Uuid;

// =========================================================================
// RFC 2104 HMAC-SHA256（Pure Rust / ゼロ unsafe）
// =========================================================================

fn hex_to_bytes_32(hex: &str) -> [u8; 32] {
    let mut out = [0u8; 32];
    let bytes = hex.as_bytes();
    for (i, slot) in out.iter_mut().enumerate() {
        let hi = bytes.get(i * 2).copied().unwrap_or(b'0');
        let lo = bytes.get(i * 2 + 1).copied().unwrap_or(b'0');
        let nibble = |c: u8| match c {
            b'0'..=b'9' => c - b'0',
            b'a'..=b'f' => c - b'a' + 10,
            b'A'..=b'F' => c - b'A' + 10,
            _ => 0,
        };
        *slot = (nibble(hi) << 4) | nibble(lo);
    }
    out
}

/// RFC 2104 準拠 HMAC-SHA256 ダイジェスト（64 文字小文字 hex）
pub fn hmac_sha256_hex(secret: &str, message: &[u8]) -> String {
    const BLOCK_SIZE: usize = 64;
    let mut key_block = [0u8; BLOCK_SIZE];
    let key_bytes = secret.as_bytes();
    if key_bytes.len() > BLOCK_SIZE {
        let hashed = hex_to_bytes_32(&sha256_hex(key_bytes));
        key_block[..32].copy_from_slice(&hashed);
    } else {
        key_block[..key_bytes.len()].copy_from_slice(key_bytes);
    }

    let mut ipad = [0x36u8; BLOCK_SIZE];
    let mut opad = [0x5cu8; BLOCK_SIZE];
    for i in 0..BLOCK_SIZE {
        ipad[i] ^= key_block[i];
        opad[i] ^= key_block[i];
    }

    let mut inner = Vec::with_capacity(BLOCK_SIZE + message.len());
    inner.extend_from_slice(&ipad);
    inner.extend_from_slice(message);
    let inner_hash = hex_to_bytes_32(&sha256_hex(&inner));

    let mut outer = Vec::with_capacity(BLOCK_SIZE + 32);
    outer.extend_from_slice(&opad);
    outer.extend_from_slice(&inner_hash);
    sha256_hex(&outer)
}

// =========================================================================
// LAN 分散レンダーファーム
// =========================================================================

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FarmChunkStatus {
    #[default]
    Pending,
    Claimed,
    Completed,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RenderFarmWorkerNode {
    pub worker_id: String,
    pub host: String,
    pub port: u16,
    pub cpu_threads: u32,
    pub weight: f32,
    #[serde(default = "default_true")]
    pub healthy: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RenderFarmChunk {
    pub chunk_id: usize,
    pub worker_id: String,
    pub start_frame: u64,
    pub end_frame: u64,
    #[serde(default)]
    pub status: FarmChunkStatus,
    pub hmac_signature: String,
    pub output_segment_path: String,
    #[serde(default)]
    pub elapsed_ms: u64,
}

/// プロジェクト単位の LAN 分散レンダーファーム設定（`Project::render_farm`）
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RenderFarmConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_farm_port")]
    pub coordinator_port: u16,
    #[serde(default = "default_farm_token")]
    pub bearer_token: String,
    #[serde(default = "default_farm_secret")]
    pub hmac_secret: String,
    #[serde(default = "default_total_frames")]
    pub total_frames: u64,
    #[serde(default = "default_farm_fps")]
    pub fps: f64,
    #[serde(default = "default_farm_workers")]
    pub workers: Vec<RenderFarmWorkerNode>,
    #[serde(default)]
    pub chunks: Vec<RenderFarmChunk>,
}

fn default_true() -> bool {
    true
}
fn default_farm_port() -> u16 {
    19480
}
fn default_farm_token() -> String {
    "ve-lan-farm-local-token".into()
}
fn default_farm_secret() -> String {
    "ve-lan-farm-hmac-secret-v10".into()
}
fn default_total_frames() -> u64 {
    600
}
fn default_farm_fps() -> f64 {
    30.0
}
fn default_farm_workers() -> Vec<RenderFarmWorkerNode> {
    vec![
        RenderFarmWorkerNode {
            worker_id: "node-local-main".into(),
            host: "127.0.0.1".into(),
            port: 19481,
            cpu_threads: 16,
            weight: 1.5,
            healthy: true,
        },
        RenderFarmWorkerNode {
            worker_id: "node-lan-gpu02".into(),
            host: "192.168.1.22".into(),
            port: 19481,
            cpu_threads: 12,
            weight: 1.0,
            healthy: true,
        },
        RenderFarmWorkerNode {
            worker_id: "node-lan-render03".into(),
            host: "192.168.1.23".into(),
            port: 19481,
            cpu_threads: 8,
            weight: 0.75,
            healthy: true,
        },
    ]
}

impl Default for RenderFarmConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            coordinator_port: default_farm_port(),
            bearer_token: default_farm_token(),
            hmac_secret: default_farm_secret(),
            total_frames: default_total_frames(),
            fps: default_farm_fps(),
            workers: default_farm_workers(),
            chunks: Vec::new(),
        }
    }
}

fn chunk_payload_string(
    chunk_id: usize,
    worker_id: &str,
    start_frame: u64,
    end_frame: u64,
) -> String {
    format!("{chunk_id}:{worker_id}:{start_frame}:{end_frame}")
}

/// 健全なワーカーノードの重みに比例して全フレームを連続チャンク分割し、HMAC-SHA256 署名を付与する
pub fn plan_render_farm_chunks(cfg: &RenderFarmConfig) -> Vec<RenderFarmChunk> {
    let healthy: Vec<&RenderFarmWorkerNode> = cfg.workers.iter().filter(|w| w.healthy).collect();
    if healthy.is_empty() || cfg.total_frames == 0 {
        return Vec::new();
    }
    let total_weight: f64 = healthy
        .iter()
        .map(|w| (w.weight.max(0.1)) as f64)
        .sum::<f64>()
        .max(0.1);

    let mut chunks = Vec::with_capacity(healthy.len());
    let mut cursor: u64 = 0;
    for (idx, w) in healthy.iter().enumerate() {
        let is_last = idx + 1 == healthy.len();
        let share = if is_last {
            cfg.total_frames.saturating_sub(cursor)
        } else {
            ((cfg.total_frames as f64) * (w.weight.max(0.1) as f64 / total_weight))
                .round()
                .max(1.0) as u64
        };
        let end = (cursor + share).min(cfg.total_frames);
        if end <= cursor {
            continue;
        }
        let payload = chunk_payload_string(idx, &w.worker_id, cursor, end);
        let sig = hmac_sha256_hex(&cfg.hmac_secret, payload.as_bytes());
        chunks.push(RenderFarmChunk {
            chunk_id: idx,
            worker_id: w.worker_id.clone(),
            start_frame: cursor,
            end_frame: end,
            status: FarmChunkStatus::Pending,
            hmac_signature: sig,
            output_segment_path: format!("farm_segments/chunk_{idx:03}_{}.mkv", w.worker_id),
            elapsed_ms: 0,
        });
        cursor = end;
    }
    chunks
}

/// チャンクの HMAC-SHA256 署名を検証する
pub fn verify_chunk_hmac(secret: &str, chunk: &RenderFarmChunk) -> bool {
    let payload = chunk_payload_string(
        chunk.chunk_id,
        &chunk.worker_id,
        chunk.start_frame,
        chunk.end_frame,
    );
    let expected = hmac_sha256_hex(secret, payload.as_bytes());
    expected == chunk.hmac_signature
}

/// FFmpeg concat demuxer による無劣化チャンク結合マニフェストを生成する
pub fn build_farm_concat_manifest(chunks: &[RenderFarmChunk]) -> String {
    let mut ordered = chunks.to_vec();
    ordered.sort_by_key(|c| c.start_frame);
    let mut out = String::from(
        "# VE LAN Render Farm concat manifest (ffmpeg -f concat -safe 0 -i concat.txt -c copy out.mp4)\n",
    );
    for c in ordered {
        let safe = c.output_segment_path.replace('\'', "");
        out.push_str(&format!("file '{safe}'\n"));
    }
    out
}

/// 非ブロッキング `127.0.0.1` LAN レンダーファーム コーディネータ HTTP/1.1 サーバー
pub struct RenderFarmCoordinatorServer {
    listener: TcpListener,
    pub port: u16,
    pub bearer_token: String,
    pub hmac_secret: String,
    pub chunks: Vec<RenderFarmChunk>,
    pub requests_served: u64,
}

impl RenderFarmCoordinatorServer {
    pub fn bind_localhost(cfg: &RenderFarmConfig) -> Result<Self, String> {
        let listener = TcpListener::bind(("127.0.0.1", cfg.coordinator_port))
            .or_else(|_| TcpListener::bind(("127.0.0.1", 0)))
            .map_err(|e| format!("LAN ファームコーディネータ bind 失敗: {e}"))?;
        let port = listener
            .local_addr()
            .map_err(|e| format!("local_addr 取得失敗: {e}"))?
            .port();
        listener
            .set_nonblocking(true)
            .map_err(|e| format!("nonblocking 設定失敗: {e}"))?;
        let chunks = if cfg.chunks.is_empty() {
            plan_render_farm_chunks(cfg)
        } else {
            cfg.chunks.clone()
        };
        Ok(Self {
            listener,
            port,
            bearer_token: cfg.bearer_token.clone(),
            hmac_secret: cfg.hmac_secret.clone(),
            chunks,
            requests_served: 0,
        })
    }

    /// 保留中のワーカー HTTP リクエストを非ブロッキング処理する
    pub fn poll_once(&mut self) -> usize {
        let mut handled = 0usize;
        while let Ok((mut stream, _)) = self.listener.accept() {
            self.handle_connection(&mut stream);
            self.requests_served += 1;
            handled += 1;
            if handled >= 16 {
                break;
            }
        }
        handled
    }

    fn handle_connection(&mut self, stream: &mut TcpStream) {
        let _ = stream.set_read_timeout(Some(std::time::Duration::from_millis(80)));
        let mut buf = [0u8; 4096];
        let n = match stream.read(&mut buf) {
            Ok(n) if n > 0 => n,
            _ => return,
        };
        let req = String::from_utf8_lossy(&buf[..n]);
        let mut lines = req.lines();
        let first = lines.next().unwrap_or("");
        let mut parts = first.split_whitespace();
        let method = parts.next().unwrap_or("");
        let path = parts.next().unwrap_or("/");

        let mut auth_header = String::new();
        for line in lines {
            if line.trim().is_empty() {
                break;
            }
            if let Some((k, v)) = line.split_once(':')
                && k.trim().eq_ignore_ascii_case("authorization")
            {
                auth_header = v.trim().to_string();
            }
        }

        if path == "/health" {
            let body = format!(
                "{{\"status\":\"ok\",\"chunks\":{},\"completed\":{}}}",
                self.chunks.len(),
                self.chunks
                    .iter()
                    .filter(|c| c.status == FarmChunkStatus::Completed)
                    .count()
            );
            write_http_json(stream, "200 OK", &body);
            return;
        }

        // Bearer 認証検証
        let expected_auth = format!("Bearer {}", self.bearer_token);
        if auth_header != expected_auth {
            write_http_json(stream, "401 Unauthorized", "{\"error\":\"invalid_bearer\"}");
            return;
        }

        if method == "POST" && path == "/api/v1/claim" {
            if let Some(chunk) = self
                .chunks
                .iter_mut()
                .find(|c| c.status == FarmChunkStatus::Pending)
            {
                chunk.status = FarmChunkStatus::Claimed;
                let body = serde_json::to_string(chunk).unwrap_or_else(|_| "{}".into());
                write_http_json(stream, "200 OK", &body);
            } else {
                write_http_json(stream, "200 OK", "{\"status\":\"no_pending_chunks\"}");
            }
            return;
        }

        if method == "POST" && path.starts_with("/api/v1/complete/") {
            let id_str = path.trim_start_matches("/api/v1/complete/");
            if let Ok(cid) = id_str.parse::<usize>()
                && let Some(chunk) = self.chunks.iter_mut().find(|c| c.chunk_id == cid)
            {
                chunk.status = FarmChunkStatus::Completed;
                chunk.elapsed_ms = 120;
                write_http_json(stream, "200 OK", &format!("{{\"completed_chunk\":{cid}}}"));
                return;
            }
            write_http_json(stream, "404 Not Found", "{\"error\":\"chunk_not_found\"}");
            return;
        }

        write_http_json(stream, "404 Not Found", "{\"error\":\"not_found\"}");
    }
}

fn write_http_json(stream: &mut TcpStream, status: &str, body: &str) {
    let resp = format!(
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(resp.as_bytes());
    let _ = stream.flush();
}

// =========================================================================
// 自律 AI シネマプロデューサー（台本→絵コンテ・視聴維持率予測・A/B 自動編集）
// =========================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ShotFraming {
    TitleCard,
    EstablishingWide,
    MediumShot,
    CloseUp,
    DynamicInsert,
}

impl ShotFraming {
    pub fn label(self) -> &'static str {
        match self {
            Self::TitleCard => "TITLE CARD",
            Self::EstablishingWide => "WIDE / ESTABLISHING",
            Self::MediumShot => "MEDIUM SHOT",
            Self::CloseUp => "CLOSE-UP",
            Self::DynamicInsert => "DYNAMIC INSERT",
        }
    }

    pub fn mood_color(self) -> RgbaColor {
        match self {
            Self::TitleCard => RgbaColor {
                r: 0.10,
                g: 0.14,
                b: 0.24,
                a: 1.0,
            },
            Self::EstablishingWide => RgbaColor {
                r: 0.12,
                g: 0.22,
                b: 0.34,
                a: 1.0,
            },
            Self::MediumShot => RgbaColor {
                r: 0.18,
                g: 0.24,
                b: 0.30,
                a: 1.0,
            },
            Self::CloseUp => RgbaColor {
                r: 0.28,
                g: 0.16,
                b: 0.22,
                a: 1.0,
            },
            Self::DynamicInsert => RgbaColor {
                r: 0.16,
                g: 0.28,
                b: 0.24,
                a: 1.0,
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StoryboardBeat {
    pub shot_index: usize,
    pub heading: String,
    pub dialogue: String,
    pub camera_note: String,
    pub framing: ShotFraming,
    pub start_sec: f64,
    pub duration_sec: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StoryboardPlan {
    pub title: String,
    pub total_duration_sec: f64,
    pub beats: Vec<StoryboardBeat>,
}

/// 台本テキスト（Markdown 見出し・話者セリフ・`[カメラ指示]` 対応）を構造解析し、絵コンテ計画を生成する
pub fn parse_script_to_storyboard(script_text: &str, _fps: FrameRate) -> StoryboardPlan {
    let mut title = "Autonomous Cinema Storyboard".to_string();
    let mut current_heading = "Opening".to_string();
    let mut beats = Vec::new();
    let mut cursor = 0.0f64;

    for raw in script_text.lines() {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        if let Some(h1) = line.strip_prefix("# ") {
            title = h1.trim().to_string();
            beats.push(StoryboardBeat {
                shot_index: beats.len() + 1,
                heading: title.clone(),
                dialogue: title.clone(),
                camera_note: "Slow Push-In Title Reveal".into(),
                framing: ShotFraming::TitleCard,
                start_sec: cursor,
                duration_sec: 2.2,
            });
            cursor += 2.2;
            continue;
        }
        if let Some(h2) = line.strip_prefix("## ") {
            current_heading = h2.trim().to_string();
            continue;
        }

        // `[...]` または `（...）` 内のカメラ演出指示を抽出
        let (camera_note, dialogue) = if let Some(rest) = line.strip_prefix('[')
            && let Some((note, tail)) = rest.split_once(']')
        {
            (note.trim().to_string(), tail.trim().to_string())
        } else {
            ("Steady Dolly Track".to_string(), line.to_string())
        };
        let dialogue = if dialogue.is_empty() {
            current_heading.clone()
        } else {
            dialogue
        };

        let lower = format!("{camera_note} {dialogue}").to_ascii_lowercase();
        let framing = if lower.contains("wide") || lower.contains("俯瞰") || lower.contains("全景")
        {
            ShotFraming::EstablishingWide
        } else if lower.contains("close") || lower.contains("アップ") || lower.contains("表情")
        {
            ShotFraming::CloseUp
        } else if lower.contains("insert")
            || lower.contains("カットイン")
            || lower.contains("強調")
            || dialogue.contains('!')
            || dialogue.contains('！')
        {
            ShotFraming::DynamicInsert
        } else {
            ShotFraming::MediumShot
        };

        let char_count = dialogue.chars().count();
        let dur = ((char_count as f64) / 7.5).clamp(1.8, 6.5);
        beats.push(StoryboardBeat {
            shot_index: beats.len() + 1,
            heading: current_heading.clone(),
            dialogue,
            camera_note,
            framing,
            start_sec: cursor,
            duration_sec: (dur * 10.0).round() / 10.0,
        });
        cursor += (dur * 10.0).round() / 10.0;
    }

    if beats.is_empty() {
        beats.push(StoryboardBeat {
            shot_index: 1,
            heading: "Scene 1".into(),
            dialogue: "台本を入力すると自動で絵コンテとアニマティックを構築します".into(),
            camera_note: "Establishing Pan".into(),
            framing: ShotFraming::EstablishingWide,
            start_sec: 0.0,
            duration_sec: 3.0,
        });
        cursor = 3.0;
    }

    StoryboardPlan {
        title,
        total_duration_sec: cursor,
        beats,
    }
}

/// 絵コンテ計画をマルチレイヤーのプレイアブル・アニマティック（背景カード＋ショットヘッダー＋字幕＋マーカー）へ変換する 1-Undo コマンド列を生成する
pub fn build_storyboard_animatic_commands(
    plan: &StoryboardPlan,
    scene_width: u32,
    scene_height: u32,
) -> Vec<EditCommand> {
    let mut cmds = Vec::new();
    let w = scene_width as f32;
    let h = scene_height as f32;

    for beat in &plan.beats {
        let start = RationalTime::from_seconds(beat.start_sec);
        let dur = RationalTime::from_seconds(beat.duration_sec.max(0.5));

        // Layer 0: ショットムード背景カード
        let mut bg = TimelineItem::new(
            format!("Shot{:02}_BG_{}", beat.shot_index, beat.framing.label()),
            start,
            dur,
            ItemKind::Shape {
                shape: ShapeKind::Rectangle,
                width: w * 0.92,
                height: h * 0.86,
                color: beat.framing.mood_color(),
            },
        );
        bg.transform.x = w * 0.5;
        bg.transform.y = h * 0.5;
        cmds.push(EditCommand::AddObject {
            layer_index: 0,
            item: bg,
        });

        // Layer 1: ショットフレーミング＆カメラノート注記
        let mut cam_label = TimelineItem::new(
            format!("Shot{:02}_CamNote", beat.shot_index),
            start,
            dur,
            ItemKind::Text {
                content: format!(
                    "SHOT {:02} [{}] — {}",
                    beat.shot_index,
                    beat.framing.label(),
                    beat.camera_note
                ),
                font_family: "sans-serif".into(),
                font_size: (h * 0.032).clamp(20.0, 44.0),
                color: RgbaColor {
                    r: 0.68,
                    g: 0.88,
                    b: 1.0,
                    a: 0.95,
                },
                animator: TextAnimator::default(),
            },
        );
        cam_label.transform.x = w * 0.5;
        cam_label.transform.y = h * 0.14;
        cmds.push(EditCommand::AddObject {
            layer_index: 1,
            item: cam_label,
        });

        // Layer 2: 台詞・ナレーション字幕
        let mut sub = TimelineItem::new(
            format!("Shot{:02}_Sub", beat.shot_index),
            start,
            dur,
            ItemKind::Text {
                content: beat.dialogue.clone(),
                font_family: "sans-serif".into(),
                font_size: (h * 0.048).clamp(26.0, 64.0),
                color: RgbaColor::WHITE,
                animator: TextAnimator::default(),
            },
        );
        sub.transform.x = w * 0.5;
        sub.transform.y = h * 0.80;
        cmds.push(EditCommand::AddObject {
            layer_index: 2,
            item: sub,
        });

        // タイムラインマーカー
        cmds.push(EditCommand::AddMarker {
            marker: TimelineMarker {
                id: Uuid::new_v4(),
                time: start,
                label: format!("S{:02}: {}", beat.shot_index, beat.heading),
                color: RgbaColor {
                    r: 0.25,
                    g: 0.86,
                    b: 0.65,
                    a: 1.0,
                },
            },
        });
    }
    cmds
}

/// 秒ごとの視聴維持率予測ポイント
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct RetentionPoint {
    pub time_sec: f32,
    /// 予測視聴維持率（0.0..=100.0%）
    pub retention_pct: f32,
    /// この秒の視覚／編集エンゲージメント強度（0.0..=1.0）
    pub engagement_score: f32,
}

/// タイムライン視聴維持率診断レポート
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RetentionDiagnostics {
    pub total_duration_sec: f32,
    /// 冒頭 3 秒フック強度（0..=100）
    pub hook_score: f32,
    /// 予測平均視聴維持率（0..=100%）
    pub predicted_avg_retention_pct: f32,
    /// 予測完走率（末尾時点の維持率 %）
    pub predicted_completion_pct: f32,
    /// 平均ショット長 ASL（秒）
    pub average_shot_length_sec: f32,
    /// 離脱リスクが高いタイムスタンプ（秒）
    pub dropoff_risk_timestamps: Vec<f32>,
    /// 改善提案
    pub recommendations: Vec<String>,
    /// 1 秒刻みの維持率カーブ
    pub curve: Vec<RetentionPoint>,
}

/// プロジェクトタイムラインの編集リズム・フック強度・字幕密度・視覚多様性から秒別の視聴維持率を予測診断する
pub fn analyze_viewer_retention(
    project: &Project,
    scene_id: Option<SceneId>,
) -> RetentionDiagnostics {
    let scene = scene_id
        .and_then(|id| project.scenes.iter().find(|s| s.id == id))
        .or_else(|| project.active_scene());
    let Some(scene) = scene else {
        return RetentionDiagnostics {
            total_duration_sec: 0.0,
            hook_score: 0.0,
            predicted_avg_retention_pct: 0.0,
            predicted_completion_pct: 0.0,
            average_shot_length_sec: 0.0,
            dropoff_risk_timestamps: Vec::new(),
            recommendations: vec!["アクティブなシーンが存在しません".into()],
            curve: Vec::new(),
        };
    };

    let mut max_end = 0.0f64;
    let mut cut_times: Vec<f64> = Vec::new();
    let mut all_items: Vec<&TimelineItem> = Vec::new();
    for layer in &scene.layers {
        if !layer.visible {
            continue;
        }
        for item in &layer.items {
            let s = item.start.as_seconds();
            let e = (item.start + item.duration).as_seconds();
            max_end = max_end.max(e);
            cut_times.push(s);
            all_items.push(item);
        }
    }

    let total_sec = max_end.max(1.0) as f32;
    let buckets = (total_sec.ceil() as usize).clamp(1, 300);
    cut_times.sort_by(|a, b| a.total_cmp(b));
    let distinct_cuts = cut_times
        .windows(2)
        .filter(|w| (w[1] - w[0]) > 0.25)
        .count()
        + usize::from(!cut_times.is_empty());
    let asl = total_sec / (distinct_cuts.max(1) as f32);

    let mut curve = Vec::with_capacity(buckets);
    let mut dropoffs = Vec::new();
    let mut current_ret = 100.0f32;
    let mut hook_eng_sum = 0.0f32;
    let mut hook_secs = 0usize;

    for sec in 0..buckets {
        let t0 = sec as f64;
        let t1 = (sec + 1) as f64;
        let mut active_count = 0usize;
        let mut has_text = false;
        let mut has_fx_or_motion = false;
        let mut has_cut_here = false;

        for item in &all_items {
            let s = item.start.as_seconds();
            let e = (item.start + item.duration).as_seconds();
            if s < t1 && e > t0 {
                active_count += 1;
                if matches!(item.kind, ItemKind::Text { .. }) {
                    has_text = true;
                }
                if !item.effect_chain.is_empty()
                    || !item.keyframes.is_empty()
                    || matches!(
                        item.kind,
                        ItemKind::ParticleEmitter { .. }
                            | ItemKind::Mesh3D { .. }
                            | ItemKind::FluidSim { .. }
                            | ItemKind::ClothSim { .. }
                            | ItemKind::VtuberAvatar { .. }
                            | ItemKind::DataChart { .. }
                    )
                {
                    has_fx_or_motion = true;
                }
            }
            if s >= t0 && s < t1 {
                has_cut_here = true;
            }
        }

        let engagement = if active_count == 0 {
            0.05
        } else {
            (0.28
                + (active_count.min(4) as f32) * 0.10
                + if has_text { 0.16 } else { 0.0 }
                + if has_fx_or_motion { 0.18 } else { 0.0 }
                + if has_cut_here { 0.14 } else { 0.0 })
            .clamp(0.05, 1.0)
        };

        if sec < 3 {
            hook_eng_sum += engagement;
            hook_secs += 1;
        }

        // エンゲージメントが低い区間や冒頭は離脱率が大きくなる
        let base_decay = if sec < 3 { 4.2 } else { 1.8 };
        let drop = base_decay * (1.25 - engagement);
        current_ret = (current_ret - drop).clamp(12.0, 100.0);

        if engagement < 0.35 && sec > 0 {
            dropoffs.push(sec as f32);
        }

        curve.push(RetentionPoint {
            time_sec: sec as f32,
            retention_pct: (current_ret * 10.0).round() / 10.0,
            engagement_score: (engagement * 100.0).round() / 100.0,
        });
    }

    let hook_score = if hook_secs > 0 {
        ((hook_eng_sum / hook_secs as f32) * 100.0).clamp(0.0, 100.0)
    } else {
        0.0
    };
    let avg_ret = curve.iter().map(|p| p.retention_pct).sum::<f32>() / curve.len().max(1) as f32;
    let completion = curve.last().map(|p| p.retention_pct).unwrap_or(0.0);

    let mut recs = Vec::new();
    if hook_score < 65.0 {
        recs.push(
            "冒頭 3 秒のフック強度が低めです。インパクト字幕またはトランジション追加を推奨します"
                .into(),
        );
    }
    if asl > 4.5 {
        recs.push(format!(
            "平均ショット長が {asl:.1}s と長めです。2.5〜3.5s 間隔のカットまたは B ロール挿入で維持率が向上します"
        ));
    }
    if !dropoffs.is_empty() {
        recs.push(format!(
            "離脱リスク区間が {} 箇所検出されました（先頭: {:.0}s）。空白詰め・演出追加を検討してください",
            dropoffs.len(),
            dropoffs[0]
        ));
    }
    if recs.is_empty() {
        recs.push("冒頭フック・カットテンポ・視覚密度のバランスが良好です".into());
    }

    RetentionDiagnostics {
        total_duration_sec: total_sec,
        hook_score: (hook_score * 10.0).round() / 10.0,
        predicted_avg_retention_pct: (avg_ret * 10.0).round() / 10.0,
        predicted_completion_pct: completion,
        average_shot_length_sec: (asl * 100.0).round() / 100.0,
        dropoff_risk_timestamps: dropoffs,
        recommendations: recs,
        curve,
    }
}

/// A/B 自動編集バリアント戦略
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EditVariantStyle {
    #[default]
    FastPacedShorts,
    CinematicNarrative,
    RetentionBoostHook,
}

impl EditVariantStyle {
    pub const ALL: &'static [Self] = &[
        Self::FastPacedShorts,
        Self::CinematicNarrative,
        Self::RetentionBoostHook,
    ];

    pub fn display_name(self) -> &'static str {
        match self {
            Self::FastPacedShorts => "⚡ 高速テンポ A/B (Fast-Paced Shorts)",
            Self::CinematicNarrative => "🎞 シネマティック A/B (Cinematic Look)",
            Self::RetentionBoostHook => "🚀 冒頭フック強化 A/B (Retention Boost)",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AbVariantSummary {
    pub new_scene_id: SceneId,
    pub new_scene_name: String,
    pub style: EditVariantStyle,
    pub baseline_retention_pct: f32,
    pub variant_retention_pct: f32,
    pub retention_delta_pct: f32,
}

/// アクティブシーンを複製・戦略別自動再編集した A/B 比較シーン作成コマンドと維持率比較サマリーを返す
pub fn build_ab_edit_variant_command(
    project: &Project,
    style: EditVariantStyle,
    switch_to: bool,
) -> Result<(EditCommand, AbVariantSummary), String> {
    let base_scene = project
        .active_scene()
        .ok_or_else(|| "アクティブなシーンが見つかりません".to_string())?;
    let baseline = analyze_viewer_retention(project, Some(base_scene.id));

    let mut variant: Scene = base_scene.clone();
    variant.id = Uuid::new_v4();
    let suffix = match style {
        EditVariantStyle::FastPacedShorts => "AB_FastPaced",
        EditVariantStyle::CinematicNarrative => "AB_Cinematic",
        EditVariantStyle::RetentionBoostHook => "AB_HookBoost",
    };
    variant.name = format!("{}_{suffix}", base_scene.name);

    match style {
        EditVariantStyle::FastPacedShorts => {
            // クリップ尺を 20% 引き締め、レイヤー内ギャップを詰めて高テンポ化
            for layer in &mut variant.layers {
                layer.items.sort_by_key(|it| it.start);
                let mut cursor = RationalTime::ZERO;
                for it in &mut layer.items {
                    let tight_dur =
                        RationalTime::from_seconds((it.duration.as_seconds() * 0.80).max(0.4));
                    if it.start > cursor {
                        it.start = cursor;
                    }
                    it.duration = tight_dur;
                    cursor = it.start + it.duration;
                }
            }
        }
        EditVariantStyle::CinematicNarrative => {
            // シネマフィニッシング（ハレーション＋2.39:1 シネマスコープ）＋シネマ露出微調整
            variant.cinema_finishing = CinemaFinishingConfig {
                enabled: true,
                halation_strength: 0.32,
                film_grain: 0.14,
                ..CinemaFinishingConfig::default()
            };
            variant.color_grade.enabled = true;
            variant.color_grade.contrast = 1.12;
        }
        EditVariantStyle::RetentionBoostHook => {
            // 冒頭 0..2.2s に強いフックタイトル＋ハイライトマーカーを自動追加
            if let Some(top_layer) = variant.layers.last_mut() {
                let mut hook = TimelineItem::new(
                    "AI_Hook_Banner",
                    RationalTime::ZERO,
                    RationalTime::from_seconds(2.2),
                    ItemKind::Text {
                        content: "🔥 3 秒でわかるハイライト！".into(),
                        font_family: "sans-serif".into(),
                        font_size: (variant.height as f32 * 0.062).clamp(32.0, 84.0),
                        color: RgbaColor {
                            r: 1.0,
                            g: 0.90,
                            b: 0.24,
                            a: 1.0,
                        },
                        animator: TextAnimator::default(),
                    },
                );
                hook.transform.x = variant.width as f32 * 0.5;
                hook.transform.y = variant.height as f32 * 0.22;
                top_layer.items.push(hook);
            }
            variant.markers.push(TimelineMarker {
                id: Uuid::new_v4(),
                time: RationalTime::ZERO,
                label: "AI Retention Hook".into(),
                color: RgbaColor {
                    r: 1.0,
                    g: 0.42,
                    b: 0.18,
                    a: 1.0,
                },
            });
        }
    }

    // 予測維持率の変化を仮プロジェクト上で計算
    let mut temp_proj = project.clone();
    let new_id = variant.id;
    let new_name = variant.name.clone();
    temp_proj.scenes.push(variant.clone());
    let after = analyze_viewer_retention(&temp_proj, Some(new_id));

    let summary = AbVariantSummary {
        new_scene_id: new_id,
        new_scene_name: new_name,
        style,
        baseline_retention_pct: baseline.predicted_avg_retention_pct,
        variant_retention_pct: after.predicted_avg_retention_pct,
        retention_delta_pct: ((after.predicted_avg_retention_pct
            - baseline.predicted_avg_retention_pct)
            * 10.0)
            .round()
            / 10.0,
    };

    Ok((
        EditCommand::CreateScene {
            scene: variant,
            switch_to,
        },
        summary,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::{CommandBus, CommandSource};
    use crate::project::ProjectSettings;

    #[test]
    fn rfc4231_hmac_sha256_vector_and_farm_coordinator_loopback() {
        // RFC 4231 Test Case 2: Key = "Jefe", Data = "what do ya want for nothing?"
        let mac = hmac_sha256_hex("Jefe", b"what do ya want for nothing?");
        assert_eq!(
            mac,
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
        );

        let mut cfg = RenderFarmConfig {
            enabled: true,
            coordinator_port: 0,
            total_frames: 300,
            ..RenderFarmConfig::default()
        };
        cfg.chunks = plan_render_farm_chunks(&cfg);
        assert_eq!(cfg.chunks.len(), 3);
        assert_eq!(cfg.chunks.first().unwrap().start_frame, 0);
        assert_eq!(cfg.chunks.last().unwrap().end_frame, 300);
        assert!(
            cfg.chunks
                .iter()
                .all(|c| verify_chunk_hmac(&cfg.hmac_secret, c))
        );
        assert!(build_farm_concat_manifest(&cfg.chunks).contains("file 'farm_segments/chunk_000"));

        // 非ブロッキングコーディネータの実際の TCP ループバックテスト
        let mut srv = RenderFarmCoordinatorServer::bind_localhost(&cfg).unwrap();
        let port = srv.port;
        let token = cfg.bearer_token.clone();

        let client = std::thread::spawn(move || {
            let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
            let req = format!(
                "POST /api/v1/claim HTTP/1.1\r\nHost: 127.0.0.1\r\nAuthorization: Bearer {token}\r\nContent-Length: 0\r\n\r\n"
            );
            s.write_all(req.as_bytes()).unwrap();
            let mut resp = String::new();
            let _ = s.read_to_string(&mut resp);
            resp
        });
        for _ in 0..20 {
            if srv.poll_once() > 0 {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let resp = client.join().unwrap();
        assert!(resp.contains("200 OK") && resp.contains("chunk_id"));
        assert_eq!(srv.chunks[0].status, FarmChunkStatus::Claimed);
    }

    #[test]
    fn autonomous_cinema_producer_storyboard_retention_and_ab_variants_work() {
        let script = "# AI Cinema Launch\n## Hook\n[Close Up] 驚くほど速いローカル編集！\n## Demo\n[Wide] 360度映像と流体物理を完全オフラインで合成します。\n";
        let plan = parse_script_to_storyboard(script, FrameRate::FPS_30);
        assert_eq!(plan.beats.len(), 3);
        assert_eq!(plan.beats[0].framing, ShotFraming::TitleCard);
        assert_eq!(plan.beats[1].framing, ShotFraming::CloseUp);
        assert_eq!(plan.beats[2].framing, ShotFraming::EstablishingWide);

        let mut proj = Project::new("CinemaOS", ProjectSettings::default());
        let mut bus = CommandBus::new();
        let cmds = build_storyboard_animatic_commands(&plan, 1280, 720);
        bus.execute(
            &mut proj,
            EditCommand::Batch {
                label: "Build Storyboard Animatic".into(),
                commands: cmds,
            },
            CommandSource::Gui,
            None,
        )
        .unwrap();

        let diag = analyze_viewer_retention(&proj, None);
        assert!(diag.total_duration_sec > 5.0);
        assert!(diag.predicted_avg_retention_pct > 50.0);
        assert!(!diag.curve.is_empty());

        let (ab_cmd, summary) =
            build_ab_edit_variant_command(&proj, EditVariantStyle::RetentionBoostHook, false)
                .unwrap();
        bus.execute(&mut proj, ab_cmd, CommandSource::Gui, None)
            .unwrap();
        assert_eq!(proj.scenes.len(), 2);
        assert!(summary.variant_retention_pct >= summary.baseline_retention_pct - 1.0);
    }
}
