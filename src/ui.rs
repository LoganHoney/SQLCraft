//! The SSMS-style shell: menu bar, toolbars, docked tool windows and the
//! status bar.

use std::time::{Duration, Instant};

use eframe::egui::{
    self, Align, Color32, Id, Key, KeyboardShortcut, Layout, Modifiers, RichText, collapsing_header::CollapsingState,
};
use egui_dock::{DockArea, DockState, TabViewer, tab_viewer::OnCloseResponse};

use crate::app::{Action, App, Cmd, Confirm, Pane, ResultsMode, ScriptKind, ServerState, node_ui_id};
use crate::explorer::{Kind, Node};
use crate::format::qi;
use crate::templates::TEMPLATES;
use crate::theme;

/// Keyboard shortcuts, matching SSMS's defaults.
const KEYS: &[(Modifiers, Key, Cmd)] = &[
    (Modifiers::CTRL, Key::N, Cmd::NewQuery),
    (Modifiers::CTRL, Key::O, Cmd::OpenFile),
    (Modifiers::CTRL.plus(Modifiers::SHIFT), Key::S, Cmd::SaveAll),
    (Modifiers::CTRL, Key::S, Cmd::Save),
    (Modifiers::CTRL, Key::F4, Cmd::Close),
    (Modifiers::CTRL, Key::F5, Cmd::Parse),
    (Modifiers::NONE, Key::F5, Cmd::Execute),
    (Modifiers::CTRL, Key::E, Cmd::Execute),
    (Modifiers::ALT, Key::Pause, Cmd::Cancel),
    (Modifiers::CTRL, Key::R, Cmd::ToggleResults),
    (Modifiers::CTRL, Key::D, Cmd::ResultsToGrid),
    (Modifiers::CTRL, Key::T, Cmd::ResultsToText),
    (Modifiers::CTRL.plus(Modifiers::SHIFT), Key::F, Cmd::ResultsToFile),
    (Modifiers::CTRL.plus(Modifiers::SHIFT), Key::M, Cmd::TemplateParams),
    (Modifiers::CTRL.plus(Modifiers::SHIFT), Key::U, Cmd::Upper),
    (Modifiers::CTRL.plus(Modifiers::SHIFT), Key::L, Cmd::Lower),
    (Modifiers::CTRL, Key::F, Cmd::Find),
    (Modifiers::CTRL, Key::H, Cmd::Replace),
    (Modifiers::NONE, Key::F3, Cmd::FindNext),
    (Modifiers::CTRL, Key::G, Cmd::GoTo),
    (Modifiers::NONE, Key::F8, Cmd::ShowObjectExplorer),
    (Modifiers::NONE, Key::F4, Cmd::ShowProperties),
    (Modifiers::CTRL.plus(Modifiers::ALT), Key::T, Cmd::ShowTemplates),
    (Modifiers::CTRL.plus(Modifiers::ALT), Key::O, Cmd::ShowOutput),
    (Modifiers::SHIFT.plus(Modifiers::ALT), Key::Enter, Cmd::FullScreen),
    (Modifiers::ALT, Key::F1, Cmd::SpHelp),
    (Modifiers::CTRL, Key::Num1, Cmd::SpWho),
    (Modifiers::CTRL, Key::Num2, Cmd::SpLock),
    (Modifiers::CTRL, Key::Tab, Cmd::NextDocument),
];

/// The shortcut shown next to a menu item.
fn shortcut_text(cmd: Cmd) -> &'static str {
    match cmd {
        Cmd::NewQuery => "Ctrl+N",
        Cmd::OpenFile => "Ctrl+O",
        Cmd::Save => "Ctrl+S",
        Cmd::SaveAll => "Ctrl+Shift+S",
        Cmd::Close => "Ctrl+F4",
        Cmd::Exit => "Alt+F4",
        Cmd::Find => "Ctrl+F",
        Cmd::Replace => "Ctrl+H",
        Cmd::FindNext => "F3",
        Cmd::GoTo => "Ctrl+G",
        Cmd::Comment => "Ctrl+K, Ctrl+C",
        Cmd::Uncomment => "Ctrl+K, Ctrl+U",
        Cmd::Upper => "Ctrl+Shift+U",
        Cmd::Lower => "Ctrl+Shift+L",
        Cmd::Indent => "Tab",
        Cmd::Unindent => "Shift+Tab",
        Cmd::ShowObjectExplorer => "F8",
        Cmd::ShowProperties => "F4",
        Cmd::ShowOutput => "Ctrl+Alt+O",
        Cmd::ShowTemplates => "Ctrl+Alt+T",
        Cmd::FullScreen => "Shift+Alt+Enter",
        Cmd::Execute => "F5",
        Cmd::Parse => "Ctrl+F5",
        Cmd::Cancel => "Alt+Break",
        Cmd::ResultsToGrid => "Ctrl+D",
        Cmd::ResultsToText => "Ctrl+T",
        Cmd::ResultsToFile => "Ctrl+Shift+F",
        Cmd::ToggleResults => "Ctrl+R",
        Cmd::TemplateParams => "Ctrl+Shift+M",
        Cmd::SpHelp => "Alt+F1",
        Cmd::SpWho => "Ctrl+1",
        Cmd::SpLock => "Ctrl+2",
        Cmd::NextDocument => "Ctrl+Tab",
        _ => "",
    }
}

