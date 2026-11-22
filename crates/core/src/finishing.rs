//! v5.0 #40: プロ・シネマフィニッシング（ハレーション・アナモルフィックフレア・フィルムグレイン・レターボックス・バーンイン）
//! ＆ 業界標準プロ交換フォーマット書き出し（CMX 3600 EDL / Final Cut Pro XML v1.10）

use crate::collab::format_smpte_timecode;
use crate::project::{ItemKind, Project};
use crate::time::RationalTime;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LetterboxAspect {
    #[default]
    None,
    Cinema239,
    Flat185,
    Academy43,
}

impl LetterboxAspect {
    pub const ALL: &'static [Self] = &[Self::None, Self::Cinema239, Self::Flat185, Self::Academy43];

    pub fn display_name(self) -> &'static str {
        match self {
            Self::None => "なし (フルフレーム)",
            Self::Cinema239 => "🎬 2.39:1 シネマスコープ",
            Self::Flat185 => "🎞 1.85:1 ビスタサイズ",
            Self::Academy43 => "📽 4:3 レトロアカデミー (ピラーボックス)",
        }
    }

    pub fn target_ratio(self) -> Option<f32> {
        match self {
            Self::None => None,
            Self::Cinema239 => Some(2.39),
            Self::Flat185 => Some(1.85),
            Self::Academy43 => Some(4.0 / 3.0),
        }
    }
}

/// シーン単位のシネマフィニッシング設定（`Scene::cinema_finishing`）
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CinemaFinishingConfig {
    #[serde(default)]
    pub enabled: bool,
    /// 35mm フィルム・ハレーション強度 (0.0..=1.5)
    #[serde(default)]
    pub halation_strength: f32,
    /// ハレーション発火輝度しきい値 (0.4..=1.0)
    #[serde(default = "default_halation_thresh")]
    pub halation_threshold: f32,
    /// アナモルフィック水平レンズフレア強度 (0.0..=1.5)
    #[serde(default)]
    pub anamorphic_flare: f32,
    /// アナモルフィックフレア色味 RGB（既定: シネマシアンブルー）
    #[serde(default = "default_flare_tint")]
    pub anamorphic_tint: [f32; 3],
    /// オーガニック 35mm フィルムグレイン強度 (0.0..=0.5)
    #[serde(default)]
    pub film_grain: f32,
    /// 周辺減光ビネット強度 (0.0..=1.0)
    #[serde(default)]
    pub vignette: f32,
    /// シネマスコープ・レターボックス
    #[serde(default)]
    pub letterbox: LetterboxAspect,
    /// クライアント確認用 SMPTE タイムコード・プロバーンイン表示
    #[serde(default)]
    pub burn_in_timecode: bool,
    /// カスタム透かしバーンイン文字列（例: "CONFIDENTIAL - ROUGH CUT"）
    #[serde(default)]
    pub watermark_text: String,
}

fn default_halation_thresh() -> f32 {
    0.72
}

fn default_flare_tint() -> [f32; 3] {
    [0.32, 0.72, 1.0]
}

impl Default for CinemaFinishingConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            halation_strength: 0.45,
            halation_threshold: default_halation_thresh(),
            anamorphic_flare: 0.35,
            anamorphic_tint: default_flare_tint(),
            film_grain: 0.08,
            vignette: 0.22,
            letterbox: LetterboxAspect::None,
            burn_in_timecode: false,
            watermark_text: String::new(),
        }
    }
}

impl CinemaFinishingConfig {
    pub fn preset_kodak_35mm() -> Self {
        Self {
            enabled: true,
            halation_strength: 0.65,
            halation_threshold: 0.68,
            anamorphic_flare: 0.15,
            anamorphic_tint: [1.0, 0.72, 0.38],
            film_grain: 0.14,
            vignette: 0.28,
            letterbox: LetterboxAspect::Cinema239,
            burn_in_timecode: false,
            watermark_text: String::new(),
        }
    }

    pub fn preset_scifi_anamorphic() -> Self {
        Self {
            enabled: true,
            halation_strength: 0.30,
            halation_threshold: 0.72,
            anamorphic_flare: 0.85,
            anamorphic_tint: [0.22, 0.78, 1.0],
            film_grain: 0.06,
            vignette: 0.32,
            letterbox: LetterboxAspect::Cinema239,
            burn_in_timecode: false,
            watermark_text: String::new(),
        }
    }

    pub fn preset_agency_review_burnin() -> Self {
        Self {
            enabled: true,
            halation_strength: 0.0,
            halation_threshold: 0.8,
            anamorphic_flare: 0.0,
            anamorphic_tint: default_flare_tint(),
            film_grain: 0.0,
            vignette: 0.0,
            letterbox: LetterboxAspect::None,
            burn_in_timecode: true,
            watermark_text: "CLIENT REVIEW - DO NOT DISTRIBUTE".into(),
        }
    }
}

