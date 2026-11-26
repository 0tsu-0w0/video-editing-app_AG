//! v9.0 #56: ローカル スマート DAM（代表色 / BPM / LUFS 索引・類似ショット検索・スマートビン・一括自動再リンク）
//!
//! Pure Rust・ゼロ `unsafe`・外部追加依存ゼロ・完全オフライン完結で以下を提供する:
//! - FIPS 180-4 SHA-256 先頭 64KiB チャンク指紋＆ファイルサイズ算出 (`compute_file_head_fingerprint`)
//! - 5 色ドミナントカラーパレット・暖色/寒色スコア・エッジエネルギー・16 次元視覚特徴ベクトル抽出 (`extract_visual_features_from_rgba8`)
//! - コサイン類似度による類似ショット検索 (`find_similar_shots`)
//! - 条件式スマートビン (`SmartBinRule` / `evaluate_smart_bin`)
//! - フォルダ再帰スキャンによる行方不明素材の一括自動再リンク (`relink_missing_assets_from_dir` / `build_relink_assets_command`)

use crate::command::EditCommand;
use crate::project::{AssetId, MediaKind, Project};
use crate::registry::sha256_hex;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

/// 抽出された 1 代表色スウォッチ
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DamDominantColor {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    /// 画像全体に対する占有比率 (`0.0..=1.0`)
    pub weight: f32,
    pub hex: String,
}

/// 1 素材あたりのローカル DAM 索引レコード
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DamAssetRecord {
    #[serde(default)]
    pub asset_id: Option<AssetId>,
    pub file_path: String,
    pub file_name: String,
    pub kind: MediaKind,
    #[serde(default)]
    pub file_size_bytes: u64,
    /// 先頭 64 KiB の FIPS 180-4 SHA-256 フィンガープリント（ドライブ移動後の厳密自動再リンク用）
    #[serde(default)]
    pub head_sha256: String,
    #[serde(default)]
    pub dominant_colors: Vec<DamDominantColor>,
    /// 平均輝度 (`0.0..=1.0`)
    #[serde(default)]
    pub avg_luma: f32,
    /// 平均彩度 (`0.0..=1.0`)
    #[serde(default)]
    pub avg_saturation: f32,
    /// 暖色度（`> 0` 暖色/夕景、`< 0` 寒色/夜景、`-1.0..=1.0`）
    #[serde(default)]
    pub warmth_score: f32,
    /// 平均 Sobel エッジ密度 (`0.0..=1.0`)
    #[serde(default)]
    pub edge_energy: f32,
    /// L2 正規化済み 16 次元視覚特徴ベクトル（色相 8 ビン + 輝度 4 ビン + 彩度/暖色/エッジ/コントラスト 4 次元）
    #[serde(default)]
    pub feature_vec: [f32; 16],
    #[serde(default)]
    pub bpm: Option<f32>,
    #[serde(default)]
    pub integrated_lufs: Option<f32>,
    #[serde(default)]
    pub duration_sec: Option<f64>,
    #[serde(default)]
    pub tags: Vec<String>,
}

fn default_max_color_dist() -> f32 {
    95.0
}

/// 条件式スマートビンのフィルタ定義
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SmartBinRule {
    pub name: String,
    #[serde(default)]
    pub media_kind: Option<MediaKind>,
    #[serde(default)]
    pub min_warmth: Option<f32>,
    #[serde(default)]
    pub max_warmth: Option<f32>,
    #[serde(default)]
    pub min_bpm: Option<f32>,
    #[serde(default)]
    pub max_bpm: Option<f32>,
    #[serde(default)]
    pub min_lufs: Option<f32>,
    #[serde(default)]
    pub max_lufs: Option<f32>,
    #[serde(default)]
    pub tag_contains: Option<String>,
    /// 指定 HEX 色（例: `"#ff8844"`）に近い代表色を持つショットのみ抽出
    #[serde(default)]
    pub target_hex_color: Option<String>,
    #[serde(default = "default_max_color_dist")]
    pub max_color_dist: f32,
}