fn item(ui: &mut egui::Ui, label: &str, cmd: Cmd, enabled: bool, cmds: &mut Vec<Cmd>) {
    let b = egui::Button::new(label).shortcut_text(shortcut_text(cmd));
    if ui.add_enabled(enabled, b).clicked() {
        cmds.push(cmd);
        ui.close();
    }
}

fn check_item(ui: &mut egui::Ui, label: &str, cmd: Cmd, checked: bool, enabled: bool, cmds: &mut Vec<Cmd>) {
    let b = egui::Button::new(label).selected(checked).shortcut_text(shortcut_text(cmd));
    if ui.add_enabled(enabled, b).clicked() {
        cmds.push(cmd);
        ui.close();
    }
}

/// A greyed-out entry for SSMS features that are not built yet.
fn later(ui: &mut egui::Ui, label: &str, shortcut: &str) {
    ui.add_enabled(false, egui::Button::new(label).shortcut_text(shortcut)).on_disabled_hover_text("Planned for a later SQLCraft release");
}

impl App {
    pub(crate) fn frame(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        self.pump(&ctx);
        let mut cmds = Vec::new();
        self.shortcuts(&ctx, &mut cmds);

        if ctx.input(|i| i.viewport().close_requested()) && !self.allow_quit {
            if self.tabs.iter().any(|t| t.dirty() && !t.sql.trim().is_empty() && t.path.is_some()) {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                self.confirm = Some(Confirm::Quit);
            }
        }

        let title = match self.current_tab() {
            Some(t) => format!("{} - SQLCraft", t.caption(self.server_name(t).as_deref())),
            None => "SQLCraft".to_string(),
        };
        ctx.send_viewport_cmd(egui::ViewportCommand::Title(title));

        egui::Panel::top("menu").show(ui, |ui| {
            self.menu_bar(ui, &mut cmds);
            self.toolbars(ui, &mut cmds);
            ui.add_space(2.0);
        });
        self.main_status_bar(ui);

        let mut dock = std::mem::replace(&mut self.dock, DockState::new(Vec::new()));
        let mut viewer = Viewer { app: self, actions: Vec::new(), cmds: Vec::new() };
        let mut style = egui_dock::Style::from_egui(ui.style());
        let p = theme::palette(viewer.app.settings.theme);
        style.tab_bar.bg_fill = ui.visuals().panel_fill;
        style.tab.active.bg_fill = p.editor_bg;
        style.tab.focused.bg_fill = p.accent;
        style.tab.focused.text_color = Color32::WHITE;
        style.tab.tab_body.bg_fill = ui.visuals().panel_fill;
        egui::CentralPanel::no_frame().show(ui, |ui| {
            DockArea::new(&mut dock)
                .id(Id::unique("dock"))
                .style(style)
                .show_leaf_collapse_buttons(false)
                .show_leaf_close_all_buttons(false)
                .show_inside(ui, &mut viewer);
        });
        let Viewer { actions, cmds: pane_cmds, .. } = viewer;
        self.dock = dock;
        cmds.extend(pane_cmds);
        for a in actions {
            self.apply(&ctx, a);
        }
        for c in cmds {
            self.command(&ctx, c);
        }
        self.tidy_dock();

        self.connect_dialog(&ctx);
        self.confirm_dialog(&ctx);
        self.about_dialog(&ctx);
        self.options_dialog(&ctx);
        self.goto_dialog(&ctx);
        self.template_params_dialog(&ctx);
        self.toast_ui(&ctx);

        if self.tabs.iter().any(|t| t.running) {
            ctx.request_repaint_after(Duration::from_millis(250));
        }
    }

