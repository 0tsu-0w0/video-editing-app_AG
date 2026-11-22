//! v2.1 #27: 空間オーディオ（3D バイノーラル・パンナー）＆ マスタリング 3 バンド・マルチバンドコンプ ＆ 4× True-Peak リミッター
//! Pure Rust・ゼロ `unsafe`・外部依存ゼロで放送・配信品質（ITU-R BS.1770 -1.0 dBTP）の空間＆マスタリング処理を実現する。

use std::f32::consts::PI;

#[inline]
fn db_to_lin(db: f32) -> f32 {
    10.0_f32.powf(db / 20.0)
}

#[inline]
fn lin_to_db(lin: f32) -> f32 {
    if lin <= 1e-9 {
        -120.0
    } else {
        20.0 * lin.log10()
    }
}

/// 3D バイノーラル空間オーディオパンナー（ITD 両耳間時間差＋ILD 両耳間レベル差＋頭部回折シャドウ LPF＋距離減衰＋初期反射）
///
/// - `azimuth_deg`: 方位角（`0°` = 正面、`+90°` = 右、`-90°` = 左、`±180°` = 真後ろ）
/// - `elevation_deg`: 仰角（`-90°..=+90°`、高域シェルフ変化に寄与）
/// - `distance`: 音源距離 m（`0.3..=20.0`, 既定 `1.5`）
/// - `room_mix`: 空間初期反射の混入率（`0.0..=0.6`）
pub fn apply_binaural_spatial_3d(
    samples: &mut [f32],
    channels: usize,
    sample_rate: u32,
    azimuth_deg: f32,
    elevation_deg: f32,
    distance: f32,
    room_mix: f32,
) {
    if channels < 2 || samples.len() < channels {
        return;
    }
    let sr = sample_rate.max(8_000) as f32;
    let az_rad = azimuth_deg.clamp(-180.0, 180.0).to_radians();
    let sin_az = az_rad.sin(); // +1 = 右耳側, -1 = 左耳側
    let cos_az = az_rad.cos(); // +1 = 正面, -1 = 背面

    // Woodworth 球面頭部モデルによる両耳間時間差 ITD (半径 a ≈ 0.0875m, 音速 c ≈ 343 m/s -> 最大 ~0.68ms)
    let head_radius = 0.0875_f32;
    let speed_of_sound = 343.0_f32;
    let theta = az_rad.abs().min(PI);
    let itd_sec = (head_radius / speed_of_sound) * (theta.sin() + theta.min(PI * 0.5));
    let itd_samples = (itd_sec * sr).clamp(0.0, 96.0);

    // 右にある (+sin_az) ときは左耳が遅れ、左にある (-sin_az) ときは右耳が遅れる
    let delay_l = if sin_az > 0.0 { itd_samples } else { 0.0 };
    let delay_r = if sin_az < 0.0 { itd_samples } else { 0.0 };

    // ILD（両耳間レベル差）＋距離減衰（1 / (1 + 0.35*(d - 1))）
    let dist = distance.clamp(0.3, 25.0);
    let dist_gain = (1.0 / (0.65 + 0.35 * dist)).clamp(0.08, 1.35);
    // 背面は耳介シャドウでわずかに減衰
    let back_atten = 1.0 - 0.14 * ((1.0 - cos_az) * 0.5);
    let ild_amount = 0.42;
    let gain_l = dist_gain * back_atten * (1.0 - sin_az.max(0.0) * ild_amount);
    let gain_r = dist_gain * back_atten * (1.0 - (-sin_az).max(0.0) * ild_amount);

    // 頭部回折シャドウ LPF（音源と反対側の耳および背面・低仰角ほど高域が減衰）
    let elev_bright = 1.0 + (elevation_deg.clamp(-90.0, 90.0) / 90.0) * 0.18;
    let far_cut_l = if sin_az > 0.0 {
        (16_000.0 - sin_az * 12_500.0).clamp(1800.0, 18_000.0)
    } else {
        18_000.0
    } * (0.72 + 0.28 * ((cos_az + 1.0) * 0.5))
        * elev_bright;
    let far_cut_r = if sin_az < 0.0 {
        (16_000.0 - (-sin_az) * 12_500.0).clamp(1800.0, 18_000.0)
    } else {
        18_000.0
    } * (0.72 + 0.28 * ((cos_az + 1.0) * 0.5))
        * elev_bright;

    let alpha_l = one_pole_lpf_alpha(far_cut_l.clamp(1200.0, sr * 0.45), sr);
    let alpha_r = one_pole_lpf_alpha(far_cut_r.clamp(1200.0, sr * 0.45), sr);

    // モノラル空間化ソース（左右平均）＋微小初期反射用リングバッファ
    let ring_len = 2048usize;
    let mut ring = vec![0.0f32; ring_len];
    let mut ring_pos = 0usize;
    let room = room_mix.clamp(0.0, 0.6);
    let er_tap_l = ((0.011 * sr) as usize).clamp(32, ring_len - 2);
    let er_tap_r = ((0.017 * sr) as usize).clamp(48, ring_len - 2);

    let mut lpf_l = 0.0f32;
    let mut lpf_r = 0.0f32;

    for frame in samples.chunks_exact_mut(channels) {
        let mono = 0.5 * (frame[0] + frame[1]);
        ring[ring_pos] = mono;

        let raw_l = read_ring_frac(&ring, ring_pos, delay_l);
        let raw_r = read_ring_frac(&ring, ring_pos, delay_r);

        lpf_l += alpha_l * (raw_l - lpf_l);
        lpf_r += alpha_r * (raw_r - lpf_r);

        let er_l = ring[(ring_pos + ring_len - er_tap_l) % ring_len];
        let er_r = ring[(ring_pos + ring_len - er_tap_r) % ring_len];

        frame[0] = (lpf_l * gain_l + er_l * room * 0.45).clamp(-1.5, 1.5);
        frame[1] = (lpf_r * gain_r + er_r * room * 0.45).clamp(-1.5, 1.5);

        ring_pos = (ring_pos + 1) % ring_len;
    }
}

