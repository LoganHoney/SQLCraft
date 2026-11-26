use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc as std_mpsc;
use std::time::{Duration, Instant};

use eframe::egui::{
    self, Align, Color32, Id, Key, KeyboardShortcut, Layout, Modifiers, RichText, Sense, TextStyle,
    collapsing_header::CollapsingState,
};
use egui_extras::{Column, TableBuilder};
use serde::{Deserialize, Serialize};

use crate::batch;
use crate::db::{self, ColumnInfo, ConnectionInfo, Event, Message, Request, ServerInfo, Session, Table};
use crate::explorer::{self, Kind, Node};
use crate::format::{csv_field, qi};
use crate::settings::{self, Settings};

const EXECUTE: KeyboardShortcut = KeyboardShortcut::new(Modifiers::NONE, Key::F5);
const EXECUTE_ALT: KeyboardShortcut = KeyboardShortcut::new(Modifiers::CTRL, Key::E);
const CANCEL: KeyboardShortcut = KeyboardShortcut::new(Modifiers::ALT, Key::Pause);
const NEW_QUERY: KeyboardShortcut = KeyboardShortcut::new(Modifiers::CTRL, Key::N);
const OPEN: KeyboardShortcut = KeyboardShortcut::new(Modifiers::CTRL, Key::O);
const SAVE: KeyboardShortcut = KeyboardShortcut::new(Modifiers::CTRL, Key::S);
const CLOSE_TAB: KeyboardShortcut = KeyboardShortcut::new(Modifiers::CTRL, Key::F4);
const CLOSE_TAB_ALT: KeyboardShortcut = KeyboardShortcut::new(Modifiers::CTRL, Key::W);
const TOGGLE_RESULTS: KeyboardShortcut = KeyboardShortcut::new(Modifiers::CTRL, Key::R);
const TOGGLE_EXPLORER: KeyboardShortcut = KeyboardShortcut::new(Modifiers::NONE, Key::F8);
const NEXT_TAB: KeyboardShortcut = KeyboardShortcut::new(Modifiers::CTRL, Key::Tab);

/// Tags at or above this are fetches whose result nobody waits for.
const FIRE_AND_FORGET: u64 = u64::MAX / 2;

// ----- Model --------------------------------------------------------------

#[derive(PartialEq)]
enum ServerState {
    Connecting,
    Connected,
    Failed(String),
}

struct Server {
    id: u64,
    session: Session,
    info: Option<ServerInfo>,
    state: ServerState,
    root: Node,
    databases: Vec<String>,
}

struct ResultSet {
    columns: Vec<ColumnInfo>,
    rows: Vec<Vec<Option<String>>>,
    widths: Vec<f32>,
}

impl ResultSet {
    fn new(columns: Vec<ColumnInfo>) -> Self {
        let widths = columns.iter().map(|c| text_width(&c.name)).collect();
        Self { columns, rows: Vec::new(), widths }
    }

    fn add_rows(&mut self, rows: Vec<Vec<Option<String>>>) {
        // Size columns from the first rows only, like SSMS.
        if self.rows.len() < 200 {
            for row in rows.iter().take(200 - self.rows.len()) {
                for (w, v) in self.widths.iter_mut().zip(row) {
                    *w = w.max(text_width(v.as_deref().unwrap_or("NULL")));
                }
            }
        }
        self.rows.extend(rows);
    }

    fn header_line(&self, sep: &str) -> String {
        self.columns.iter().map(|c| c.name.as_str()).collect::<Vec<_>>().join(sep)
    }

    fn row_line(&self, r: usize, sep: &str) -> String {
        self.rows[r].iter().map(|v| v.as_deref().unwrap_or("NULL")).collect::<Vec<_>>().join(sep)
    }

    fn to_csv(&self) -> String {
        let mut s = self.columns.iter().map(|c| csv_field(&c.name)).collect::<Vec<_>>().join(",");
        s.push_str("\r\n");
        for row in &self.rows {
            let line: Vec<_> = row.iter().map(|v| v.as_deref().map(csv_field).unwrap_or_default()).collect();
            s.push_str(&line.join(","));
            s.push_str("\r\n");
        }
        s
    }
}

fn text_width(s: &str) -> f32 {
    let chars = s.lines().next().unwrap_or("").chars().count().min(60) as f32;
    (chars * 7.5 + 18.0).clamp(48.0, 420.0)
}

#[derive(PartialEq, Clone, Copy)]
enum Outcome {
    Success,
    Errors,
    Cancelled,
}

#[derive(PartialEq, Clone, Copy)]
enum ResultsView {
    Set(usize),
    Messages,
}

struct QueryTab {
    id: u64,
    title: String,
    path: Option<PathBuf>,
    sql: String,
    saved_sql: String,
    server: Option<u64>,
    session: Option<Session>,
    server_info: Option<ServerInfo>,
    database: String,
    connect_error: Option<String>,
    running: bool,
    started: Option<Instant>,
    elapsed: Option<Duration>,
    outcome: Option<Outcome>,
    results: Vec<ResultSet>,
    messages: Vec<Message>,
    view: ResultsView,
    show_results: bool,
    /// Character range selected in the editor.
    selection: Option<(usize, usize)>,
    /// (result set, row, column) of the selected grid cell.
    cell: Option<(usize, usize, usize)>,
    focus_editor: bool,
}

impl QueryTab {
    fn dirty(&self) -> bool {
        self.sql != self.saved_sql
    }

    fn total_rows(&self) -> usize {
        self.results.iter().map(|r| r.rows.len()).sum()
    }
}

/// Tabs saved across restarts, like SSMS auto-recover.
#[derive(Serialize, Deserialize)]
struct SavedTab {
    title: String,
    path: Option<PathBuf>,
    sql: String,
    saved_sql: String,
}

#[derive(Clone)]
enum ScriptKind {
    SelectTop(u32),
    Create,
    Insert,
    Update,
    Delete,
    Drop(&'static str),
    Modify,
    Definition,
    Execute,
}

/// Why an Object Explorer fetch was sent.
enum Pending {
    Children { server: u64, node: u64 },
    Databases { server: u64 },
    Script { server: u64, db: String, schema: String, name: String, kind: ScriptKind, run: bool },
}

enum Action {
    Expand { server: u64, node: u64 },
    Refresh { server: u64, node: u64 },
    NewQuery { server: u64, db: Option<String> },
    Script { server: u64, db: String, schema: String, name: String, kind: ScriptKind, run: bool },
    Disconnect { server: u64 },
}

/// Results of background work that is not tied to one connection.
enum AppMsg {
    Opened(PathBuf, String),
    SaveTo { tab: u64, path: PathBuf },
    ExportCsv { path: PathBuf, csv: String },
    Password { key: String, password: String },
    /// Who the current Kerberos ticket belongs to.
    Principal(Result<String, String>),
    Error(String),
}

struct ConnectDialog {
    info: ConnectionInfo,
    show_options: bool,
    /// Open a query window once the connection is made.
    then_query: bool,
    /// Connect this tab instead of adding a server to Object Explorer.
    for_tab: Option<u64>,
    /// The current Kerberos ticket, once looked up.
    principal: Option<Result<String, String>>,
    checking_principal: bool,
}

enum Confirm {
    CloseTab(u64),
    Quit,
}

pub struct App {
    rt: tokio::runtime::Runtime,
    settings: Settings,
    servers: Vec<Server>,
    tabs: Vec<QueryTab>,
    active: usize,
    next_id: u64,
    query_counter: u32,
    pending: HashMap<u64, Pending>,
    selected_node: Option<(u64, u64)>,
    show_explorer: bool,
    connect: Option<ConnectDialog>,
    confirm: Option<Confirm>,
    toast: Option<(String, bool, Instant)>,
    show_about: bool,
    msg_tx: std_mpsc::Sender<AppMsg>,
    msg_rx: std_mpsc::Receiver<AppMsg>,
    allow_quit: bool,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("tokio runtime");
        let settings = Settings::load();
        cc.egui_ctx.set_theme(if settings.dark { egui::Theme::Dark } else { egui::Theme::Light });
        apply_font_size(&cc.egui_ctx, settings.font_size);
        // Hack covers box-drawing and arrow glyphs the UI font lacks.
        let mut fonts = egui::FontDefinitions::default();
        if let Some(family) = fonts.families.get_mut(&egui::FontFamily::Proportional) {
            family.push("Hack".to_owned());
        }
        cc.egui_ctx.set_fonts(fonts);
        let (msg_tx, msg_rx) = std_mpsc::channel();
        let mut app = Self {
            rt,
            settings,
            servers: Vec::new(),
            tabs: Vec::new(),
            active: 0,
            next_id: 1,
            query_counter: 0,
            pending: HashMap::new(),
            selected_node: None,
            show_explorer: true,
            connect: None,
            confirm: None,
            toast: None,
            show_about: false,
            msg_tx,
            msg_rx,
            allow_quit: false,
        };
        let saved: Vec<SavedTab> = cc
            .storage
            .and_then(|s| s.get_string("tabs"))
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        for t in saved {
            let id = app.add_tab(None, None);
            let tab = app.tab_mut(id).unwrap();
            tab.title = t.title;
            tab.path = t.path;
            tab.sql = t.sql;
            tab.saved_sql = t.saved_sql;
        }
        app.active = 0;
        app.open_connect_dialog(false, None);
        app
    }

