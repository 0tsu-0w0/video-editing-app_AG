//! v9.0 #55: OSC / MIDI-over-UDP / Stream Deck WebSocket ハードウェアコントローラー連携
//!
//! Pure Rust・ゼロ `unsafe`・外部追加依存ゼロで以下を実現する:
//! - OSC 1.0 (UDP) バイナリ受信＆フィードバック送信
//! - MIDI-over-UDP（生 3 バイト MIDI Channel Voice & RTP-MIDI ペイロード両対応）パーサ
//! - Pure-Rust FIPS 180-4 SHA-1 ＆ RFC 6455 WebSocket ハンドシェイク／テキストフレームコーデック（Stream Deck 連携）
//! - `CommandBus` 統合ディスパッチャ（カラーホイール・フェーダー・ライブスイッチャー・トランスポート・Hub コマンド）

use crate::command::EditCommand;
use crate::encoding::base64_encode;
use crate::project::{Project, RgbaColor, TimelineMarker};
use crate::time::RationalTime;
use crate::vtuber_rig::{OscArg, decode_osc_packet, encode_osc_message};
use serde::{Deserialize, Serialize};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream, UdpSocket};
use uuid::Uuid;

// ============================================================================
// 1. Pure-Rust FIPS 180-4 SHA-1 & RFC 6455 WebSocket（Stream Deck Bridge 用）
// ============================================================================

/// Pure-Rust ゼロ `unsafe` FIPS 180-4 SHA-1 ダイジェスト（RFC 6455 `Sec-WebSocket-Accept` 算出専用）
pub fn sha1_digest(input: &[u8]) -> [u8; 20] {
    let mut h0: u32 = 0x6745_2301;
    let mut h1: u32 = 0xEFCD_AB89;
    let mut h2: u32 = 0x98BA_DCFE;
    let mut h3: u32 = 0x1032_5476;
    let mut h4: u32 = 0xC3D2_E1F0;

    let bit_len = (input.len() as u64).wrapping_mul(8);
    let mut msg = input.to_vec();
    msg.push(0x80);
    while (msg.len() % 64) != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bit_len.to_be_bytes());

    let mut w = [0u32; 80];
    for block in msg.as_chunks::<64>().0 {
        for (i, chunk) in block.as_chunks::<4>().0.iter().enumerate() {
            w[i] = u32::from_be_bytes(*chunk);
        }
        for i in 16..80 {
            w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
        }

        let (mut a, mut b, mut c, mut d, mut e) = (h0, h1, h2, h3, h4);
        for (i, &wi) in w.iter().enumerate() {
            let (f, k) = match i {
                0..=19 => ((b & c) | ((!b) & d), 0x5A82_7999),
                20..=39 => (b ^ c ^ d, 0x6ED9_EBA1),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8F1B_BCDC),
                _ => (b ^ c ^ d, 0xCA62_C1D6),
            };
            let temp = a
                .rotate_left(5)
                .wrapping_add(f)
                .wrapping_add(e)
                .wrapping_add(k)
                .wrapping_add(wi);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = temp;
        }
        h0 = h0.wrapping_add(a);
        h1 = h1.wrapping_add(b);
        h2 = h2.wrapping_add(c);
        h3 = h3.wrapping_add(d);
        h4 = h4.wrapping_add(e);
    }

    let mut out = [0u8; 20];
    out[0..4].copy_from_slice(&h0.to_be_bytes());
    out[4..8].copy_from_slice(&h1.to_be_bytes());
    out[8..12].copy_from_slice(&h2.to_be_bytes());
    out[12..16].copy_from_slice(&h3.to_be_bytes());
    out[16..20].copy_from_slice(&h4.to_be_bytes());
    out
}

/// RFC 6455 `Sec-WebSocket-Accept` ヘッダ値を計算する
pub fn compute_websocket_accept(sec_websocket_key: &str) -> String {
    const WS_GUID: &str = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11";
    let combined = format!("{}{WS_GUID}", sec_websocket_key.trim());
    base64_encode(&sha1_digest(combined.as_bytes()))
}

