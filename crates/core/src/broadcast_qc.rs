//! v4.0 #36: 放送・配信コンプライアンス QC 監査（ITU-R BS.1770 / BT.1702 光過敏フラッシュ検査）＆
//! ゼロサーバー完結 HTML5 インタラクティブレビュープレイヤー書き出し

use crate::collab::format_smpte_timecode;
use crate::project::Project;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BroadcastProfile {
    #[default]
    YouTube1080p,
    BroadcastItuEbuR128,
    ShortsTikTok9x16,
    CinemaDci4k,
}

impl BroadcastProfile {
    pub const ALL: &'static [Self] = &[
        Self::YouTube1080p,
        Self::BroadcastItuEbuR128,
        Self::ShortsTikTok9x16,
        Self::CinemaDci4k,
    ];

    pub fn display_name(self) -> &'static str {
        match self {
            Self::YouTube1080p => "📺 YouTube / 配信標準 (-14 LUFS / -1 dBTP)",
            Self::BroadcastItuEbuR128 => "📡 放送準拠 EBU R128 (-23 LUFS / -1 dBTP)",
            Self::ShortsTikTok9x16 => "📱 Shorts / TikTok 縦動画 (9:16)",
            Self::CinemaDci4k => "🎬 シネマ 4K マスター (UHD / ACEScg)",
        }
    }

    pub fn target_lufs(self) -> f32 {
        match self {
            Self::BroadcastItuEbuR128 => -23.0,
            Self::CinemaDci4k => -24.0,
            Self::YouTube1080p | Self::ShortsTikTok9x16 => -14.0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QcStatus {
    Pass,
    Warning,
    Fail,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QcCheckItem {
    pub code: String,
    pub category: String,
    pub status: QcStatus,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BroadcastQcMetrics {
    pub peak_luma_ire: f32,
    pub clipped_highlight_pct: f32,
    pub crushed_shadow_pct: f32,
    pub max_frame_luma_jump_ire: f32,
    pub true_peak_dbtp: f32,
    pub integrated_lufs: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BroadcastQcReport {
    pub profile: BroadcastProfile,
    pub overall_pass: bool,
    pub pass_count: usize,
    pub warning_count: usize,
    pub fail_count: usize,
    pub metrics: BroadcastQcMetrics,
    pub checks: Vec<QcCheckItem>,
}

/// 計測済み映像・音声メトリクスとプロジェクト構造から放送・配信コンプライアンス QC レポートを作成する
pub fn run_broadcast_qc_audit(
    project: &Project,
    profile: BroadcastProfile,
    metrics: BroadcastQcMetrics,
) -> BroadcastQcReport {
    let mut checks = Vec::new();
    let Some(scene) = project.active_scene() else {
        return BroadcastQcReport {
            profile,
            overall_pass: false,
            pass_count: 0,
            warning_count: 0,
            fail_count: 1,
            metrics,
            checks: vec![QcCheckItem {
                code: "QC_NO_SCENE".into(),
                category: "Structure".into(),
                status: QcStatus::Fail,
                detail: "アクティブシーンが存在しません".into(),
            }],
        };
    };

    // 1. 解像度・アスペクト比チェック
    let res_ok = match profile {
        BroadcastProfile::ShortsTikTok9x16 => scene.height > scene.width,
        BroadcastProfile::CinemaDci4k => scene.width >= 3840 && scene.height >= 2160,
        _ => scene.width >= 1280 && scene.height >= 720,
    };
    checks.push(QcCheckItem {
        code: "QC_RESOLUTION".into(),
        category: "Video".into(),
        status: if res_ok {
            QcStatus::Pass
        } else {
            QcStatus::Warning
        },
        detail: format!(
            "シーン解像度: {}×{} @ {:.2}fps",
            scene.width,
            scene.height,
            scene.fps.as_f64()
        ),
    });

    // 2. 輝度クリッピングチェック（白飛び / 黒潰れ）
    let clip_status = if metrics.clipped_highlight_pct > 5.0 {
        QcStatus::Warning
    } else {
        QcStatus::Pass
    };
    checks.push(QcCheckItem {
        code: "QC_LUMA_GAMUT".into(),
        category: "Color".into(),
        status: clip_status,
        detail: format!(
            "Peak {:.1} IRE / 白飛び {:.2}% / 黒潰れ {:.2}%",
            metrics.peak_luma_ire, metrics.clipped_highlight_pct, metrics.crushed_shadow_pct
        ),
    });

    // 3. 光過敏フラッシュ検査（ITU-R BT.1702: 急激な全面輝度ジャンプ > 35 IRE 警告）
    let flash_status = if metrics.max_frame_luma_jump_ire > 55.0 {
        QcStatus::Fail
    } else if metrics.max_frame_luma_jump_ire > 35.0 {
        QcStatus::Warning
    } else {
        QcStatus::Pass
    };
    checks.push(QcCheckItem {
        code: "QC_PHOTOSENSITIVE_FLASH".into(),
        category: "Safety".into(),
        status: flash_status,
        detail: format!(
            "最大フレーム間輝度変動: {:.1} IRE（基準 <= 35 IRE）",
            metrics.max_frame_luma_jump_ire
        ),
    });

    // 4. ITU-R BS.1770 True-Peak 上限（<= -1.0 dBTP）
    let tp_status = if metrics.true_peak_dbtp > -0.95 {
        QcStatus::Fail
    } else {
        QcStatus::Pass
    };
    checks.push(QcCheckItem {
        code: "QC_TRUE_PEAK".into(),
        category: "Audio".into(),
        status: tp_status,
        detail: format!(
            "True-Peak: {:.2} dBTP（規定上限 <= -1.0 dBTP）",
            metrics.true_peak_dbtp
        ),
    });

    // 5. ラウドネス目標チェック（無音 -70 未満は警告扱い）
    let target_lufs = profile.target_lufs();
    let lufs_status = if metrics.integrated_lufs < -65.0 {
        QcStatus::Warning
    } else if (metrics.integrated_lufs - target_lufs).abs() <= 2.5 {
        QcStatus::Pass
    } else {
        QcStatus::Warning
    };
    checks.push(QcCheckItem {
        code: "QC_LOUDNESS".into(),
        category: "Audio".into(),
        status: lufs_status,
        detail: format!(
            "Integrated: {:.1} LUFS（ターゲット {:.1} LUFS ±2.5）",
            metrics.integrated_lufs, target_lufs
        ),
    });

    // 6. カラー管理パイプライン状態
    checks.push(QcCheckItem {
        code: "QC_COLOR_PIPELINE".into(),
        category: "Color".into(),
        status: QcStatus::Pass,
        detail: if scene.color_management.enabled {
            format!(
                "ACES/OCIO 有効: {} → {} ({})",
                scene.color_management.working_space.display_name(),
                scene.color_management.output_odt.display_name(),
                scene.color_management.tone_mapper.display_name()
            )
        } else {
            "標準 Linear sRGB / Rec.709 パイプライン".into()
        },
    });

    let pass_count = checks.iter().filter(|c| c.status == QcStatus::Pass).count();
    let warning_count = checks
        .iter()
        .filter(|c| c.status == QcStatus::Warning)
        .count();
    let fail_count = checks.iter().filter(|c| c.status == QcStatus::Fail).count();

    BroadcastQcReport {
        profile,
        overall_pass: fail_count == 0,
        pass_count,
        warning_count,
        fail_count,
        metrics,
        checks,
    }
}

/// ゼロ依存・サーバー不要の自己完結型 HTML5 インタラクティブレビュープレイヤーを生成する
/// （フレーム PNG サムネイル群 Base64 埋め込み・SMPTE スクラバー・チャプターマーカー・レビューコメント・QC バッジ付き）
pub fn export_portable_web_review_html(
    project: &Project,
    qc: &BroadcastQcReport,
    frame_previews_b64: &[(f64, String)],
) -> String {
    let scene_name = project
        .active_scene()
        .map(|s| s.name.as_str())
        .unwrap_or("Untitled");
    let fps = project
        .active_scene()
        .map(|s| s.fps)
        .unwrap_or(project.settings.fps);
    let markers_html: String = project
        .active_scene()
        .map(|s| {
            s.markers
                .iter()
                .map(|m| {
                    format!(
                        "<button class=\"chip\" onclick=\"seekTo({:.3})\">{} {}</button>",
                        m.time.as_seconds(),
                        escape_html(&format_smpte_timecode(m.time, fps)),
                        escape_html(&m.label)
                    )
                })
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default();

    let comments_html: String = project
        .active_scene()
        .map(|s| {
            s.review_comments
                .iter()
                .map(|c| {
                    format!(
                        "<li><strong>{}</strong> [{}]: {}</li>",
                        escape_html(&c.author),
                        escape_html(&format_smpte_timecode(c.time, fps)),
                        escape_html(&c.body)
                    )
                })
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default();

    let checks_html: String = qc
        .checks
        .iter()
        .map(|c| {
            let badge = match c.status {
                QcStatus::Pass => "PASS",
                QcStatus::Warning => "WARN",
                QcStatus::Fail => "FAIL",
            };
            format!(
                "<tr><td>{}</td><td class=\"st-{}\">{}</td><td>{}</td></tr>",
                escape_html(&c.code),
                badge.to_lowercase(),
                badge,
                escape_html(&c.detail)
            )
        })
        .collect::<Vec<_>>()
        .join("\n");

    let frames_json: String = serde_json::to_string(
        &frame_previews_b64
            .iter()
            .map(|(t, b64)| serde_json::json!({ "t": t, "src": format!("data:image/png;base64,{b64}") }))
            .collect::<Vec<_>>(),
    )
    .unwrap_or_else(|_| "[]".into());

    format!(
        r#"<!DOCTYPE html>
<html lang="ja">
<head>
<meta charset="utf-8" />
<meta name="viewport" content="width=device-width,initial-scale=1" />
<title>{title} — Portable Review Player (v4.0)</title>
<style>
  body {{ font-family: system-ui, sans-serif; background:#0e1118; color:#e8edf6; margin:0; padding:24px; }}
  .wrap {{ max-width:980px; margin:0 auto; }}
  .stage {{ background:#06080d; border:1px solid #293245; border-radius:10px; overflow:hidden; text-align:center; }}
  .stage img {{ max-width:100%; max-height:480px; display:block; margin:0 auto; }}
  .bar {{ display:flex; gap:12px; align-items:center; margin:14px 0; }}
  input[type=range] {{ flex:1; }}
  .chip {{ background:#1f283b; color:#cfe3ff; border:1px solid #354769; border-radius:999px; padding:4px 10px; cursor:pointer; margin:2px; }}
  table {{ width:100%; border-collapse:collapse; margin-top:12px; font-size:13px; }}
  th,td {{ border-bottom:1px solid #242c3e; padding:6px 8px; text-align:left; }}
  .st-pass {{ color:#4ade80; font-weight:700; }}
  .st-warn {{ color:#facc15; font-weight:700; }}
  .st-fail {{ color:#f87171; font-weight:700; }}
</style>
</head>
<body>
<div class="wrap">
  <h1>🎬 {title} <small style="opacity:.7">({scene} · v4.0 Portable Review)</small></h1>
  <div class="stage"><img id="frameImg" alt="Preview Frame" /></div>
  <div class="bar">
    <button class="chip" onclick="togglePlay()">▶ 再生/一時停止</button>
    <input id="scrubber" type="range" min="0" max="0" step="1" value="0" oninput="showIdx(+this.value)" />
    <span id="tc">0.00s</span>
  </div>
  <div><strong>チャプター / マーカー:</strong> {markers}</div>
  <h2>📡 Broadcast QC Audit ({qc_badge})</h2>
  <table><thead><tr><th>Code</th><th>Status</th><th>Detail</th></tr></thead><tbody>{checks}</tbody></table>
  <h2>💬 Review Comments</h2>
  <ul>{comments}</ul>
</div>
<script>
const FRAMES = {frames_json};
let idx = 0, playing = false, timer = null;
const img = document.getElementById('frameImg');
const sc = document.getElementById('scrubber');
const tc = document.getElementById('tc');
sc.max = Math.max(0, FRAMES.length - 1);
function showIdx(i) {{
  if (!FRAMES.length) return;
  idx = Math.max(0, Math.min(FRAMES.length - 1, i));
  sc.value = idx;
  img.src = FRAMES[idx].src;
  tc.textContent = FRAMES[idx].t.toFixed(2) + 's';
}}
function seekTo(sec) {{
  let best = 0, bestD = 1e9;
  FRAMES.forEach((f, i) => {{ const d = Math.abs(f.t - sec); if (d < bestD) {{ bestD = d; best = i; }} }});
  showIdx(best);
}}
function togglePlay() {{
  playing = !playing;
  if (playing) {{ timer = setInterval(() => showIdx((idx + 1) % Math.max(1, FRAMES.length)), 450); }}
  else {{ clearInterval(timer); }}
}}
showIdx(0);
</script>
</body>
</html>
"#,
        title = escape_html(&project.name),
        scene = escape_html(scene_name),
        markers = markers_html,
        qc_badge = if qc.overall_pass {
            "PASS"
        } else {
            "ACTION REQUIRED"
        },
        checks = checks_html,
        comments = comments_html,
        frames_json = frames_json,
    )
}

fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::ProjectSettings;

    #[test]
    fn broadcast_qc_flags_true_peak_and_flash_violations_and_exports_html_player() {
        let proj = Project::new("BroadcastDemo", ProjectSettings::default());
        let good = BroadcastQcMetrics {
            peak_luma_ire: 92.0,
            clipped_highlight_pct: 0.4,
            crushed_shadow_pct: 1.2,
            max_frame_luma_jump_ire: 14.0,
            true_peak_dbtp: -1.4,
            integrated_lufs: -14.2,
        };
        let rep_ok = run_broadcast_qc_audit(&proj, BroadcastProfile::YouTube1080p, good);
        assert!(rep_ok.overall_pass);
        assert_eq!(rep_ok.fail_count, 0);

        let bad = BroadcastQcMetrics {
            peak_luma_ire: 100.0,
            clipped_highlight_pct: 12.0,
            crushed_shadow_pct: 0.0,
            max_frame_luma_jump_ire: 72.0,
            true_peak_dbtp: 0.8,
            integrated_lufs: -8.0,
        };
        let rep_bad = run_broadcast_qc_audit(&proj, BroadcastProfile::BroadcastItuEbuR128, bad);
        assert!(!rep_bad.overall_pass);
        assert_eq!(rep_bad.fail_count, 2);

        let html = export_portable_web_review_html(&proj, &rep_ok, &[(0.0, "AAAA".into())]);
        assert!(html.contains("<!DOCTYPE html>"));
        assert!(html.contains("BroadcastDemo"));
        assert!(html.contains("data:image/png;base64,AAAA"));
    }
}