    fn id(&mut self) -> u64 {
        self.next_id += 1;
        self.next_id
    }

    fn tab_mut(&mut self, id: u64) -> Option<&mut QueryTab> {
        self.tabs.iter_mut().find(|t| t.id == id)
    }

    fn server(&self, id: u64) -> Option<&Server> {
        self.servers.iter().find(|s| s.id == id)
    }

    fn server_mut(&mut self, id: u64) -> Option<&mut Server> {
        self.servers.iter_mut().find(|s| s.id == id)
    }

    fn notify(&mut self, text: impl Into<String>, error: bool) {
        self.toast = Some((text.into(), error, Instant::now()));
    }

    fn current_server(&self) -> Option<u64> {
        if let Some((s, _)) = self.selected_node {
            if self.server(s).is_some() {
                return Some(s);
            }
        }
        if let Some(t) = self.tabs.get(self.active) {
            if let Some(s) = t.server {
                if self.server(s).is_some() {
                    return Some(s);
                }
            }
        }
        self.servers.first().map(|s| s.id)
    }

    // ----- Connections ----------------------------------------------------

    fn open_connect_dialog(&mut self, then_query: bool, for_tab: Option<u64>) {
        let info = self.settings.recent.first().cloned().unwrap_or_default();
        let key = info.key();
        let remember = info.remember_password;
        self.connect = Some(ConnectDialog {
            info,
            show_options: false,
            then_query,
            for_tab,
            principal: None,
            checking_principal: false,
        });
        if remember {
            self.load_password(key);
        }
    }

    fn load_password(&self, key: String) {
        let tx = self.msg_tx.clone();
        self.rt.spawn_blocking(move || {
            if let Some(password) = settings::load_password(&key) {
                let _ = tx.send(AppMsg::Password { key, password });
            }
        });
    }

    fn connect_server(&mut self, ctx: &egui::Context, info: ConnectionInfo) -> u64 {
        let id = self.id();
        let root_id = self.id();
        let session = Session::open(self.rt.handle(), info.clone(), None, ctx.clone());
        self.servers.push(Server {
            id,
            root: Node::new(root_id, info.server.trim(), Kind::Server),
            session,
            info: None,
            state: ServerState::Connecting,
            databases: Vec::new(),
        });
        self.selected_node = Some((id, root_id));
        id
    }

    fn remember_connection(&mut self, info: &ConnectionInfo) {
        self.settings.remember(info);
        let key = info.key();
        if info.remember_password && !info.password.is_empty() {
            let pw = info.password.clone();
            let tx = self.msg_tx.clone();
            self.rt.spawn_blocking(move || {
                if let Err(e) = settings::store_password(&key, &pw) {
                    let _ = tx.send(AppMsg::Error(format!("Could not save the password in the keyring: {e}")));
                }
            });
        } else if !info.remember_password {
            self.rt.spawn_blocking(move || settings::delete_password(&key));
        }
    }

    fn fetch(&mut self, server: u64, sql: String, why: Pending) {
        let tag = self.id();
        if let Some(s) = self.server(server) {
            s.session.send(Request::Fetch { tag, sql });
            self.pending.insert(tag, why);
        }
    }

    fn add_tab(&mut self, server: Option<u64>, db: Option<String>) -> u64 {
        let id = self.id();
        self.query_counter += 1;
        let mut tab = QueryTab {
            id,
            title: format!("SQLQuery{}.sql", self.query_counter),
            path: None,
            sql: String::new(),
            saved_sql: String::new(),
            server: None,
            session: None,
            server_info: None,
            database: String::new(),
            connect_error: None,
            running: false,
            started: None,
            elapsed: None,
            outcome: None,
            results: Vec::new(),
            messages: Vec::new(),
            view: ResultsView::Messages,
            show_results: false,
            selection: None,
            cell: None,
            focus_editor: true,
        };
        if let Some(s) = server.and_then(|s| self.server(s)) {
            tab.server = Some(s.id);
            tab.database = db.clone().unwrap_or_default();
        }
        self.tabs.push(tab);
        self.active = self.tabs.len() - 1;
        id
    }

    /// Gives a tab its own connection to `server`, as SSMS does per window.
    fn attach_tab(&mut self, ctx: &egui::Context, tab_id: u64, server: u64, db: Option<String>) {
        let Some(info) = self.server(server).map(|s| s.session.info.clone()) else { return };
        let handle = self.rt.handle().clone();
        let Some(tab) = self.tab_mut(tab_id) else { return };
        let db = db.or_else(|| Some(tab.database.clone()).filter(|d| !d.is_empty()));
        tab.server = Some(server);
        tab.session = Some(Session::open(&handle, info, db, ctx.clone()));
        tab.server_info = None;
        tab.connect_error = None;
    }

    fn new_query(&mut self, ctx: &egui::Context, server: Option<u64>, db: Option<String>, sql: &str) -> u64 {
        let server = server.or_else(|| self.current_server());
        let id = self.add_tab(server, db.clone());
        if let Some(t) = self.tab_mut(id) {
            t.sql = sql.to_string();
        }
        if let Some(s) = server {
            self.attach_tab(ctx, id, s, db);
        }
        id
    }

    fn execute(&mut self, ctx: &egui::Context, tab_id: u64) {
        let has_session = self.tab_mut(tab_id).is_some_and(|t| t.session.is_some());
        if !has_session {
            match self.tab_mut(tab_id).and_then(|t| t.server).or_else(|| self.current_server()) {
                Some(server) => self.attach_tab(ctx, tab_id, server, None),
                None => {
                    self.open_connect_dialog(false, Some(tab_id));
                    return;
                }
            }
        }
        let max_rows = self.settings.max_rows;
        let Some(tab) = self.tab_mut(tab_id) else { return };
        if tab.running {
            return;
        }
        let text = match tab.selection {
            Some((a, b)) if a != b => tab.sql.chars().skip(a).take(b - a).collect::<String>(),
            _ => tab.sql.clone(),
        };
        // Line numbers in errors should match the editor even when running a selection.
        let offset = match tab.selection {
            Some((a, b)) if a != b => tab.sql.chars().take(a).filter(|c| *c == '\n').count() as u32,
            _ => 0,
        };
        let mut batches = batch::split(&text);
        for b in &mut batches {
            b.start_line += offset;
        }
        tab.results.clear();
        tab.messages.clear();
        tab.cell = None;
        tab.show_results = true;
        tab.view = ResultsView::Messages;
        tab.outcome = None;
        tab.elapsed = None;
        if batches.is_empty() {
            tab.messages.push(Message { text: "Commands completed successfully.".into(), is_error: false });
            tab.outcome = Some(Outcome::Success);
            return;
        }
        tab.running = true;
        tab.started = Some(Instant::now());
        if let Some(s) = &tab.session {
            s.send(Request::Run { batches, max_rows });
        }
    }