/// RFC 6455 サーバー→クライアント非マスクテキストフレームをエンコードする
pub fn encode_ws_text_frame(text: &str) -> Vec<u8> {
    let payload = text.as_bytes();
    let mut out = Vec::with_capacity(payload.len() + 10);
    out.push(0x81); // FIN + Text opcode
    if payload.len() < 126 {
        out.push(payload.len() as u8);
    } else if payload.len() <= u16::MAX as usize {
        out.push(126);
        out.extend_from_slice(&(payload.len() as u16).to_be_bytes());
    } else {
        out.push(127);
        out.extend_from_slice(&(payload.len() as u64).to_be_bytes());
    }
    out.extend_from_slice(payload);
    out
}

/// RFC 6455 クライアント→サーバー（マスク付きまたは非マスク）テキストフレームをデコードする
pub fn decode_ws_text_frame(frame: &[u8]) -> Option<String> {
    if frame.len() < 2 {
        return None;
    }
    let opcode = frame[0] & 0x0F;
    if opcode != 0x01 {
        return None;
    }
    let masked = (frame[1] & 0x80) != 0;
    let mut len = (frame[1] & 0x7F) as usize;
    let mut pos = 2usize;
    if len == 126 {
        if frame.len() < 4 {
            return None;
        }
        len = u16::from_be_bytes([frame[2], frame[3]]) as usize;
        pos = 4;
    } else if len == 127 {
        if frame.len() < 10 {
            return None;
        }
        len = u64::from_be_bytes([
            frame[2], frame[3], frame[4], frame[5], frame[6], frame[7], frame[8], frame[9],
        ]) as usize;
        pos = 10;
    }
    let mask_key = if masked {
        if frame.len() < pos + 4 {
            return None;
        }
        let k = [frame[pos], frame[pos + 1], frame[pos + 2], frame[pos + 3]];
        pos += 4;
        Some(k)
    } else {
        None
    };
    if pos + len > frame.len() {
        return None;
    }
    let mut payload = frame[pos..pos + len].to_vec();
    if let Some(k) = mask_key {
        for (i, b) in payload.iter_mut().enumerate() {
            *b ^= k[i & 3];
        }
    }
    String::from_utf8(payload).ok()
}

// ============================================================================
// 2. MIDI-over-UDP パーサ
// ============================================================================

/// デコード済み MIDI イベント
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MidiMessage {
    NoteOn { channel: u8, note: u8, velocity: u8 },
    NoteOff { channel: u8, note: u8 },
    ControlChange { channel: u8, cc: u8, value: u8 },
    PitchBend { channel: u8, value14: u16 },
}

impl MidiMessage {
    /// マッチング用パターンキー（例: `"cc:1:7"`, `"note:1:60"`）と正規化値 (`0.0..=1.0`)
    pub fn to_pattern_and_value(self) -> (String, f32) {
        match self {
            Self::ControlChange { channel, cc, value } => {
                (format!("cc:{}:{cc}", channel + 1), value as f32 / 127.0)
            }
            Self::NoteOn {
                channel,
                note,
                velocity,
            } => (
                format!("note:{}:{note}", channel + 1),
                velocity as f32 / 127.0,
            ),
            Self::NoteOff { channel, note } => (format!("note:{}:{note}", channel + 1), 0.0),
            Self::PitchBend { channel, value14 } => {
                (format!("pitch:{}", channel + 1), value14 as f32 / 16383.0)
            }
        }
    }
}

