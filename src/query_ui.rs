//! A query window: the editor, the results pane and its status bar.

use eframe::egui::{self, Align, Color32, Id, Key, Layout, RichText, Sense, TextStyle, text::CCursor, text_selection::CCursorRange};
use egui_extras::{Column, TableBuilder};

use crate::app::{App, Item, Outcome, QueryTab, ResultSet, ResultsMode, ResultsView};
use crate::theme::{self, Palette};
use crate::tsql;

enum FindAction {
    Next,
    Replace,
    ReplaceAll,
    Close,
}

impl App {
    pub(crate) fn query_pane(&mut self, ui: &mut egui::Ui, id: u64) {
        let p = theme::palette(self.settings.theme);
        let custom = self.tab(id).and_then(|t| self.status_color(t));
        let server = self.tab(id).and_then(|t| self.server_name(t));
        let mut export = None;
        let mut find_action = None;
        let Some(tab) = self.tabs.iter_mut().find(|t| t.id == id) else { return };

        egui::Panel::bottom(Id::unique(("status", id)))
            .exact_size(24.0)
            .frame(egui::Frame::new().fill(custom.unwrap_or(p.query_status)).inner_margin(egui::Margin::symmetric(6, 2)))
            .show(ui, |ui| status_bar(ui, tab, server.as_deref(), custom));

        if tab.show_results {
            egui::Panel::bottom(Id::unique(("results", id)))
                .resizable(true)
                .default_size(ui.available_height() * 0.45)
                .min_size(80.0)
                .show(ui, |ui| results_ui(ui, tab, &p, &mut export));
        }
        if tab.find.is_some() {
            egui::Panel::top(Id::unique(("find", id))).show(ui, |ui| find_action = find_bar(ui, tab));
        }
        egui::CentralPanel::no_frame().show(ui, |ui| editor_ui(ui, tab, &p));

        if let Some(csv) = export {
            self.export_csv(csv);
        }
        match find_action {
            Some(FindAction::Next) => self.find_next(),
            Some(FindAction::Replace) => self.replace_one(),
            Some(FindAction::ReplaceAll) => self.replace_all(),
            Some(FindAction::Close) => {
                if let Some(t) = self.tab_mut(id) {
                    t.find = None;
                    t.focus_editor = true;
                }
            }
            None => {}
        }
    }
}

fn find_bar(ui: &mut egui::Ui, tab: &mut QueryTab) -> Option<FindAction> {
    let mut action = None;
    let f = tab.find.as_mut()?;
    ui.add_space(2.0);
    ui.horizontal(|ui| {
        let r = ui.add(egui::TextEdit::singleline(&mut f.needle).hint_text("Find…").desired_width(220.0));
        if f.focus {
            r.request_focus();
            f.focus = false;
        }
        if r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
            action = Some(FindAction::Next);
            r.request_focus();
        }
        if ui.button("Find Next").on_hover_text("F3").clicked() {
            action = Some(FindAction::Next);
        }
        ui.toggle_value(&mut f.match_case, "Aa").on_hover_text("Match case");
        if f.replace.is_none() && ui.small_button("⇄").on_hover_text("Toggle replace (Ctrl+H)").clicked() {
            f.replace = Some(String::new());
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if ui.small_button("🗙").on_hover_text("Close (Esc)").clicked() {
                action = Some(FindAction::Close);
            }
        });
    });
    if let Some(rep) = &mut f.replace {
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(rep).hint_text("Replace…").desired_width(220.0));
            if ui.button("Replace").clicked() {
                action = Some(FindAction::Replace);
            }
            if ui.button("Replace All").clicked() {
                action = Some(FindAction::ReplaceAll);
            }
        });
    }
    if ui.input(|i| i.key_pressed(Key::Escape)) {
        action = Some(FindAction::Close);
    }
    ui.add_space(2.0);
    action
}