    fn cancel(&mut self, tab_id: u64) {
        if let Some(t) = self.tab_mut(tab_id) {
            if t.running {
                if let Some(s) = &t.session {
                    s.cancel();
                }
            }
        }
    }

    fn change_database(&mut self, tab_id: u64, db: String) {
        let tag = FIRE_AND_FORGET + self.id();
        if let Some(t) = self.tab_mut(tab_id) {
            match &t.session {
                Some(s) if !t.running => s.send(Request::Fetch { tag, sql: format!("USE {}", qi(&db)) }),
                _ => t.database = db,
            }
        }
    }

    fn request_close_tab(&mut self, tab_id: u64) {
        if self.tabs.iter().any(|t| t.id == tab_id && t.dirty() && !t.sql.trim().is_empty()) {
            self.confirm = Some(Confirm::CloseTab(tab_id));
        } else {
            self.close_tab(tab_id);
        }
    }

    fn close_tab(&mut self, tab_id: u64) {
        if let Some(i) = self.tabs.iter().position(|t| t.id == tab_id) {
            let t = self.tabs.remove(i);
            if t.running {
                if let Some(s) = &t.session {
                    s.cancel();
                }
            }
            if self.active >= self.tabs.len() {
                self.active = self.tabs.len().saturating_sub(1);
            }
        }
    }

    // ----- Files ------------------------------------------------------------

    fn open_file(&self) {
        let tx = self.msg_tx.clone();
        self.rt.spawn(async move {
            let picked = rfd::AsyncFileDialog::new()
                .set_title("Open File")
                .add_filter("SQL files", &["sql"])
                .add_filter("All files", &["*"])
                .pick_file()
                .await;
            if let Some(f) = picked {
                let path = f.path().to_path_buf();
                let msg = match tokio::fs::read(&path).await {
                    Ok(bytes) => AppMsg::Opened(path, decode_text(&bytes)),
                    Err(e) => AppMsg::Error(format!("Could not open {}: {e}", path.display())),
                };
                let _ = tx.send(msg);
            }
        });
    }

    fn save_tab(&mut self, tab_id: u64, save_as: bool) {
        let Some(tab) = self.tabs.iter().find(|t| t.id == tab_id) else { return };
        if let (Some(path), false) = (&tab.path, save_as) {
            let path = path.clone();
            self.write_tab(tab_id, path);
            return;
        }
        let tx = self.msg_tx.clone();
        let name = tab.title.trim_end_matches(" *").to_string();
        self.rt.spawn(async move {
            if let Some(f) = rfd::AsyncFileDialog::new()
                .set_title("Save File As")
                .set_file_name(name)
                .add_filter("SQL files", &["sql"])
                .save_file()
                .await
            {
                let _ = tx.send(AppMsg::SaveTo { tab: tab_id, path: f.path().to_path_buf() });
            }
        });
    }

    fn write_tab(&mut self, tab_id: u64, path: PathBuf) {
        let Some(tab) = self.tab_mut(tab_id) else { return };
        match std::fs::write(&path, &tab.sql) {
            Ok(()) => {
                tab.saved_sql = tab.sql.clone();
                tab.title = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                tab.path = Some(path);
            }
            Err(e) => {
                let text = format!("Could not save {}: {e}", path.display());
                self.notify(text, true);
            }
        }
    }

    fn export_csv(&self, csv: String) {
        let tx = self.msg_tx.clone();
        self.rt.spawn(async move {
            if let Some(f) = rfd::AsyncFileDialog::new()
                .set_title("Save Results As")
                .set_file_name("Results.csv")
                .add_filter("CSV (Comma delimited)", &["csv"])
                .save_file()
                .await
            {
                let _ = tx.send(AppMsg::ExportCsv { path: f.path().to_path_buf(), csv });
            }
        });
    }

    // ----- Event pumps ----------------------------------------------------

    fn pump(&mut self, ctx: &egui::Context) {
        while let Ok(msg) = self.msg_rx.try_recv() {
            match msg {
                AppMsg::Opened(path, text) => {
                    let id = self.new_query(ctx, None, None, &text);
                    if let Some(t) = self.tab_mut(id) {
                        t.saved_sql = text;
                        t.title = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                        t.path = Some(path);
                    }
                }
                AppMsg::SaveTo { tab, path } => self.write_tab(tab, path),
                AppMsg::ExportCsv { path, csv } => match std::fs::write(&path, csv) {
                    Ok(()) => self.notify(format!("Saved {}", path.display()), false),
                    Err(e) => self.notify(format!("Could not save {}: {e}", path.display()), true),
                },
                AppMsg::Password { key, password } => {
                    if let Some(d) = &mut self.connect {
                        if d.info.key() == key && d.info.password.is_empty() {
                            d.info.password = password;
                        }
                    }
                }
                AppMsg::Principal(p) => {
                    if let Some(d) = &mut self.connect {
                        d.principal = Some(p);
                        d.checking_principal = false;
                    }
                }
                AppMsg::Error(e) => self.notify(e, true),
            }
        }

        // Object Explorer connections.
        let mut fetched = Vec::new();
        let mut newly_connected = Vec::new();
        for s in &mut self.servers {
            for ev in s.session.poll() {
                match ev {
                    Event::Connected(info) => {
                        s.root.label = format!(
                            "{} (SQL Server {} - {})",
                            s.session.info.server.trim(),
                            info.version,
                            info.login
                        );
                        s.info = Some(info);
                        s.state = ServerState::Connected;
                        newly_connected.push(s.id);
                    }
                    Event::ConnectFailed(e) => s.state = ServerState::Failed(e),
                    Event::Fetched { tag, result } => fetched.push((tag, result)),
                    Event::Disconnected(e) => s.root.error = Some(e),
                    _ => {}
                }
            }
        }
        for id in newly_connected {
            self.fetch(
                id,
                "SELECT name FROM sys.databases WHERE state = 0 AND HAS_DBACCESS(name) = 1 ORDER BY name".into(),
                Pending::Databases { server: id },
            );
            // Open the server node like SSMS does after connecting.
            if let Some(s) = self.server(id) {
                let root = s.root.id;
                CollapsingState::load_with_default_open(ctx, node_ui_id(id, root), true).store(ctx);
            }
        }
        // Drop servers that never connected, after showing why.
        let failed: Vec<(u64, String)> = self
            .servers
            .iter()
            .filter_map(|s| match &s.state {
                ServerState::Failed(e) => Some((s.id, e.clone())),
                _ => None,
            })
            .collect();
        for (id, e) in failed {
            self.servers.retain(|s| s.id != id);
            self.notify(format!("Cannot connect: {e}"), true);
            if self.connect.is_none() {
                // Reopen the dialog so the user can fix the details.
                self.open_connect_dialog(false, None);
            }
        }
        for (tag, result) in fetched {
            self.on_fetched(ctx, tag, result);
        }

        // Query windows.
        for tab in &mut self.tabs {
            let Some(session) = &tab.session else { continue };
            let mut lost = false;
            for ev in session.poll() {
                match ev {
                    Event::Connected(info) => {
                        tab.database = info.database.clone();
                        tab.server_info = Some(info);
                    }
                    Event::ConnectFailed(e) => {
                        tab.connect_error = Some(e.clone());
                        tab.messages.push(Message { text: e, is_error: true });
                        tab.show_results = true;
                        tab.view = ResultsView::Messages;
                        if tab.running {
                            tab.running = false;
                            tab.outcome = Some(Outcome::Errors);
                        }
                        lost = true;
                    }
                    Event::ResultStart(cols) => {
                        if tab.results.is_empty() {
                            tab.view = ResultsView::Set(0);
                        }
                        tab.results.push(ResultSet::new(cols));
                    }
                    Event::Rows(rows) => {
                        if let Some(r) = tab.results.last_mut() {
                            r.add_rows(rows);
                        }
                    }
                    Event::Message(m) => tab.messages.push(m),
                    Event::DatabaseChanged(db) => tab.database = db,
                    Event::RunDone { elapsed, errors, cancelled } => {
                        tab.running = false;
                        tab.elapsed = Some(elapsed);
                        tab.outcome = Some(if cancelled {
                            Outcome::Cancelled
                        } else if errors {
                            Outcome::Errors
                        } else {
                            Outcome::Success
                        });
                        if !errors && !cancelled && tab.messages.iter().all(|m| m.text.is_empty()) {
                            tab.messages.clear();
                        }
                        if tab.messages.is_empty() && tab.results.is_empty() {
                            tab.messages.push(Message {
                                text: "Commands completed successfully.".into(),
                                is_error: false,
                            });
                        }
                        tab.messages.push(Message {
                            text: format!("\nCompletion time: {}", chrono::Local::now().format("%Y-%m-%dT%H:%M:%S%.7f%:z")),
                            is_error: false,
                        });
                        if errors || cancelled {
                            tab.view = ResultsView::Messages;
                        }
                    }
                    Event::Fetched { result: Err(e), .. } => {
                        tab.messages.push(Message { text: e, is_error: true });
                        tab.show_results = true;
                        tab.view = ResultsView::Messages;
                    }
                    Event::Fetched { .. } => {}
                    Event::Disconnected(e) => tab.messages.push(Message { text: e, is_error: true }),
                }
            }
            if lost {
                tab.session = None;
            }
        }
    }