/// 生 MIDI-over-UDP または RTP-MIDI (RFC 6295) パケットから MIDI イベント列をパースする
pub fn parse_midi_udp_packet(data: &[u8]) -> Vec<MidiMessage> {
    let slice = if data.len() > 14 && (data[0] & 0xC0) == 0x80 && (data[1] & 0x7F) == 97 {
        // RTP-MIDI 12 バイトヘッダ + 1〜2 バイトコマンド長をスキップ
        let cmd_hdr = data[12];
        let skip = if (cmd_hdr & 0x80) != 0 { 14 } else { 13 };
        &data[skip.min(data.len())..]
    } else {
        data
    };

    let mut out = Vec::new();
    let mut i = 0usize;
    while i < slice.len() {
        let status = slice[i];
        let kind = status & 0xF0;
        let ch = status & 0x0F;
        match kind {
            0x80 if i + 2 < slice.len() => {
                out.push(MidiMessage::NoteOff {
                    channel: ch,
                    note: slice[i + 1] & 0x7F,
                });
                i += 3;
            }
            0x90 if i + 2 < slice.len() => {
                let vel = slice[i + 2] & 0x7F;
                if vel == 0 {
                    out.push(MidiMessage::NoteOff {
                        channel: ch,
                        note: slice[i + 1] & 0x7F,
                    });
                } else {
                    out.push(MidiMessage::NoteOn {
                        channel: ch,
                        note: slice[i + 1] & 0x7F,
                        velocity: vel,
                    });
                }
                i += 3;
            }
            0xB0 if i + 2 < slice.len() => {
                out.push(MidiMessage::ControlChange {
                    channel: ch,
                    cc: slice[i + 1] & 0x7F,
                    value: slice[i + 2] & 0x7F,
                });
                i += 3;
            }
            0xE0 if i + 2 < slice.len() => {
                let lsb = (slice[i + 1] & 0x7F) as u16;
                let msb = (slice[i + 2] & 0x7F) as u16;
                out.push(MidiMessage::PitchBend {
                    channel: ch,
                    value14: (msb << 7) | lsb,
                });
                i += 3;
            }
            _ => i += 1,
        }
    }
    out
}

// ============================================================================
// 3. ハードウェアコントローラー バインディング設定 ＆ CommandBus マッピング
// ============================================================================

/// ハードウェア入力プロトコル種別
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HardwareProtocol {
    #[default]
    OscUdp,
    MidiUdp,
    StreamDeckWs,
}

impl HardwareProtocol {
    pub const ALL: &'static [Self] = &[Self::OscUdp, Self::MidiUdp, Self::StreamDeckWs];

    pub fn display_name(self) -> &'static str {
        match self {
            Self::OscUdp => "📡 OSC (UDP)",
            Self::MidiUdp => "🎹 MIDI-over-UDP",
            Self::StreamDeckWs => "🎛 Stream Deck (WebSocket)",
        }
    }
}

/// ハードウェア操作に割り当てる NLE / カラー / 音声 / ライブ配信アクション
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HardwareTargetAction {
    TransportTogglePlay,
    TransportStepFrame { delta: i64 },
    TriggerUndo,
    TriggerRedo,
    SetColorExposure,
    SetColorSaturation,
    SetColorTemperature,
    SetLayerVolume { layer_index: usize },
    LiveSwitchProgram { channel: usize },
    LiveTriggerStinger,
    AddMarkerAtPlayhead { label: String },
    TriggerHubCommand { command_id: String },
}

/// 1 件のハードウェア入力→アクションバインディング
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HardwareBinding {
    pub id: Uuid,
    pub name: String,
    pub protocol: HardwareProtocol,
    /// 例: `"/ve/color/exposure"`, `"cc:1:7"`, `"deck:btn_cam1"`
    pub match_pattern: String,
    #[serde(default)]
    pub min_out: f32,
    #[serde(default = "default_one")]
    pub max_out: f32,
    pub action: HardwareTargetAction,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_one() -> f32 {
    1.0
}
fn default_true() -> bool {
    true
}

/// 組み込みハードウェアサーフェスプリセット
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HardwareControllerPreset {
    ColoristConsole,
    AudioFaderBank,
    LiveStreamDeck,
    FullBroadcastRig,
}

impl HardwareControllerPreset {
    pub const ALL: &'static [Self] = &[
        Self::ColoristConsole,
        Self::AudioFaderBank,
        Self::LiveStreamDeck,
        Self::FullBroadcastRig,
    ];

    pub fn display_name(self) -> &'static str {
        match self {
            Self::ColoristConsole => "🎨 カラーグレーディングコンソール (OSC/MIDI)",
            Self::AudioFaderBank => "🎚 オーディオフェーダーバンク (MIDI CC)",
            Self::LiveStreamDeck => "🎛 Stream Deck ライブ配信＆カット集",
            Self::FullBroadcastRig => "🚀 フル放送＆制作リグ (全プロトコル統合)",
        }
    }
}