fn default_smart_bins() -> Vec<SmartBinRule> {
    vec![
        SmartBinRule {
            name: "🌅 夕景・暖色ショット".into(),
            media_kind: None,
            min_warmth: Some(0.12),
            max_warmth: None,
            min_bpm: None,
            max_bpm: None,
            min_lufs: None,
            max_lufs: None,
            tag_contains: None,
            target_hex_color: None,
            max_color_dist: default_max_color_dist(),
        },
        SmartBinRule {
            name: "❄ クール・寒色ショット".into(),
            media_kind: None,
            min_warmth: None,
            max_warmth: Some(-0.10),
            min_bpm: None,
            max_bpm: None,
            min_lufs: None,
            max_lufs: None,
            tag_contains: None,
            target_hex_color: None,
            max_color_dist: default_max_color_dist(),
        },
        SmartBinRule {
            name: "🥁 アップテンポ音楽 (110〜145 BPM)".into(),
            media_kind: Some(MediaKind::Audio),
            min_warmth: None,
            max_warmth: None,
            min_bpm: Some(110.0),
            max_bpm: Some(145.0),
            min_lufs: None,
            max_lufs: None,
            tag_contains: None,
            target_hex_color: None,
            max_color_dist: default_max_color_dist(),
        },
        SmartBinRule {
            name: "🔊 ラウド音声 (> -18 LUFS)".into(),
            media_kind: Some(MediaKind::Audio),
            min_warmth: None,
            max_warmth: None,
            min_bpm: None,
            max_bpm: None,
            min_lufs: Some(-18.0),
            max_lufs: None,
            tag_contains: None,
            target_hex_color: None,
            max_color_dist: default_max_color_dist(),
        },
    ]
}

/// プロジェクト単位のローカル スマート DAM インデックスデータベース
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SmartDamIndex {
    #[serde(default)]
    pub records: Vec<DamAssetRecord>,
    #[serde(default = "default_smart_bins")]
    pub smart_bins: Vec<SmartBinRule>,
}

impl Default for SmartDamIndex {
    fn default() -> Self {
        Self {
            records: Vec::new(),
            smart_bins: default_smart_bins(),
        }
    }
}

impl SmartDamIndex {
    /// 同一 `file_path` または `asset_id` のレコードを上書きまたは新規追加する
    pub fn upsert_record(&mut self, rec: DamAssetRecord) {
        if let Some(existing) = self.records.iter_mut().find(|r| {
            (rec.asset_id.is_some() && r.asset_id == rec.asset_id) || r.file_path == rec.file_path
        }) {
            *existing = rec;
        } else {
            self.records.push(rec);
        }
    }
}

