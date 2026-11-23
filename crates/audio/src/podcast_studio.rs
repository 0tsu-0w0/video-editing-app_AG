//! v6.0 #43: マルチトラック・ポッドキャスト＆コラボ実況オート編集エンジン
//! Pure Rust・ゼロ `unsafe` で、複数話者の音声トラックから発話区間（話者ダイアリゼーション）を
//! 50ms ブロックエネルギー＋ヒステリシスで自動検出し、アクティブ話者カメラ自動切替キーフレーム＆
//! 話者間 -16 LUFS 均一化レベラーを一括適用する。

use crate::dsp::{db_to_linear, linear_to_db, measure_integrated_lufs};
use crate::spatial_master::apply_true_peak_limiter;
use ve_core::{EditCommand, Interpolation, Keyframe, RationalTime, Scene, TimelineMarker};

#[derive(Debug, Clone, PartialEq)]
pub struct SpeakerTrackSpec {
    pub name: String,
    pub audio_layer_index: usize,
    pub camera_layer_index: Option<usize>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SpeakerTurnSegment {
    pub speaker_idx: usize,
    pub speaker_name: String,
    pub start_sec: f64,
    pub end_sec: f64,
    pub avg_rms_db: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PodcastAnalysisResult {
    pub speaker_lufs: Vec<(String, f32)>,
    pub recommended_gains_db: Vec<f32>,
    pub turns: Vec<SpeakerTurnSegment>,
    pub target_lufs: f32,
}

/// 複数話者のステレオ PCM トラックを解析し、アクティブ話者ターン区間および
/// ポッドキャスト基準 (`target_lufs`, 既定 `-16.0 LUFS`) に揃える推奨ゲインを算出する
pub fn analyze_podcast_speakers(
    track_pcms: &[(String, Vec<f32>)],
    sample_rate: u32,
    min_turn_sec: f64,
    silence_gate_db: f32,
    target_lufs: f32,
) -> PodcastAnalysisResult {
    let sr = sample_rate.max(8_000);
    let mut speaker_lufs = Vec::with_capacity(track_pcms.len());
    let mut recommended_gains_db = Vec::with_capacity(track_pcms.len());

    for (name, pcm) in track_pcms {
        let lufs = if pcm.is_empty() {
            -70.0
        } else {
            measure_integrated_lufs(pcm, sr)
        };
        let gain_db = if lufs <= -65.0 {
            0.0
        } else {
            (target_lufs - lufs).clamp(-12.0, 14.0)
        };
        speaker_lufs.push((name.clone(), lufs));
        recommended_gains_db.push(gain_db);
    }

    if track_pcms.is_empty() {
        return PodcastAnalysisResult {
            speaker_lufs,
            recommended_gains_db,
            turns: Vec::new(),
            target_lufs,
        };
    }

    let block_sec = 0.05f64;
    let block_frames = ((f64::from(sr) * block_sec).round() as usize).max(64);
    let block_samples = block_frames * 2;
    let max_samples = track_pcms.iter().map(|(_, p)| p.len()).max().unwrap_or(0);
    let num_blocks = max_samples / block_samples;
    let gate_lin = db_to_linear(silence_gate_db.clamp(-70.0, -10.0));

    let mut raw_blocks: Vec<Option<(usize, f32)>> = Vec::with_capacity(num_blocks);
    for b in 0..num_blocks {
        let s0 = b * block_samples;
        let s1 = s0 + block_samples;
        let mut best: Option<(usize, f32)> = None;
        for (idx, (_, pcm)) in track_pcms.iter().enumerate() {
            if s1 <= pcm.len() {
                let slice = &pcm[s0..s1];
                let sum_sq: f32 = slice.iter().map(|&x| x * x).sum();
                let rms = (sum_sq / (slice.len().max(1) as f32)).sqrt();
                if rms >= gate_lin && best.is_none_or(|(_, prev)| rms > prev) {
                    best = Some((idx, rms));
                }
            }
        }
        raw_blocks.push(best);
    }

    // 連続同話者ブロックをマージし、短すぎる割り込み (< min_turn_sec) を平滑化
    let mut turns: Vec<SpeakerTurnSegment> = Vec::new();
    let mut cur: Option<(usize, usize, f32, usize)> = None; // (speaker_idx, start_b, rms_sum, cnt)
    for (b, entry) in raw_blocks.iter().enumerate() {
        match (cur, *entry) {
            (Some((sp, sb, sum, cnt)), Some((nsp, rms))) if sp == nsp => {
                cur = Some((sp, sb, sum + rms, cnt + 1));
            }
            (Some((sp, sb, sum, cnt)), next) => {
                let start_sec = (sb as f64) * block_sec;
                let end_sec = (b as f64) * block_sec;
                if end_sec - start_sec >= min_turn_sec.clamp(0.1, 5.0) {
                    turns.push(SpeakerTurnSegment {
                        speaker_idx: sp,
                        speaker_name: track_pcms[sp].0.clone(),
                        start_sec,
                        end_sec,
                        avg_rms_db: linear_to_db((sum / (cnt.max(1) as f32)).max(1e-6)),
                    });
                }
                cur = next.map(|(nsp, rms)| (nsp, b, rms, 1));
            }
            (None, Some((nsp, rms))) => {
                cur = Some((nsp, b, rms, 1));
            }
            (None, None) => {}
        }
    }
    if let Some((sp, sb, sum, cnt)) = cur {
        let start_sec = (sb as f64) * block_sec;
        let end_sec = (num_blocks as f64) * block_sec;
        if end_sec - start_sec >= min_turn_sec.clamp(0.1, 5.0) {
            turns.push(SpeakerTurnSegment {
                speaker_idx: sp,
                speaker_name: track_pcms[sp].0.clone(),
                start_sec,
                end_sec,
                avg_rms_db: linear_to_db((sum / (cnt.max(1) as f32)).max(1e-6)),
            });
        }
    }

    // 隣接する同一話者セグメントを結合（無音ギャップ吸収）
    let mut merged: Vec<SpeakerTurnSegment> = Vec::new();
    for seg in turns {
        if let Some(last) = merged.last_mut()
            && last.speaker_idx == seg.speaker_idx
            && seg.start_sec - last.end_sec <= 0.65
        {
            last.end_sec = seg.end_sec;
            continue;
        }
        merged.push(seg);
    }

    PodcastAnalysisResult {
        speaker_lufs,
        recommended_gains_db,
        turns: merged,
        target_lufs,
    }
}

/// 話者音声の音量ムラ（小さい声・大きい声）を滑らかな AGC レベラー＋True-Peak -1.0 dBTP で均一化する
pub fn apply_podcast_voice_leveler(pcm: &mut [f32], sample_rate: u32, target_lufs: f32) {
    if pcm.len() < 4 {
        return;
    }
    let sr = sample_rate.max(8_000);
    let cur_lufs = measure_integrated_lufs(pcm, sr);
    if cur_lufs > -65.0 {
        let macro_gain = db_to_linear((target_lufs - cur_lufs).clamp(-10.0, 14.0));
        for s in pcm.iter_mut() {
            *s *= macro_gain;
        }
    }
    // 滑らかな短期 AGC ライド（発話中の音量段差をさらに均一化）
    let target_rms = db_to_linear((target_lufs + 3.0).clamp(-30.0, -6.0));
    let mut env = target_rms;
    let alpha = (1.0 / (sr as f32 * 0.12)).clamp(1e-4, 0.05);
    for lr in pcm.as_chunks_mut::<2>().0.iter_mut() {
        let energy = (lr[0].abs().max(lr[1].abs())).max(1e-5);
        env += alpha * (energy - env);
        let dyn_gain = if env > 0.015 {
            (target_rms / env).clamp(0.55, 1.85)
        } else {
            1.0
        };
        lr[0] *= dyn_gain;
        lr[1] *= dyn_gain;
    }
    apply_true_peak_limiter(pcm, 2, sr, -1.0, 60.0);
}

/// 解析された話者ターンおよび推奨ゲインから、1-Undo でタイムラインへ反映するコマンド列を生成する
/// （話者マーカー追加・対応カメラレイヤーのアクティブ自動切替キーフレーム・音声レイヤーフェーダー均一化）
pub fn build_podcast_auto_switch_commands(
    scene: &Scene,
    speakers: &[SpeakerTrackSpec],
    analysis: &PodcastAnalysisResult,
    apply_leveler_faders: bool,
) -> Vec<EditCommand> {
    let mut cmds = Vec::new();

    for turn in &analysis.turns {
        let t = RationalTime::from_seconds(turn.start_sec);
        cmds.push(EditCommand::AddMarker {
            marker: TimelineMarker::new(t, format!("🎙 {}", turn.speaker_name)),
        });

        // 対応カメラレイヤーが存在する話者間で、発話中カメラの不透明度を 1.0・非発話カメラを 0.0 へ切替
        for (sp_idx, spec) in speakers.iter().enumerate() {
            if let Some(cam_layer_idx) = spec.camera_layer_index
                && let Some(cam_layer) = scene.layers.get(cam_layer_idx)
            {
                let target_op = if sp_idx == turn.speaker_idx { 1.0 } else { 0.0 };
                for item in &cam_layer.items {
                    if t >= item.start && t <= item.end() {
                        cmds.push(EditCommand::SetKeyframe {
                            item_id: item.id,
                            property: "opacity".into(),
                            keyframe: Keyframe {
                                time: t - item.start,
                                value: target_op,
                                interpolation: Interpolation::Hold,
                            },
                        });
                    }
                }
            }
        }
    }

    if apply_leveler_faders {
        for (sp_idx, spec) in speakers.iter().enumerate() {
            if let Some(&gain_db) = analysis.recommended_gains_db.get(sp_idx)
                && let Some(layer) = scene.layers.get(spec.audio_layer_index)
            {
                let next_vol = (layer.volume * db_to_linear(gain_db)).clamp(0.15, 3.5);
                cmds.push(EditCommand::SetLayerMixer {
                    layer_index: spec.audio_layer_index,
                    volume: next_vol,
                    pan: layer.pan,
                    muted: layer.muted,
                    solo: layer.solo,
                    bus: layer.bus.clone(),
                    send_reverb: layer.send_reverb,
                });
            }
        }
    }

    cmds
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::TAU;

    #[test]
    fn podcast_diarizer_detects_alternating_speakers_and_levels_lufs() {
        let sr = 48_000u32;
        let frames = sr as usize * 3; // 3 seconds
        let mut host_a = vec![0.0f32; frames * 2];
        let mut host_b = vec![0.0f32; frames * 2];

        // 0.0..1.4s: Host A talks quietly (amp 0.12)
        // 1.5..2.9s: Host B talks loudly (amp 0.55)
        for (i, lr) in host_a.as_chunks_mut::<2>().0.iter_mut().enumerate() {
            let t = i as f32 / sr as f32;
            if t < 1.4 {
                let s = (TAU * 220.0 * t).sin() * 0.12;
                lr[0] = s;
                lr[1] = s;
            }
        }
        for (i, lr) in host_b.as_chunks_mut::<2>().0.iter_mut().enumerate() {
            let t = i as f32 / sr as f32;
            if (1.5..2.9).contains(&t) {
                let s = (TAU * 165.0 * t).sin() * 0.55;
                lr[0] = s;
                lr[1] = s;
            }
        }

        let res = analyze_podcast_speakers(
            &[("Host A".into(), host_a.clone()), ("Host B".into(), host_b)],
            sr,
            0.3,
            -42.0,
            -16.0,
        );
        assert_eq!(
            res.turns.len(),
            2,
            "expected 2 speaker turns, got {:?}",
            res.turns
        );
        assert_eq!(res.turns[0].speaker_idx, 0);
        assert_eq!(res.turns[1].speaker_idx, 1);
        // 声の小さい Host A には正のゲイン補正が推奨されること
        assert!(res.recommended_gains_db[0] > res.recommended_gains_db[1]);

        let before_lufs = measure_integrated_lufs(&host_a, sr);
        apply_podcast_voice_leveler(&mut host_a, sr, -16.0);
        let after_lufs = measure_integrated_lufs(&host_a, sr);
        assert!(
            after_lufs > before_lufs + 3.0,
            "voice leveler should boost quiet host: before={before_lufs} after={after_lufs}"
        );
    }
}