fn default_osc_port() -> u16 {
    9000
}
fn default_midi_port() -> u16 {
    5004
}
fn default_ws_port() -> u16 {
    28196
}

/// プロジェクト単位のハードウェアコントローラー連携設定
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HardwareControllerConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_osc_port")]
    pub osc_udp_port: u16,
    #[serde(default = "default_midi_port")]
    pub midi_udp_port: u16,
    #[serde(default = "default_ws_port")]
    pub stream_deck_ws_port: u16,
    #[serde(default)]
    pub bindings: Vec<HardwareBinding>,
    /// 双方向 OSC フィードバック送信先（例: `"127.0.0.1:9001"`）
    #[serde(default)]
    pub osc_feedback_addr: Option<String>,
}

impl Default for HardwareControllerConfig {
    fn default() -> Self {
        Self::from_preset(HardwareControllerPreset::FullBroadcastRig)
    }
}

impl HardwareControllerConfig {
    pub fn from_preset(preset: HardwareControllerPreset) -> Self {
        let mut bindings = Vec::new();
        let add_color = |b: &mut Vec<HardwareBinding>| {
            b.push(HardwareBinding {
                id: Uuid::new_v4(),
                name: "Exposure Wheel".into(),
                protocol: HardwareProtocol::OscUdp,
                match_pattern: "/ve/color/exposure".into(),
                min_out: -2.0,
                max_out: 2.0,
                action: HardwareTargetAction::SetColorExposure,
                enabled: true,
            });
            b.push(HardwareBinding {
                id: Uuid::new_v4(),
                name: "Saturation Wheel".into(),
                protocol: HardwareProtocol::OscUdp,
                match_pattern: "/ve/color/saturation".into(),
                min_out: 0.0,
                max_out: 2.0,
                action: HardwareTargetAction::SetColorSaturation,
                enabled: true,
            });
            b.push(HardwareBinding {
                id: Uuid::new_v4(),
                name: "Temperature Dial (MIDI CC1)".into(),
                protocol: HardwareProtocol::MidiUdp,
                match_pattern: "cc:1:1".into(),
                min_out: -1.0,
                max_out: 1.0,
                action: HardwareTargetAction::SetColorTemperature,
                enabled: true,
            });
        };
        let add_faders = |b: &mut Vec<HardwareBinding>| {
            for ch in 0..4 {
                b.push(HardwareBinding {
                    id: Uuid::new_v4(),
                    name: format!("Layer {} Volume Fader (CC{})", ch + 1, 7 + ch),
                    protocol: HardwareProtocol::MidiUdp,
                    match_pattern: format!("cc:1:{}", 7 + ch),
                    min_out: 0.0,
                    max_out: 2.0,
                    action: HardwareTargetAction::SetLayerVolume { layer_index: ch },
                    enabled: true,
                });
            }
        };
        let add_deck = |b: &mut Vec<HardwareBinding>| {
            for ch in 0..4 {
                b.push(HardwareBinding {
                    id: Uuid::new_v4(),
                    name: format!("Stream Deck CAM {}", ch + 1),
                    protocol: HardwareProtocol::StreamDeckWs,
                    match_pattern: format!("deck:cam_{}", ch + 1),
                    min_out: 0.0,
                    max_out: 1.0,
                    action: HardwareTargetAction::LiveSwitchProgram { channel: ch },
                    enabled: true,
                });
            }
            b.push(HardwareBinding {
                id: Uuid::new_v4(),
                name: "Stream Deck AUTO STINGER".into(),
                protocol: HardwareProtocol::StreamDeckWs,
                match_pattern: "deck:stinger".into(),
                min_out: 0.0,
                max_out: 1.0,
                action: HardwareTargetAction::LiveTriggerStinger,
                enabled: true,
            });
            b.push(HardwareBinding {
                id: Uuid::new_v4(),
                name: "Stream Deck Play/Pause".into(),
                protocol: HardwareProtocol::StreamDeckWs,
                match_pattern: "deck:play".into(),
                min_out: 0.0,
                max_out: 1.0,
                action: HardwareTargetAction::TransportTogglePlay,
                enabled: true,
            });
            b.push(HardwareBinding {
                id: Uuid::new_v4(),
                name: "Stream Deck Drop Highlight Marker".into(),
                protocol: HardwareProtocol::StreamDeckWs,
                match_pattern: "deck:marker".into(),
                min_out: 0.0,
                max_out: 1.0,
                action: HardwareTargetAction::AddMarkerAtPlayhead {
                    label: "⭐ Hardware Marker".into(),
                },
                enabled: true,
            });
        };

        match preset {
            HardwareControllerPreset::ColoristConsole => add_color(&mut bindings),
            HardwareControllerPreset::AudioFaderBank => add_faders(&mut bindings),
            HardwareControllerPreset::LiveStreamDeck => add_deck(&mut bindings),
            HardwareControllerPreset::FullBroadcastRig => {
                add_color(&mut bindings);
                add_faders(&mut bindings);
                add_deck(&mut bindings);
            }
        }

        Self {
            enabled: false,
            osc_udp_port: default_osc_port(),
            midi_udp_port: default_midi_port(),
            stream_deck_ws_port: default_ws_port(),
            bindings,
            osc_feedback_addr: None,
        }
    }
}