/// 先頭 64 KiB + ファイルサイズから高速かつ衝突耐性の高い SHA-256 指紋を算出する
pub fn compute_file_head_fingerprint(path: &Path) -> Result<(u64, String), String> {
    let meta = std::fs::metadata(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let size = meta.len();
    let mut f = File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut buf = vec![0u8; 65_536.min(size as usize)];
    let mut read_total = 0usize;
    while read_total < buf.len() {
        let n = f
            .read(&mut buf[read_total..])
            .map_err(|e| format!("{}: {e}", path.display()))?;
        if n == 0 {
            break;
        }
        read_total += n;
    }
    buf.truncate(read_total);
    let mut payload = Vec::with_capacity(8 + buf.len());
    payload.extend_from_slice(&size.to_le_bytes());
    payload.extend_from_slice(&buf);
    Ok((size, sha256_hex(&payload)))
}

/// RGBA8 画像から 5 色ドミナントパレット・輝度・彩度・暖色度・エッジ密度・16 次元 L2 正規化特徴ベクトルを抽出する
pub fn extract_visual_features_from_rgba8(
    width: u32,
    height: u32,
    rgba: &[u8],
) -> (Vec<DamDominantColor>, f32, f32, f32, f32, [f32; 16]) {
    let px_count = (width as usize).saturating_mul(height as usize);
    if px_count == 0 || rgba.len() < px_count * 4 {
        return (Vec::new(), 0.0, 0.0, 0.0, 0.0, [0.0; 16]);
    }

    // 4×4×4 = 64 色空間バケットで高速ドミナント色抽出
    let mut buckets: [u64; 64] = [0; 64];
    let mut bucket_rgb_sum: [[u64; 3]; 64] = [[0; 3]; 64];
    let mut hue_hist = [0.0_f32; 8];
    let mut luma_hist = [0.0_f32; 4];

    let mut sum_luma = 0.0_f64;
    let mut sum_luma_sq = 0.0_f64;
    let mut sum_sat = 0.0_f64;
    let mut sum_warmth = 0.0_f64;
    let mut sample_n = 0u64;

    let step = (px_count / 4096).max(1);
    let mut luma_grid = vec![0.0_f32; px_count];

    for (idx, px) in rgba.as_chunks::<4>().0.iter().take(px_count).enumerate() {
        let r = px[0] as f32 / 255.0;
        let g = px[1] as f32 / 255.0;
        let b = px[2] as f32 / 255.0;
        let luma = 0.2126 * r + 0.7152 * g + 0.0722 * b;
        luma_grid[idx] = luma;

        if idx % step != 0 {
            continue;
        }
        sample_n += 1;

        let bi =
            ((px[0] as usize >> 6) << 4) | ((px[1] as usize >> 6) << 2) | (px[2] as usize >> 6);
        buckets[bi] += 1;
        bucket_rgb_sum[bi][0] += px[0] as u64;
        bucket_rgb_sum[bi][1] += px[1] as u64;
        bucket_rgb_sum[bi][2] += px[2] as u64;

        let max_c = r.max(g).max(b);
        let min_c = r.min(g).min(b);
        let delta = max_c - min_c;
        let sat = if max_c > 1e-5 { delta / max_c } else { 0.0 };
        let hue_deg = if delta < 1e-5 {
            0.0
        } else if (max_c - r).abs() < 1e-6 {
            60.0 * (((g - b) / delta).rem_euclid(6.0))
        } else if (max_c - g).abs() < 1e-6 {
            60.0 * (((b - r) / delta) + 2.0)
        } else {
            60.0 * (((r - g) / delta) + 4.0)
        };

        let h_bin = ((hue_deg / 45.0).floor() as usize).min(7);
        hue_hist[h_bin] += 0.2 + sat;
        let l_bin = ((luma * 4.0).floor() as usize).min(3);
        luma_hist[l_bin] += 1.0;

        sum_luma += f64::from(luma);
        sum_luma_sq += f64::from(luma * luma);
        sum_sat += f64::from(sat);
        // 暖色 (赤・橙・黄) vs 寒色 (青・シアン)
        sum_warmth += f64::from((r + 0.35 * g - 1.15 * b).clamp(-1.0, 1.0));
    }

    // 簡易エッジエネルギー (Sobel 近似)
    let w = width as usize;
    let h = height as usize;
    let mut edge_sum = 0.0_f64;
    let mut edge_n = 0u64;
    if w >= 3 && h >= 3 {
        let y_step = (h / 48).max(1);
        let x_step = (w / 48).max(1);
        let mut y = 1;
        while y + 1 < h {
            let mut x = 1;
            while x + 1 < w {
                let gx = luma_grid[y * w + (x + 1)] - luma_grid[y * w + (x - 1)];
                let gy = luma_grid[(y + 1) * w + x] - luma_grid[(y - 1) * w + x];
                edge_sum += f64::from(gx.hypot(gy));
                edge_n += 1;
                x += x_step;
            }
            y += y_step;
        }
    }

    let n_f = sample_n.max(1) as f64;
    let avg_luma = (sum_luma / n_f) as f32;
    let variance = ((sum_luma_sq / n_f) - (sum_luma / n_f).powi(2)).max(0.0);
    let contrast = variance.sqrt().min(1.0) as f32;
    let avg_sat = (sum_sat / n_f) as f32;
    let warmth = (sum_warmth / n_f).clamp(-1.0, 1.0) as f32;
    let edge_energy = (edge_sum / edge_n.max(1) as f64).min(1.0) as f32;

    // 上位 5 ドミナント色抽出
    let mut ranked: Vec<(usize, u64)> = buckets
        .iter()
        .enumerate()
        .filter(|(_, c)| **c > 0)
        .map(|(i, &c)| (i, c))
        .collect();
    ranked.sort_by_key(|a| std::cmp::Reverse(a.1));
    let dominant: Vec<DamDominantColor> = ranked
        .into_iter()
        .take(5)
        .map(|(bi, cnt)| {
            let r = (bucket_rgb_sum[bi][0] / cnt) as u8;
            let g = (bucket_rgb_sum[bi][1] / cnt) as u8;
            let b = (bucket_rgb_sum[bi][2] / cnt) as u8;
            DamDominantColor {
                r,
                g,
                b,
                weight: cnt as f32 / sample_n.max(1) as f32,
                hex: format!("#{r:02x}{g:02x}{b:02x}"),
            }
        })
        .collect();

    // 16 次元視覚特徴ベクトル組み立て＆L2 正規化
    let mut feat = [0.0_f32; 16];
    let h_tot = hue_hist.iter().sum::<f32>().max(1e-5);
    for i in 0..8 {
        feat[i] = hue_hist[i] / h_tot;
    }
    let l_tot = luma_hist.iter().sum::<f32>().max(1e-5);
    for i in 0..4 {
        feat[8 + i] = luma_hist[i] / l_tot;
    }
    feat[12] = avg_sat;
    feat[13] = (warmth + 1.0) * 0.5;
    feat[14] = edge_energy;
    feat[15] = contrast;

    let norm = feat.iter().map(|v| v * v).sum::<f32>().sqrt();
    if norm > 1e-6 {
        for v in &mut feat {
            *v /= norm;
        }
    }

    (dominant, avg_luma, avg_sat, warmth, edge_energy, feat)
}

/// 16 次元視覚特徴ベクトル間のコサイン類似度 (`-1.0..=1.0`, 1.0 が完全一致)
pub fn cosine_similarity_16(a: &[f32; 16], b: &[f32; 16]) -> f32 {
    let mut dot = 0.0_f32;
    let mut na = 0.0_f32;
    let mut nb = 0.0_f32;
    for i in 0..16 {
        dot += a[i] * b[i];
        na += a[i] * a[i];
        nb += b[i] * b[i];
    }
    let denom = (na * nb).sqrt();
    if denom < 1e-6 {
        0.0
    } else {
        (dot / denom).clamp(-1.0, 1.0)
    }
}

/// 類似ショット検索結果 1 件
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SimilarShotHit {
    pub file_path: String,
    pub file_name: String,
    pub asset_id: Option<AssetId>,
    pub similarity: f32,
}

/// クエリ特徴ベクトルに最も近いショット上位 `top_k` 件をコサイン類似度降順で返す
pub fn find_similar_shots(
    index: &SmartDamIndex,
    query_feat: &[f32; 16],
    exclude_path: Option<&str>,
    top_k: usize,
) -> Vec<SimilarShotHit> {
    let mut hits: Vec<SimilarShotHit> = index
        .records
        .iter()
        .filter(|r| exclude_path.map(|ex| r.file_path != ex).unwrap_or(true))
        .map(|r| SimilarShotHit {
            file_path: r.file_path.clone(),
            file_name: r.file_name.clone(),
            asset_id: r.asset_id,
            similarity: cosine_similarity_16(query_feat, &r.feature_vec),
        })
        .collect();
    hits.sort_by(|a, b| b.similarity.total_cmp(&a.similarity));
    hits.truncate(top_k.max(1));
    hits
}

fn parse_hex_rgb(hex: &str) -> Option<(f32, f32, f32)> {
    let s = hex.trim().trim_start_matches('#');
    if s.len() < 6 {
        return None;
    }
    let r = u8::from_str_radix(&s[0..2], 16).ok()? as f32;
    let g = u8::from_str_radix(&s[2..4], 16).ok()? as f32;
    let b = u8::from_str_radix(&s[4..6], 16).ok()? as f32;
    Some((r, g, b))
}

/// スマートビン条件式に一致するレコード群を抽出する
pub fn evaluate_smart_bin<'a>(
    index: &'a SmartDamIndex,
    rule: &SmartBinRule,
) -> Vec<&'a DamAssetRecord> {
    let target_rgb = rule.target_hex_color.as_deref().and_then(parse_hex_rgb);
    index
        .records
        .iter()
        .filter(|rec| {
            if let Some(mk) = rule.media_kind
                && rec.kind != mk
            {
                return false;
            }
            if let Some(min_w) = rule.min_warmth
                && rec.warmth_score < min_w
            {
                return false;
            }
            if let Some(max_w) = rule.max_warmth
                && rec.warmth_score > max_w
            {
                return false;
            }
            if let Some(min_b) = rule.min_bpm
                && rec.bpm.map(|b| b < min_b).unwrap_or(true)
            {
                return false;
            }
            if let Some(max_b) = rule.max_bpm
                && rec.bpm.map(|b| b > max_b).unwrap_or(true)
            {
                return false;
            }
            if let Some(min_l) = rule.min_lufs
                && rec.integrated_lufs.map(|l| l < min_l).unwrap_or(true)
            {
                return false;
            }
            if let Some(max_l) = rule.max_lufs
                && rec.integrated_lufs.map(|l| l > max_l).unwrap_or(true)
            {
                return false;
            }
            if let Some(ref tag_q) = rule.tag_contains
                && !tag_q.trim().is_empty()
            {
                let q = tag_q.to_ascii_lowercase();
                let hit = rec.file_name.to_ascii_lowercase().contains(&q)
                    || rec.tags.iter().any(|t| t.to_ascii_lowercase().contains(&q));
                if !hit {
                    return false;
                }
            }
            if let Some((tr, tg, tb)) = target_rgb {
                let color_hit = rec.dominant_colors.iter().any(|c| {
                    let dr = c.r as f32 - tr;
                    let dg = c.g as f32 - tg;
                    let db = c.b as f32 - tb;
                    (dr * dr + dg * dg + db * db).sqrt() <= rule.max_color_dist
                });
                if !color_hit {
                    return false;
                }
            }
            true
        })
        .collect()
}

