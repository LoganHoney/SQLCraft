//! Application state and the actions behind menus, toolbars and shortcuts.
//! The UI lives in `ui.rs` (shell, tool windows), `query_ui.rs` (query
//! windows) and `dialogs.rs`.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc as std_mpsc;
use std::time::{Duration, Instant};

use eframe::egui::{self, Color32, collapsing_header::CollapsingState};
use egui_dock::{DockState, NodeIndex};
use serde::{Deserialize, Serialize};

use crate::batch::{self, Batch};
use crate::db::{ColumnInfo, ConnectionInfo, Event, Message, Request, ServerInfo, Session, Table};
use crate::explorer::{self, Kind, Node};
use crate::format::{csv_field, lit, qi};
use crate::settings::{self, Settings};
use crate::templates::Param;
use crate::theme;
use crate::tsql;

/// Tags at or above this are fetches whose result nobody waits for.
const FIRE_AND_FORGET: u64 = u64::MAX / 2;

// ----- Model --------------------------------------------------------------

#[derive(PartialEq)]
pub(crate) enum ServerState {
    Connecting,
    Connected,
    Failed(String),
}

pub(crate) struct Server {
    pub id: u64,
    pub session: Session,
    pub info: Option<ServerInfo>,
    pub state: ServerState,
    pub root: Node,
    pub databases: Vec<String>,
}

pub(crate) struct ResultSet {
    pub columns: Vec<ColumnInfo>,
    pub rows: Vec<Vec<Option<String>>>,
    pub widths: Vec<f32>,
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

    pub fn header_line(&self, sep: &str) -> String {
        self.columns.iter().map(|c| c.name.as_str()).collect::<Vec<_>>().join(sep)
    }

    pub fn row_line(&self, r: usize, sep: &str) -> String {
        self.rows[r].iter().map(|v| v.as_deref().unwrap_or("NULL")).collect::<Vec<_>>().join(sep)
    }

    pub fn to_csv(&self) -> String {
        let mut s = self.columns.iter().map(|c| csv_field(&c.name)).collect::<Vec<_>>().join(",");
        s.push_str("\r\n");
        for row in &self.rows {
            let line: Vec<_> = row.iter().map(|v| v.as_deref().map(csv_field).unwrap_or_default()).collect();
            s.push_str(&line.join(","));
            s.push_str("\r\n");
        }
        s
    }

    /// The fixed-width layout of "Results to Text".
    pub fn to_text(&self) -> String {
        const MAX: usize = 256;
        let cell = |v: &Option<String>| -> String {
            v.as_deref().unwrap_or("NULL").replace(['\r', '\n', '\t'], " ").chars().take(MAX).collect()
        };
        let mut widths: Vec<usize> = self.columns.iter().map(|c| c.name.chars().count().min(MAX)).collect();
        for row in &self.rows {
            for (w, v) in widths.iter_mut().zip(row) {
                *w = (*w).max(cell(v).chars().count());
            }
        }
        let pad = |s: &str, w: usize, right: bool| {
            if right { format!("{s:>w$}") } else { format!("{s:<w$}") }
        };
        let mut out = String::new();
        let header: Vec<_> = self.columns.iter().zip(&widths).map(|(c, w)| pad(&c.name, *w, false)).collect();
        out.push_str(header.join(" ").trim_end());
        out.push('\n');
        let dashes: Vec<_> = widths.iter().map(|w| "-".repeat(*w)).collect();
        out.push_str(&dashes.join(" "));
        out.push('\n');
        for row in &self.rows {
            let line: Vec<_> = row
                .iter()
                .zip(&widths)
                .zip(&self.columns)
                .map(|((v, w), c)| pad(&cell(v), *w, c.numeric))
                .collect();
            out.push_str(line.join(" ").trim_end());
            out.push('\n');
        }
        out
    }
}

fn text_width(s: &str) -> f32 {
    let chars = s.lines().next().unwrap_or("").chars().count().min(60) as f32;
    (chars * 7.5 + 18.0).clamp(48.0, 420.0)
}

#[derive(PartialEq, Clone, Copy)]
pub(crate) enum Outcome {
    Success,
    Errors,
    Cancelled,
}

#[derive(PartialEq, Clone, Copy, Debug)]
pub(crate) enum ResultsMode {
    Grid,
    Text,
    File,
}

#[derive(PartialEq, Clone, Copy)]
pub(crate) enum ResultsView {
    Results,
    Messages,
}

/// What arrived, in order, so Results to Text can interleave sets and messages.
#[derive(Clone, Copy)]
pub(crate) enum Item {
    Set(usize),
    Message(usize),
}

pub(crate) struct FindBar {
    pub needle: String,
    pub replace: Option<String>,
    pub match_case: bool,
    pub focus: bool,
}

pub(crate) struct QueryTab {
    pub id: u64,
    pub title: String,
    pub path: Option<PathBuf>,
    pub sql: String,
    pub saved_sql: String,
    pub server: Option<u64>,
    pub session: Option<Session>,
    pub server_info: Option<ServerInfo>,
    pub database: String,
    pub connect_error: Option<String>,
    pub running: bool,
    pub started: Option<Instant>,
    pub elapsed: Option<Duration>,
    pub outcome: Option<Outcome>,
    pub results: Vec<ResultSet>,
    pub messages: Vec<Message>,
    pub order: Vec<Item>,
    pub view: ResultsView,
    pub mode: ResultsMode,
    pub show_results: bool,
    /// Character range selected in the editor.
    pub selection: Option<(usize, usize)>,
    /// Selection to put into the editor on the next frame.
    pub set_selection: Option<(usize, usize)>,
    pub scroll_to_cursor: bool,
    /// (result set, row, column) of the selected grid cell.
    pub cell: Option<(usize, usize, usize)>,
    pub focus_editor: bool,
    pub find: Option<FindBar>,
    /// Where "Results to File" writes once the run finishes.
    pub file_target: Option<PathBuf>,
}