    fn shortcuts(&mut self, ctx: &egui::Context, cmds: &mut Vec<Cmd>) {
        if self.connect.is_some() || self.confirm.is_some() || self.goto.is_some() || self.template_params.is_some() || self.show_options {
            return;
        }
        // Ctrl+K, Ctrl+C / Ctrl+K, Ctrl+U chords. Ctrl+C arrives as a Copy event.
        if ctx.input_mut(|i| i.consume_key(Modifiers::CTRL, Key::K)) {
            self.chord = Some(Instant::now());
        } else if let Some(t) = self.chord {
            if t.elapsed() > Duration::from_secs(3) {
                self.chord = None;
            } else {
                let copied = ctx.input_mut(|i| {
                    let had = i.events.iter().any(|e| matches!(e, egui::Event::Copy));
                    i.events.retain(|e| !matches!(e, egui::Event::Copy));
                    had
                });
                if copied {
                    cmds.push(Cmd::Comment);
                    self.chord = None;
                } else if ctx.input_mut(|i| i.consume_key(Modifiers::CTRL, Key::U)) {
                    cmds.push(Cmd::Uncomment);
                    self.chord = None;
                } else if ctx.input(|i| i.events.iter().any(|e| matches!(e, egui::Event::Key { pressed: true, .. }))) {
                    self.chord = None;
                }
            }
        }
        // Tab / Shift+Tab indent a multi-line selection instead of replacing it.
        if let Some(tab) = self.current_tab() {
            let focused = ctx.memory(|m| m.has_focus(tab.editor_id()));
            let multiline = tab.selection.is_some_and(|(a, b)| a != b && tab.sql.chars().skip(a).take(b - a).any(|c| c == '\n'));
            if focused && multiline {
                if ctx.input_mut(|i| i.consume_key(Modifiers::SHIFT, Key::Tab)) {
                    cmds.push(Cmd::Unindent);
                } else if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Tab)) {
                    cmds.push(Cmd::Indent);
                }
            } else if focused && ctx.input_mut(|i| i.consume_key(Modifiers::SHIFT, Key::Tab)) {
                cmds.push(Cmd::Unindent);
            }
        }
        for (m, k, cmd) in KEYS {
            if ctx.input_mut(|i| i.consume_shortcut(&KeyboardShortcut::new(*m, *k))) {
                cmds.push(*cmd);
            }
        }
    }

    fn menu_bar(&mut self, ui: &mut egui::Ui, cmds: &mut Vec<Cmd>) {
        let has_tab = self.current.is_some();
        let (mode, show_results, running) =
            self.current_tab().map_or((ResultsMode::Grid, false, false), |t| (t.mode, t.show_results, t.running));
        egui::MenuBar::new().ui(ui, |ui| {
            ui.menu_button("File", |ui| {
                ui.menu_button("New", |ui| {
                    item(ui, "Query with Current Connection", Cmd::NewQuery, true, cmds);
                    item(ui, "Database Engine Query", Cmd::ConnectObjectExplorer, true, cmds);
                });
                ui.menu_button("Open", |ui| {
                    item(ui, "File…", Cmd::OpenFile, true, cmds);
                });
                ui.separator();
                item(ui, "Connect Object Explorer…", Cmd::ConnectObjectExplorer, true, cmds);
                item(ui, "Disconnect Object Explorer", Cmd::ShowObjectExplorer, false, cmds);
                ui.separator();
                item(ui, "Close", Cmd::Close, has_tab, cmds);
                item(ui, "Save", Cmd::Save, has_tab, cmds);
                item(ui, "Save As…", Cmd::SaveAs, has_tab, cmds);
                item(ui, "Save All", Cmd::SaveAll, has_tab, cmds);
                ui.separator();
                let recent = self.settings.recent_files.clone();
                ui.menu_button("Recent Files", |ui| {
                    if recent.is_empty() {
                        ui.weak("(none)");
                    }
                    for (i, p) in recent.iter().enumerate() {
                        if ui.button(format!("{} {}", i + 1, p.display())).clicked() {
                            self.open_recent(p.clone());
                            ui.close();
                        }
                    }
                });
                ui.separator();
                item(ui, "Exit", Cmd::Exit, true, cmds);
            });
            ui.menu_button("Edit", |ui| {
                ui.menu_button("Find and Replace", |ui| {
                    item(ui, "Quick Find", Cmd::Find, has_tab, cmds);
                    item(ui, "Quick Replace", Cmd::Replace, has_tab, cmds);
                    item(ui, "Find Next", Cmd::FindNext, has_tab, cmds);
                });
                item(ui, "Go To…", Cmd::GoTo, has_tab, cmds);
                ui.separator();
                ui.menu_button("Advanced", |ui| {
                    item(ui, "Make Uppercase", Cmd::Upper, has_tab, cmds);
                    item(ui, "Make Lowercase", Cmd::Lower, has_tab, cmds);
                    ui.separator();
                    item(ui, "Increase Line Indent", Cmd::Indent, has_tab, cmds);
                    item(ui, "Decrease Line Indent", Cmd::Unindent, has_tab, cmds);
                    ui.separator();
                    item(ui, "Comment Selection", Cmd::Comment, has_tab, cmds);
                    item(ui, "Uncomment Selection", Cmd::Uncomment, has_tab, cmds);
                });
            });
            ui.menu_button("View", |ui| {
                item(ui, "Object Explorer", Cmd::ShowObjectExplorer, true, cmds);
                later(ui, "Object Explorer Details", "F7");
                later(ui, "Registered Servers", "Ctrl+Alt+G");
                item(ui, "Template Browser", Cmd::ShowTemplates, true, cmds);
                item(ui, "Properties Window", Cmd::ShowProperties, true, cmds);
                item(ui, "Output", Cmd::ShowOutput, true, cmds);
                ui.separator();
                item(ui, "Full Screen", Cmd::FullScreen, true, cmds);
            });
            if has_tab {
                ui.menu_button("Query", |ui| {
                    ui.menu_button("Connection", |ui| {
                        item(ui, "Change Connection…", Cmd::ChangeConnection, true, cmds);
                        item(ui, "Disconnect", Cmd::Disconnect, !running, cmds);
                    });
                    ui.separator();
                    item(ui, "Execute", Cmd::Execute, !running, cmds);
                    later(ui, "Display Estimated Execution Plan", "Ctrl+L");
                    item(ui, "Cancel Executing Query", Cmd::Cancel, running, cmds);
                    item(ui, "Parse", Cmd::Parse, !running, cmds);
                    ui.separator();
                    later(ui, "Include Actual Execution Plan", "Ctrl+M");
                    later(ui, "Include Client Statistics", "Shift+Alt+S");
                    ui.separator();
                    ui.menu_button("Results To", |ui| {
                        check_item(ui, "Results to Text", Cmd::ResultsToText, mode == ResultsMode::Text, true, cmds);
                        check_item(ui, "Results to Grid", Cmd::ResultsToGrid, mode == ResultsMode::Grid, true, cmds);
                        check_item(ui, "Results to File", Cmd::ResultsToFile, mode == ResultsMode::File, true, cmds);
                    });
                    check_item(ui, "Show Results Pane", Cmd::ToggleResults, show_results, true, cmds);
                    item(ui, "Save Results As…", Cmd::SaveResults, true, cmds);
                    ui.separator();
                    item(ui, "Specify Values for Template Parameters…", Cmd::TemplateParams, true, cmds);
                });
            }
            ui.menu_button("Tools", |ui| {
                later(ui, "SQL Server Profiler", "");
                later(ui, "Activity Monitor", "Ctrl+Alt+A");
                ui.separator();
                item(ui, "Options…", Cmd::Options, true, cmds);
            });
            ui.menu_button("Window", |ui| {
                item(ui, "Reset Window Layout", Cmd::ResetLayout, true, cmds);
                item(ui, "Close All Documents", Cmd::CloseAll, has_tab, cmds);
                item(ui, "Next Document", Cmd::NextDocument, has_tab, cmds);
                if !self.tabs.is_empty() {
                    ui.separator();
                }
                let docs: Vec<(u64, String)> = self.tabs.iter().map(|t| (t.id, t.caption(self.server_name(t).as_deref()))).collect();
                for (i, (id, caption)) in docs.into_iter().enumerate() {
                    if ui.add(egui::Button::new(format!("{} {caption}", i + 1)).selected(self.current == Some(id))).clicked() {
                        self.current = Some(id);
                        if let Some(path) = self.dock.find_tab(&Pane::Query(id)) {
                            let _ = self.dock.set_active_tab(path);
                        }
                        ui.close();
                    }
                }
            });
            ui.menu_button("Help", |ui| {
                item(ui, "Keyboard Shortcuts", Cmd::About, true, cmds);
                item(ui, "About SQLCraft", Cmd::About, true, cmds);
            });
        });
    }

    fn toolbars(&mut self, ui: &mut egui::Ui, cmds: &mut Vec<Cmd>) {
        let tb = |ui: &mut egui::Ui, text: &str, tip: &str, enabled: bool| ui.add_enabled(enabled, egui::Button::new(text).frame_when_inactive(false)).on_hover_text(tip).clicked();
        let has_tab = self.current.is_some();
        let (running, mode, server, db) = self.current_tab().map_or((false, ResultsMode::Grid, None, String::new()), |t| {
            (t.running, t.mode, t.server.filter(|_| t.session.is_some()), t.database.clone())
        });
        // Standard toolbar.
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            if ui.add(egui::Button::new("📄 New Query").frame_when_inactive(false)).on_hover_text("New Query (Ctrl+N)").clicked() {
                cmds.push(Cmd::NewQuery);
            }
            if tb(ui, "🗄", "Database Engine Query", true) {
                cmds.push(Cmd::ConnectObjectExplorer);
            }
            ui.separator();
            if tb(ui, "📂", "Open File (Ctrl+O)", true) {
                cmds.push(Cmd::OpenFile);
            }
            if tb(ui, "💾", "Save (Ctrl+S)", has_tab) {
                cmds.push(Cmd::Save);
            }
            if tb(ui, "🗐", "Save All (Ctrl+Shift+S)", has_tab) {
                cmds.push(Cmd::SaveAll);
            }
            ui.separator();
            if tb(ui, "🌲", "Object Explorer (F8)", true) {
                cmds.push(Cmd::ShowObjectExplorer);
            }
            if tb(ui, "📋", "Template Browser (Ctrl+Alt+T)", true) {
                cmds.push(Cmd::ShowTemplates);
            }
            if tb(ui, "🔧", "Properties Window (F4)", true) {
                cmds.push(Cmd::ShowProperties);
            }

            // SQL Editor toolbar.
            ui.separator();
            let dbs = server.and_then(|s| self.server(s)).map(|s| s.databases.clone()).unwrap_or_default();
            let mut chosen = None;
            ui.add_enabled_ui(has_tab && !running && server.is_some(), |ui| {
                egui::ComboBox::from_id_salt("dbpick")
                    .width(170.0)
                    .selected_text(if db.is_empty() { String::new() } else { db.clone() })
                    .show_ui(ui, |ui| {
                        for d in &dbs {
                            if ui.selectable_label(*d == db, d).clicked() {
                                chosen = Some(d.clone());
                            }
                        }
                    })
                    .response
                    .on_hover_text("Available Databases (Ctrl+U)");
            });
            if let (Some(d), Some(id)) = (chosen, self.current) {
                self.change_database(id, d);
            }
            let p = theme::palette(self.settings.theme);
            let exec = ui.add_enabled(has_tab && !running, egui::Button::new(RichText::new("▶ Execute").color(Color32::from_rgb(56, 163, 82))).frame_when_inactive(false));
            if exec.on_hover_text("Execute (F5)").clicked() {
                cmds.push(Cmd::Execute);
            }
            if tb(ui, "■", "Cancel Executing Query (Alt+Break)", running) {
                cmds.push(Cmd::Cancel);
            }
            if tb(ui, "✔", "Parse (Ctrl+F5)", has_tab && !running) {
                cmds.push(Cmd::Parse);
            }
            ui.separator();
            for (glyph, tip, cmd, m) in [
                ("≡", "Results to Text (Ctrl+T)", Cmd::ResultsToText, ResultsMode::Text),
                ("▦", "Results to Grid (Ctrl+D)", Cmd::ResultsToGrid, ResultsMode::Grid),
                ("🖹", "Results to File (Ctrl+Shift+F)", Cmd::ResultsToFile, ResultsMode::File),
            ] {
                let on = has_tab && mode == m;
                let b = egui::Button::new(RichText::new(glyph).color(if on { p.accent } else { ui.visuals().text_color() })).selected(on).frame_when_inactive(false);
                if ui.add_enabled(has_tab, b).on_hover_text(tip).clicked() {
                    cmds.push(cmd);
                }
            }
            ui.separator();
            for (glyph, tip, cmd) in [
                ("⁄⁄", "Comment out the selected lines (Ctrl+K, Ctrl+C)", Cmd::Comment),
                ("⁄̸", "Uncomment the selected lines (Ctrl+K, Ctrl+U)", Cmd::Uncomment),
                ("⇤", "Decrease Indent", Cmd::Unindent),
                ("⇥", "Increase Indent", Cmd::Indent),
            ] {
                if tb(ui, glyph, tip, has_tab) {
                    cmds.push(cmd);
                }
            }
            ui.separator();
            if tb(ui, "⟨⟩", "Specify Values for Template Parameters (Ctrl+Shift+M)", has_tab) {
                cmds.push(Cmd::TemplateParams);
            }
        });
    }

    fn main_status_bar(&mut self, ui: &mut egui::Ui) {
        let p = theme::palette(self.settings.theme);
        let running = self.tabs.iter().any(|t| t.running);
        let (line, col, ch) = self
            .current_tab()
            .and_then(|t| t.selection.map(|(_, b)| (t, b)))
            .map_or((1, 1, 1), |(t, b)| {
                let (l, c) = crate::tsql::line_col(&t.sql, b);
                let ch = t.sql.chars().take(b).collect::<String>().rsplit('\n').next().map_or(0, |s| s.chars().count()) + 1;
                (l, c, ch)
            });
        egui::Panel::bottom("statusbar")
            .exact_size(22.0)
            .frame(egui::Frame::new().fill(p.status_bar).inner_margin(egui::Margin::symmetric(8, 2)))
            .show(ui, |ui| {
                ui.horizontal_centered(|ui| {
                    let text = if running { "Executing…" } else { "Ready" };
                    ui.label(RichText::new(text).color(p.status_text));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if self.current.is_some() {
                            ui.label(RichText::new("INS").color(p.status_text));
                            ui.add_space(24.0);
                            ui.label(RichText::new(format!("Ln {line}    Col {col}    Ch {ch}")).color(p.status_text));
                        }
                    });
                });
            });
    }

    // ----- Tool windows -------------------------------------------------

    fn object_explorer(&mut self, ui: &mut egui::Ui, actions: &mut Vec<Action>, cmds: &mut Vec<Cmd>) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            ui.menu_button("Connect ⏷", |ui| {
                if ui.button("Database Engine…").clicked() {
                    cmds.push(Cmd::ConnectObjectExplorer);
                    ui.close();
                }
            });
            let sel = self.selected_node;
            let tb = |ui: &mut egui::Ui, t: &str, tip: &str, en: bool| ui.add_enabled(en, egui::Button::new(t).frame_when_inactive(false)).on_hover_text(tip).clicked();
            if tb(ui, "🔌", "Connect Object Explorer", true) {
                cmds.push(Cmd::ConnectObjectExplorer);
            }
            if tb(ui, "⏏", "Disconnect", sel.is_some()) {
                actions.push(Action::Disconnect { server: sel.unwrap().0 });
            }
            if tb(ui, "⟳", "Refresh", sel.is_some()) {
                let (s, n) = sel.unwrap();
                actions.push(Action::Refresh { server: s, node: n });
            }
            if tb(ui, "⊟", "Collapse All", !self.servers.is_empty()) {
                actions.push(Action::CollapseAll);
            }
        });
        ui.separator();
        egui::ScrollArea::both().auto_shrink([false, false]).show(ui, |ui| {
            for s in &mut self.servers {
                match &s.state {
                    ServerState::Connecting => {
                        ui.horizontal(|ui| {
                            ui.spinner();
                            ui.label(format!("Connecting to {}…", s.session.info.server.trim()));
                        });
                    }
                    ServerState::Failed(_) => {}
                    ServerState::Connected => node_ui(ui, s.id, &mut s.root, &mut self.selected_node, actions),
                }
            }
        });
    }

    fn properties(&mut self, ui: &mut egui::Ui) {
        if self.props.node.is_none() {
            ui.weak("Select an item in Object Explorer.");
            return;
        }
        ui.strong(&self.props.title);
        ui.separator();
        if self.props.loading {
            ui.spinner();
            return;
        }
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            egui::Grid::new("props").num_columns(2).striped(true).spacing([12.0, 4.0]).show(ui, |ui| {
                for (k, v) in &self.props.rows {
                    ui.label(k);
                    ui.add(egui::Label::new(v).truncate()).on_hover_text(v);
                    ui.end_row();
                }
            });
        });
    }

    fn output(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label("Show output from:");
            ui.strong("Object Explorer and queries");
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.small_button("Clear All").clicked() {
                    self.output.clear();
                }
            });
        });
        ui.separator();
        egui::ScrollArea::both().auto_shrink([false, false]).stick_to_bottom(true).show(ui, |ui| {
            for line in &self.output {
                ui.add(egui::Label::new(RichText::new(line).monospace()).selectable(true));
            }
        });
    }

    fn template_browser(&mut self, ui: &mut egui::Ui, actions: &mut Vec<Action>) {
        ui.weak("Double-click a template to open it. Fill in its parameters with Ctrl+Shift+M.");
        ui.separator();
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            let mut folders: Vec<&str> = TEMPLATES.iter().map(|t| t.folder).collect();
            folders.dedup();
            for folder in folders {
                egui::CollapsingHeader::new(format!("📁 {folder}")).default_open(true).show(ui, |ui| {
                    for (i, t) in TEMPLATES.iter().enumerate().filter(|(_, t)| t.folder == folder) {
                        let r = ui.selectable_label(false, format!("📄 {}", t.name));
                        if r.double_clicked() {
                            actions.push(Action::OpenTemplate(i));
                        }
                        r.context_menu(|ui| {
                            if ui.button("Open").clicked() {
                                actions.push(Action::OpenTemplate(i));
                                ui.close();
                            }
                        });
                    }
                });
            }
        });
    }

    fn start_page(&mut self, ui: &mut egui::Ui, cmds: &mut Vec<Cmd>) {
        let p = theme::palette(self.settings.theme);
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            ui.add_space(28.0);
            ui.horizontal(|ui| {
                ui.add_space(28.0);
                ui.vertical(|ui| {
                    ui.label(RichText::new("SQLCraft").size(30.0).color(p.accent));
                    ui.label("SQL Server Management Studio for Linux");
                    ui.add_space(20.0);
                    ui.heading("Get started");
                    ui.add_space(6.0);
                    if ui.link("🔌  Connect to a server…").clicked() {
                        cmds.push(Cmd::ConnectObjectExplorer);
                    }
                    if ui.link("📄  New query").clicked() {
                        cmds.push(Cmd::NewQuery);
                    }
                    if ui.link("📂  Open a file…").clicked() {
                        cmds.push(Cmd::OpenFile);
                    }
                    if ui.link("📋  Browse templates").clicked() {
                        cmds.push(Cmd::ShowTemplates);
                    }
                    ui.add_space(20.0);
                    let files = self.settings.recent_files.clone();
                    if !files.is_empty() {
                        ui.heading("Recent files");
                        ui.add_space(6.0);
                        for f in files {
                            if ui.link(f.display().to_string()).clicked() {
                                self.open_recent(f);
                            }
                        }
                    }
                });
            });
        });
    }
}