fn editor_ui(ui: &mut egui::Ui, tab: &mut QueryTab, p: &Palette) {
    let font = TextStyle::Monospace.resolve(ui.style());
    let id = tab.editor_id();
    if let Some((a, b)) = tab.set_selection.take() {
        let mut state = egui::text_edit::TextEditState::load(ui.ctx(), id).unwrap_or_default();
        state.cursor.set_char_range(Some(CCursorRange::two(CCursor::new(a), CCursor::new(b))));
        state.store(ui.ctx(), id);
        tab.selection = Some((a, b));
        tab.focus_editor = true;
        tab.scroll_to_cursor = true;
    }
    let palette = *p;
    let layout_font = font.clone();
    let mut layouter = move |ui: &egui::Ui, text: &dyn egui::TextBuffer, _wrap: f32| {
        let mut job = tsql::highlight(text.as_str(), &palette, layout_font.clone());
        job.wrap.max_width = f32::INFINITY;
        ui.fonts_mut(|f| f.layout_job(job))
    };
    let lines = tab.sql.matches('\n').count() + 1;
    egui::Frame::new().fill(p.editor_bg).show(ui, |ui| {
        ui.set_min_size(ui.available_size());
        egui::ScrollArea::both().id_salt(("editor", tab.id)).auto_shrink([false, false]).show(ui, |ui| {
            ui.horizontal_top(|ui| {
                let gutter: String = (1..=lines).map(|n| format!("{n:>5}\n")).collect();
                ui.add(egui::Label::new(RichText::new(gutter.trim_end()).font(font.clone()).color(p.gutter)).selectable(false).wrap_mode(egui::TextWrapMode::Extend));
                ui.add_space(6.0);
                let out = egui::TextEdit::multiline(&mut tab.sql)
                    .id(id)
                    .code_editor()
                    .frame(egui::Frame::NONE)
                    .background_color(p.editor_bg)
                    .desired_width(ui.available_width())
                    .desired_rows(30)
                    .lock_focus(true)
                    .layouter(&mut layouter)
                    .show(ui);
                if tab.focus_editor {
                    out.response.request_focus();
                    tab.focus_editor = false;
                }
                if let Some(range) = out.cursor_range {
                    let [a, b] = range.sorted_cursors();
                    tab.selection = Some((a.index.into(), b.index.into()));
                    if tab.scroll_to_cursor {
                        let rect = out.galley.pos_from_cursor(range.primary).translate(out.galley_pos.to_vec2());
                        ui.scroll_to_rect(rect.expand(30.0), Some(Align::Center));
                        tab.scroll_to_cursor = false;
                    }
                }
            });
        });
    });
}

fn status_bar(ui: &mut egui::Ui, tab: &QueryTab, server: Option<&str>, custom: Option<Color32>) {
    let fg = custom.map(theme::contrast).unwrap_or_else(|| ui.visuals().text_color());
    let ok = if custom.is_some() { fg } else { Color32::from_rgb(56, 163, 82) };
    let bad = if custom.is_some() { fg } else { Color32::from_rgb(214, 66, 56) };
    ui.horizontal_centered(|ui| {
        let (icon, text, color) = if tab.running {
            ("⏳", "Executing query…", fg)
        } else if let Some(e) = &tab.connect_error {
            ("⚠", e.lines().next().unwrap_or(""), bad)
        } else {
            match tab.outcome {
                Some(Outcome::Success) => ("✔", "Query executed successfully.", ok),
                Some(Outcome::Errors) => ("⚠", "Query completed with errors.", bad),
                Some(Outcome::Cancelled) => ("⏹", "Query was canceled by user.", bad),
                None if tab.session.is_none() => ("○", "Disconnected.", fg),
                None if tab.server_info.is_none() => ("…", "Connecting…", fg),
                None => ("●", "Connected. (1/1)", ok),
            }
        };
        ui.label(RichText::new(icon).color(color));
        ui.label(RichText::new(text).color(color));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let elapsed = if tab.running { tab.started.map(|s| s.elapsed()) } else { tab.elapsed };
            let rows = tab.total_rows();
            ui.label(RichText::new(format!("{rows} rows")).color(fg));
            ui.separator();
            let s = elapsed.map_or(0, |e| e.as_secs());
            ui.label(RichText::new(format!("{:02}:{:02}:{:02}", s / 3600, (s / 60) % 60, s % 60)).color(fg));
            ui.separator();
            if let Some(info) = &tab.server_info {
                ui.label(RichText::new(&tab.database).color(fg));
                ui.separator();
                ui.label(RichText::new(format!("{} ({})", info.login, info.spid)).color(fg));
                ui.separator();
                ui.label(RichText::new(format!("{} ({})", server.unwrap_or(&info.server_name), info.version)).color(fg)).on_hover_text(&info.edition);
            }
        });
    });
}

