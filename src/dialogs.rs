//! Modal dialogs: Connect to Server, Options, Go To Line, template parameters.

use std::time::{Duration, Instant};

use eframe::egui::{self, Align, Color32, Id, Key, Layout, RichText};

use crate::app::{App, AppMsg, Confirm, apply_font_size};
use crate::db::{self, ConnectionInfo};
use crate::settings;
use crate::theme::{self, ThemeKind};

impl App {
    pub(crate) fn connect_dialog(&mut self, ctx: &egui::Context) {
        if let Some(d) = &mut self.connect {
            if d.info.auth == db::Auth::Kerberos && d.principal.is_none() && !d.checking_principal {
                d.checking_principal = true;
                let tx = self.msg_tx.clone();
                self.rt.spawn_blocking(move || {
                    let _ = tx.send(AppMsg::Principal(crate::kerberos::current_principal()));
                });
            }
        }
        let Some(d) = &mut self.connect else { return };
        let mut go = false;
        let mut cancel = false;
        let mut pick: Option<ConnectionInfo> = None;
        let mut forget: Option<String> = None;
        let recent = &self.settings.recent;
        let resp = egui::Modal::new(Id::unique("connect")).show(ctx, |ui| {
            ui.set_width(470.0);
            ui.label(RichText::new("Connect to Server").size(18.0));
            ui.add_space(8.0);
            egui::Grid::new("connect_grid").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
                ui.label("Server type:");
                ui.add_enabled_ui(false, |ui| {
                    egui::ComboBox::from_id_salt("stype").width(270.0).selected_text("Database Engine").show_ui(ui, |_| {});
                });
                ui.end_row();

                ui.label("Server name:");
                ui.horizontal(|ui| {
                    let r = ui.add(egui::TextEdit::singleline(&mut d.info.server).desired_width(244.0).hint_text(r"host, host\instance or host,port"));
                    if d.info.server.is_empty() && !r.has_focus() {
                        r.request_focus();
                    }
                    ui.menu_button("⏷", |ui| {
                        if recent.is_empty() {
                            ui.weak("No recent connections");
                        }
                        for c in recent {
                            ui.horizontal(|ui| {
                                if ui.button(c.display()).clicked() {
                                    pick = Some(c.clone());
                                    ui.close();
                                }
                                if ui.small_button("🗑").on_hover_text("Remove from list").clicked() {
                                    forget = Some(c.key());
                                }
                            });
                        }
                    });
                });
                ui.end_row();

                ui.label("Authentication:");
                egui::ComboBox::from_id_salt("auth").width(270.0).selected_text(d.info.auth.label()).show_ui(ui, |ui| {
                    for a in db::Auth::ALL {
                        ui.selectable_value(&mut d.info.auth, a, a.label());
                    }
                });
                ui.end_row();

                let kerberos = d.info.auth == db::Auth::Kerberos;
                let (user_label, user_hint, pw_hint) = match d.info.auth {
                    db::Auth::Kerberos => (
                        "User name:",
                        match &d.principal {
                            Some(Ok(p)) => p.clone(),
                            _ => "user@DOMAIN.COM".to_string(),
                        },
                        "blank = use current ticket",
                    ),
                    db::Auth::Windows => ("User name:", r"DOMAIN\user".to_string(), ""),
                    db::Auth::SqlServer => ("Login:", String::new(), ""),
                };
                ui.label(user_label);
                ui.add(egui::TextEdit::singleline(&mut d.info.user).hint_text(user_hint).desired_width(270.0));
                ui.end_row();

                ui.label("Password:");
                let pw = ui.add(egui::TextEdit::singleline(&mut d.info.password).password(true).hint_text(pw_hint).desired_width(270.0));
                if pw.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                    go = true;
                }
                ui.end_row();

                ui.label("");
                ui.checkbox(&mut d.info.remember_password, "Remember password");
                ui.end_row();

                if kerberos {
                    ui.label("");
                    match &d.principal {
                        None => {
                            ui.horizontal(|ui| {
                                ui.spinner();
                                ui.weak("Checking for a Kerberos ticket…");
                            });
                        }
                        Some(Ok(p)) => {
                            ui.label(RichText::new(format!("✔ Signed in to the domain as {p}")).color(Color32::from_rgb(56, 163, 82)));
                        }
                        Some(Err(e)) => {
                            let text = format!("No Kerberos ticket. Enter your domain user and password to sign in.\n{}", e.lines().next().unwrap_or(""));
                            ui.vertical(|ui| {
                                ui.set_max_width(300.0);
                                ui.add(egui::Label::new(RichText::new(text).weak().small()).wrap());
                            });
                        }
                    }
                    ui.end_row();
                }
            });
            ui.add_space(4.0);
            egui::CollapsingHeader::new("Options >>").default_open(d.show_options).show(ui, |ui| {
                egui::Grid::new("connect_opts").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
                    ui.label("Connect to database:");
                    ui.add(egui::TextEdit::singleline(&mut d.info.database).hint_text("<default>").desired_width(240.0));
                    ui.end_row();
                    ui.label("Encryption:");
                    egui::ComboBox::from_id_salt("encrypt").width(240.0).selected_text(d.info.encrypt.label()).show_ui(ui, |ui| {
                        for e in db::Encrypt::ALL {
                            ui.selectable_value(&mut d.info.encrypt, e, e.label());
                        }
                    });
                    ui.end_row();
                    ui.label("");
                    ui.checkbox(&mut d.info.trust_cert, "Trust server certificate");
                    ui.end_row();
                    ui.label("Connection time-out:");
                    ui.add(egui::DragValue::new(&mut d.info.timeout_secs).range(1..=600).suffix(" seconds"));
                    ui.end_row();
                    ui.label("Application name:");
                    ui.add(egui::TextEdit::singleline(&mut d.info.app_name).desired_width(240.0));
                    ui.end_row();
                    if d.info.auth == db::Auth::Kerberos {
                        ui.label("Service principal (SPN):");
                        ui.add(egui::TextEdit::singleline(&mut d.info.spn).hint_text("automatic: MSSQLSvc/<fqdn>:<port>").desired_width(240.0));
                        ui.end_row();
                    }
                    ui.label("");
                    ui.horizontal(|ui| {
                        let mut use_color = d.info.color.is_some();
                        if ui.checkbox(&mut use_color, "Use custom color").changed() {
                            d.info.color = use_color.then_some([255, 200, 200]);
                        }
                        if let Some(c) = &mut d.info.color {
                            ui.color_edit_button_srgb(c).on_hover_text("Tints the query window status bar, e.g. red for production");
                        }
                    });
                    ui.end_row();
                });
            });
            ui.add_space(10.0);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.button("Cancel").clicked() {
                    cancel = true;
                }
                let ok = !d.info.server.trim().is_empty();
                if ui.add_enabled(ok, egui::Button::new("Connect")).clicked() {
                    go = true;
                }
            });
        });
        if resp.should_close() {
            cancel = true;
        }
        if let Some(mut p) = pick {
            let d = self.connect.as_mut().unwrap();
            p.password.clear();
            let key = p.key();
            let remember = p.remember_password;
            d.info = p;
            d.principal = None;
            d.checking_principal = false;
            if remember {
                self.load_password(key);
            }
        }
        if let Some(key) = forget {
            self.settings.forget(&key);
            self.rt.spawn_blocking(move || settings::delete_password(&key));
        }
        if go && self.connect.as_ref().is_some_and(|d| !d.info.server.trim().is_empty()) {
            let d = self.connect.take().unwrap();
            self.remember_connection(&d.info);
            match d.for_tab {
                Some(tab) => {
                    // Reuse an Object Explorer connection to the same server, or add one.
                    let key = d.info.key();
                    let existing = self.servers.iter().find(|s| s.session.info.key() == key).map(|s| s.id);
                    let server = existing.unwrap_or_else(|| self.connect_server(ctx, d.info.clone()));
                    if let Some(t) = self.tab_mut(tab) {
                        t.session = None;
                        t.database.clear();
                    }
                    let db = Some(d.info.database.clone()).filter(|d| !d.trim().is_empty());
                    self.attach_tab(ctx, tab, server, db);
                }
                None => {
                    let server = self.connect_server(ctx, d.info.clone());
                    if d.then_query {
                        self.new_query(ctx, Some(server), None, "");
                    }
                }
            }
        } else if cancel {
            self.connect = None;
        }
    }

    pub(crate) fn confirm_dialog(&mut self, ctx: &egui::Context) {
        let Some(c) = &self.confirm else { return };
        let text = match c {
            Confirm::CloseTab(id) => {
                let name = self.tab(*id).map(|t| t.title.clone()).unwrap_or_default();
                format!("Save changes to the following items?\n\n{name}")
            }
            Confirm::Quit => "Some query windows have unsaved changes. Exit anyway?".to_string(),
        };
        let mut choice = None;
        egui::Modal::new(Id::unique("confirm")).show(ctx, |ui| {
            ui.set_width(380.0);
            ui.label(RichText::new("SQLCraft").strong());
            ui.add_space(6.0);
            ui.label(text);
            ui.add_space(12.0);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.button("Cancel").clicked() {
                    choice = Some(0);
                }
                match c {
                    Confirm::CloseTab(_) => {
                        if ui.button("No").clicked() {
                            choice = Some(1);
                        }
                        if ui.button("Yes").clicked() {
                            choice = Some(2);
                        }
                    }
                    Confirm::Quit => {
                        if ui.button("Exit").clicked() {
                            choice = Some(1);
                        }
                    }
                }
            });
        });
        let Some(choice) = choice else { return };
        let c = self.confirm.take().unwrap();
        match (c, choice) {
            (_, 0) => {}
            (Confirm::CloseTab(id), 1) => self.close_tab(id),
            (Confirm::CloseTab(id), _) => self.save_tab(id, false),
            (Confirm::Quit, _) => {
                self.allow_quit = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
    }

    pub(crate) fn options_dialog(&mut self, ctx: &egui::Context) {
        if !self.show_options {
            return;
        }
        let mut close = false;
        let r = egui::Modal::new(Id::unique("options")).show(ctx, |ui| {
            ui.set_width(420.0);
            ui.label(RichText::new("Options").size(18.0));
            ui.add_space(8.0);
            egui::Grid::new("opts").num_columns(2).spacing([12.0, 10.0]).show(ui, |ui| {
                ui.label("Color theme:");
                egui::ComboBox::from_id_salt("theme").selected_text(self.settings.theme.label()).show_ui(ui, |ui| {
                    for t in ThemeKind::ALL {
                        if ui.selectable_value(&mut self.settings.theme, t, t.label()).clicked() {
                            theme::apply(ctx, t);
                        }
                    }
                });
                ui.end_row();
                ui.label("Font size:");
                if ui.add(egui::DragValue::new(&mut self.settings.font_size).range(9.0..=28.0).speed(0.2)).changed() {
                    apply_font_size(ctx, self.settings.font_size);
                }
                ui.end_row();
                ui.label("Default results:");
                ui.horizontal(|ui| {
                    ui.radio_value(&mut self.settings.results_to_text, false, "Results to grid");
                    ui.radio_value(&mut self.settings.results_to_text, true, "Results to text");
                });
                ui.end_row();
                ui.label("Maximum rows per result:");
                ui.add(egui::DragValue::new(&mut self.settings.max_rows).range(100..=10_000_000).speed(1000));
                ui.end_row();
            });
            ui.add_space(10.0);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.button("OK").clicked() {
                    close = true;
                }
            });
        });
        if close || r.should_close() {
            self.show_options = false;
            self.settings.save();
        }
    }

    pub(crate) fn goto_dialog(&mut self, ctx: &egui::Context) {
        let lines = self.current_tab().map_or(1, |t| t.sql.matches('\n').count() + 1);
        let Some(text) = &mut self.goto else { return };
        let mut go = None;
        let mut close = false;
        let r = egui::Modal::new(Id::unique("goto")).show(ctx, |ui| {
            ui.set_width(260.0);
            ui.label(RichText::new("Go To Line").strong());
            ui.add_space(6.0);
            ui.label(format!("Line number (1 - {lines}):"));
            let r = ui.text_edit_singleline(text);
            r.request_focus();
            if ui.input(|i| i.key_pressed(Key::Enter)) {
                go = text.trim().parse::<usize>().ok();
            }
            ui.add_space(8.0);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.button("Cancel").clicked() {
                    close = true;
                }
                if ui.button("OK").clicked() {
                    go = text.trim().parse::<usize>().ok();
                }
            });
        });
        if let Some(line) = go {
            if let Some(t) = self.current.and_then(|id| self.tab_mut(id)) {
                let at = crate::tsql::line_start(&t.sql, line.clamp(1, lines));
                t.set_selection = Some((at, at));
            }
            close = true;
        }
        if close || r.should_close() {
            self.goto = None;
        }
    }

    pub(crate) fn template_params_dialog(&mut self, ctx: &egui::Context) {
        let Some((tab, params)) = &mut self.template_params else { return };
        let tab = *tab;
        let mut ok = false;
        let mut close = false;
        let r = egui::Modal::new(Id::unique("tparams")).show(ctx, |ui| {
            ui.set_width(520.0);
            ui.label(RichText::new("Specify Values for Template Parameters").strong());
            ui.add_space(8.0);
            egui::Grid::new("tp").num_columns(3).striped(true).spacing([12.0, 6.0]).show(ui, |ui| {
                ui.strong("Parameter");
                ui.strong("Type");
                ui.strong("Value");
                ui.end_row();
                for p in params.iter_mut() {
                    ui.label(&p.name);
                    ui.label(&p.ty);
                    ui.add(egui::TextEdit::singleline(&mut p.value).desired_width(240.0));
                    ui.end_row();
                }
            });
            ui.add_space(10.0);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.button("Cancel").clicked() {
                    close = true;
                }
                if ui.button("OK").clicked() {
                    ok = true;
                }
            });
        });
        if ok {
            let values = params.clone();
            if let Some(t) = self.tab_mut(tab) {
                t.sql = crate::templates::fill(&t.sql, &values);
            }
            close = true;
        }
        if close || r.should_close() {
            self.template_params = None;
        }
    }

    pub(crate) fn about_dialog(&mut self, ctx: &egui::Context) {
        if !self.show_about {
            return;
        }
        let r = egui::Modal::new(Id::unique("about")).show(ctx, |ui| {
            ui.label(RichText::new(format!("SQLCraft {}", env!("CARGO_PKG_VERSION"))).size(18.0));
            ui.label("SQL Server Management Studio for Linux.");
            ui.add_space(8.0);
            egui::ScrollArea::vertical().max_height(360.0).show(ui, |ui| {
                egui::Grid::new("keys").num_columns(2).striped(true).show(ui, |ui| {
                    for (k, v) in [
                        ("F5 / Ctrl+E", "Execute (selection, or the whole script)"),
                        ("Ctrl+F5", "Parse"),
                        ("Alt+Break", "Cancel executing query"),
                        ("Ctrl+N", "New query with current connection"),
                        ("Ctrl+O / Ctrl+S / Ctrl+Shift+S", "Open / Save / Save All"),
                        ("Ctrl+F4", "Close the query window"),
                        ("Ctrl+Tab", "Next query window"),
                        ("Ctrl+R", "Show / hide the results pane"),
                        ("Ctrl+D / Ctrl+T / Ctrl+Shift+F", "Results to Grid / Text / File"),
                        ("Ctrl+F / Ctrl+H / F3", "Find / Replace / Find next"),
                        ("Ctrl+G", "Go to line"),
                        ("Ctrl+K, Ctrl+C / Ctrl+K, Ctrl+U", "Comment / uncomment lines"),
                        ("Ctrl+Shift+U / Ctrl+Shift+L", "Make uppercase / lowercase"),
                        ("Tab / Shift+Tab", "Indent / outdent selected lines"),
                        ("Ctrl+Shift+M", "Specify values for template parameters"),
                        ("Alt+F1", "sp_help on the selected object"),
                        ("Ctrl+1 / Ctrl+2", "sp_who / sp_lock"),
                        ("F8 / F4", "Object Explorer / Properties"),
                        ("Ctrl+Alt+T / Ctrl+Alt+O", "Template Browser / Output"),
                        ("Shift+Alt+Enter", "Full screen"),
                    ] {
                        ui.monospace(k);
                        ui.label(v);
                        ui.end_row();
                    }
                });
            });
            ui.add_space(8.0);
            if ui.button("Close").clicked() {
                self.show_about = false;
            }
        });
        if r.should_close() {
            self.show_about = false;
        }
    }

    pub(crate) fn toast_ui(&mut self, ctx: &egui::Context) {
        let Some((text, error, at)) = &self.toast else { return };
        let at: Instant = *at;
        if at.elapsed() > Duration::from_secs(if *error { 10 } else { 4 }) {
            self.toast = None;
            return;
        }
        let mut close = false;
        egui::Area::new(Id::unique("toast")).anchor(egui::Align2::RIGHT_BOTTOM, [-16.0, -40.0]).show(ctx, |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.set_max_width(440.0);
                ui.horizontal(|ui| {
                    let color = if *error { Color32::from_rgb(214, 66, 56) } else { Color32::from_rgb(56, 163, 82) };
                    ui.label(RichText::new(if *error { "⚠" } else { "✔" }).color(color));
                    ui.add(egui::Label::new(text.as_str()).wrap());
                    if ui.small_button("🗙").clicked() {
                        close = true;
                    }
                });
            });
        });
        if close {
            self.toast = None;
        }
        ctx.request_repaint_after(Duration::from_millis(500));
    }
}