struct Viewer<'a> {
    app: &'a mut App,
    actions: Vec<Action>,
    cmds: Vec<Cmd>,
}

impl TabViewer for Viewer<'_> {
    type Tab = Pane;

    fn id(&mut self, tab: &mut Pane) -> Id {
        Id::unique(("pane", format!("{tab:?}")))
    }

    fn title(&mut self, tab: &mut Pane) -> egui::WidgetText {
        match tab {
            Pane::ObjectExplorer => "Object Explorer".into(),
            Pane::Properties => "Properties".into(),
            Pane::Output => "Output".into(),
            Pane::TemplateBrowser => "Template Browser".into(),
            Pane::Start => "Start".into(),
            Pane::Query(id) => match self.app.tab(*id) {
                Some(t) => {
                    let caption = t.caption(self.app.server_name(t).as_deref());
                    if t.running { format!("⏳ {caption}").into() } else { caption.into() }
                }
                None => "Query".into(),
            },
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, tab: &mut Pane) {
        match tab {
            Pane::ObjectExplorer => self.app.object_explorer(ui, &mut self.actions, &mut self.cmds),
            Pane::Properties => self.app.properties(ui),
            Pane::Output => self.app.output(ui),
            Pane::TemplateBrowser => self.app.template_browser(ui, &mut self.actions),
            Pane::Start => self.app.start_page(ui, &mut self.cmds),
            Pane::Query(id) => {
                let id = *id;
                if ui.ui_contains_pointer() && ui.input(|i| i.pointer.any_pressed()) {
                    self.app.current = Some(id);
                }
                self.app.query_pane(ui, id);
            }
        }
    }

    fn on_close(&mut self, tab: &mut Pane) -> OnCloseResponse {
        if let Pane::Query(id) = tab {
            if self.app.tab(*id).is_some_and(|t| t.dirty() && !t.sql.trim().is_empty()) {
                self.app.confirm = Some(Confirm::CloseTab(*id));
                return OnCloseResponse::Ignore;
            }
        }
        OnCloseResponse::Close
    }

    fn scroll_bars(&self, _tab: &Pane) -> [bool; 2] {
        [false, false]
    }
}