fn results_ui(ui: &mut egui::Ui, tab: &mut QueryTab, p: &Palette, export: &mut Option<String>) {
    ui.horizontal(|ui| {
        let label = match tab.mode {
            ResultsMode::Grid => "▦ Results",
            ResultsMode::Text => "≡ Results",
            ResultsMode::File => "🖹 Results",
        };
        if tab.mode != ResultsMode::File || !tab.results.is_empty() {
            ui.selectable_value(&mut tab.view, ResultsView::Results, label);
        }
        let errors = tab.messages.iter().any(|m| m.is_error);
        let msg = if errors { RichText::new("💬 Messages").color(Color32::from_rgb(214, 66, 56)) } else { RichText::new("💬 Messages") };
        ui.selectable_value(&mut tab.view, ResultsView::Messages, msg);
    });
    ui.separator();
    match tab.view {
        ResultsView::Messages => {
            egui::ScrollArea::both().id_salt(("msgs", tab.id)).auto_shrink([false, false]).stick_to_bottom(tab.running).show(ui, |ui| {
                for m in &tab.messages {
                    let text = RichText::new(&m.text).monospace();
                    let text = if m.is_error { text.color(Color32::from_rgb(214, 66, 56)) } else { text };
                    ui.add(egui::Label::new(text).selectable(true));
                }
            });
        }
        ResultsView::Results if tab.mode == ResultsMode::Grid => {
            let n = tab.results.len();
            if n == 0 {
                return;
            }
            // SSMS stacks every result set's grid, each with its own scroll bar.
            let each = ((ui.available_height() - 4.0 * n as f32) / n as f32).max(140.0);
            egui::ScrollArea::vertical().id_salt(("grids", tab.id)).auto_shrink([false, false]).show(ui, |ui| {
                for i in 0..n {
                    ui.allocate_ui(egui::vec2(ui.available_width(), each), |ui| {
                        ui.set_height(each);
                        grid_ui(ui, tab.id, i, &tab.results[i], &mut tab.cell, p, export);
                    });
                    if i + 1 < n {
                        ui.separator();
                    }
                }
            });
        }
        ResultsView::Results => {
            // Results to Text, with messages in the order they arrived.
            let mut text = String::new();
            for item in &tab.order {
                match *item {
                    Item::Set(i) => {
                        text.push_str(&tab.results[i].to_text());
                        text.push('\n');
                    }
                    Item::Message(i) => {
                        text.push_str(&tab.messages[i].text);
                        text.push('\n');
                    }
                }
            }
            egui::ScrollArea::both().id_salt(("text", tab.id)).auto_shrink([false, false]).show(ui, |ui| {
                ui.add(egui::Label::new(RichText::new(text).monospace()).selectable(true).wrap_mode(egui::TextWrapMode::Extend));
            });
        }
    }
}

