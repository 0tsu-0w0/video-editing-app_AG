//! v6.0 #41: データ駆動モーショングラフィックス＆インフォグラフィック・チャートエンジン
//! CSV/JSON データからアニメーション棒グラフ・折れ線・ドーナツ・KPI カウンターを
//! 決定論イージングで自動描画する設定・パーサを提供する。

use crate::project::RgbaColor;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChartKind {
    #[default]
    BarChart,
    LineChart,
    DonutChart,
    KpiCounter,
}

impl ChartKind {
    pub const ALL: &'static [Self] = &[
        Self::BarChart,
        Self::LineChart,
        Self::DonutChart,
        Self::KpiCounter,
    ];

    pub fn display_name(self) -> &'static str {
        match self {
            Self::BarChart => "📊 アニメーション棒グラフ (Bar Chart)",
            Self::LineChart => "📈 トレンド折れ線＆エリア (Line Chart)",
            Self::DonutChart => "🍩 ドーナツ割合チャート (Donut Chart)",
            Self::KpiCounter => "🔢 KPI ロールアップカウンター (KPI Counter)",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DataSeriesPoint {
    pub label: String,
    pub value: f64,
    #[serde(default)]
    pub color: Option<RgbaColor>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DataChartConfig {
    #[serde(default)]
    pub kind: ChartKind,
    #[serde(default = "default_chart_title")]
    pub title: String,
    #[serde(default)]
    pub unit_suffix: String,
    #[serde(default = "default_chart_points")]
    pub points: Vec<DataSeriesPoint>,
    /// 0% → 100% 立ち上がりアニメーション秒数
    #[serde(default = "default_animate_in")]
    pub animate_in_sec: f32,
    #[serde(default = "default_chart_w")]
    pub width: f32,
    #[serde(default = "default_chart_h")]
    pub height: f32,
    #[serde(default = "default_chart_bg")]
    pub bg_color: RgbaColor,
    #[serde(default = "default_chart_accent")]
    pub accent_color: RgbaColor,
    #[serde(default = "default_true")]
    pub show_grid: bool,
    #[serde(default = "default_true")]
    pub show_values: bool,
}

fn default_chart_title() -> String {
    "GROWTH METRICS".into()
}
fn default_animate_in() -> f32 {
    1.2
}
fn default_chart_w() -> f32 {
    640.0
}
fn default_chart_h() -> f32 {
    360.0
}
fn default_true() -> bool {
    true
}
fn default_chart_bg() -> RgbaColor {
    RgbaColor {
        r: 0.06,
        g: 0.08,
        b: 0.14,
        a: 0.90,
    }
}
fn default_chart_accent() -> RgbaColor {
    RgbaColor {
        r: 0.22,
        g: 0.78,
        b: 0.98,
        a: 1.0,
    }
}

fn default_chart_points() -> Vec<DataSeriesPoint> {
    vec![
        DataSeriesPoint {
            label: "Q1".into(),
            value: 32.0,
            color: None,
        },
        DataSeriesPoint {
            label: "Q2".into(),
            value: 54.0,
            color: None,
        },
        DataSeriesPoint {
            label: "Q3".into(),
            value: 78.0,
            color: None,
        },
        DataSeriesPoint {
            label: "Q4".into(),
            value: 112.0,
            color: None,
        },
    ]
}

impl Default for DataChartConfig {
    fn default() -> Self {
        Self {
            kind: ChartKind::BarChart,
            title: default_chart_title(),
            unit_suffix: "%".into(),
            points: default_chart_points(),
            animate_in_sec: default_animate_in(),
            width: default_chart_w(),
            height: default_chart_h(),
            bg_color: default_chart_bg(),
            accent_color: default_chart_accent(),
            show_grid: true,
            show_values: true,
        }
    }
}

impl DataChartConfig {
    pub fn for_kind(kind: ChartKind) -> Self {
        match kind {
            ChartKind::BarChart => Self {
                kind,
                title: "QUARTERLY REVENUE".into(),
                unit_suffix: "M".into(),
                points: default_chart_points(),
                ..Self::default()
            },
            ChartKind::LineChart => Self {
                kind,
                title: "AUDIENCE RETENTION".into(),
                unit_suffix: "%".into(),
                points: vec![
                    DataSeriesPoint {
                        label: "00S".into(),
                        value: 100.0,
                        color: None,
                    },
                    DataSeriesPoint {
                        label: "15S".into(),
                        value: 84.0,
                        color: None,
                    },
                    DataSeriesPoint {
                        label: "30S".into(),
                        value: 79.0,
                        color: None,
                    },
                    DataSeriesPoint {
                        label: "45S".into(),
                        value: 88.0,
                        color: None,
                    },
                    DataSeriesPoint {
                        label: "60S".into(),
                        value: 92.0,
                        color: None,
                    },
                ],
                accent_color: RgbaColor {
                    r: 0.30,
                    g: 0.92,
                    b: 0.62,
                    a: 1.0,
                },
                ..Self::default()
            },
            ChartKind::DonutChart => Self {
                kind,
                title: "PLATFORM SHARE".into(),
                unit_suffix: "%".into(),
                points: vec![
                    DataSeriesPoint {
                        label: "YT".into(),
                        value: 46.0,
                        color: None,
                    },
                    DataSeriesPoint {
                        label: "SHORTS".into(),
                        value: 31.0,
                        color: None,
                    },
                    DataSeriesPoint {
                        label: "X".into(),
                        value: 15.0,
                        color: None,
                    },
                    DataSeriesPoint {
                        label: "WEB".into(),
                        value: 8.0,
                        color: None,
                    },
                ],
                ..Self::default()
            },
            ChartKind::KpiCounter => Self {
                kind,
                title: "SUBSCRIBERS".into(),
                unit_suffix: "K".into(),
                points: vec![DataSeriesPoint {
                    label: "TOTAL".into(),
                    value: 250.0,
                    color: None,
                }],
                accent_color: RgbaColor {
                    r: 1.0,
                    g: 0.78,
                    b: 0.24,
                    a: 1.0,
                },
                ..Self::default()
            },
        }
    }
}

/// CSV テキスト（`label,value[,#RRGGBB]` 形式、ヘッダ行自動スキップ）からチャートデータ列をパースする
pub fn parse_chart_csv(csv_text: &str) -> Vec<DataSeriesPoint> {
    let mut out = Vec::new();
    for raw_line in csv_text.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let cols: Vec<&str> = line
            .split(',')
            .map(|c| c.trim().trim_matches('"'))
            .collect();
        if cols.len() < 2 {
            continue;
        }
        let Ok(val) = cols[1].parse::<f64>() else {
            // ヘッダ行（例: "label,value"）をスキップ
            continue;
        };
        if !val.is_finite() {
            continue;
        }
        let color = cols.get(2).and_then(|hex| parse_hex_rgba(hex));
        out.push(DataSeriesPoint {
            label: cols[0].to_string(),
            value: val,
            color,
        });
    }
    out
}

fn parse_hex_rgba(raw: &str) -> Option<RgbaColor> {
    let hex = raw.trim().strip_prefix('#').unwrap_or(raw.trim());
    if hex.len() != 6 {
        return None;
    }
    let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
    let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
    let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
    Some(RgbaColor {
        r: f32::from(r) / 255.0,
        g: f32::from(g) / 255.0,
        b: f32::from(b) / 255.0,
        a: 1.0,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_csv_with_header_and_optional_hex_colors() {
        let csv = "label,value,color\nJan,42.5,#ff4488\nFeb,68.0\nMar,95.2,#22ccff\n";
        let pts = parse_chart_csv(csv);
        assert_eq!(pts.len(), 3);
        assert_eq!(pts[0].label, "Jan");
        assert!((pts[0].value - 42.5).abs() < 1e-6);
        assert!(pts[0].color.is_some());
        assert!(pts[1].color.is_none());
        assert!(pts[2].color.is_some());
    }
}