// ----- Object Explorer tree ------------------------------------------------

fn node_ui(ui: &mut egui::Ui, server: u64, node: &mut Node, selected: &mut Option<(u64, u64)>, actions: &mut Vec<Action>) {
    let is_selected = *selected == Some((server, node.id));
    let label = format!("{} {}", node.kind.icon(), node.label);
    let header = |ui: &mut egui::Ui, node: &Node, selected: &mut Option<(u64, u64)>, actions: &mut Vec<Action>| {
        let r = ui.selectable_label(is_selected, &label);
        if r.clicked() || r.secondary_clicked() {
            *selected = Some((server, node.id));
        }
        context_menu(&r, server, node, actions);
        r
    };
    if !node.kind.expandable() {
        ui.horizontal(|ui| {
            ui.add_space(ui.spacing().indent);
            let r = header(ui, node, selected, actions);
            if r.double_clicked() {
                if let Kind::Procedure { db, schema, name } | Kind::Function { db, schema, name, .. } = &node.kind {
                    actions.push(Action::Script { server, db: db.clone(), schema: schema.clone(), name: name.clone(), kind: ScriptKind::Modify, run: false });
                }
            }
        });
        return;
    }
    let state = CollapsingState::load_with_default_open(ui.ctx(), node_ui_id(server, node.id), false);
    if state.is_open() && node.children.is_none() && !node.loading {
        actions.push(Action::Expand { server, node: node.id });
    }
    let mut toggle = false;
    state
        .show_header(ui, |ui| {
            let r = header(ui, node, selected, actions);
            if r.double_clicked() {
                toggle = true;
            }
        })
        .body(|ui| {
            if node.loading {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.weak("Loading…");
                });
            }
            if let Some(e) = &node.error {
                ui.colored_label(Color32::from_rgb(220, 80, 70), e);
            }
            if let Some(children) = &mut node.children {
                if children.is_empty() && node.error.is_none() {
                    ui.weak("(empty)");
                }
                for c in children {
                    node_ui(ui, server, c, selected, actions);
                }
            }
        });
    if toggle {
        let mut s = CollapsingState::load_with_default_open(ui.ctx(), node_ui_id(server, node.id), false);
        s.toggle(ui);
        s.store(ui.ctx());
    }
}