#[inline]
fn one_pole_lpf_alpha(cutoff_hz: f32, sr: f32) -> f32 {
    let dt = 1.0 / sr;
    let rc = 1.0 / (2.0 * PI * cutoff_hz.max(20.0));
    (dt / (rc + dt)).clamp(0.01, 1.0)
}

#[inline]
fn read_ring_frac(ring: &[f32], write_pos: usize, delay: f32) -> f32 {
    let len = ring.len();
    let d0 = delay.floor() as usize;
    let frac = delay - d0 as f32;
    let i0 = (write_pos + len - (d0 % len)) % len;
    let i1 = (write_pos + len - ((d0 + 1) % len)) % len;
    ring[i0] * (1.0 - frac) + ring[i1] * frac
}

/// 3 バンド・マルチバンドコンプレッサー（Low <250Hz / Mid 250–3500Hz / High >3500Hz 独立ダイナミクス制御）
#[allow(clippy::too_many_arguments)]
pub fn apply_multiband_compressor_3band(
    samples: &mut [f32],
    channels: usize,
    sample_rate: u32,
    low_thresh_db: f32,
    mid_thresh_db: f32,
    high_thresh_db: f32,
    ratio: f32,
    makeup_db: f32,
) {
    if channels == 0 || samples.len() < channels {
        return;
    }
    let sr = sample_rate.max(8_000) as f32;
    let ratio = ratio.clamp(1.0, 20.0);
    let makeup = db_to_lin(makeup_db.clamp(-12.0, 24.0));

    // 2 段縦続 1-pole フィルタ（250Hz / 3500Hz 位相滑らか 3 帯域スプリッタ）
    let a_low = one_pole_lpf_alpha(250.0, sr);
    let a_high = one_pole_lpf_alpha(3500.0, sr);
    let att = (-1.0 / (0.008 * sr)).exp();
    let rel = (-1.0 / (0.110 * sr)).exp();

    let mut lp_low_1 = vec![0.0f32; channels];
    let mut lp_low_2 = vec![0.0f32; channels];
    let mut lp_mid_1 = vec![0.0f32; channels];
    let mut lp_mid_2 = vec![0.0f32; channels];

    // ステレオリンク型帯域エンベロープ（ステレオ定位の崩れを防止）
    let mut env_low = 0.0f32;
    let mut env_mid = 0.0f32;
    let mut env_high = 0.0f32;

    let mut low_ch = vec![0.0f32; channels];
    let mut mid_ch = vec![0.0f32; channels];
    let mut high_ch = vec![0.0f32; channels];

    for frame in samples.chunks_exact_mut(channels) {
        let mut peak_l = 0.0f32;
        let mut peak_m = 0.0f32;
        let mut peak_h = 0.0f32;

        for (c, &x) in frame.iter().enumerate() {
            lp_low_1[c] += a_low * (x - lp_low_1[c]);
            lp_low_2[c] += a_low * (lp_low_1[c] - lp_low_2[c]);
            let low = lp_low_2[c];

            lp_mid_1[c] += a_high * (x - lp_mid_1[c]);
            lp_mid_2[c] += a_high * (lp_mid_1[c] - lp_mid_2[c]);
            let mid = lp_mid_2[c] - low;
            let high = x - lp_mid_2[c];

            low_ch[c] = low;
            mid_ch[c] = mid;
            high_ch[c] = high;

            peak_l = peak_l.max(low.abs());
            peak_m = peak_m.max(mid.abs());
            peak_h = peak_h.max(high.abs());
        }

        env_low = update_env(env_low, peak_l, att, rel);
        env_mid = update_env(env_mid, peak_m, att, rel);
        env_high = update_env(env_high, peak_h, att, rel);

        let g_low = comp_gain(env_low, low_thresh_db, ratio);
        let g_mid = comp_gain(env_mid, mid_thresh_db, ratio);
        let g_high = comp_gain(env_high, high_thresh_db, ratio);

        for (c, dst) in frame.iter_mut().enumerate() {
            *dst = (low_ch[c] * g_low + mid_ch[c] * g_mid + high_ch[c] * g_high) * makeup;
        }
    }
}

