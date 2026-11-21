use egui::{ColorImage, TextureHandle, TextureOptions, Ui};
use std::collections::HashMap;
use ve_core::{CommandSource, EditCommand, ItemId};
use ve_mcp::EditorSession;
use ve_render::TextRenderer;

const SAMPLE_W: u32 = 160;
const SAMPLE_H: u32 = 24;
const MAX_FONT_PREVIEWS: usize = 80;

/// フォント視覚化ピッカー＆お気に入りフォント管理（フォント視覚化MOD 相当）
#[derive(Default)]
pub struct FontPickerState {
    cached_fonts: Option<Vec<String>>,
    pub search_query: String,
    pub favorites_only: bool,
    pub sample_text: String,
    previews: HashMap<String, TextureHandle>,
}

impl FontPickerState {
    pub fn new() -> Self {
        Self {
            cached_fonts: None,
            search_query: String::new(),
            favorites_only: false,
            sample_text: "あいう ABC 123".to_string(),
            previews: HashMap::new(),
        }
    }

    pub fn ensure_fonts_loaded(&mut self, text_renderer: &TextRenderer) -> &[String] {
        if self.cached_fonts.is_none() {
            let mut list = vec!["sans-serif".to_string()];
            list.extend(text_renderer.list_system_fonts());
            self.cached_fonts = Some(list);
        }
        self.cached_fonts.as_deref().unwrap_or(&[])
    }

    /// インスペクタ内に書体サンプルプレビュー付きフォントピッカーを描画する
    pub fn show(
        &mut self,
        ui: &mut Ui,
        session: &mut EditorSession,
        text_renderer: &mut TextRenderer,
        item_id: ItemId,
        current_font: &str,
        current_size: f32,
    ) {
        self.ensure_fonts_loaded(text_renderer);
        let all_fonts = self.cached_fonts.clone().unwrap_or_default();
        let favorites = session.project.favorite_fonts.clone();

        ui.horizontal(|ui| {
            ui.label("🔍");
            ui.add(
                egui::TextEdit::singleline(&mut self.search_query)
                    .hint_text("フォント名検索...")
                    .desired_width(140.0),
            );
            ui.checkbox(&mut self.favorites_only, "★お気に入りのみ");
        });

        let query_lower = self.search_query.trim().to_lowercase();
        let mut filtered: Vec<String> = all_fonts
            .into_iter()
            .filter(|f| {
                let is_fav = favorites.iter().any(|x| x == f);
                if self.favorites_only && !is_fav {
                    return false;
                }
                if query_lower.is_empty() {
                    true
                } else {
                    f.to_lowercase().contains(&query_lower)
                }
            })
            .collect();

        // お気に入りフォントを先頭へピン留め表示
        filtered.sort_by(|a, b| {
            let fa = favorites.iter().any(|x| x == a);
            let fb = favorites.iter().any(|x| x == b);
            fb.cmp(&fa).then_with(|| a.cmp(b))
        });

        let mut picked_font: Option<String> = None;
        let mut toggled_fav: Option<String> = None;
        let sample = if self.sample_text.trim().is_empty() {
            "あいう ABC 123"
        } else {
            self.sample_text.as_str()
        };

        egui::ScrollArea::vertical()
            .id_salt("visual_font_picker_scroll")
            .max_height(180.0)
            .show(ui, |ui| {
                for font_name in filtered.iter().take(60) {
                    let is_fav = favorites.iter().any(|x| x == font_name);
                    let is_selected = current_font == font_name;

                    if !self.previews.contains_key(font_name) {
                        if self.previews.len() >= MAX_FONT_PREVIEWS {
                            self.previews.clear();
                        }
                        let rgba = text_renderer
                            .render_font_sample_rgba8(font_name, sample, SAMPLE_W, SAMPLE_H);
                        let img = ColorImage::from_rgba_unmultiplied(
                            [SAMPLE_W as usize, SAMPLE_H as usize],
                            &rgba,
                        );
                        let tex = ui.ctx().load_texture(
                            format!("font_preview_{font_name}"),
                            img,
                            TextureOptions::LINEAR,
                        );
                        self.previews.insert(font_name.clone(), tex);
                    }

                    ui.horizontal(|ui| {
                        let star = if is_fav { "★" } else { "☆" };
                        if ui.small_button(star).clicked() {
                            toggled_fav = Some(font_name.clone());
                        }
                        if ui.selectable_label(is_selected, font_name).clicked() {
                            picked_font = Some(font_name.clone());
                        }
                        if let Some(tex) = self.previews.get(font_name) {
                            let resp = ui.add(
                                egui::Image::new((
                                    tex.id(),
                                    egui::vec2(SAMPLE_W as f32 * 0.85, SAMPLE_H as f32 * 0.85),
                                ))
                                .sense(egui::Sense::click()),
                            );
                            if resp.clicked() {
                                picked_font = Some(font_name.clone());
                            }
                        }
                    });
                }
                if filtered.len() > 60 {
                    ui.weak(format!(
                        "ほか {} 件（検索欄で絞り込めます）",
                        filtered.len() - 60
                    ));
                }
            });

        if let Some(fav_font) = toggled_fav {
            let mut next = favorites;
            if let Some(pos) = next.iter().position(|x| x == &fav_font) {
                next.remove(pos);
            } else {
                next.push(fav_font);
            }
            let _ = session.bus.execute(
                &mut session.project,
                EditCommand::SetFavoriteFonts { fonts: next },
                CommandSource::Gui,
                None,
            );
        }

        if let Some(font_family) = picked_font {
            let _ = session.bus.execute(
                &mut session.project,
                EditCommand::SetTextFont {
                    item_id,
                    font_family,
                    font_size: current_size,
                },
                CommandSource::Gui,
                None,
            );
        }
    }
}