    fn on_fetched(&mut self, ctx: &egui::Context, tag: u64, result: Result<Vec<Table>, String>) {
        let Some(why) = self.pending.remove(&tag) else { return };
        match why {
            Pending::Databases { server } => {
                if let (Some(s), Ok(tables)) = (self.server_mut(server), &result) {
                    if let Some(t) = tables.first() {
                        s.databases = (0..t.rows.len()).map(|r| t.get(r, "name").to_string()).collect();
                    }
                }
            }
            Pending::Children { server, node } => {
                let mut next = self.next_id;
                if let Some(s) = self.server_mut(server) {
                    if let Some(n) = s.root.find_mut(node) {
                        n.loading = false;
                        match result {
                            Ok(tables) => {
                                let table = tables.into_iter().next().unwrap_or_default();
                                n.children = Some(explorer::build_children(&n.kind, &table, &mut || {
                                    next += 1;
                                    next
                                }));
                                n.error = None;
                            }
                            Err(e) => {
                                n.error = Some(e);
                                n.children = Some(Vec::new());
                            }
                        }
                    }
                }
                self.next_id = next;
            }
            Pending::Script { server, db, schema, name, kind, run } => {
                let tables = match result {
                    Ok(t) => t,
                    Err(e) => {
                        self.notify(e, true);
                        return;
                    }
                };
                let first = tables.first().cloned().unwrap_or_default();
                let columns = || -> Vec<(String, String)> {
                    (0..first.rows.len())
                        .map(|r| {
                            (
                                first.get(r, "name").to_string(),
                                explorer::column_type(
                                    first.get(r, "type_name"),
                                    first.get(r, "max_length"),
                                    first.get(r, "precision"),
                                    first.get(r, "scale"),
                                ),
                            )
                        })
                        .collect()
                };
                let sql = match kind {
                    ScriptKind::SelectTop(n) => {
                        let names: Vec<String> = columns().into_iter().map(|c| c.0).collect();
                        explorer::select_top(&db, &schema, &name, &names, n)
                    }
                    ScriptKind::Create => explorer::script_create_table(&db, &schema, &name, &tables),
                    ScriptKind::Insert => explorer::script_insert(&db, &schema, &name, &columns()),
                    ScriptKind::Update => explorer::script_update(&db, &schema, &name, &columns()),
                    ScriptKind::Execute => explorer::script_exec(&db, &schema, &name, &first),
                    ScriptKind::Modify | ScriptKind::Definition => {
                        let def = first.rows.first().and_then(|r| r[0].clone());
                        let Some(def) = def else {
                            self.notify("The definition is encrypted or you lack VIEW DEFINITION permission.", true);
                            return;
                        };
                        let body = if matches!(kind, ScriptKind::Modify) { explorer::as_alter(&def) } else { def };
                        format!("USE {}\nGO\n\n{}\nGO\n", qi(&db), body.trim_end())
                    }
                    ScriptKind::Delete | ScriptKind::Drop(_) => unreachable!("scripted without a fetch"),
                };
                let tab = self.new_query(ctx, Some(server), Some(db), &sql);
                if run {
                    self.execute(ctx, tab);
                }
            }
        }
    }

    fn apply(&mut self, ctx: &egui::Context, action: Action) {
        match action {
            Action::Expand { server, node } | Action::Refresh { server, node } => {
                let refresh = matches!(action, Action::Refresh { .. });
                let mut next = self.next_id;
                let mut query = None;
                if let Some(s) = self.server_mut(server) {
                    if let Some(n) = s.root.find_mut(node) {
                        if refresh {
                            n.reset();
                        }
                        if n.children.is_none() && !n.loading {
                            if let Some(kids) = explorer::static_children(&n.kind, &mut || {
                                next += 1;
                                next
                            }) {
                                n.children = Some(kids);
                            } else if let Some(sql) = explorer::children_sql(&n.kind) {
                                n.loading = true;
                                query = Some(sql);
                            } else {
                                n.children = Some(Vec::new());
                            }
                        }
                    }
                }
                self.next_id = next;
                if let Some(sql) = query {
                    self.fetch(server, sql, Pending::Children { server, node });
                }
                if refresh {
                    self.fetch(
                        server,
                        "SELECT name FROM sys.databases WHERE state = 0 AND HAS_DBACCESS(name) = 1 ORDER BY name".into(),
                        Pending::Databases { server },
                    );
                }
            }
            Action::NewQuery { server, db } => {
                self.new_query(ctx, Some(server), db, "");
            }
            Action::Script { server, db, schema, name, kind, run } => {
                let sql = match &kind {
                    ScriptKind::Delete => Some(explorer::script_delete(&db, &schema, &name)),
                    ScriptKind::Drop(what) => Some(explorer::script_drop(&db, what, &schema, &name)),
                    _ => None,
                };
                if let Some(sql) = sql {
                    self.new_query(ctx, Some(server), Some(db), &sql);
                    return;
                }
                let fetch_sql = match &kind {
                    ScriptKind::Create => explorer::table_definition_sql(&db, &schema, &name),
                    ScriptKind::Modify | ScriptKind::Definition => explorer::definition_sql(&db, &schema, &name),
                    ScriptKind::Execute => explorer::exec_sql(&db, &schema, &name),
                    _ => format!(
                        "SELECT c.name, t.name AS type_name, c.max_length, c.precision, c.scale FROM {d}.sys.columns c \
                         JOIN {d}.sys.types t ON t.user_type_id = c.user_type_id WHERE c.object_id = OBJECT_ID({o}) ORDER BY c.column_id",
                        d = qi(&db),
                        o = crate::format::lit(&format!("{}.{}.{}", qi(&db), qi(&schema), qi(&name)))
                    ),
                };
                self.fetch(server, fetch_sql, Pending::Script { server, db, schema, name, kind, run });
            }
            Action::Disconnect { server } => {
                self.servers.retain(|s| s.id != server);
                if self.selected_node.is_some_and(|(s, _)| s == server) {
                    self.selected_node = None;
                }
            }
        }
    }