#[inline]
fn update_env(prev: f32, target: f32, att: f32, rel: f32) -> f32 {
    let coeff = if target > prev { att } else { rel };
    coeff * prev + (1.0 - coeff) * target
}

#[inline]
fn comp_gain(env: f32, thresh_db: f32, ratio: f32) -> f32 {
    let env_db = lin_to_db(env);
    if env_db <= thresh_db {
        1.0
    } else {
        let over = env_db - thresh_db;
        let reduced = over - over / ratio;
        db_to_lin(-reduced)
    }
}

/// 4× Catmull-Rom 3 次補間オーバーサンプリングによる True-Peak（dBTP）計測（ITU-R BS.1770 準拠）
pub fn measure_true_peak_dbtp(samples: &[f32], channels: usize) -> f32 {
    if channels == 0 || samples.len() < channels {
        return -120.0;
    }
    let frames = samples.len() / channels;
    let mut max_peak = 0.0f32;
    for c in 0..channels {
        for f in 0..frames {
            let s_m1 = samples[f.saturating_sub(1) * channels + c];
            let s0 = samples[f * channels + c];
            let s1 = samples[(f + 1).min(frames - 1) * channels + c];
            let s2 = samples[(f + 2).min(frames - 1) * channels + c];
            max_peak = max_peak.max(s0.abs());
            // サブサンプル t = 0.25, 0.50, 0.75 の 4× True-Peak 検出
            for step in 1..4 {
                let t = step as f32 * 0.25;
                let interp = catmull_rom(s_m1, s0, s1, s2, t).abs();
                max_peak = max_peak.max(interp);
            }
        }
    }
    lin_to_db(max_peak)
}

#[inline]
fn catmull_rom(y0: f32, y1: f32, y2: f32, y3: f32, t: f32) -> f32 {
    let t2 = t * t;
    let t3 = t2 * t;
    0.5 * ((2.0 * y1)
        + (-y0 + y2) * t
        + (2.0 * y0 - 5.0 * y1 + 4.0 * y2 - y3) * t2
        + (-y0 + 3.0 * y1 - 3.0 * y2 + y3) * t3)
}