impl QueryTab {
    pub fn dirty(&self) -> bool {
        self.sql != self.saved_sql
    }

    pub fn total_rows(&self) -> usize {
        self.results.iter().map(|r| r.rows.len()).sum()
    }

    pub fn editor_id(&self) -> egui::Id {
        egui::Id::unique(("sql", self.id))
    }

    /// SSMS's window caption: `SQLQuery1.sql - SQL01.master (sa (55))*`.
    pub fn caption(&self, server: Option<&str>) -> String {
        let star = if self.dirty() { "*" } else { "" };
        match (&self.server_info, server) {
            (Some(i), Some(s)) => format!("{} - {s}.{} ({} ({})){star}", self.title, self.database, i.login, i.spid),
            (None, Some(s)) if self.session.is_some() => format!("{} - {s} (connecting…){star}", self.title),
            _ => format!("{} - not connected{star}", self.title),
        }
    }

    /// Messages and result sets as one text, for Results to Text / File.
    pub fn text_output(&self) -> String {
        let mut out = String::new();
        for item in &self.order {
            match *item {
                Item::Set(i) => {
                    out.push_str(&self.results[i].to_text());
                    out.push('\n');
                }
                Item::Message(i) => {
                    out.push_str(&self.messages[i].text);
                    out.push('\n');
                }
            }
        }
        out
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
pub(crate) enum ScriptKind {
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
    Properties { server: u64, node: u64 },
}

pub(crate) enum Action {
    Expand { server: u64, node: u64 },
    Refresh { server: u64, node: u64 },
    NewQuery { server: u64, db: Option<String> },
    Script { server: u64, db: String, schema: String, name: String, kind: ScriptKind, run: bool },
    Disconnect { server: u64 },
    CollapseAll,
    OpenTemplate(usize),
}

/// Results of background work that is not tied to one connection.
pub(crate) enum AppMsg {
    Opened(PathBuf, String),
    SaveTo { tab: u64, path: PathBuf },
    ExportCsv { path: PathBuf, csv: String },
    RunToFile { tab: u64, path: PathBuf },
    Password { key: String, password: String },
    /// Who the current Kerberos ticket belongs to.
    Principal(Result<String, String>),
    Error(String),
}

pub(crate) struct ConnectDialog {
    pub info: ConnectionInfo,
    pub show_options: bool,
    /// Open a query window once the connection is made.
    pub then_query: bool,
    /// Connect this tab instead of adding a server to Object Explorer.
    pub for_tab: Option<u64>,
    /// The current Kerberos ticket, once looked up.
    pub principal: Option<Result<String, String>>,
    pub checking_principal: bool,
}

pub(crate) enum Confirm {
    CloseTab(u64),
    Quit,
}

/// What the docking area holds.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Pane {
    ObjectExplorer,
    Properties,
    Output,
    TemplateBrowser,
    Start,
    Query(u64),
}

pub(crate) struct Properties {
    pub node: Option<(u64, u64)>,
    pub title: String,
    pub loading: bool,
    pub rows: Vec<(String, String)>,
}

/// Every command reachable from menus, toolbars and shortcuts.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) enum Cmd {
    ConnectObjectExplorer,
    NewQuery,
    OpenFile,
    Save,
    SaveAs,
    SaveAll,
    Close,
    CloseAll,
    Exit,
    Find,
    Replace,
    FindNext,
    GoTo,
    Comment,
    Uncomment,
    Upper,
    Lower,
    Indent,
    Unindent,
    ShowObjectExplorer,
    ShowProperties,
    ShowOutput,
    ShowTemplates,
    FullScreen,
    Execute,
    Parse,
    Cancel,
    ChangeConnection,
    Disconnect,
    ResultsToGrid,
    ResultsToText,
    ResultsToFile,
    ToggleResults,
    TemplateParams,
    SpHelp,
    SpWho,
    SpLock,
    SaveResults,
    Options,
    ResetLayout,
    NextDocument,
    About,
}

pub struct App {
    pub(crate) rt: tokio::runtime::Runtime,
    pub(crate) settings: Settings,
    pub(crate) servers: Vec<Server>,
    pub(crate) tabs: Vec<QueryTab>,
    /// The query window menus and toolbars act on.
    pub(crate) current: Option<u64>,
    pub(crate) dock: DockState<Pane>,
    next_id: u64,
    query_counter: u32,
    pending: HashMap<u64, Pending>,
    pub(crate) selected_node: Option<(u64, u64)>,
    pub(crate) connect: Option<ConnectDialog>,
    pub(crate) confirm: Option<Confirm>,
    pub(crate) toast: Option<(String, bool, Instant)>,
    pub(crate) show_about: bool,
    pub(crate) show_options: bool,
    pub(crate) goto: Option<String>,
    pub(crate) template_params: Option<(u64, Vec<Param>)>,
    pub(crate) props: Properties,
    pub(crate) output: Vec<String>,
    pub(crate) msg_tx: std_mpsc::Sender<AppMsg>,
    msg_rx: std_mpsc::Receiver<AppMsg>,
    pub(crate) allow_quit: bool,
    /// Ctrl+K was pressed; waiting for the second key of the chord.
    pub(crate) chord: Option<Instant>,
    pub(crate) fullscreen: bool,
}