/// プロトコル非依存の正規化ハードウェア入力イベント
#[derive(Debug, Clone, PartialEq)]
pub struct HardwareEvent {
    pub protocol: HardwareProtocol,
    pub pattern: String,
    pub value_01: f32,
}

/// ハードウェアイベントを評価したディスパッチ結果
#[derive(Debug, Clone)]
pub enum HardwareDispatchOutcome {
    /// `CommandBus` 経由で非破壊・1-Undo 実行する編集コマンド
    EditCommand(Box<EditCommand>),
    /// 再生／一時停止トグル
    TogglePlay,
    /// コマ送り／戻し
    StepFrame(i64),
    /// Undo / Redo
    Undo,
    Redo,
    /// Studio Hub / コマンドパレットコマンド起動
    HubCommand(String),
}

/// 正規化ハードウェアイベントを設定済みバインディングに照合し、対応する `EditCommand` 等のアクションへ変換する
pub fn map_hardware_event_to_outcome(
    cfg: &HardwareControllerConfig,
    event: &HardwareEvent,
    project: &Project,
    playhead: RationalTime,
) -> Option<HardwareDispatchOutcome> {
    let binding = cfg
        .bindings
        .iter()
        .find(|b| b.enabled && b.protocol == event.protocol && b.match_pattern == event.pattern)?;

    let v01 = event.value_01.clamp(0.0, 1.0);
    let mapped = binding.min_out + (binding.max_out - binding.min_out) * v01;

    match &binding.action {
        HardwareTargetAction::TransportTogglePlay => Some(HardwareDispatchOutcome::TogglePlay),
        HardwareTargetAction::TransportStepFrame { delta } => {
            Some(HardwareDispatchOutcome::StepFrame(*delta))
        }
        HardwareTargetAction::TriggerUndo => Some(HardwareDispatchOutcome::Undo),
        HardwareTargetAction::TriggerRedo => Some(HardwareDispatchOutcome::Redo),
        HardwareTargetAction::TriggerHubCommand { command_id } => {
            Some(HardwareDispatchOutcome::HubCommand(command_id.clone()))
        }
        HardwareTargetAction::SetColorExposure => {
            let mut cg = project.active_scene()?.color_grade.clone();
            cg.enabled = true;
            cg.exposure = mapped.clamp(-3.0, 3.0);
            Some(HardwareDispatchOutcome::EditCommand(Box::new(
                EditCommand::SetSceneColorGrade { grade: cg },
            )))
        }
        HardwareTargetAction::SetColorSaturation => {
            let mut cg = project.active_scene()?.color_grade.clone();
            cg.enabled = true;
            cg.saturation = mapped.clamp(0.0, 3.0);
            Some(HardwareDispatchOutcome::EditCommand(Box::new(
                EditCommand::SetSceneColorGrade { grade: cg },
            )))
        }
        HardwareTargetAction::SetColorTemperature => {
            let mut cg = project.active_scene()?.color_grade.clone();
            cg.enabled = true;
            cg.temperature = mapped.clamp(-1.0, 1.0);
            Some(HardwareDispatchOutcome::EditCommand(Box::new(
                EditCommand::SetSceneColorGrade { grade: cg },
            )))
        }
        HardwareTargetAction::SetLayerVolume { layer_index } => {
            let layer = project.active_scene()?.layers.get(*layer_index)?;
            Some(HardwareDispatchOutcome::EditCommand(Box::new(
                EditCommand::SetLayerSettings {
                    layer_index: *layer_index,
                    name: layer.name.clone(),
                    visible: layer.visible,
                    muted: layer.muted,
                    offset_x: layer.offset_x,
                    offset_y: layer.offset_y,
                    volume: mapped.clamp(0.0, 2.0),
                    color_tag: layer.color_tag,
                },
            )))
        }
        HardwareTargetAction::LiveSwitchProgram { channel } => {
            let mut sw = project.active_scene()?.live_switcher.clone();
            sw.enabled = true;
            sw.cut_to_channel(*channel, playhead.as_seconds());
            Some(HardwareDispatchOutcome::EditCommand(Box::new(
                EditCommand::SetLiveSwitcher { live_switcher: sw },
            )))
        }
        HardwareTargetAction::LiveTriggerStinger => {
            let mut sw = project.active_scene()?.live_switcher.clone();
            sw.enabled = true;
            sw.trigger_take(playhead.as_seconds(), true, "Hardware Stinger");
            Some(HardwareDispatchOutcome::EditCommand(Box::new(
                EditCommand::SetLiveSwitcher { live_switcher: sw },
            )))
        }
        HardwareTargetAction::AddMarkerAtPlayhead { label } => Some(
            HardwareDispatchOutcome::EditCommand(Box::new(EditCommand::AddMarker {
                marker: TimelineMarker {
                    id: Uuid::new_v4(),
                    time: playhead,
                    label: label.clone(),
                    color: RgbaColor {
                        r: 1.0,
                        g: 0.78,
                        b: 0.22,
                        a: 1.0,
                    },
                },
            })),
        ),
    }
}