/// 業界標準 CMX 3600 EDL（DaVinci Resolve / Premiere Pro / Avid Media Composer コンフォーム互換）を書き出す
pub fn export_cmx3600_edl(project: &Project) -> String {
    let Some(scene) = project.active_scene() else {
        return "TITLE: EMPTY\nFCM: NON-DROP FRAME\n".into();
    };
    let fps = scene.fps;
    let mut out = String::new();
    out.push_str(&format!(
        "TITLE: {}\n",
        project.name.replace('\n', " ").trim()
    ));
    out.push_str("FCM: NON-DROP FRAME\n\n");

    let mut events: Vec<(usize, &crate::project::TimelineItem)> = Vec::new();
    for (l_idx, layer) in scene.layers.iter().enumerate() {
        for item in &layer.items {
            if !matches!(
                item.kind,
                ItemKind::FilterObject { .. }
                    | ItemKind::Transition { .. }
                    | ItemKind::GroupControl { .. }
            ) {
                events.push((l_idx, item));
            }
        }
    }
    events.sort_by_key(|(l_idx, it)| (it.start, *l_idx));

    for (i, (_l_idx, item)) in events.iter().enumerate() {
        let edit_num = i + 1;
        let (channel, src_in) = match &item.kind {
            ItemKind::Audio { source_offset, .. } => ("A", *source_offset),
            ItemKind::Video { source_offset, .. } => ("V", *source_offset),
            _ => ("V", RationalTime::ZERO),
        };
        let src_out = src_in + item.duration;
        let rec_in = item.start;
        let rec_out = item.end();

        let reel: String = item
            .name
            .chars()
            .filter(|c| c.is_ascii_alphanumeric())
            .take(8)
            .collect::<String>()
            .to_uppercase();
        let reel = if reel.is_empty() {
            format!("REEL{:04}", edit_num)
        } else {
            reel
        };

        out.push_str(&format!(
            "{edit_num:03}  {reel:<8} {channel:<4}  C        {} {} {} {}\n",
            format_smpte_timecode(src_in, fps),
            format_smpte_timecode(src_out, fps),
            format_smpte_timecode(rec_in, fps),
            format_smpte_timecode(rec_out, fps),
        ));
        out.push_str(&format!("* FROM CLIP NAME: {}\n", item.name));
        if item.speed_ramp.enabled {
            out.push_str(&format!(
                "* SPEED RAMP: {:?} ({} knots)\n",
                item.speed_ramp.preset,
                item.speed_ramp.knots.len()
            ));
        }
        out.push('\n');
    }

    out
}