fn context_menu(r: &egui::Response, server: u64, node: &Node, actions: &mut Vec<Action>) {
    let script = |db: &str, schema: &str, name: &str, kind: ScriptKind, run: bool| Action::Script {
        server,
        db: db.into(),
        schema: schema.into(),
        name: name.into(),
        kind,
        run,
    };
    r.context_menu(|ui| {
        match &node.kind {
            Kind::Table { db, schema, name } => {
                if ui.button("Select Top 1000 Rows").clicked() {
                    actions.push(script(db, schema, name, ScriptKind::SelectTop(1000), true));
                }
                ui.menu_button("Script Table as", |ui| {
                    for (label, kind) in [
                        ("CREATE To ▸ New Query Editor Window", ScriptKind::Create),
                        ("SELECT To ▸ New Query Editor Window", ScriptKind::SelectTop(1000)),
                        ("INSERT To ▸ New Query Editor Window", ScriptKind::Insert),
                        ("UPDATE To ▸ New Query Editor Window", ScriptKind::Update),
                        ("DELETE To ▸ New Query Editor Window", ScriptKind::Delete),
                        ("DROP To ▸ New Query Editor Window", ScriptKind::Drop("TABLE")),
                    ] {
                        if ui.button(label).clicked() {
                            actions.push(script(db, schema, name, kind, false));
                        }
                    }
                });
                ui.separator();
            }
            Kind::View { db, schema, name } => {
                if ui.button("Select Top 1000 Rows").clicked() {
                    actions.push(script(db, schema, name, ScriptKind::SelectTop(1000), true));
                }
                ui.menu_button("Script View as", |ui| {
                    if ui.button("CREATE To ▸ New Query Editor Window").clicked() {
                        actions.push(script(db, schema, name, ScriptKind::Definition, false));
                    }
                    if ui.button("ALTER To ▸ New Query Editor Window").clicked() {
                        actions.push(script(db, schema, name, ScriptKind::Modify, false));
                    }
                    if ui.button("DROP To ▸ New Query Editor Window").clicked() {
                        actions.push(script(db, schema, name, ScriptKind::Drop("VIEW"), false));
                    }
                });
                ui.separator();
            }
            Kind::Procedure { db, schema, name } => {
                if ui.button("Modify").clicked() {
                    actions.push(script(db, schema, name, ScriptKind::Modify, false));
                }
                if ui.button("Execute Stored Procedure…").clicked() {
                    actions.push(script(db, schema, name, ScriptKind::Execute, false));
                }
                ui.menu_button("Script Stored Procedure as", |ui| {
                    if ui.button("CREATE To ▸ New Query Editor Window").clicked() {
                        actions.push(script(db, schema, name, ScriptKind::Definition, false));
                    }
                    if ui.button("ALTER To ▸ New Query Editor Window").clicked() {
                        actions.push(script(db, schema, name, ScriptKind::Modify, false));
                    }
                    if ui.button("DROP To ▸ New Query Editor Window").clicked() {
                        actions.push(script(db, schema, name, ScriptKind::Drop("PROCEDURE"), false));
                    }
                });
                ui.separator();
            }
            Kind::Function { db, schema, name, .. } => {
                if ui.button("Modify").clicked() {
                    actions.push(script(db, schema, name, ScriptKind::Modify, false));
                }
                ui.menu_button("Script Function as", |ui| {
                    if ui.button("CREATE To ▸ New Query Editor Window").clicked() {
                        actions.push(script(db, schema, name, ScriptKind::Definition, false));
                    }
                    if ui.button("DROP To ▸ New Query Editor Window").clicked() {
                        actions.push(script(db, schema, name, ScriptKind::Drop("FUNCTION"), false));
                    }
                });
                ui.separator();
            }
            _ => {}
        }
        if matches!(node.kind, Kind::Server) || node.kind.database().is_some() {
            if ui.button("New Query").clicked() {
                actions.push(Action::NewQuery { server, db: node.kind.database().map(String::from) });
            }
        }
        if matches!(node.kind, Kind::Server) && ui.button("Disconnect").clicked() {
            actions.push(Action::Disconnect { server });
        }
        if node.kind.expandable() {
            ui.separator();
            if ui.button("Refresh").clicked() {
                actions.push(Action::Refresh { server, node: node.id });
            }
        }
        if ui.button("Copy Name").clicked() {
            let name = match &node.kind {
                Kind::Table { schema, name, .. } | Kind::View { schema, name, .. } | Kind::Procedure { schema, name, .. } | Kind::Function { schema, name, .. } => {
                    format!("{}.{}", qi(schema), qi(name))
                }
                Kind::Database { db } => qi(db),
                _ => node.label.split(" (").next().unwrap_or(&node.label).to_string(),
            };
            ui.ctx().copy_text(name);
        }
    });
}