/// ノンブロッキング ローカル 3 プロトコル統合ハードウェアブリッジ（OSC UDP + MIDI UDP + Stream Deck RFC 6455 WebSocket）
pub struct HardwareBridgeServer {
    osc_sock: UdpSocket,
    midi_sock: UdpSocket,
    ws_listener: TcpListener,
    ws_clients: Vec<TcpStream>,
    pub osc_port: u16,
    pub midi_port: u16,
    pub ws_port: u16,
    pub events_received: u64,
}

impl HardwareBridgeServer {
    /// `0` ポート指定で OS 割り当て空きポートにバインド可能（テスト・競合回避向け）
    pub fn bind_localhost(osc_port: u16, midi_port: u16, ws_port: u16) -> Result<Self, String> {
        let osc_sock = UdpSocket::bind(("127.0.0.1", osc_port)).map_err(|e| e.to_string())?;
        osc_sock.set_nonblocking(true).map_err(|e| e.to_string())?;
        let osc_p = osc_sock.local_addr().map_err(|e| e.to_string())?.port();

        let midi_sock = UdpSocket::bind(("127.0.0.1", midi_port)).map_err(|e| e.to_string())?;
        midi_sock.set_nonblocking(true).map_err(|e| e.to_string())?;
        let midi_p = midi_sock.local_addr().map_err(|e| e.to_string())?.port();

        let ws_listener = TcpListener::bind(("127.0.0.1", ws_port)).map_err(|e| e.to_string())?;
        ws_listener
            .set_nonblocking(true)
            .map_err(|e| e.to_string())?;
        let ws_p = ws_listener.local_addr().map_err(|e| e.to_string())?.port();

        Ok(Self {
            osc_sock,
            midi_sock,
            ws_listener,
            ws_clients: Vec::new(),
            osc_port: osc_p,
            midi_port: midi_p,
            ws_port: ws_p,
            events_received: 0,
        })
    }