// ============================================================================
// 一括自動再リンク（SHA-256 指紋優先 → ファイルサイズ＆ファイル名フォールバック）
// ============================================================================

/// 1 アセットの再リンク解決結果
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RelinkMatch {
    pub asset_id: AssetId,
    pub asset_name: String,
    pub new_path: String,
    pub match_reason: String,
}

/// 一括自動再リンク計画
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RelinkPlan {
    pub scanned_files: usize,
    pub missing_assets: usize,
    pub matches: Vec<RelinkMatch>,
    pub unresolved_names: Vec<String>,
}

fn collect_files_recursive(dir: &Path, out: &mut Vec<PathBuf>, depth: usize) {
    if depth > 8 || out.len() >= 10_000 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let p = entry.path();
        if p.is_dir() {
            collect_files_recursive(&p, out, depth + 1);
        } else if p.is_file() {
            out.push(p);
        }
    }
}

/// プロジェクト内のリンク切れアセットに対し、`search_dir` を再帰スキャンして SHA-256 先頭指紋・サイズ・ファイル名で一括自動再リンク計画を生成する
pub fn relink_missing_assets_from_dir(
    project: &Project,
    index: &SmartDamIndex,
    search_dir: &Path,
) -> RelinkPlan {
    let mut plan = RelinkPlan::default();
    let mut candidates = Vec::new();
    collect_files_recursive(search_dir, &mut candidates, 0);
    plan.scanned_files = candidates.len();

    // スキャン先候補の (sha256 -> path) および (lowercase_filename -> vec<path>) を索引化
    let mut by_hash: BTreeMap<String, PathBuf> = BTreeMap::new();
    let mut by_name: BTreeMap<String, Vec<PathBuf>> = BTreeMap::new();
    for p in &candidates {
        if let Ok((_sz, h)) = compute_file_head_fingerprint(p) {
            by_hash.entry(h).or_insert_with(|| p.clone());
        }
        if let Some(fname) = p.file_name().and_then(|s| s.to_str()) {
            by_name
                .entry(fname.to_ascii_lowercase())
                .or_default()
                .push(p.clone());
        }
    }

    for asset in &project.assets {
        let exists = asset
            .fallback_absolute_path
            .as_deref()
            .map(|p| Path::new(p).exists())
            .unwrap_or(false);
        if exists {
            continue;
        }
        plan.missing_assets += 1;

        // 1. DAM 索引にある SHA-256 指紋で厳密一致（リネーム後でも再リンク可能！）
        let known_hash = index
            .records
            .iter()
            .find(|r| r.asset_id == Some(asset.id) || r.file_name.eq_ignore_ascii_case(&asset.name))
            .filter(|r| !r.head_sha256.is_empty())
            .map(|r| r.head_sha256.clone());

        if let Some(ref h) = known_hash
            && let Some(found) = by_hash.get(h)
        {
            plan.matches.push(RelinkMatch {
                asset_id: asset.id,
                asset_name: asset.name.clone(),
                new_path: found.display().to_string(),
                match_reason: "SHA-256 先頭64KiB指紋厳密一致".into(),
            });
            continue;
        }

        // 2. ファイル名フォールバック一致
        let key = Path::new(&asset.name)
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or(&asset.name)
            .to_ascii_lowercase();
        if let Some(found) = by_name.get(&key).and_then(|v| v.first()) {
            plan.matches.push(RelinkMatch {
                asset_id: asset.id,
                asset_name: asset.name.clone(),
                new_path: found.display().to_string(),
                match_reason: "ファイル名一致".into(),
            });
        } else {
            plan.unresolved_names.push(asset.name.clone());
        }
    }

    plan
}