fn grid_ui(ui: &mut egui::Ui, tab_id: u64, set_idx: usize, set: &ResultSet, cell: &mut Option<(usize, usize, usize)>, p: &Palette, export: &mut Option<String>) {
    let row_h = ui.text_style_height(&TextStyle::Monospace) + 6.0;
    let selected_bg = ui.visuals().selection.bg_fill;
    let current = cell.filter(|c| c.0 == set_idx);

    // Copy the selected cell with Ctrl+C when no text field has focus.
    if let Some((_, r, c)) = current {
        let copy = ui.input(|i| i.events.iter().any(|e| matches!(e, egui::Event::Copy)));
        if copy && ui.memory(|m| m.focused().is_none()) {
            if let Some(v) = set.rows.get(r).and_then(|row| row.get(c)) {
                ui.ctx().copy_text(v.clone().unwrap_or_else(|| "NULL".into()));
            }
        }
    }

    let mut clicked = None;
    let max_height = (ui.available_height() - row_h - 10.0).max(row_h);
    let mut table = TableBuilder::new(ui)
        .id_salt(("grid", tab_id, set_idx))
        .striped(true)
        .resizable(true)
        .auto_shrink([false, false])
        .min_scrolled_height(0.0)
        .max_scroll_height(max_height)
        .cell_layout(Layout::left_to_right(Align::Center))
        .column(Column::exact(52.0));
    for w in &set.widths {
        table = table.column(Column::initial(*w).at_least(30.0).clip(true).resizable(true));
    }
    table
        .header(row_h + 2.0, |mut header| {
            header.col(|ui| {
                ui.painter().rect_filled(ui.max_rect(), 0.0, p.grid_header);
            });
            for c in &set.columns {
                header.col(|ui| {
                    ui.painter().rect_filled(ui.max_rect(), 0.0, p.grid_header);
                    ui.label(&c.name).on_hover_text(c.type_name);
                });
            }
        })
        .body(|body| {
            body.rows(row_h, set.rows.len(), |mut row| {
                let r = row.index();
                row.col(|ui| {
                    ui.painter().rect_filled(ui.max_rect(), 0.0, p.grid_header);
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.weak(format!("{}", r + 1));
                    });
                });
                for (c, value) in set.rows[r].iter().enumerate() {
                    let numeric = set.columns[c].numeric;
                    let (_, resp) = row.col(|ui| {
                        if current.is_some_and(|x| x.1 == r && x.2 == c) {
                            ui.painter().rect_filled(ui.max_rect(), 0.0, selected_bg);
                        }
                        let layout = if numeric { Layout::right_to_left(Align::Center) } else { Layout::left_to_right(Align::Center) };
                        ui.with_layout(layout, |ui| match value {
                            None => {
                                ui.painter().rect_filled(ui.max_rect(), 0.0, p.null_bg);
                                ui.label(RichText::new("NULL").monospace());
                            }
                            Some(v) => {
                                let shown: String = v.chars().take(400).map(|ch| if matches!(ch, '\n' | '\r' | '\t') { ' ' } else { ch }).collect();
                                ui.add(egui::Label::new(RichText::new(shown).monospace()).truncate().selectable(false));
                            }
                        });
                    });
                    let resp = resp.interact(Sense::click());
                    if resp.clicked() || resp.secondary_clicked() {
                        clicked = Some((set_idx, r, c));
                    }
                    resp.context_menu(|ui| {
                        if ui.button("Copy").clicked() {
                            ui.ctx().copy_text(value.clone().unwrap_or_else(|| "NULL".into()));
                        }
                        if ui.button("Copy with Headers").clicked() {
                            ui.ctx().copy_text(format!("{}\n{}", set.columns[c].name, value.as_deref().unwrap_or("NULL")));
                        }
                        if ui.button("Copy Row").clicked() {
                            ui.ctx().copy_text(set.row_line(r, "\t"));
                        }
                        if ui.button("Copy Row with Headers").clicked() {
                            ui.ctx().copy_text(format!("{}\n{}", set.header_line("\t"), set.row_line(r, "\t")));
                        }
                        if ui.button("Copy Column").clicked() {
                            let col: Vec<_> = set.rows.iter().map(|row| row[c].as_deref().unwrap_or("NULL")).collect();
                            ui.ctx().copy_text(col.join("\n"));
                        }
                        if ui.button("Select All, Copy with Headers").clicked() {
                            let mut s = set.header_line("\t");
                            for i in 0..set.rows.len() {
                                s.push('\n');
                                s.push_str(&set.row_line(i, "\t"));
                            }
                            ui.ctx().copy_text(s);
                        }
                        ui.separator();
                        if ui.button("Save Results As…").clicked() {
                            *export = Some(set.to_csv());
                        }
                    });
                }
            });
        });
    if let Some(c) = clicked {
        *cell = Some(c);
        ui.memory_mut(|m| {
            if let Some(f) = m.focused() {
                m.surrender_focus(f);
            }
        });
    }
}