/// Apple Final Cut Pro / DaVinci Resolve 互換の FCPXML v1.10 ドキュメントを書き出す
pub fn export_fcpxml_v10(project: &Project) -> String {
    let Some(scene) = project.active_scene() else {
        return "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<fcpxml version=\"1.10\"/>\n".into();
    };
    let fps_num = scene.fps.num.max(1);
    let fps_den = scene.fps.den.max(1);
    let total_sec = scene.total_duration().as_seconds().max(1.0);
    let total_frames = (total_sec * scene.fps.as_f64()).round().max(1.0) as i64;

    let mut xml = String::new();
    xml.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    xml.push_str("<!DOCTYPE fcpxml>\n");
    xml.push_str("<fcpxml version=\"1.10\">\n");
    xml.push_str("  <resources>\n");
    xml.push_str(&format!(
        "    <format id=\"r1\" name=\"FFVideoFormat{}p{}\" frameDuration=\"{}/{}s\" width=\"{}\" height=\"{}\"/>\n",
        scene.height,
        (scene.fps.as_f64().round() as u32),
        fps_den,
        fps_num,
        scene.width,
        scene.height
    ));

    for (idx, asset) in project.assets.iter().enumerate() {
        let src = asset
            .relative_path
            .as_ref()
            .or(asset.fallback_absolute_path.as_ref())
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| asset.name.clone());
        xml.push_str(&format!(
            "    <asset id=\"a{}\" name=\"{}\" uid=\"{}\" src=\"file://{}\" hasVideo=\"1\" hasAudio=\"1\"/>\n",
            idx + 1,
            escape_xml(&asset.name),
            asset.id,
            escape_xml(&src)
        ));
    }
    xml.push_str("  </resources>\n");
    xml.push_str("  <library>\n");
    xml.push_str(&format!(
        "    <event name=\"{}\">\n",
        escape_xml(&project.name)
    ));
    xml.push_str(&format!(
        "      <project name=\"{}\">\n",
        escape_xml(&scene.name)
    ));
    xml.push_str(&format!(
        "        <sequence format=\"r1\" duration=\"{}/{}s\" tcStart=\"0s\" tcFormat=\"NDF\">\n",
        total_frames * i64::from(fps_den),
        fps_num
    ));
    xml.push_str("          <spine>\n");

    let mut items: Vec<(usize, &crate::project::TimelineItem)> = Vec::new();
    for (l_idx, layer) in scene.layers.iter().enumerate() {
        for item in &layer.items {
            items.push((l_idx, item));
        }
    }
    items.sort_by_key(|(l_idx, it)| (it.start, *l_idx));

    for (l_idx, item) in items {
        let start_f = (item.start.as_seconds() * scene.fps.as_f64()).round() as i64;
        let dur_f = (item.duration.as_seconds() * scene.fps.as_f64())
            .round()
            .max(1.0) as i64;
        let offset_attr = format!("{}/{}s", start_f * i64::from(fps_den), fps_num);
        let dur_attr = format!("{}/{}s", dur_f * i64::from(fps_den), fps_num);
        match &item.kind {
            ItemKind::Text { content, .. } => {
                xml.push_str(&format!(
                    "            <title name=\"{}\" lane=\"{}\" offset=\"{offset_attr}\" duration=\"{dur_attr}\">\n",
                    escape_xml(&item.name),
                    l_idx
                ));
                xml.push_str(&format!(
                    "              <text><text-style ref=\"ts1\">{}</text-style></text>\n",
                    escape_xml(content)
                ));
                xml.push_str("            </title>\n");
            }
            ItemKind::Video { asset_id, .. }
            | ItemKind::Audio { asset_id, .. }
            | ItemKind::Image { asset_id } => {
                let ref_id = project
                    .assets
                    .iter()
                    .position(|a| a.id == *asset_id)
                    .map(|idx| format!("a{}", idx + 1))
                    .unwrap_or_else(|| "r1".into());
                xml.push_str(&format!(
                    "            <asset-clip ref=\"{ref_id}\" name=\"{}\" lane=\"{}\" offset=\"{offset_attr}\" duration=\"{dur_attr}\"/>\n",
                    escape_xml(&item.name),
                    l_idx
                ));
            }
            _ => {
                xml.push_str(&format!(
                    "            <clip name=\"{}\" lane=\"{}\" offset=\"{offset_attr}\" duration=\"{dur_attr}\"/>\n",
                    escape_xml(&item.name),
                    l_idx
                ));
            }
        }
    }

    for marker in &scene.markers {
        let mf = (marker.time.as_seconds() * scene.fps.as_f64()).round() as i64;
        xml.push_str(&format!(
            "            <marker start=\"{}/{}s\" duration=\"{}/{}s\" value=\"{}\"/>\n",
            mf * i64::from(fps_den),
            fps_num,
            fps_den,
            fps_num,
            escape_xml(&marker.label)
        ));
    }

    xml.push_str("          </spine>\n");
    xml.push_str("        </sequence>\n");
    xml.push_str("      </project>\n");
    xml.push_str("    </event>\n");
    xml.push_str("  </library>\n");
    xml.push_str("</fcpxml>\n");
    xml
}

fn escape_xml(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::{ProjectSettings, RgbaColor, TextAnimator, TimelineItem, TimelineMarker};

    #[test]
    fn exports_valid_cmx3600_edl_and_fcpxml_v10() {
        let mut proj = Project::new("CinemaMaster_v5", ProjectSettings::default());
        {
            let sc = proj.active_scene_mut().unwrap();
            sc.layers[0].items.push(TimelineItem::new(
                "HeroTitle",
                RationalTime::from_seconds(1.0),
                RationalTime::from_seconds(3.0),
                ItemKind::Text {
                    content: "Act I & Opening".into(),
                    font_family: "Sans".into(),
                    font_size: 48.0,
                    color: RgbaColor::WHITE,
                    animator: TextAnimator::default(),
                },
            ));
            sc.markers.push(TimelineMarker::new(
                RationalTime::from_seconds(2.0),
                "Intro Beat",
            ));
        }

        let edl = export_cmx3600_edl(&proj);
        assert!(edl.contains("TITLE: CinemaMaster_v5"));
        assert!(edl.contains("FCM: NON-DROP FRAME"));
        assert!(edl.contains("001  HEROTITL"));
        assert!(edl.contains("* FROM CLIP NAME: HeroTitle"));

        let xml = export_fcpxml_v10(&proj);
        assert!(xml.contains("<fcpxml version=\"1.10\">"));
        assert!(xml.contains("Act I &amp; Opening"));
        assert!(xml.contains("Intro Beat"));
    }
}
