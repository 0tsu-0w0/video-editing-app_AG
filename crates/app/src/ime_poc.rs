use egui::{ImeEvent, Ui};

/// Phase 1a 非機能要件: `egui` の日本語IME入力・変換挙動を検証するための PoC パネル
#[derive(Default)]
pub struct ImePocPanel {
    pub single_line_sample: String,
    pub ime_event_log: Vec<String>,
}

impl ImePocPanel {
    pub fn new() -> Self {
        Self {
            single_line_sample: "日本語IMEテスト（変換候補位置・確定確認）".to_string(),
            ime_event_log: Vec::new(),
        }
    }

    pub fn capture_events(&mut self, ctx: &egui::Context) {
        ctx.input(|i| {
            for ev in &i.events {
                if let egui::Event::Ime(ime) = ev {
                    let msg = match ime {
                        ImeEvent::Preedit {
                            text,
                            active_range_chars,
                        } => format!("IME: Preedit({text:?}, active={active_range_chars:?})"),
                        ImeEvent::Commit(s) => format!("IME: Commit({s:?})"),
                        other => format!("IME: {other:?}"),
                    };
                    self.ime_event_log.push(msg);
                    if self.ime_event_log.len() > 20 {
                        self.ime_event_log.remove(0);
                    }
                }
            }
        });
    }

    pub fn ui(&mut self, ui: &mut Ui, active_text_content: &mut String) {
        ui.heading("日本語IME PoC 検証 (Phase 1a)");
        ui.label("下のテキスト欄で日本語変換を行い、変換候補位置・Preedit・確定が正常に動作するか確認します。");
        ui.separator();

        ui.label("1行入力テスト:");
        ui.text_edit_singleline(&mut self.single_line_sample);

        ui.add_space(6.0);
        ui.label("字幕テキストオブジェクト（リアルタイム cosmic-text 描画連動）:");
        ui.text_edit_multiline(active_text_content);

        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.label("直近の IME イベントログ:");
            if ui.button("クリア").clicked() {
                self.ime_event_log.clear();
            }
        });
        egui::ScrollArea::vertical()
            .max_height(120.0)
            .show(ui, |ui| {
                if self.ime_event_log.is_empty() {
                    ui.weak("（IMEイベント未検出）");
                } else {
                    for line in &self.ime_event_log {
                        ui.monospace(line);
                    }
                }
            });
    }
}