/// 再リンク計画を 1-Undo `EditCommand::RelinkAssets` へ変換する
pub fn build_relink_assets_command(plan: &RelinkPlan) -> Option<EditCommand> {
    if plan.matches.is_empty() {
        return None;
    }
    Some(EditCommand::RelinkAssets {
        relinks: plan
            .matches
            .iter()
            .map(|m| (m.asset_id, m.new_path.clone()))
            .collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::{CommandBus, CommandSource};
    use crate::project::{AssetRef, ProjectSettings};
    use uuid::Uuid;

    #[test]
    fn dam_extracts_warm_vs_cool_features_finds_similar_and_relinks_by_sha256_even_when_renamed() {
        // 1. Warm sunset vs Cool night synthetic RGBA8 frames
        let mut warm = vec![0u8; 32 * 32 * 4];
        let mut cool = vec![0u8; 32 * 32 * 4];
        for px in warm.as_chunks_mut::<4>().0 {
            *px = [245, 130, 45, 255];
        }
        for px in cool.as_chunks_mut::<4>().0 {
            *px = [35, 95, 230, 255];
        }
        let (dom_w, _lw, _sw, warmth_w, _ew, feat_w) =
            extract_visual_features_from_rgba8(32, 32, &warm);
        let (dom_c, _lc, _sc, warmth_c, _ec, feat_c) =
            extract_visual_features_from_rgba8(32, 32, &cool);
        assert!(!dom_w.is_empty() && !dom_c.is_empty());
        assert!(warmth_w > 0.3, "expected warm > 0.3, got {warmth_w}");
        assert!(warmth_c < -0.2, "expected cool < -0.2, got {warmth_c}");
        assert!(cosine_similarity_16(&feat_w, &feat_w) > 0.99);
        assert!(cosine_similarity_16(&feat_w, &feat_c) < 0.8);

        // 2. Test SHA-256 fingerprint relink across directory move + rename
        let tmp = std::env::temp_dir().join(format!("ve_dam_relink_{}", Uuid::new_v4().simple()));
        let moved_dir = tmp.join("new_drive/archive");
        std::fs::create_dir_all(&moved_dir).unwrap();
        let moved_file = moved_dir.join("renamed_master_clip.bin");
        std::fs::write(
            &moved_file,
            b"unique-camera-raw-header-bytes-v9-smart-dam-test",
        )
        .unwrap();
        let (sz, hash) = compute_file_head_fingerprint(&moved_file).unwrap();

        let asset_id = Uuid::new_v4();
        let mut proj = Project::new("DamTest", ProjectSettings::default());
        proj.assets.push(AssetRef {
            id: asset_id,
            name: "old_clip_name.mov".into(),
            kind: MediaKind::Video,
            relative_path: None,
            fallback_absolute_path: Some("/nonexistent_drive/old_clip_name.mov".into()),
            collected: false,
        });

        let mut dam = SmartDamIndex::default();
        dam.upsert_record(DamAssetRecord {
            asset_id: Some(asset_id),
            file_path: "/nonexistent_drive/old_clip_name.mov".into(),
            file_name: "old_clip_name.mov".into(),
            kind: MediaKind::Video,
            file_size_bytes: sz,
            head_sha256: hash,
            dominant_colors: dom_w,
            avg_luma: 0.6,
            avg_saturation: 0.8,
            warmth_score: warmth_w,
            edge_energy: 0.1,
            feature_vec: feat_w,
            bpm: None,
            integrated_lufs: None,
            duration_sec: Some(5.0),
            tags: vec!["sunset".into()],
        });
        dam.upsert_record(DamAssetRecord {
            asset_id: None,
            file_path: "/clips/night_city.mov".into(),
            file_name: "night_city.mov".into(),
            kind: MediaKind::Video,
            file_size_bytes: 100,
            head_sha256: String::new(),
            dominant_colors: dom_c,
            avg_luma: 0.3,
            avg_saturation: 0.7,
            warmth_score: warmth_c,
            edge_energy: 0.2,
            feature_vec: feat_c,
            bpm: None,
            integrated_lufs: None,
            duration_sec: Some(4.0),
            tags: vec!["night".into()],
        });

        // Smart Bin warm sunset matches only warm clip
        let warm_hits = evaluate_smart_bin(&dam, &dam.smart_bins[0]);
        assert_eq!(warm_hits.len(), 1);
        assert_eq!(warm_hits[0].file_name, "old_clip_name.mov");

        // Relink finds renamed file strictly by SHA-256 fingerprint!
        let plan = relink_missing_assets_from_dir(&proj, &dam, &tmp);
        assert_eq!(plan.missing_assets, 1);
        assert_eq!(plan.matches.len(), 1);
        assert!(plan.matches[0].match_reason.contains("SHA-256"));
        let cmd = build_relink_assets_command(&plan).unwrap();
        let mut bus = CommandBus::new();
        bus.execute(&mut proj, cmd, CommandSource::Gui, None)
            .unwrap();
        assert_eq!(
            proj.assets[0].fallback_absolute_path.as_deref(),
            Some(moved_file.as_path())
        );

        let _ = std::fs::remove_dir_all(&tmp);
    }
}