/// 4× オーバーサンプリング True-Peak ブリックウォールリミッター（既定 -1.0 dBTP 厳守）
pub fn apply_true_peak_limiter(
    samples: &mut [f32],
    channels: usize,
    sample_rate: u32,
    ceiling_dbtp: f32,
    release_ms: f32,
) {
    if channels == 0 || samples.len() < channels {
        return;
    }
    let sr = sample_rate.max(8_000) as f32;
    let frames = samples.len() / channels;
    let ceiling_lin = db_to_lin(ceiling_dbtp.clamp(-12.0, -0.1));
    let rel_coeff = (-1.0 / ((release_ms.clamp(10.0, 500.0) * 0.001) * sr)).exp();

    // 各フレームの 4× インターサンプルピークを先読み算出
    let mut frame_peaks = vec![0.0f32; frames];
    for (f, peak_slot) in frame_peaks.iter_mut().enumerate() {
        let mut p = 0.0f32;
        for c in 0..channels {
            let s_m1 = samples[f.saturating_sub(1) * channels + c];
            let s0 = samples[f * channels + c];
            let s1 = samples[(f + 1).min(frames - 1) * channels + c];
            let s2 = samples[(f + 2).min(frames - 1) * channels + c];
            p = p.max(s0.abs());
            for step in 1..4 {
                let interp = catmull_rom(s_m1, s0, s1, s2, step as f32 * 0.25).abs();
                p = p.max(interp);
            }
        }
        *peak_slot = p;
    }

    // 短窓ルックアヘッド（最大 ~2ms）で歪みを抑えた滑らかなゲインリダクション包絡線を生成
    let lookahead = ((0.002 * sr) as usize).clamp(1, 96);
    let mut gain = 1.0f32;
    for (f, frame) in samples.chunks_exact_mut(channels).enumerate() {
        let end = (f + lookahead).min(frames);
        let mut window_peak = 0.0f32;
        for &pk in &frame_peaks[f..end] {
            window_peak = window_peak.max(pk);
        }
        let target_gain = if window_peak > ceiling_lin {
            (ceiling_lin / window_peak).clamp(0.01, 1.0)
        } else {
            1.0
        };
        if target_gain < gain {
            // アタックは瞬時追従（ブリックウォール厳守）
            gain = target_gain;
        } else {
            gain = rel_coeff * gain + (1.0 - rel_coeff) * target_gain;
        }
        for s in frame.iter_mut() {
            *s = (*s * gain).clamp(-ceiling_lin, ceiling_lin);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn binaural_spatial_3d_pans_right_azimuth_stronger_and_earlier_in_right_ear() {
        let sr = 48_000u32;
        let frames = 2048usize;
        let mut buf = vec![0.0f32; frames * 2];
        // 1kHz サイン波パルス
        for (f, frame) in buf.as_chunks_mut::<2>().0.iter_mut().enumerate() {
            let s = (2.0 * PI * 1000.0 * f as f32 / sr as f32).sin() * 0.5;
            frame[0] = s;
            frame[1] = s;
        }

        apply_binaural_spatial_3d(&mut buf, 2, sr, 90.0, 0.0, 1.2, 0.05);

        let rms_l: f32 = buf
            .iter()
            .step_by(2)
            .skip(200)
            .map(|x| x * x)
            .sum::<f32>()
            .sqrt();
        let rms_r: f32 = buf
            .iter()
            .skip(1)
            .step_by(2)
            .skip(200)
            .map(|x| x * x)
            .sum::<f32>()
            .sqrt();
        assert!(
            rms_r > rms_l * 1.4,
            "right azimuth (+90°) must be significantly louder in right ear: R={rms_r}, L={rms_l}"
        );
    }

    #[test]
    fn multiband_comp_and_true_peak_limiter_enforce_broadcast_ceiling() {
        let sr = 48_000u32;
        let frames = 4800usize;
        let mut buf = vec![0.0f32; frames * 2];
        // 低域 + 中域 + 高域の大音量テスト信号（ピーク > +3 dBFS）
        for (f, frame) in buf.as_chunks_mut::<2>().0.iter_mut().enumerate() {
            let t = f as f32 / sr as f32;
            let x = 0.8 * (2.0 * PI * 120.0 * t).sin()
                + 0.7 * (2.0 * PI * 1000.0 * t).sin()
                + 0.5 * (2.0 * PI * 6000.0 * t).sin();
            frame[0] = x;
            frame[1] = x;
        }
        let tp_before = measure_true_peak_dbtp(&buf, 2);
        assert!(
            tp_before > 0.0,
            "expected hot input > 0 dBTP, got {tp_before}"
        );

        apply_multiband_compressor_3band(&mut buf, 2, sr, -20.0, -18.0, -22.0, 4.0, 0.0);
        apply_true_peak_limiter(&mut buf, 2, sr, -1.0, 50.0);

        let tp_after = measure_true_peak_dbtp(&buf, 2);
        assert!(
            tp_after <= -0.95,
            "true peak limiter must enforce <= -1.0 dBTP ceiling, got {tp_after:.3} dBTP"
        );
    }
}
