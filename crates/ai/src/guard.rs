use crate::credentials::ApiProvider;
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum GuardError {
    #[error(
        "外部API送信にはユーザーの明示確認が必要です（対象: {provider}, 尺: {duration_sec:.1}秒, 概算コスト: ${estimated_usd:.4}）"
    )]
    ConfirmationRequired {
        provider: &'static str,
        duration_sec: f64,
        estimated_usd: f64,
    },
    #[error("音声尺 ({duration_sec:.1}秒) が安全上限 ({max_duration_sec:.1}秒) を超えています")]
    DurationLimitExceeded {
        duration_sec: f64,
        max_duration_sec: f64,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CloudCostEstimate {
    pub provider: String,
    pub duration_sec: f64,
    pub estimated_tokens: u64,
    pub estimated_usd: f64,
    pub requires_warning: bool,
    pub summary_message: String,
}

/// 外部 AI API（Gemini / Claude）への長尺音声誤送信・課金防止ガード（仕様書 2.3 A）
#[derive(Debug, Clone)]
pub struct CloudUploadGuard {
    /// 1回のAPI呼び出しで許可する最大音声尺（秒、既定 30分）
    pub max_duration_sec: f64,
    /// 長尺警告を出すしきい値（秒、既定 5分）
    pub warning_duration_sec: f64,
}

impl Default for CloudUploadGuard {
    fn default() -> Self {
        Self {
            max_duration_sec: 1800.0,
            warning_duration_sec: 300.0,
        }
    }
}

impl CloudUploadGuard {
    /// 音声尺またはテキスト長から概算トークン・概算コストを算出する
    pub fn estimate_audio_transcription(
        &self,
        provider: ApiProvider,
        duration_sec: f64,
    ) -> CloudCostEstimate {
        let duration_sec = if duration_sec.is_finite() {
            duration_sec.max(0.0)
        } else {
            0.0
        };
        // Gemini 音声入力目安: ~32 audio tokens/sec, ~$0.0005 / min オーダー
        // Claude 字幕補正目安: ~5 文字/sec -> ~4 tokens/sec
        let (tokens, usd) = match provider {
            ApiProvider::Gemini => {
                let t = (duration_sec * 32.0).ceil() as u64;
                let cost = (t as f64 / 1_000_000.0) * 1.0 + 0.0002;
                (t, cost)
            }
            ApiProvider::Claude => {
                let t = (duration_sec * 6.0).ceil() as u64 + 500;
                let cost = (t as f64 / 1_000_000.0) * 15.0;
                (t, cost)
            }
        };
        let requires_warning = duration_sec >= self.warning_duration_sec;
        let summary_message = format!(
            "{} へ {:.1}秒 のデータを送信します（概算 {} tokens / 約 ${:.4} USD）{}",
            provider.display_name(),
            duration_sec,
            tokens,
            usd,
            if requires_warning {
                " [長尺注意]"
            } else {
                ""
            }
        );
        CloudCostEstimate {
            provider: provider.display_name().to_string(),
            duration_sec,
            estimated_tokens: tokens,
            estimated_usd: usd,
            requires_warning,
            summary_message,
        }
    }

    /// 送信直前の安全検証: 有限値・尺上限チェックとユーザーの明示確認フラグを強制する
    pub fn verify_before_send(
        &self,
        provider: ApiProvider,
        duration_sec: f64,
        user_confirmed: bool,
    ) -> Result<CloudCostEstimate, GuardError> {
        if !duration_sec.is_finite() || duration_sec < 0.0 || duration_sec > self.max_duration_sec {
            return Err(GuardError::DurationLimitExceeded {
                duration_sec,
                max_duration_sec: self.max_duration_sec,
            });
        }
        let estimate = self.estimate_audio_transcription(provider, duration_sec);
        if !user_confirmed {
            return Err(GuardError::ConfirmationRequired {
                provider: provider.display_name(),
                duration_sec,
                estimated_usd: estimate.estimated_usd,
            });
        }
        Ok(estimate)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_unconfirmed_or_excessive_cloud_uploads() {
        let guard = CloudUploadGuard::default();

        // 未確認はブロック
        let err = guard
            .verify_before_send(ApiProvider::Gemini, 60.0, false)
            .unwrap_err();
        assert!(matches!(err, GuardError::ConfirmationRequired { .. }));

        // 上限超過・NaN・無限大・負値は確認済みでもブロック
        for bad in [3600.0, f64::NAN, f64::INFINITY, -1.0] {
            let err = guard
                .verify_before_send(ApiProvider::Gemini, bad, true)
                .unwrap_err();
            assert!(matches!(err, GuardError::DurationLimitExceeded { .. }));
        }

        // 確認済みかつ範囲内は許可
        let est = guard
            .verify_before_send(ApiProvider::Gemini, 120.0, true)
            .unwrap();
        assert!(est.estimated_usd > 0.0);
        assert!(!est.requires_warning);
    }
}