pub(crate) fn default_dock() -> DockState<Pane> {
    let mut dock = DockState::new(vec![Pane::Start]);
    dock.main_surface_mut().split_left(NodeIndex::root(), 0.22, vec![Pane::ObjectExplorer]);
    dock
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("tokio runtime");
        let settings = Settings::load();
        // Hack covers box-drawing and arrow glyphs the UI font lacks.
        let mut fonts = egui::FontDefinitions::default();
        if let Some(family) = fonts.families.get_mut(&egui::FontFamily::Proportional) {
            family.push("Hack".to_owned());
        }
        cc.egui_ctx.set_fonts(fonts);
        theme::apply(&cc.egui_ctx, settings.theme);
        apply_font_size(&cc.egui_ctx, settings.font_size);
        let (msg_tx, msg_rx) = std_mpsc::channel();
        let mut app = Self {
            rt,
            settings,
            servers: Vec::new(),
            tabs: Vec::new(),
            current: None,
            dock: default_dock(),
            next_id: 1,
            query_counter: 0,
            pending: HashMap::new(),
            selected_node: None,
            connect: None,
            confirm: None,
            toast: None,
            show_about: false,
            show_options: false,
            goto: None,
            template_params: None,
            props: Properties { node: None, title: String::new(), loading: false, rows: Vec::new() },
            output: Vec::new(),
            msg_tx,
            msg_rx,
            allow_quit: false,
            chord: None,
            fullscreen: false,
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
        app.log("SQLCraft started.");
        app.open_connect_dialog(false, None);
        app
    }

    pub(crate) fn id(&mut self) -> u64 {
        self.next_id += 1;
        self.next_id
    }

    pub(crate) fn tab(&self, id: u64) -> Option<&QueryTab> {
        self.tabs.iter().find(|t| t.id == id)
    }

    pub(crate) fn tab_mut(&mut self, id: u64) -> Option<&mut QueryTab> {
        self.tabs.iter_mut().find(|t| t.id == id)
    }

    pub(crate) fn current_tab(&self) -> Option<&QueryTab> {
        self.current.and_then(|id| self.tab(id))
    }

    pub(crate) fn server(&self, id: u64) -> Option<&Server> {
        self.servers.iter().find(|s| s.id == id)
    }

    fn server_mut(&mut self, id: u64) -> Option<&mut Server> {
        self.servers.iter_mut().find(|s| s.id == id)
    }

    pub(crate) fn server_name(&self, tab: &QueryTab) -> Option<String> {
        tab.session.as_ref().map(|s| s.info.server.trim().to_string())
    }

    pub(crate) fn notify(&mut self, text: impl Into<String>, error: bool) {
        let text = text.into();
        self.log(&text);
        self.toast = Some((text, error, Instant::now()));
    }

    /// A line in the Output window.
    pub(crate) fn log(&mut self, text: &str) {
        let ts = chrono::Local::now().format("%H:%M:%S");
        for line in text.lines() {
            self.output.push(format!("[{ts}] {line}"));
        }
        if self.output.len() > 5000 {
            self.output.drain(..1000);
        }
    }

    fn current_server(&self) -> Option<u64> {
        if let Some((s, _)) = self.selected_node {
            if self.server(s).is_some() {
                return Some(s);
            }
        }
        if let Some(s) = self.current_tab().and_then(|t| t.server) {
            if self.server(s).is_some() {
                return Some(s);
            }
        }
        self.servers.first().map(|s| s.id)
    }

    // ----- Docking ------------------------------------------------------------

    /// Shows a tool window, docking it where SSMS puts it if it is closed.
    pub(crate) fn show_pane(&mut self, pane: Pane) {
        if let Some(path) = self.dock.find_tab(&pane) {
            let _ = self.dock.set_active_tab(path);
            self.dock.set_focused_node_and_surface(path.node_path());
            return;
        }
        let tree = self.dock.main_surface_mut();
        match pane {
            Pane::ObjectExplorer => {
                tree.split_left(NodeIndex::root(), 0.22, vec![pane]);
            }
            Pane::Properties | Pane::TemplateBrowser => {
                let other = if pane == Pane::Properties { Pane::TemplateBrowser } else { Pane::Properties };
                match self.dock.find_tab(&other) {
                    Some(path) => {
                        let _ = self.dock.leaf_mut(path.node_path()).map(|leaf| leaf.append_tab(pane));
                    }
                    None => {
                        self.dock.main_surface_mut().split_right(NodeIndex::root(), 0.78, vec![pane]);
                    }
                }
            }
            Pane::Output => {
                tree.split_below(NodeIndex::root(), 0.75, vec![pane]);
            }
            Pane::Start | Pane::Query(_) => self.place_document(pane),
        }
    }

    /// Puts a document next to the other documents (replacing the Start page).
    fn place_document(&mut self, pane: Pane) {
        let anchor = self
            .current
            .and_then(|id| self.dock.find_tab(&Pane::Query(id)))
            .or_else(|| self.dock.find_tab_from(|p| matches!(p, Pane::Query(_))))
            .or_else(|| self.dock.find_tab(&Pane::Start));
        match anchor {
            Some(path) => {
                if let Ok(leaf) = self.dock.leaf_mut(path.node_path()) {
                    leaf.append_tab(pane.clone());
                }
            }
            None => {
                // No document well left: open one to the right of everything.
                if self.dock.main_surface().is_empty() {
                    self.dock = DockState::new(vec![pane.clone()]);
                } else {
                    self.dock.main_surface_mut().split_right(NodeIndex::root(), 0.25, vec![pane.clone()]);
                }
            }
        }
        if pane != Pane::Start {
            if let Some(start) = self.dock.find_tab(&Pane::Start) {
                self.dock.remove_tab(start);
            }
        }
        if let Some(path) = self.dock.find_tab(&pane) {
            let _ = self.dock.set_active_tab(path);
            self.dock.set_focused_node_and_surface(path.node_path());
        }
    }

    /// Keeps the Start page in the document well when no queries are open.
    pub(crate) fn tidy_dock(&mut self) {
        let has_doc = self.dock.find_tab_from(|p| matches!(p, Pane::Query(_) | Pane::Start)).is_some();
        if !has_doc {
            self.place_document(Pane::Start);
        }
        // Query windows whose tab was closed in the dock.
        let open: Vec<u64> = self.dock.iter_all_tabs().filter_map(|(_, p)| if let Pane::Query(id) = p { Some(*id) } else { None }).collect();
        let gone: Vec<u64> = self.tabs.iter().map(|t| t.id).filter(|id| !open.contains(id)).collect();
        for id in gone {
            self.drop_tab(id);
        }
        // Follow focus to pick the current query window.
        if let Some((_, Pane::Query(id))) = self.dock.find_active_focused() {
            self.current = Some(*id);
        }
        if self.current.is_some_and(|id| self.tab(id).is_none()) {
            self.current = self.tabs.last().map(|t| t.id);
        }
    }

    // ----- Connections ----------------------------------------------------

    pub(crate) fn open_connect_dialog(&mut self, then_query: bool, for_tab: Option<u64>) {
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

    pub(crate) fn load_password(&self, key: String) {
        let tx = self.msg_tx.clone();
        self.rt.spawn_blocking(move || {
            if let Some(password) = settings::load_password(&key) {
                let _ = tx.send(AppMsg::Password { key, password });
            }
        });
    }

    pub(crate) fn connect_server(&mut self, ctx: &egui::Context, info: ConnectionInfo) -> u64 {
        let id = self.id();
        let root_id = self.id();
        self.log(&format!("Connecting to {}…", info.server.trim()));
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
        self.show_pane(Pane::ObjectExplorer);
        id
    }

    pub(crate) fn remember_connection(&mut self, info: &ConnectionInfo) {
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

    pub(crate) fn add_tab(&mut self, server: Option<u64>, db: Option<String>) -> u64 {
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
            order: Vec::new(),
            view: ResultsView::Messages,
            mode: if self.settings.results_to_text { ResultsMode::Text } else { ResultsMode::Grid },
            show_results: false,
            selection: None,
            set_selection: None,
            scroll_to_cursor: false,
            cell: None,
            focus_editor: true,
            find: None,
            file_target: None,
        };
        if let Some(s) = server.and_then(|s| self.server(s)) {
            tab.server = Some(s.id);
            tab.database = db.clone().unwrap_or_default();
        }
        self.tabs.push(tab);
        self.current = Some(id);
        self.place_document(Pane::Query(id));
        id
    }

    /// Gives a tab its own connection to `server`, as SSMS does per window.
    pub(crate) fn attach_tab(&mut self, ctx: &egui::Context, tab_id: u64, server: u64, db: Option<String>) {
        let Some(info) = self.server(server).map(|s| s.session.info.clone()) else { return };
        let handle = self.rt.handle().clone();
        let Some(tab) = self.tab_mut(tab_id) else { return };
        let db = db.or_else(|| Some(tab.database.clone()).filter(|d| !d.is_empty()));
        tab.server = Some(server);
        tab.session = Some(Session::open(&handle, info, db, ctx.clone()));
        tab.server_info = None;
        tab.connect_error = None;
    }

    pub(crate) fn new_query(&mut self, ctx: &egui::Context, server: Option<u64>, db: Option<String>, sql: &str) -> u64 {
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

    /// Makes sure the tab has a connection; returns false if the user must connect first.
    fn ensure_session(&mut self, ctx: &egui::Context, tab_id: u64) -> bool {
        if self.tab(tab_id).is_some_and(|t| t.session.is_some()) {
            return true;
        }
        match self.tab(tab_id).and_then(|t| t.server).or_else(|| self.current_server()) {
            Some(server) => {
                self.attach_tab(ctx, tab_id, server, None);
                true
            }
            None => {
                self.open_connect_dialog(false, Some(tab_id));
                false
            }
        }
    }

    /// Sends batches to the tab's connection and resets its results.
    fn run(&mut self, ctx: &egui::Context, tab_id: u64, batches: Vec<Batch>) {
        if !self.ensure_session(ctx, tab_id) {
            return;
        }
        let max_rows = self.settings.max_rows;
        let Some(tab) = self.tab_mut(tab_id) else { return };
        if tab.running {
            return;
        }
        tab.results.clear();
        tab.messages.clear();
        tab.order.clear();
        tab.cell = None;
        tab.show_results = true;
        tab.view = ResultsView::Messages;
        tab.outcome = None;
        tab.elapsed = None;
        if batches.is_empty() {
            tab.messages.push(Message { text: "Commands completed successfully.".into(), is_error: false });
            tab.order.push(Item::Message(0));
            tab.outcome = Some(Outcome::Success);
            return;
        }
        tab.running = true;
        tab.started = Some(Instant::now());
        if let Some(s) = &tab.session {
            s.send(Request::Run { batches, max_rows });
        }
    }

    /// The editor text to run: the selection if there is one, else everything.
    fn script_to_run(tab: &QueryTab) -> Vec<Batch> {
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
        batches
    }

    pub(crate) fn execute(&mut self, ctx: &egui::Context, tab_id: u64) {
        let Some(tab) = self.tab(tab_id) else { return };
        if tab.mode == ResultsMode::File && tab.file_target.is_none() {
            // SSMS asks for the file before running.
            let tx = self.msg_tx.clone();
            let name = tab.title.trim_end_matches(".sql").to_string();
            self.rt.spawn(async move {
                if let Some(f) = rfd::AsyncFileDialog::new()
                    .set_title("Save Results")
                    .set_file_name(format!("{name}.rpt"))
                    .add_filter("Report Files", &["rpt"])
                    .add_filter("All files", &["*"])
                    .save_file()
                    .await
                {
                    let _ = tx.send(AppMsg::RunToFile { tab: tab_id, path: f.path().to_path_buf() });
                }
            });
            return;
        }
        let batches = Self::script_to_run(tab);
        self.run(ctx, tab_id, batches);
    }

    /// Ctrl+F5: checks syntax without running anything.
    fn parse(&mut self, ctx: &egui::Context, tab_id: u64) {
        let Some(tab) = self.tab(tab_id) else { return };
        let mut batches = Self::script_to_run(tab);
        if batches.is_empty() {
            return;
        }
        batches.insert(0, Batch { sql: "SET PARSEONLY ON".into(), start_line: 1, repeat: 1 });
        batches.push(Batch { sql: "SET PARSEONLY OFF".into(), start_line: 1, repeat: 1 });
        self.run(ctx, tab_id, batches);
    }

    /// Alt+F1, Ctrl+1, Ctrl+2: SSMS's built-in query shortcuts.
    fn query_shortcut(&mut self, ctx: &egui::Context, tab_id: u64, cmd: Cmd) {
        let Some(tab) = self.tab(tab_id) else { return };
        let sql = match cmd {
            Cmd::SpWho => "EXEC sp_who".to_string(),
            Cmd::SpLock => "EXEC sp_lock".to_string(),
            _ => {
                let target = match tab.selection {
                    Some((a, b)) if a != b => tab.sql.chars().skip(a).take(b - a).collect::<String>(),
                    Some((a, _)) => tsql::word_at(&tab.sql, a),
                    None => String::new(),
                };
                let target = target.trim();
                if target.is_empty() { "EXEC sp_help".to_string() } else { format!("EXEC sp_help {}", lit(target)) }
            }
        };
        self.run(ctx, tab_id, vec![Batch { sql, start_line: 1, repeat: 1 }]);
    }

    pub(crate) fn cancel(&mut self, tab_id: u64) {
        if let Some(t) = self.tab_mut(tab_id) {
            if t.running {
                if let Some(s) = &t.session {
                    s.cancel();
                }
            }
        }
    }

    pub(crate) fn change_database(&mut self, tab_id: u64, db: String) {
        let tag = FIRE_AND_FORGET + self.id();
        if let Some(t) = self.tab_mut(tab_id) {
            match &t.session {
                Some(s) if !t.running => s.send(Request::Fetch { tag, sql: format!("USE {}", qi(&db)) }),
                _ => t.database = db,
            }
        }
    }

    pub(crate) fn request_close_tab(&mut self, tab_id: u64) {
        if self.tabs.iter().any(|t| t.id == tab_id && t.dirty() && !t.sql.trim().is_empty()) {
            self.confirm = Some(Confirm::CloseTab(tab_id));
        } else {
            self.close_tab(tab_id);
        }
    }

    pub(crate) fn close_tab(&mut self, tab_id: u64) {
        if let Some(path) = self.dock.find_tab(&Pane::Query(tab_id)) {
            self.dock.remove_tab(path);
        }
        self.drop_tab(tab_id);
    }

    fn drop_tab(&mut self, tab_id: u64) {
        if let Some(i) = self.tabs.iter().position(|t| t.id == tab_id) {
            let t = self.tabs.remove(i);
            if t.running {
                if let Some(s) = &t.session {
                    s.cancel();
                }
            }
        }
        if self.current == Some(tab_id) {
            self.current = self.tabs.last().map(|t| t.id);
        }
    }

    // ----- Editor commands --------------------------------------------------

    /// Applies a line or case edit to the current editor's selection.
    fn edit(&mut self, cmd: Cmd) {
        let Some(id) = self.current else { return };
        let Some(tab) = self.tab_mut(id) else { return };
        let (a, b) = tab.selection.unwrap_or((0, 0));
        let new = match cmd {
            Cmd::Comment => Some(tsql::comment(&mut tab.sql, a, b)),
            Cmd::Uncomment => Some(tsql::uncomment(&mut tab.sql, a, b)),
            Cmd::Indent => Some(tsql::indent(&mut tab.sql, a, b)),
            Cmd::Unindent => Some(tsql::unindent(&mut tab.sql, a, b)),
            Cmd::Upper | Cmd::Lower => {
                tsql::change_case(&mut tab.sql, a, b, cmd == Cmd::Upper);
                Some((a, b))
            }
            _ => None,
        };
        if let Some(sel) = new {
            tab.set_selection = Some(sel);
            tab.focus_editor = true;
        }
    }

    pub(crate) fn find_next(&mut self) {
        let Some(id) = self.current else { return };
        let Some(tab) = self.tab_mut(id) else { return };
        let Some(f) = &tab.find else { return };
        let from = tab.selection.map_or(0, |s| s.1);
        match tsql::find(&tab.sql, &f.needle, from, f.match_case) {
            Some(r) => {
                tab.set_selection = Some(r);
                tab.scroll_to_cursor = true;
            }
            None => {
                let needle = f.needle.clone();
                self.notify(format!("The following specified text was not found: {needle}"), false);
            }
        }
    }

    pub(crate) fn replace_one(&mut self) {
        let Some(id) = self.current else { return };
        let Some(tab) = self.tab_mut(id) else { return };
        let Some(f) = &tab.find else { return };
        let (Some(rep), Some((a, b))) = (f.replace.clone(), tab.selection) else { return };
        let selected: String = tab.sql.chars().skip(a).take(b - a).collect();
        let same = if f.match_case { selected == f.needle } else { selected.to_lowercase() == f.needle.to_lowercase() };
        if a != b && same {
            let (ba, bb) = (tsql::char_to_byte(&tab.sql, a), tsql::char_to_byte(&tab.sql, b));
            tab.sql.replace_range(ba..bb, &rep);
            tab.selection = Some((a, a + rep.chars().count()));
        }
        self.find_next();
    }

    pub(crate) fn replace_all(&mut self) {
        let Some(id) = self.current else { return };
        let Some(tab) = self.tab_mut(id) else { return };
        let Some(f) = &tab.find else { return };
        let Some(rep) = f.replace.clone() else { return };
        let (needle, case) = (f.needle.clone(), f.match_case);
        let mut count = 0;
        let mut from = 0;
        while let Some((a, b)) = tsql::find(&tab.sql, &needle, from, case) {
            if a < from || count > 100_000 {
                break; // wrapped around
            }
            let (ba, bb) = (tsql::char_to_byte(&tab.sql, a), tsql::char_to_byte(&tab.sql, b));
            tab.sql.replace_range(ba..bb, &rep);
            from = a + rep.chars().count();
            count += 1;
        }
        self.notify(format!("{count} occurrence(s) replaced."), false);
    }

    // ----- Files ------------------------------------------------------------

    pub(crate) fn open_file(&self) {
        let tx = self.msg_tx.clone();
        self.rt.spawn(async move {
            let picked = rfd::AsyncFileDialog::new()
                .set_title("Open File")
                .add_filter("SQL Server files", &["sql"])
                .add_filter("All files", &["*"])
                .pick_file()
                .await;
            if let Some(f) = picked {
                let path = f.path().to_path_buf();
                let _ = tx.send(read_file(path).await);
            }
        });
    }

    pub(crate) fn open_recent(&self, path: PathBuf) {
        let tx = self.msg_tx.clone();
        self.rt.spawn(async move {
            let _ = tx.send(read_file(path).await);
        });
    }

    pub(crate) fn save_tab(&mut self, tab_id: u64, save_as: bool) {
        let Some(tab) = self.tab(tab_id) else { return };
        if let (Some(path), false) = (&tab.path, save_as) {
            let path = path.clone();
            self.write_tab(tab_id, path);
            return;
        }
        let tx = self.msg_tx.clone();
        let name = tab.title.clone();
        self.rt.spawn(async move {
            if let Some(f) = rfd::AsyncFileDialog::new()
                .set_title("Save File As")
                .set_file_name(name)
                .add_filter("SQL Server files", &["sql"])
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
                tab.path = Some(path.clone());
                self.settings.remember_file(&path);
                self.log(&format!("Saved {}", path.display()));
            }
            Err(e) => {
                let text = format!("Could not save {}: {e}", path.display());
                self.notify(text, true);
            }
        }
    }

    pub(crate) fn export_csv(&self, csv: String) {
        let tx = self.msg_tx.clone();
        self.rt.spawn(async move {
            if let Some(f) = rfd::AsyncFileDialog::new()
                .set_title("Save Grid Results")
                .set_file_name("Results.csv")
                .add_filter("CSV (Comma delimited)", &["csv"])
                .save_file()
                .await
            {
                let _ = tx.send(AppMsg::ExportCsv { path: f.path().to_path_buf(), csv });
            }
        });
    }

    // ----- Commands ---------------------------------------------------------

    pub(crate) fn command(&mut self, ctx: &egui::Context, cmd: Cmd) {
        let cur = self.current;
        match cmd {
            Cmd::ConnectObjectExplorer => self.open_connect_dialog(false, None),
            Cmd::NewQuery => {
                if self.servers.is_empty() {
                    self.open_connect_dialog(true, None);
                } else {
                    self.new_query(ctx, None, None, "");
                }
            }
            Cmd::OpenFile => self.open_file(),
            Cmd::Save | Cmd::SaveAs => {
                if let Some(id) = cur {
                    self.save_tab(id, cmd == Cmd::SaveAs);
                }
            }
            Cmd::SaveAll => {
                let dirty: Vec<u64> = self.tabs.iter().filter(|t| t.dirty()).map(|t| t.id).collect();
                for id in dirty {
                    self.save_tab(id, false);
                }
            }
            Cmd::Close => {
                if let Some(id) = cur {
                    self.request_close_tab(id);
                }
            }
            Cmd::CloseAll => {
                let ids: Vec<u64> = self.tabs.iter().map(|t| t.id).collect();
                for id in ids {
                    self.request_close_tab(id);
                }
            }
            Cmd::Exit => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
            Cmd::Find | Cmd::Replace => {
                if let Some(t) = cur.and_then(|id| self.tab_mut(id)) {
                    let selected: String = match t.selection {
                        Some((a, b)) if a != b && b - a < 200 => t.sql.chars().skip(a).take(b - a).collect(),
                        _ => String::new(),
                    };
                    let f = t.find.get_or_insert(FindBar { needle: String::new(), replace: None, match_case: false, focus: true });
                    if !selected.is_empty() && !selected.contains('\n') {
                        f.needle = selected;
                    }
                    f.focus = true;
                    if cmd == Cmd::Replace && f.replace.is_none() {
                        f.replace = Some(String::new());
                    }
                }
            }
            Cmd::FindNext => self.find_next(),
            Cmd::GoTo => {
                if cur.is_some() {
                    self.goto = Some(String::new());
                }
            }
            Cmd::Comment | Cmd::Uncomment | Cmd::Upper | Cmd::Lower | Cmd::Indent | Cmd::Unindent => self.edit(cmd),
            Cmd::ShowObjectExplorer => self.show_pane(Pane::ObjectExplorer),
            Cmd::ShowProperties => self.show_pane(Pane::Properties),
            Cmd::ShowOutput => self.show_pane(Pane::Output),
            Cmd::ShowTemplates => self.show_pane(Pane::TemplateBrowser),
            Cmd::FullScreen => {
                self.fullscreen = !self.fullscreen;
                ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(self.fullscreen));
            }
            Cmd::Execute => {
                if let Some(id) = cur {
                    self.execute(ctx, id);
                }
            }
            Cmd::Parse => {
                if let Some(id) = cur {
                    self.parse(ctx, id);
                }
            }
            Cmd::Cancel => {
                if let Some(id) = cur {
                    self.cancel(id);
                }
            }
            Cmd::ChangeConnection => self.open_connect_dialog(false, cur),
            Cmd::Disconnect => {
                if let Some(t) = cur.and_then(|id| self.tab_mut(id)) {
                    t.session = None;
                    t.server_info = None;
                }
            }
            Cmd::ResultsToGrid | Cmd::ResultsToText | Cmd::ResultsToFile => {
                if let Some(t) = cur.and_then(|id| self.tab_mut(id)) {
                    t.mode = match cmd {
                        Cmd::ResultsToGrid => ResultsMode::Grid,
                        Cmd::ResultsToText => ResultsMode::Text,
                        _ => ResultsMode::File,
                    };
                    t.file_target = None;
                }
            }
            Cmd::ToggleResults => {
                if let Some(t) = cur.and_then(|id| self.tab_mut(id)) {
                    t.show_results = !t.show_results;
                }
            }
            Cmd::TemplateParams => {
                if let Some(t) = cur.and_then(|id| self.tab(id)) {
                    let params = crate::templates::params(&t.sql);
                    if params.is_empty() {
                        self.notify("There are no template parameters in this query.", false);
                    } else {
                        self.template_params = Some((t.id, params));
                    }
                }
            }
            Cmd::SpHelp | Cmd::SpWho | Cmd::SpLock => {
                if let Some(id) = cur {
                    self.query_shortcut(ctx, id, cmd);
                }
            }
            Cmd::SaveResults => {
                let csv = self.current_tab().and_then(|t| t.results.first().map(|r| r.to_csv()));
                match csv {
                    Some(csv) => self.export_csv(csv),
                    None => self.notify("There are no results to save.", true),
                }
            }
            Cmd::Options => self.show_options = true,
            Cmd::ResetLayout => {
                let ids: Vec<u64> = self.tabs.iter().map(|t| t.id).collect();
                self.dock = default_dock();
                for id in ids {
                    self.place_document(Pane::Query(id));
                }
            }
            Cmd::NextDocument => {
                if !self.tabs.is_empty() {
                    let i = cur.and_then(|id| self.tabs.iter().position(|t| t.id == id)).map_or(0, |i| (i + 1) % self.tabs.len());
                    let id = self.tabs[i].id;
                    self.current = Some(id);
                    if let Some(path) = self.dock.find_tab(&Pane::Query(id)) {
                        let _ = self.dock.set_active_tab(path);
                        self.dock.set_focused_node_and_surface(path.node_path());
                    }
                }
            }
            Cmd::About => self.show_about = true,
        }
    }

    // ----- Event pumps ----------------------------------------------------

    pub(crate) fn pump(&mut self, ctx: &egui::Context) {
        while let Ok(msg) = self.msg_rx.try_recv() {
            match msg {
                AppMsg::Opened(path, text) => {
                    self.settings.remember_file(&path);
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
                AppMsg::RunToFile { tab, path } => {
                    if let Some(t) = self.tab_mut(tab) {
                        t.file_target = Some(path);
                    }
                    self.execute(ctx, tab);
                }
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
        let mut logs = Vec::new();
        for s in &mut self.servers {
            for ev in s.session.poll() {
                match ev {
                    Event::Connected(info) => {
                        s.root.label = format!("{} (SQL Server {} - {})", s.session.info.server.trim(), info.version, info.login);
                        logs.push(format!("Connected to {} as {} (SQL Server {}).", s.session.info.server.trim(), info.login, info.version));
                        s.info = Some(info);
                        s.state = ServerState::Connected;
                        newly_connected.push(s.id);
                    }
                    Event::ConnectFailed(e) => s.state = ServerState::Failed(e),
                    Event::Fetched { tag, result } => fetched.push((tag, result)),
                    Event::Disconnected(e) => {
                        logs.push(e.clone());
                        s.root.error = Some(e);
                    }
                    _ => {}
                }
            }
        }
        for l in logs {
            self.log(&l);
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
        let mut finished = Vec::new();
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
                        tab.order.push(Item::Message(tab.messages.len() - 1));
                        tab.show_results = true;
                        tab.view = ResultsView::Messages;
                        if tab.running {
                            tab.running = false;
                            tab.outcome = Some(Outcome::Errors);
                        }
                        lost = true;
                    }
                    Event::ResultStart(cols) => {
                        if tab.results.is_empty() && tab.mode != ResultsMode::File {
                            tab.view = ResultsView::Results;
                        }
                        tab.results.push(ResultSet::new(cols));
                        tab.order.push(Item::Set(tab.results.len() - 1));
                    }
                    Event::Rows(rows) => {
                        if let Some(r) = tab.results.last_mut() {
                            r.add_rows(rows);
                        }
                    }
                    Event::Message(m) => {
                        tab.messages.push(m);
                        tab.order.push(Item::Message(tab.messages.len() - 1));
                    }
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
                        if tab.messages.is_empty() && tab.results.is_empty() {
                            tab.messages.push(Message { text: "Commands completed successfully.".into(), is_error: false });
                            tab.order.push(Item::Message(tab.messages.len() - 1));
                        }
                        tab.messages.push(Message {
                            text: format!("\nCompletion time: {}", chrono::Local::now().format("%Y-%m-%dT%H:%M:%S%.7f%:z")),
                            is_error: false,
                        });
                        tab.order.push(Item::Message(tab.messages.len() - 1));
                        if errors || cancelled {
                            tab.view = ResultsView::Messages;
                        }
                        if let Some(path) = tab.file_target.take() {
                            finished.push((path, tab.text_output()));
                        }
                    }
                    Event::Fetched { result: Err(e), .. } => {
                        tab.messages.push(Message { text: e, is_error: true });
                        tab.order.push(Item::Message(tab.messages.len() - 1));
                        tab.show_results = true;
                        tab.view = ResultsView::Messages;
                    }
                    Event::Fetched { .. } => {}
                    Event::Disconnected(e) => {
                        tab.messages.push(Message { text: e, is_error: true });
                        tab.order.push(Item::Message(tab.messages.len() - 1));
                    }
                }
            }
            if lost {
                tab.session = None;
            }
        }
        for (path, text) in finished {
            match std::fs::write(&path, text) {
                Ok(()) => self.notify(format!("Results saved to {}", path.display()), false),
                Err(e) => self.notify(format!("Could not write {}: {e}", path.display()), true),
            }
        }

        // Properties window follows the Object Explorer selection.
        if self.props.node != self.selected_node && self.dock.find_tab(&Pane::Properties).is_some() {
            self.load_properties();
        }
    }

    fn load_properties(&mut self) {
        self.props.node = self.selected_node;
        self.props.rows.clear();
        self.props.loading = false;
        let Some((server, node)) = self.selected_node else { return };
        let Some(s) = self.servers.iter_mut().find(|s| s.id == server) else { return };
        let Some(n) = s.root.find_mut(node) else { return };
        self.props.title = n.label.clone();
        if let Some(sql) = explorer::properties_sql(&n.kind) {
            self.props.loading = true;
            self.fetch(server, sql, Pending::Properties { server, node });
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
            Pending::Properties { server, node } => {
                if self.props.node != Some((server, node)) {
                    return;
                }
                self.props.loading = false;
                match result {
                    Ok(tables) => {
                        if let Some(t) = tables.first().filter(|t| !t.rows.is_empty()) {
                            self.props.rows = t.columns.iter().enumerate().map(|(i, c)| (c.clone(), t.rows[0][i].clone().unwrap_or_default())).collect();
                        }
                    }
                    Err(e) => self.props.rows = vec![("Error".into(), e)],
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
                                explorer::column_type(first.get(r, "type_name"), first.get(r, "max_length"), first.get(r, "precision"), first.get(r, "scale")),
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

    pub(crate) fn apply(&mut self, ctx: &egui::Context, action: Action) {
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
                    self.props.node = None;
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
                        o = lit(&format!("{}.{}.{}", qi(&db), qi(&schema), qi(&name)))
                    ),
                };
                self.fetch(server, fetch_sql, Pending::Script { server, db, schema, name, kind, run });
            }
            Action::Disconnect { server } => {
                if let Some(s) = self.server(server) {
                    let name = s.session.info.server.trim().to_string();
                    self.log(&format!("Disconnected from {name}."));
                }
                self.servers.retain(|s| s.id != server);
                if self.selected_node.is_some_and(|(s, _)| s == server) {
                    self.selected_node = None;
                }
            }
            Action::CollapseAll => {
                for s in &self.servers {
                    collapse(ctx, s.id, &s.root);
                }
            }
            Action::OpenTemplate(i) => {
                let t = &crate::templates::TEMPLATES[i];
                let id = self.new_query(ctx, None, None, t.sql);
                if let Some(tab) = self.tab_mut(id) {
                    tab.title = format!("{}.sql", t.name.replace(' ', ""));
                }
            }
        }
    }

    pub(crate) fn save_state(&self, storage: &mut dyn eframe::Storage) {
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

    pub(crate) fn status_color(&self, tab: &QueryTab) -> Option<Color32> {
        tab.session.as_ref().and_then(|s| s.info.color).map(|[r, g, b]| Color32::from_rgb(r, g, b))
    }
}

fn collapse(ctx: &egui::Context, server: u64, node: &Node) {
    let mut s = CollapsingState::load_with_default_open(ctx, node_ui_id(server, node.id), false);
    s.set_open(false);
    s.store(ctx);
    if let Some(children) = &node.children {
        for c in children {
            collapse(ctx, server, c);
        }
    }
}

async fn read_file(path: PathBuf) -> AppMsg {
    match tokio::fs::read(&path).await {
        Ok(bytes) => AppMsg::Opened(path, decode_text(&bytes)),
        Err(e) => AppMsg::Error(format!("Could not open {}: {e}", path.display())),
    }
}

pub(crate) fn apply_font_size(ctx: &egui::Context, size: f32) {
    ctx.all_styles_mut(|style| {
        for (ts, font) in style.text_styles.iter_mut() {
            font.size = match ts {
                egui::TextStyle::Small => size * 0.75,
                egui::TextStyle::Heading => size * 1.4,
                _ => size,
            };
        }
    });
}

fn decode_text(bytes: &[u8]) -> String {
    // SSMS saves UTF-16 LE with a BOM by default.
    if bytes.starts_with(&[0xFF, 0xFE]) {
        let units: Vec<u16> = bytes[2..].chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
        return String::from_utf16_lossy(&units).replace("\r\n", "\n");
    }
    if bytes.starts_with(&[0xFE, 0xFF]) {
        let units: Vec<u16> = bytes[2..].chunks_exact(2).map(|c| u16::from_be_bytes([c[0], c[1]])).collect();
        return String::from_utf16_lossy(&units).replace("\r\n", "\n");
    }
    let bytes = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    String::from_utf8_lossy(bytes).replace("\r\n", "\n")
}

pub(crate) fn node_ui_id(server: u64, node: u64) -> egui::Id {
    egui::Id::unique(("oe", server, node))
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.frame(ui);
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        self.save_state(storage);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_results() {
        let mut r = ResultSet::new(vec![
            ColumnInfo { name: "id".into(), type_name: "int", numeric: true },
            ColumnInfo { name: "name".into(), type_name: "nvarchar", numeric: false },
        ]);
        r.add_rows(vec![vec![Some("1".into()), Some("Ann".into())], vec![Some("10".into()), None]]);
        assert_eq!(r.to_text(), "id name\n-- ----\n 1 Ann\n10 NULL\n");
    }
}
