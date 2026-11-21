use egui::{ImeEvent, Ui};

/// 日本語IME 入力・変換サポートおよび診断ログパネル
#[derive(Default)]
pub struct ImePocPanel {
    pub ime_event_log: Vec<String>,
}

impl ImePocPanel {
    pub fn new() -> Self {
        Self {
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
        ui.strong("📝 字幕テキスト編集（日本語IME・リアルタイム描画連動）");
        ui.add(
            egui::TextEdit::multiline(active_text_content)
                .desired_rows(3)
                .desired_width(f32::INFINITY),
        );

        ui.add_space(4.0);
        ui.collapsing(
            format!("🔍 日本語IME 診断ログ ({}件)", self.ime_event_log.len()),
            |ui| {
                ui.horizontal(|ui| {
                    ui.weak("変換候補・Preedit・確定イベントの診断表示");
                    if ui.small_button("クリア").clicked() {
                        self.ime_event_log.clear();
                    }
                });
                egui::ScrollArea::vertical()
                    .max_height(100.0)
                    .show(ui, |ui| {
                        if self.ime_event_log.is_empty() {
                            ui.weak("（IMEイベント未検出）");
                        } else {
                            for line in &self.ime_event_log {
                                ui.monospace(line);
                            }
                        }
                    });
            },
        );
    }
}