    /// 3 プロトコル全ての着信を非ブロッキングでドレインして `HardwareEvent` 列を返す
    pub fn poll_events(&mut self) -> Vec<HardwareEvent> {
        let mut events = Vec::new();
        let mut buf = [0u8; 4096];

        // 1. OSC over UDP
        for _ in 0..32 {
            match self.osc_sock.recv_from(&mut buf) {
                Ok((n, _)) => {
                    for msg in decode_osc_packet(&buf[..n]) {
                        let val = msg
                            .args
                            .first()
                            .and_then(OscArg::as_f32)
                            .unwrap_or(1.0)
                            .clamp(0.0, 1.0);
                        events.push(HardwareEvent {
                            protocol: HardwareProtocol::OscUdp,
                            pattern: msg.address,
                            value_01: val,
                        });
                    }
                }
                Err(_) => break,
            }
        }

        // 2. MIDI over UDP
        for _ in 0..32 {
            match self.midi_sock.recv_from(&mut buf) {
                Ok((n, _)) => {
                    for m in parse_midi_udp_packet(&buf[..n]) {
                        let (pat, val) = m.to_pattern_and_value();
                        events.push(HardwareEvent {
                            protocol: HardwareProtocol::MidiUdp,
                            pattern: pat,
                            value_01: val,
                        });
                    }
                }
                Err(_) => break,
            }
        }

        // 3. Stream Deck WebSocket (Accept + Handshake + Frame Read)
        for _ in 0..8 {
            match self.ws_listener.accept() {
                Ok((mut stream, _)) => {
                    let _ = stream.set_read_timeout(Some(std::time::Duration::from_millis(25)));
                    let mut req_buf = [0u8; 2048];
                    if let Ok(n) = stream.read(&mut req_buf)
                        && let Ok(req_str) = std::str::from_utf8(&req_buf[..n])
                        && let Some(key_line) = req_str
                            .lines()
                            .find(|l| l.to_ascii_lowercase().starts_with("sec-websocket-key:"))
                        && let Some(key) = key_line.split_once(':').map(|(_, v)| v.trim())
                    {
                        let accept = compute_websocket_accept(key);
                        let resp = format!(
                            "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {accept}\r\n\r\n"
                        );
                        if stream.write_all(resp.as_bytes()).is_ok() {
                            let _ = stream.set_nonblocking(true);
                            self.ws_clients.push(stream);
                        }
                    }
                }
                Err(_) => break,
            }
        }

        let mut alive = Vec::with_capacity(self.ws_clients.len());
        for mut client in self.ws_clients.drain(..) {
            let mut keep = true;
            match client.read(&mut buf) {
                Ok(0) => keep = false,
                Ok(n) => {
                    if let Some(txt) = decode_ws_text_frame(&buf[..n]) {
                        // JSON `{"action":"deck:cam_2","value":1.0}` またはプレーン `"deck:cam_2"` の両対応
                        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&txt)
                            && let Some(act) = v.get("action").and_then(|x| x.as_str())
                        {
                            let val = v.get("value").and_then(|x| x.as_f64()).unwrap_or(1.0) as f32;
                            events.push(HardwareEvent {
                                protocol: HardwareProtocol::StreamDeckWs,
                                pattern: act.to_string(),
                                value_01: val.clamp(0.0, 1.0),
                            });
                            let ack = encode_ws_text_frame(r#"{"status":"ok"}"#);
                            let _ = client.write_all(&ack);
                        } else if !txt.trim().is_empty() {
                            events.push(HardwareEvent {
                                protocol: HardwareProtocol::StreamDeckWs,
                                pattern: txt.trim().to_string(),
                                value_01: 1.0,
                            });
                        }
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(_) => keep = false,
            }
            if keep {
                alive.push(client);
            }
        }
        self.ws_clients = alive;

        self.events_received += events.len() as u64;
        events
    }

    /// 双方向 OSC フィードバックを送信する（カラー値やタリーのモーターフェーダー同期向け）
    pub fn send_osc_feedback(
        &self,
        target_addr: &str,
        address: &str,
        value: f32,
    ) -> Result<(), String> {
        let pkt = encode_osc_message(address, &[OscArg::Float(value)]);
        self.osc_sock
            .send_to(&pkt, target_addr)
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::{CommandBus, CommandSource};
    use crate::project::ProjectSettings;

    #[test]
    fn rfc6455_sha1_websocket_accept_matches_official_spec_vector() {
        // RFC 6455 Section 1.3 official test vector
        let accept = compute_websocket_accept("dGhlIHNhbXBsZSBub25jZQ==");
        assert_eq!(accept, "s3pPLMBiTxaQ9kYGzzhZRbK+xOo=");
    }

    #[test]
    fn hardware_bridge_receives_osc_midi_and_stream_deck_ws_and_dispatches_commands() {
        let mut srv = HardwareBridgeServer::bind_localhost(0, 0, 0).unwrap();
        let udp = UdpSocket::bind(("127.0.0.1", 0)).unwrap();

        // 1. Send OSC exposure 0.75 (-> mapped [-2..2] = +1.0)
        let osc_pkt = encode_osc_message("/ve/color/exposure", &[OscArg::Float(0.75)]);
        udp.send_to(&osc_pkt, ("127.0.0.1", srv.osc_port)).unwrap();

        // 2. Send MIDI CC7 (Layer 1 volume) = 64 (~0.5 -> volume ~1.0)
        udp.send_to(&[0xB0, 7, 64], ("127.0.0.1", srv.midi_port))
            .unwrap();

        // 3. Connect RFC 6455 WebSocket client & send masked JSON Stream Deck press
        let mut ws = TcpStream::connect(("127.0.0.1", srv.ws_port)).unwrap();
        let req = "GET / HTTP/1.1\r\nHost: 127.0.0.1\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\n\r\n";
        ws.write_all(req.as_bytes()).unwrap();
        // Poll once to complete server handshake
        std::thread::sleep(std::time::Duration::from_millis(15));
        let mut collected = srv.poll_events();

        let mut resp_buf = [0u8; 512];
        let rn = ws.read(&mut resp_buf).unwrap();
        let resp_str = std::str::from_utf8(&resp_buf[..rn]).unwrap();
        assert!(resp_str.contains("101 Switching Protocols"));
        assert!(resp_str.contains("s3pPLMBiTxaQ9kYGzzhZRbK+xOo="));

        // Send masked client text frame: {"action":"deck:cam_3","value":1.0}
        let body = br#"{"action":"deck:cam_3","value":1.0}"#;
        let mask = [0x37u8, 0xFA, 0x21, 0x3D];
        let mut frame = vec![0x81, 0x80 | (body.len() as u8)];
        frame.extend_from_slice(&mask);
        for (i, &b) in body.iter().enumerate() {
            frame.push(b ^ mask[i & 3]);
        }
        ws.write_all(&frame).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(15));
        collected.extend(srv.poll_events());

        assert!(
            collected.len() >= 3,
            "expected >=3 hardware events, got {:?}",
            collected
        );

        let cfg = HardwareControllerConfig::from_preset(HardwareControllerPreset::FullBroadcastRig);
        let mut proj = Project::new("HwTest", ProjectSettings::default());
        let mut bus = CommandBus::new();
        for ev in &collected {
            if let Some(HardwareDispatchOutcome::EditCommand(cmd)) =
                map_hardware_event_to_outcome(&cfg, ev, &proj, RationalTime::from_seconds(1.0))
            {
                bus.execute(&mut proj, *cmd, CommandSource::Gui, None)
                    .unwrap();
            }
        }
        let sc = proj.active_scene().unwrap();
        assert!((sc.color_grade.exposure - 1.0).abs() < 1e-3);
        assert_eq!(sc.live_switcher.program_channel, 2);
    }
}