    // ----- UI ---------------------------------------------------------------

    fn shortcuts(&mut self, ctx: &egui::Context) {
        let active = self.tabs.get(self.active).map(|t| t.id);
        let pressed = |s: &KeyboardShortcut| ctx.input_mut(|i| i.consume_shortcut(s));
        if self.connect.is_some() || self.confirm.is_some() {
            return;
        }
        if pressed(&EXECUTE) || pressed(&EXECUTE_ALT) {
            if let Some(id) = active {
                self.execute(ctx, id);
            }
        }
        if pressed(&CANCEL) {
            if let Some(id) = active {
                self.cancel(id);
            }
        }
        if pressed(&NEW_QUERY) {
            if self.servers.is_empty() {
                self.open_connect_dialog(true, None);
            } else {
                self.new_query(ctx, None, None, "");
            }
        }
        if pressed(&OPEN) {
            self.open_file();
        }
        if pressed(&SAVE) {
            if let Some(id) = active {
                self.save_tab(id, false);
            }
        }
        if pressed(&CLOSE_TAB) || pressed(&CLOSE_TAB_ALT) {
            if let Some(id) = active {
                self.request_close_tab(id);
            }
        }
        if pressed(&TOGGLE_RESULTS) {
            if let Some(t) = self.tabs.get_mut(self.active) {
                t.show_results = !t.show_results;
            }
        }
        if pressed(&TOGGLE_EXPLORER) {
            self.show_explorer = !self.show_explorer;
        }
        if pressed(&NEXT_TAB) && !self.tabs.is_empty() {
            self.active = (self.active + 1) % self.tabs.len();
        }
    }

    fn menu_bar(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let active = self.tabs.get(self.active).map(|t| t.id);
        egui::MenuBar::new().ui(ui, |ui| {
            ui.menu_button("File", |ui| {
                if ui.add(egui::Button::new("Connect Object Explorer…")).clicked() {
                    self.open_connect_dialog(false, None);
                }
                if ui.add(egui::Button::new("New Query").shortcut_text(ctx.format_shortcut(&NEW_QUERY))).clicked() {
                    if self.servers.is_empty() {
                        self.open_connect_dialog(true, None);
                    } else {
                        self.new_query(&ctx, None, None, "");
                    }
                }
                ui.separator();
                if ui.add(egui::Button::new("Open File…").shortcut_text(ctx.format_shortcut(&OPEN))).clicked() {
                    self.open_file();
                }
                ui.add_enabled_ui(active.is_some(), |ui| {
                    if ui.add(egui::Button::new("Save").shortcut_text(ctx.format_shortcut(&SAVE))).clicked() {
                        self.save_tab(active.unwrap(), false);
                    }
                    if ui.button("Save As…").clicked() {
                        self.save_tab(active.unwrap(), true);
                    }
                    if ui.add(egui::Button::new("Close").shortcut_text(ctx.format_shortcut(&CLOSE_TAB))).clicked() {
                        self.request_close_tab(active.unwrap());
                    }
                });
                ui.separator();
                if ui.button("Exit").clicked() {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            });
            ui.menu_button("Query", |ui| {
                ui.add_enabled_ui(active.is_some(), |ui| {
                    let id = active.unwrap_or_default();
                    if ui.add(egui::Button::new("Execute").shortcut_text(ctx.format_shortcut(&EXECUTE))).clicked() {
                        self.execute(&ctx, id);
                    }
                    if ui.add(egui::Button::new("Cancel Executing Query").shortcut_text(ctx.format_shortcut(&CANCEL))).clicked() {
                        self.cancel(id);
                    }
                    ui.separator();
                    if ui.button("Change Connection…").clicked() {
                        self.open_connect_dialog(false, Some(id));
                    }
                    if ui.add(egui::Button::new("Show Results Pane").shortcut_text(ctx.format_shortcut(&TOGGLE_RESULTS))).clicked() {
                        if let Some(t) = self.tab_mut(id) {
                            t.show_results = !t.show_results;
                        }
                    }
                    if ui.button("Save Results As…").clicked() {
                        let csv = self.tabs.get(self.active).and_then(|t| match t.view {
                            ResultsView::Set(i) => t.results.get(i).map(|r| r.to_csv()),
                            ResultsView::Messages => t.results.first().map(|r| r.to_csv()),
                        });
                        match csv {
                            Some(csv) => self.export_csv(csv),
                            None => self.notify("There are no results to save.", true),
                        }
                    }
                });
            });
            ui.menu_button("View", |ui| {
                if ui.add(egui::Button::new("Object Explorer").selected(self.show_explorer).shortcut_text(ctx.format_shortcut(&TOGGLE_EXPLORER))).clicked() {
                    self.show_explorer = !self.show_explorer;
                }
                ui.separator();
                if ui.radio(self.settings.dark, "Dark theme").clicked() {
                    self.settings.dark = true;
                    ctx.set_theme(egui::Theme::Dark);
                    self.settings.save();
                }
                if ui.radio(!self.settings.dark, "Light theme").clicked() {
                    self.settings.dark = false;
                    ctx.set_theme(egui::Theme::Light);
                    self.settings.save();
                }
                ui.separator();
                ui.horizontal(|ui| {
                    ui.label("Font size");
                    if ui.add(egui::DragValue::new(&mut self.settings.font_size).range(9.0..=28.0).speed(0.2)).changed() {
                        apply_font_size(&ctx, self.settings.font_size);
                        self.settings.save();
                    }
                });
                ui.horizontal(|ui| {
                    ui.label("Max rows per result");
                    if ui.add(egui::DragValue::new(&mut self.settings.max_rows).range(100..=10_000_000).speed(1000)).changed() {
                        self.settings.save();
                    }
                });
            });
            ui.menu_button("Help", |ui| {
                if ui.button("Keyboard Shortcuts / About").clicked() {
                    self.show_about = true;
                }
            });
        });
    }

    fn toolbar(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        ui.horizontal(|ui| {
            if ui.button("📄 New Query").on_hover_text("New query window (Ctrl+N)").clicked() {
                if self.servers.is_empty() {
                    self.open_connect_dialog(true, None);
                } else {
                    self.new_query(&ctx, None, None, "");
                }
            }
            if ui.button("📂").on_hover_text("Open file (Ctrl+O)").clicked() {
                self.open_file();
            }
            let active = self.tabs.get(self.active).map(|t| t.id);
            if let Some(id) = active {
                if ui.button("💾").on_hover_text("Save (Ctrl+S)").clicked() {
                    self.save_tab(id, false);
                }
                ui.separator();
                let (running, server, current_db) = {
                    let t = &self.tabs[self.active];
                    (t.running, t.server, t.database.clone())
                };
                let dbs = server.and_then(|s| self.server(s)).map(|s| s.databases.clone()).unwrap_or_default();
                let mut chosen = None;
                ui.add_enabled_ui(!running && server.is_some(), |ui| {
                    egui::ComboBox::from_id_salt("dbpick")
                        .width(180.0)
                        .selected_text(if current_db.is_empty() { "(no database)".to_string() } else { current_db.clone() })
                        .show_ui(ui, |ui| {
                            for db in &dbs {
                                if ui.selectable_label(*db == current_db, db).clicked() {
                                    chosen = Some(db.clone());
                                }
                            }
                        })
                        .response
                        .on_hover_text("Available databases");
                });
                if let Some(db) = chosen {
                    self.change_database(id, db);
                }
                let exec = ui.add_enabled(!running, egui::Button::new(RichText::new("▶ Execute").color(Color32::from_rgb(60, 170, 90))));
                if exec.on_hover_text("Execute (F5). Runs the selection if text is selected.").clicked() {
                    self.execute(&ctx, id);
                }
                if ui.add_enabled(running, egui::Button::new(RichText::new("⏹").color(Color32::from_rgb(220, 70, 60)))).on_hover_text("Cancel executing query (Alt+Break)").clicked() {
                    self.cancel(id);
                }
                if ui.button("🔌").on_hover_text("Change connection").clicked() {
                    self.open_connect_dialog(false, Some(id));
                }
            }
        });
    }

    fn explorer_ui(&mut self, ui: &mut egui::Ui) {
        let mut actions = Vec::new();
        ui.horizontal(|ui| {
            ui.strong("Object Explorer");
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if let Some((s, n)) = self.selected_node {
                    if ui.small_button("⟳").on_hover_text("Refresh").clicked() {
                        actions.push(Action::Refresh { server: s, node: n });
                    }
                    if ui.small_button("⏏").on_hover_text("Disconnect").clicked() {
                        actions.push(Action::Disconnect { server: s });
                    }
                }
                if ui.small_button("🔌 Connect").clicked() {
                    self.open_connect_dialog(false, None);
                }
            });
        });
        ui.separator();
        egui::ScrollArea::both().auto_shrink([false, false]).show(ui, |ui| {
            if self.servers.is_empty() {
                ui.weak("Not connected. Use Connect to add a server.");
            }
            for s in &mut self.servers {
                match &s.state {
                    ServerState::Connecting => {
                        ui.horizontal(|ui| {
                            ui.spinner();
                            ui.label(format!("Connecting to {}…", s.session.info.server.trim()));
                        });
                    }
                    ServerState::Failed(_) => {}
                    ServerState::Connected => {
                        node_ui(ui, s.id, &mut s.root, &mut self.selected_node, &mut actions);
                    }
                }
            }
        });
        let ctx = ui.ctx().clone();
        for a in actions {
            self.apply(&ctx, a);
        }
    }

    fn tabs_ui(&mut self, ui: &mut egui::Ui) {
        let mut close = None;
        egui::ScrollArea::horizontal().id_salt("tabstrip").show(ui, |ui| {
            ui.horizontal(|ui| {
                for (i, t) in self.tabs.iter().enumerate() {
                    let mut title = t.title.clone();
                    if t.dirty() {
                        title.push_str(" *");
                    }
                    if let Some(info) = &t.server_info {
                        title = format!("{title} - {} ({})", info.login, info.spid);
                    }
                    if t.running {
                        title = format!("⏳ {title}");
                    }
                    let r = ui.selectable_label(i == self.active, title);
                    if r.clicked() {
                        self.active = i;
                    }
                    if r.middle_clicked() {
                        close = Some(t.id);
                    }
                    if ui.small_button("🗙").on_hover_text("Close").clicked() {
                        close = Some(t.id);
                    }
                    ui.add_space(6.0);
                }
            });
        });
        if let Some(id) = close {
            self.request_close_tab(id);
        }
    }

    fn query_ui(&mut self, ui: &mut egui::Ui) {
        let Some(tab) = self.tabs.get_mut(self.active) else {
            ui.centered_and_justified(|ui| {
                ui.weak("Press Ctrl+N or click New Query to open a query window.");
            });
            return;
        };
        let mut export = None;

        egui::Panel::bottom(Id::unique(("status", tab.id))).exact_size(24.0).show(ui, |ui| status_bar(ui, tab));

        if tab.show_results {
            egui::Panel::bottom(Id::unique(("results", tab.id)))
                .resizable(true)
                .default_size(ui.available_height() * 0.45)
                .min_size(80.0)
                .show(ui, |ui| results_ui(ui, tab, &mut export));
        }
        egui::CentralPanel::no_frame().show(ui, |ui| editor_ui(ui, tab));

        if let Some(csv) = export {
            self.export_csv(csv);
        }
    }

    fn connect_dialog(&mut self, ctx: &egui::Context) {
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
            ui.set_width(460.0);
            ui.heading("Connect to Server");
            ui.add_space(6.0);
            egui::Grid::new("connect_grid").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
                ui.label("Server type:");
                ui.label("Database Engine");
                ui.end_row();

                ui.label("Server name:");
                ui.horizontal(|ui| {
                    let r = ui.add(egui::TextEdit::singleline(&mut d.info.server).desired_width(260.0).hint_text(r"host, host\instance or host,port"));
                    if d.info.server.is_empty() && !r.has_focus() {
                        r.request_focus();
                    }
                    ui.menu_button("▾", |ui| {
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
                egui::ComboBox::from_id_salt("auth").width(260.0).selected_text(d.info.auth.label()).show_ui(ui, |ui| {
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
                ui.add(egui::TextEdit::singleline(&mut d.info.user).hint_text(user_hint).desired_width(260.0));
                ui.end_row();

                ui.label("Password:");
                let pw = ui.add(egui::TextEdit::singleline(&mut d.info.password).password(true).hint_text(pw_hint).desired_width(260.0));
                if pw.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                    go = true;
                }
                ui.end_row();

                ui.label("");
                ui.checkbox(&mut d.info.remember_password, "Remember password (system keyring)");
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
                            ui.label(RichText::new(format!("✔ Signed in to the domain as {p}")).color(Color32::from_rgb(70, 170, 90)));
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
            egui::CollapsingHeader::new("Options").default_open(d.show_options).show(ui, |ui| {
                egui::Grid::new("connect_opts").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
                    ui.label("Database:");
                    ui.add(egui::TextEdit::singleline(&mut d.info.database).hint_text("<default>").desired_width(240.0));
                    ui.end_row();
                    ui.label("Encrypt:");
                    egui::ComboBox::from_id_salt("encrypt").width(240.0).selected_text(d.info.encrypt.label()).show_ui(ui, |ui| {
                        for e in db::Encrypt::ALL {
                            ui.selectable_value(&mut d.info.encrypt, e, e.label());
                        }
                    });
                    ui.end_row();
                    ui.label("");
                    ui.checkbox(&mut d.info.trust_cert, "Trust server certificate");
                    ui.end_row();
                    ui.label("Connection timeout:");
                    ui.add(egui::DragValue::new(&mut d.info.timeout_secs).range(1..=600).suffix(" s"));
                    ui.end_row();
                    ui.label("Application name:");
                    ui.add(egui::TextEdit::singleline(&mut d.info.app_name).desired_width(240.0));
                    ui.end_row();
                    if d.info.auth == db::Auth::Kerberos {
                        ui.label("Service principal (SPN):");
                        ui.add(egui::TextEdit::singleline(&mut d.info.spn).hint_text("automatic: MSSQLSvc/<fqdn>:<port>").desired_width(240.0));
                        ui.end_row();
                    }
                });
            });
            ui.add_space(8.0);
            ui.horizontal(|ui| {
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

    fn confirm_dialog(&mut self, ctx: &egui::Context) {
        let Some(c) = &self.confirm else { return };
        let text = match c {
            Confirm::CloseTab(id) => {
                let name = self.tabs.iter().find(|t| t.id == *id).map(|t| t.title.clone()).unwrap_or_default();
                format!("Save changes to {name}?")
            }
            Confirm::Quit => "Some query windows have unsaved changes. Quit anyway?".to_string(),
        };
        let mut choice = None;
        egui::Modal::new(Id::unique("confirm")).show(ctx, |ui| {
            ui.set_width(380.0);
            ui.label(text);
            ui.add_space(10.0);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.button("Cancel").clicked() {
                    choice = Some(0);
                }
                match c {
                    Confirm::CloseTab(_) => {
                        if ui.button("Don't Save").clicked() {
                            choice = Some(1);
                        }
                        if ui.button("Save").clicked() {
                            choice = Some(2);
                        }
                    }
                    Confirm::Quit => {
                        if ui.button("Quit").clicked() {
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

    fn about_dialog(&mut self, ctx: &egui::Context) {
        if !self.show_about {
            return;
        }
        let r = egui::Modal::new(Id::unique("about")).show(ctx, |ui| {
            ui.heading(format!("SQLCraft {}", env!("CARGO_PKG_VERSION")));
            ui.label("A SQL Server management studio for Linux.");
            ui.add_space(8.0);
            egui::Grid::new("keys").num_columns(2).striped(true).show(ui, |ui| {
                for (k, v) in [
                    ("F5 / Ctrl+E", "Execute (selection or whole script)"),
                    ("Alt+Break", "Cancel executing query"),
                    ("Ctrl+N", "New query window"),
                    ("Ctrl+O / Ctrl+S", "Open / save file"),
                    ("Ctrl+W / Ctrl+F4", "Close query window"),
                    ("Ctrl+Tab", "Next query window"),
                    ("Ctrl+R", "Show / hide results pane"),
                    ("F8", "Show / hide Object Explorer"),
                    ("Ctrl+C in grid", "Copy selected cell"),
                ] {
                    ui.monospace(k);
                    ui.label(v);
                    ui.end_row();
                }
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

    fn toast_ui(&mut self, ctx: &egui::Context) {
        let Some((text, error, at)) = &self.toast else { return };
        if at.elapsed() > Duration::from_secs(if *error { 10 } else { 4 }) {
            self.toast = None;
            return;
        }
        let mut close = false;
        egui::Area::new(Id::unique("toast"))
            .anchor(egui::Align2::RIGHT_BOTTOM, [-16.0, -36.0])
            .show(ctx, |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    ui.set_max_width(420.0);
                    ui.horizontal(|ui| {
                        let color = if *error { Color32::from_rgb(220, 80, 70) } else { Color32::from_rgb(70, 170, 90) };
                        ui.label(RichText::new(if *error { "⚠" } else { "✔" }).color(color));
                        ui.label(text.as_str());
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

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.pump(&ctx);
        self.shortcuts(&ctx);

        if ctx.input(|i| i.viewport().close_requested()) && !self.allow_quit {
            if self.tabs.iter().any(|t| t.dirty() && !t.sql.trim().is_empty() && t.path.is_some()) {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                self.confirm = Some(Confirm::Quit);
            }
        }

        let title = match self.tabs.get(self.active) {
            Some(t) => format!("{} - SQLCraft", t.title),
            None => "SQLCraft".to_string(),
        };
        ctx.send_viewport_cmd(egui::ViewportCommand::Title(title));

        egui::Panel::top("menu").show(ui, |ui| {
            self.menu_bar(ui);
            self.toolbar(ui);
        });
        if self.show_explorer {
            egui::Panel::left("explorer")
                .resizable(true)
                .default_size(300.0)
                .min_size(160.0)
                .show(ui, |ui| self.explorer_ui(ui));
        }
        egui::CentralPanel::default_margins().show(ui, |ui| {
            if !self.tabs.is_empty() {
                self.tabs_ui(ui);
                ui.separator();
            }
            self.query_ui(ui);
        });

        self.connect_dialog(&ctx);
        self.confirm_dialog(&ctx);
        self.about_dialog(&ctx);
        self.toast_ui(&ctx);

        if self.tabs.iter().any(|t| t.running) {
            ctx.request_repaint_after(Duration::from_millis(250));
        }
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        // Untitled scripts are restored on the next start (like SSMS auto-recover).
        let tabs: Vec<SavedTab> = self
            .tabs
            .iter()
            .filter(|t| !t.sql.trim().is_empty() || t.path.is_some())
            .map(|t| SavedTab { title: t.title.clone(), path: t.path.clone(), sql: t.sql.clone(), saved_sql: t.saved_sql.clone() })
            .collect();
        if let Ok(s) = serde_json::to_string(&tabs) {
            storage.set_string("tabs", s);
        }
    }
}

// ----- Free-standing widgets ---------------------------------------------

fn apply_font_size(ctx: &egui::Context, size: f32) {
    ctx.all_styles_mut(|style| {
        for (ts, font) in style.text_styles.iter_mut() {
            font.size = match ts {
                TextStyle::Small => size * 0.75,
                TextStyle::Heading => size * 1.4,
                _ => size,
            };
        }
    });
}

fn decode_text(bytes: &[u8]) -> String {
    // SSMS saves UTF-16 LE with a BOM by default.
    if bytes.starts_with(&[0xFF, 0xFE]) {
        let units: Vec<u16> = bytes[2..].chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
        return String::from_utf16_lossy(&units);
    }
    if bytes.starts_with(&[0xFE, 0xFF]) {
        let units: Vec<u16> = bytes[2..].chunks_exact(2).map(|c| u16::from_be_bytes([c[0], c[1]])).collect();
        return String::from_utf16_lossy(&units);
    }
    let bytes = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    String::from_utf8_lossy(bytes).replace("\r\n", "\n")
}

fn node_ui_id(server: u64, node: u64) -> Id {
    Id::unique(("oe", server, node))
}

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
                    actions.push(Action::Script {
                        server,
                        db: db.clone(),
                        schema: schema.clone(),
                        name: name.clone(),
                        kind: ScriptKind::Modify,
                        run: false,
                    });
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
                        ("CREATE To New Query Window", ScriptKind::Create),
                        ("SELECT To New Query Window", ScriptKind::SelectTop(1000)),
                        ("INSERT To New Query Window", ScriptKind::Insert),
                        ("UPDATE To New Query Window", ScriptKind::Update),
                        ("DELETE To New Query Window", ScriptKind::Delete),
                        ("DROP To New Query Window", ScriptKind::Drop("TABLE")),
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
                    if ui.button("CREATE To New Query Window").clicked() {
                        actions.push(script(db, schema, name, ScriptKind::Definition, false));
                    }
                    if ui.button("ALTER To New Query Window").clicked() {
                        actions.push(script(db, schema, name, ScriptKind::Modify, false));
                    }
                    if ui.button("DROP To New Query Window").clicked() {
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
                    if ui.button("CREATE To New Query Window").clicked() {
                        actions.push(script(db, schema, name, ScriptKind::Definition, false));
                    }
                    if ui.button("DROP To New Query Window").clicked() {
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
                    if ui.button("CREATE To New Query Window").clicked() {
                        actions.push(script(db, schema, name, ScriptKind::Definition, false));
                    }
                    if ui.button("DROP To New Query Window").clicked() {
                        actions.push(script(db, schema, name, ScriptKind::Drop("FUNCTION"), false));
                    }
                });
                ui.separator();
            }
            _ => {}
        }
        if matches!(node.kind, Kind::Server | Kind::Database { .. }) || node.kind.database().is_some() {
            if ui.button("New Query").clicked() {
                actions.push(Action::NewQuery { server, db: node.kind.database().map(String::from) });
            }
        }
        if node.kind.expandable() && ui.button("Refresh").clicked() {
            actions.push(Action::Refresh { server, node: node.id });
        }
        if matches!(node.kind, Kind::Server) && ui.button("Disconnect").clicked() {
            actions.push(Action::Disconnect { server });
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

/// Approximate monospace layout for T-SQL with syntax colours.
fn editor_ui(ui: &mut egui::Ui, tab: &mut QueryTab) {
    let theme = egui_extras::syntax_highlighting::CodeTheme::from_memory(ui.ctx(), ui.style());
    let mut layouter = |ui: &egui::Ui, text: &dyn egui::TextBuffer, _wrap: f32| {
        let mut job = egui_extras::syntax_highlighting::highlight(ui.ctx(), ui.style(), &theme, text.as_str(), "sql");
        job.wrap.max_width = f32::INFINITY;
        ui.fonts_mut(|f| f.layout_job(job))
    };
    let lines = tab.sql.matches('\n').count() + 1;
    egui::ScrollArea::both().id_salt(("editor", tab.id)).auto_shrink([false, false]).show(ui, |ui| {
        ui.horizontal_top(|ui| {
            let gutter: String = (1..=lines).map(|n| format!("{n:>4}\n")).collect();
            ui.add(
                egui::Label::new(RichText::new(gutter.trim_end()).monospace().weak())
                    .selectable(false)
                    .wrap_mode(egui::TextWrapMode::Extend),
            );
            let out = egui::TextEdit::multiline(&mut tab.sql)
                .id(Id::unique(("sql", tab.id)))
                .code_editor()
                .frame(egui::Frame::NONE)
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
            }
        });
    });
}

fn status_bar(ui: &mut egui::Ui, tab: &QueryTab) {
    ui.horizontal_centered(|ui| {
        let (icon, text, color) = if tab.running {
            ("⏳", "Executing query…", ui.visuals().text_color())
        } else if let Some(e) = &tab.connect_error {
            ("⚠", e.as_str(), Color32::from_rgb(220, 80, 70))
        } else {
            match tab.outcome {
                Some(Outcome::Success) => ("✔", "Query executed successfully.", Color32::from_rgb(70, 170, 90)),
                Some(Outcome::Errors) => ("⚠", "Query completed with errors.", Color32::from_rgb(220, 80, 70)),
                Some(Outcome::Cancelled) => ("⏹", "Query was cancelled by user.", Color32::from_rgb(220, 160, 60)),
                None if tab.session.is_none() => ("○", "Disconnected.", ui.visuals().weak_text_color()),
                None if tab.server_info.is_none() => ("…", "Connecting…", ui.visuals().text_color()),
                None => ("●", "Connected.", Color32::from_rgb(70, 170, 90)),
            }
        };
        ui.label(RichText::new(icon).color(color));
        ui.label(RichText::new(text).color(color));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let elapsed = if tab.running {
                tab.started.map(|s| s.elapsed())
            } else {
                tab.elapsed
            };
            let rows = tab.total_rows();
            ui.monospace(format!("{rows} row{}", if rows == 1 { "" } else { "s" }));
            ui.separator();
            if let Some(e) = elapsed {
                let s = e.as_secs();
                ui.monospace(format!("{:02}:{:02}:{:02}", s / 3600, (s / 60) % 60, s % 60));
                ui.separator();
            }
            if let Some(info) = &tab.server_info {
                ui.label(&tab.database);
                ui.separator();
                ui.label(format!("{} ({})", info.login, info.spid));
                ui.separator();
                ui.label(format!("{} ({})", info.server_name, info.version)).on_hover_text(&info.edition);
            }
        });
    });
}

fn results_ui(ui: &mut egui::Ui, tab: &mut QueryTab, export: &mut Option<String>) {
    ui.horizontal(|ui| {
        for i in 0..tab.results.len() {
            let label = if tab.results.len() == 1 { "▦ Results".to_string() } else { format!("▦ Results {}", i + 1) };
            ui.selectable_value(&mut tab.view, ResultsView::Set(i), label);
        }
        let errors = tab.messages.iter().any(|m| m.is_error);
        let msg_label = if errors { RichText::new("💬 Messages").color(Color32::from_rgb(220, 80, 70)) } else { RichText::new("💬 Messages") };
        ui.selectable_value(&mut tab.view, ResultsView::Messages, msg_label);
    });
    ui.separator();
    match tab.view {
        ResultsView::Messages => {
            egui::ScrollArea::both().id_salt(("msgs", tab.id)).auto_shrink([false, false]).stick_to_bottom(tab.running).show(ui, |ui| {
                for m in &tab.messages {
                    let text = RichText::new(&m.text).monospace();
                    let text = if m.is_error { text.color(Color32::from_rgb(230, 80, 70)) } else { text };
                    ui.add(egui::Label::new(text).selectable(true));
                }
            });
        }
        ResultsView::Set(i) => {
            if let Some(set) = tab.results.get(i) {
                grid_ui(ui, tab.id, i, set, &mut tab.cell, export);
            }
        }
    }
}

fn grid_ui(ui: &mut egui::Ui, tab_id: u64, set_idx: usize, set: &ResultSet, cell: &mut Option<(usize, usize, usize)>, export: &mut Option<String>) {
    let row_h = ui.text_style_height(&TextStyle::Monospace) + 6.0;
    let null_bg = if ui.visuals().dark_mode { Color32::from_rgb(70, 66, 30) } else { Color32::from_rgb(255, 255, 225) };
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
    let mut table = TableBuilder::new(ui)
        .id_salt(("grid", tab_id, set_idx))
        .striped(true)
        .resizable(true)
        .auto_shrink([false, false])
        .cell_layout(Layout::left_to_right(Align::Center))
        .column(Column::exact(52.0));
    for w in &set.widths {
        table = table.column(Column::initial(*w).at_least(30.0).clip(true).resizable(true));
    }
    table
        .header(row_h + 2.0, |mut header| {
            header.col(|_| {});
            for c in &set.columns {
                header.col(|ui| {
                    ui.strong(&c.name).on_hover_text(c.type_name);
                });
            }
        })
        .body(|body| {
            body.rows(row_h, set.rows.len(), |mut row| {
                let r = row.index();
                row.set_selected(current.is_some_and(|c| c.1 == r));
                row.col(|ui| {
                    ui.weak(format!("{}", r + 1));
                });
                for (c, value) in set.rows[r].iter().enumerate() {
                    let numeric = set.columns[c].numeric;
                    let (rect, resp) = row.col(|ui| {
                        let is_cell = current.is_some_and(|x| x.1 == r && x.2 == c);
                        if is_cell {
                            ui.painter().rect_filled(ui.max_rect(), 0.0, selected_bg);
                        }
                        let layout = if numeric { Layout::right_to_left(Align::Center) } else { Layout::left_to_right(Align::Center) };
                        ui.with_layout(layout, |ui| match value {
                            None => {
                                ui.painter().rect_filled(ui.max_rect(), 0.0, null_bg);
                                ui.label(RichText::new("NULL").monospace().italics());
                            }
                            Some(v) => {
                                let shown: String = v.chars().take(400).map(|ch| if ch == '\n' || ch == '\r' || ch == '\t' { ' ' } else { ch }).collect();
                                ui.add(egui::Label::new(RichText::new(shown).monospace()).truncate().selectable(false));
                            }
                        });
                    });
                    let _ = rect;
                    let resp = resp.interact(Sense::click());
                    if resp.clicked() || resp.secondary_clicked() {
                        clicked = Some((set_idx, r, c));
                    }
                    resp.context_menu(|ui| {
                        if ui.button("Copy").clicked() {
                            ui.ctx().copy_text(value.clone().unwrap_or_else(|| "NULL".into()));
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
                        if ui.button("Copy All with Headers").clicked() {
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
