//! Connections to SQL Server. Each `Session` owns one connection on a tokio
//! task; the UI sends it requests and polls its events every frame.

use std::sync::Arc;
use std::sync::mpsc as std_mpsc;
use std::time::{Duration, Instant};

use futures_util::TryStreamExt;
use serde::{Deserialize, Serialize};
use tiberius::{AuthMethod, Client, Config, EncryptionLevel, QueryEvent, SqlBrowser};
use tokio::net::TcpStream;
use tokio::sync::{Notify, mpsc};
use tokio_util::compat::{Compat, TokioAsyncWriteCompatExt};

use crate::batch::Batch;
use crate::format;

type Conn = Client<Compat<TcpStream>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Auth {
    #[default]
    SqlServer,
    /// NTLM with an explicit DOMAIN\user and password.
    Windows,
}

impl Auth {
    pub fn label(self) -> &'static str {
        match self {
            Auth::SqlServer => "SQL Server Authentication",
            Auth::Windows => "Windows Authentication (NTLM)",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Encrypt {
    #[default]
    Mandatory,
    Optional,
    Strict,
    Off,
}

impl Encrypt {
    pub const ALL: [Encrypt; 4] = [Encrypt::Mandatory, Encrypt::Optional, Encrypt::Strict, Encrypt::Off];

    pub fn label(self) -> &'static str {
        match self {
            Encrypt::Mandatory => "Mandatory",
            Encrypt::Optional => "Optional",
            Encrypt::Strict => "Strict (SQL Server 2022+)",
            Encrypt::Off => "Off (login only)",
        }
    }
}

/// What the connect dialog collects. The password is never serialized; it
/// lives in the system keyring when the user asks to remember it.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ConnectionInfo {
    pub server: String,
    pub auth: Auth,
    pub user: String,
    #[serde(skip)]
    pub password: String,
    pub remember_password: bool,
    pub database: String,
    pub encrypt: Encrypt,
    pub trust_cert: bool,
    pub timeout_secs: u64,
    pub app_name: String,
}

impl Default for ConnectionInfo {
    fn default() -> Self {
        Self {
            server: String::new(),
            auth: Auth::SqlServer,
            user: String::new(),
            password: String::new(),
            remember_password: false,
            database: String::new(),
            encrypt: Encrypt::Mandatory,
            trust_cert: false,
            timeout_secs: 15,
            app_name: "SQL Studio".into(),
        }
    }
}

impl ConnectionInfo {
    /// Key for recent-connection matching and the keyring entry.
    pub fn key(&self) -> String {
        format!("{}|{}", self.server.trim().to_lowercase(), self.user.trim().to_lowercase())
    }

    /// The name shown on the Object Explorer root, e.g. `sql01 (SQL Server - sa)`.
    pub fn display(&self) -> String {
        format!("{} ({})", self.server.trim(), self.user.trim())
    }

    pub fn config(&self, database: Option<&str>) -> Config {
        let mut c = Config::new();
        let (host, port, instance) = parse_server(&self.server);
        c.host(host);
        if let Some(p) = port {
            c.port(p);
        }
        if let Some(i) = instance {
            c.instance_name(i);
        }
        let db = database.unwrap_or(&self.database);
        if !db.trim().is_empty() {
            c.database(db.trim());
        }
        c.application_name(&self.app_name);
        c.encryption(match self.encrypt {
            Encrypt::Mandatory => EncryptionLevel::Required,
            Encrypt::Optional => EncryptionLevel::On,
            Encrypt::Strict => EncryptionLevel::Strict,
            Encrypt::Off => EncryptionLevel::Off,
        });
        if self.trust_cert {
            c.trust_cert();
        }
        c.handshake_timeout(Some(Duration::from_secs(self.timeout_secs.max(1))));
        c.authentication(match self.auth {
            Auth::SqlServer => AuthMethod::sql_server(&self.user, &self.password),
            Auth::Windows => AuthMethod::windows(&self.user, &self.password),
        });
        c
    }
}

/// Accepts `host`, `host,port`, `host\instance`, `tcp:host,port`, and `.`/`(local)`.
pub fn parse_server(s: &str) -> (String, Option<u16>, Option<String>) {
    let s = s.trim();
    let s = s.strip_prefix("tcp:").unwrap_or(s);
    let (rest, port) = match s.rsplit_once(',') {
        Some((h, p)) => (h, p.trim().parse().ok()),
        None => (s, None),
    };
    let (host, instance) = match rest.split_once('\\') {
        Some((h, i)) if !i.is_empty() => (h, Some(i.to_string())),
        _ => (rest, None),
    };
    let host = match host.trim() {
        "." | "(local)" | "" => "localhost".to_string(),
        h => h.to_string(),
    };
    (host, port, instance)
}

async fn connect(info: &ConnectionInfo, database: Option<&str>) -> Result<Conn, String> {
    let mut config = info.config(database);
    // Azure SQL may redirect us to another node; follow it once.
    for _ in 0..2 {
        let tcp = tokio::time::timeout(
            Duration::from_secs(info.timeout_secs.max(1)),
            TcpStream::connect_named(&config),
        )
        .await
        .map_err(|_| format!("Timed out connecting to {}", info.server))?
        .map_err(|e| e.to_string())?;
        tcp.set_nodelay(true).ok();
        match Client::connect(config.clone(), tcp.compat_write()).await {
            Ok(c) => return Ok(c),
            Err(tiberius::error::Error::Routing { host, port }) => {
                config.host(&host);
                config.port(port);
            }
            Err(e) => return Err(e.to_string()),
        }
    }
    Err("Too many redirects".into())
}

#[derive(Debug, Clone, Default)]
pub struct ServerInfo {
    pub spid: String,
    pub login: String,
    pub database: String,
    pub version: String,
    pub server_name: String,
    pub edition: String,
}

#[derive(Debug, Clone)]
pub struct ColumnInfo {
    pub name: String,
    pub type_name: &'static str,
    pub numeric: bool,
}

/// A fully fetched result, used by Object Explorer and scripting.
#[derive(Debug, Clone, Default)]
pub struct Table {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<Option<String>>>,
}

impl Table {
    pub fn get(&self, row: usize, col: &str) -> &str {
        self.columns
            .iter()
            .position(|c| c.eq_ignore_ascii_case(col))
            .and_then(|i| self.rows[row][i].as_deref())
            .unwrap_or("")
    }
}

#[derive(Debug, Clone)]
pub struct Message {
    pub text: String,
    pub is_error: bool,
}

pub enum Request {
    /// Run batches for a query window, streaming results.
    Run { batches: Vec<Batch>, max_rows: usize },
    /// Fetch every result set of `sql` in one go (Object Explorer, scripting).
    Fetch { tag: u64, sql: String },
}

pub enum Event {
    Connected(ServerInfo),
    ConnectFailed(String),
    ResultStart(Vec<ColumnInfo>),
    Rows(Vec<Vec<Option<String>>>),
    Message(Message),
    DatabaseChanged(String),
    /// A `Run` request finished. `errors` is true when any batch failed.
    RunDone { elapsed: Duration, errors: bool, cancelled: bool },
    Fetched { tag: u64, result: Result<Vec<Table>, String> },
    Disconnected(String),
}

pub struct Session {
    tx: mpsc::UnboundedSender<Request>,
    rx: std_mpsc::Receiver<Event>,
    cancel: Arc<Notify>,
    pub info: ConnectionInfo,
}

impl Session {
    /// Opens a connection in the background. `Event::Connected` or
    /// `Event::ConnectFailed` arrives once it is ready.
    pub fn open(
        rt: &tokio::runtime::Handle,
        info: ConnectionInfo,
        database: Option<String>,
        repaint: eframe::egui::Context,
    ) -> Self {
        let (tx, req_rx) = mpsc::unbounded_channel();
        let (ev_tx, rx) = std_mpsc::channel();
        let cancel = Arc::new(Notify::new());
        let worker = Worker {
            info: info.clone(),
            database,
            events: ev_tx,
            repaint,
            cancel: cancel.clone(),
        };
        rt.spawn(worker.run(req_rx));
        Self { tx, rx, cancel, info }
    }

    pub fn send(&self, req: Request) {
        let _ = self.tx.send(req);
    }

    pub fn cancel(&self) {
        self.cancel.notify_waiters();
    }

    pub fn poll(&self) -> Vec<Event> {
        self.rx.try_iter().collect()
    }
}

struct Worker {
    info: ConnectionInfo,
    database: Option<String>,
    events: std_mpsc::Sender<Event>,
    repaint: eframe::egui::Context,
    cancel: Arc<Notify>,
}

/// Rows are sent to the UI in chunks of this size, so huge results stream in
/// without flooding the channel.
const ROW_CHUNK: usize = 2000;

impl Worker {
    fn emit(&self, ev: Event) {
        let _ = self.events.send(ev);
        self.repaint.request_repaint();
    }

    async fn open(&mut self) -> Result<Conn, String> {
        let mut client = connect(&self.info, self.database.as_deref()).await?;
        let info = fetch(
            &mut client,
            "SELECT CAST(@@SPID AS nvarchar(10)) spid, SUSER_SNAME() login, DB_NAME() db, \
             CAST(SERVERPROPERTY('ProductVersion') AS nvarchar(128)) version, \
             CAST(@@SERVERNAME AS nvarchar(256)) server_name, \
             CAST(SERVERPROPERTY('Edition') AS nvarchar(128)) edition",
            &mut |_| {},
        )
        .await?;
        let t = info.first().cloned().unwrap_or_default();
        if !t.rows.is_empty() {
            self.emit(Event::Connected(ServerInfo {
                spid: t.get(0, "spid").into(),
                login: t.get(0, "login").into(),
                database: t.get(0, "db").into(),
                version: t.get(0, "version").into(),
                server_name: t.get(0, "server_name").into(),
                edition: t.get(0, "edition").into(),
            }));
        }
        Ok(client)
    }

    async fn run(mut self, mut requests: mpsc::UnboundedReceiver<Request>) {
        let mut client = match self.open().await {
            Ok(c) => Some(c),
            Err(e) => {
                self.emit(Event::ConnectFailed(e));
                return;
            }
        };

        while let Some(req) = requests.recv().await {
            // Reconnect transparently if the last request broke the link,
            // staying in the database the user had switched to.
            if client.is_none() {
                match self.open().await {
                    Ok(c) => client = Some(c),
                    Err(e) => {
                        match req {
                            Request::Run { .. } => {
                                self.emit(Event::Message(Message { text: e.clone(), is_error: true }));
                                self.emit(Event::RunDone {
                                    elapsed: Duration::ZERO,
                                    errors: true,
                                    cancelled: false,
                                });
                            }
                            Request::Fetch { tag, .. } => {
                                self.emit(Event::Fetched { tag, result: Err(e) })
                            }
                        }
                        continue;
                    }
                }
            }
            let c = client.as_mut().unwrap();
            let broken = match req {
                Request::Run { batches, max_rows } => self.run_batches(c, batches, max_rows).await,
                Request::Fetch { tag, sql } => {
                    let mut db_changed = None;
                    let result = fetch(c, &sql, &mut |db| db_changed = Some(db)).await;
                    if let Some(db) = db_changed {
                        self.database = Some(db.clone());
                        self.emit(Event::DatabaseChanged(db));
                    }
                    let broken = result.as_ref().is_err_and(|e| e.starts_with(BROKEN));
                    self.emit(Event::Fetched { tag, result });
                    broken
                }
            };
            if broken {
                client = None;
                self.emit(Event::Disconnected("The connection was lost and will reopen on the next request.".into()));
            }
        }
    }

    /// Returns true when the connection is no longer usable.
    async fn run_batches(&mut self, client: &mut Conn, batches: Vec<Batch>, max_rows: usize) -> bool {
        let started = Instant::now();
        let mut errors = false;
        let mut cancelled = false;
        let mut broken = false;

        'outer: for batch in &batches {
            for _ in 0..batch.repeat {
                let notified = self.cancel.notified();
                let work = run_one(client, &batch.sql, batch.start_line, max_rows, &self.events, &self.repaint);
                let outcome = tokio::select! {
                    r = work => Some(r),
                    _ = notified => None,
                };
                match outcome {
                    None => {
                        cancelled = true;
                        if client.cancel_query().await.is_err() {
                            broken = true;
                        }
                        self.emit(Event::Message(Message {
                            text: "Query was canceled by user.".into(),
                            is_error: true,
                        }));
                        break 'outer;
                    }
                    Some(r) => {
                        if let Some(db) = r.database {
                            self.database = Some(db);
                        }
                        if r.failed {
                            errors = true;
                        }
                        if r.broken {
                            broken = true;
                            break 'outer;
                        }
                    }
                }
            }
        }
        self.emit(Event::RunDone { elapsed: started.elapsed(), errors, cancelled });
        broken
    }
}

const BROKEN: &str = "Connection error: ";

struct BatchOutcome {
    failed: bool,
    broken: bool,
    database: Option<String>,
}

fn forward(
    events: Vec<QueryEvent>,
    start_line: u32,
    out: &std_mpsc::Sender<Event>,
    saw_error: &mut bool,
    database: &mut Option<String>,
) {
    for ev in events {
        let _ = match ev {
            QueryEvent::Message(m) => {
                let text = if m.is_error || m.class > 10 {
                    *saw_error |= m.is_error;
                    let line = m.line + start_line.saturating_sub(1);
                    let proc_part = if m.procedure.is_empty() {
                        String::new()
                    } else {
                        format!(", Procedure {}", m.procedure)
                    };
                    format!(
                        "Msg {}, Level {}, State {}{}, Line {}\n{}",
                        m.number, m.class, m.state, proc_part, line, m.message
                    )
                } else {
                    m.message
                };
                out.send(Event::Message(Message { text, is_error: m.is_error }))
            }
            QueryEvent::RowCount(n) => out.send(Event::Message(Message {
                text: format!("({n} row{} affected)", if n == 1 { "" } else { "s" }),
                is_error: false,
            })),
            QueryEvent::DatabaseChanged(db) => {
                *database = Some(db.clone());
                out.send(Event::DatabaseChanged(db))
            }
        };
    }
}

async fn run_one(
    client: &mut Conn,
    sql: &str,
    start_line: u32,
    max_rows: usize,
    out: &std_mpsc::Sender<Event>,
    repaint: &eframe::egui::Context,
) -> BatchOutcome {
    let mut saw_error = false;
    let mut database = None;
    let mut stream = match client.simple_query(sql).await {
        Ok(s) => s,
        Err(e) => {
            let _ = out.send(Event::Message(Message { text: e.to_string(), is_error: true }));
            repaint.request_repaint();
            return BatchOutcome { failed: true, broken: !is_server_error(&e), database };
        }
    };
    let mut chunk: Vec<Vec<Option<String>>> = Vec::new();
    let mut types = Vec::new();
    let mut rows_in_set = 0usize;
    let mut truncated = false;
    let mut last_flush = Instant::now();

    let flush = |chunk: &mut Vec<Vec<Option<String>>>| {
        if !chunk.is_empty() {
            let _ = out.send(Event::Rows(std::mem::take(chunk)));
            repaint.request_repaint();
        }
    };

    let result = loop {
        let item = stream.try_next().await;
        let pending = stream.drain_events();
        if !pending.is_empty() {
            flush(&mut chunk);
            forward(pending, start_line, out, &mut saw_error, &mut database);
            repaint.request_repaint();
        }
        match item {
            Ok(Some(tiberius::QueryItem::Metadata(meta))) => {
                flush(&mut chunk);
                types = meta.columns().iter().map(|c| c.column_type()).collect();
                rows_in_set = 0;
                truncated = false;
                let cols = meta
                    .columns()
                    .iter()
                    .map(|c| ColumnInfo {
                        name: c.name().to_string(),
                        type_name: format::type_name(c.column_type()),
                        numeric: format::is_numeric(c.column_type()),
                    })
                    .collect();
                let _ = out.send(Event::ResultStart(cols));
                repaint.request_repaint();
            }
            Ok(Some(tiberius::QueryItem::Row(row))) => {
                rows_in_set += 1;
                if rows_in_set > max_rows {
                    if !truncated {
                        truncated = true;
                        flush(&mut chunk);
                        let _ = out.send(Event::Message(Message {
                            text: format!("Result set truncated to {max_rows} rows in the grid."),
                            is_error: false,
                        }));
                    }
                    continue;
                }
                let cells = row
                    .cells()
                    .zip(types.iter())
                    .map(|((_, data), ty)| format::cell(data, *ty))
                    .collect();
                chunk.push(cells);
                if chunk.len() >= ROW_CHUNK || last_flush.elapsed() > Duration::from_millis(150) {
                    flush(&mut chunk);
                    last_flush = Instant::now();
                }
            }
            Ok(None) => break Ok(()),
            Err(e) => break Err(e),
        }
    };
    flush(&mut chunk);
    drop(stream);
    match result {
        Ok(()) => BatchOutcome { failed: saw_error, broken: false, database },
        Err(e) => {
            let server = is_server_error(&e);
            // Server errors were already shown in order as messages.
            if !(server && saw_error) {
                let _ = out.send(Event::Message(Message { text: e.to_string(), is_error: true }));
                repaint.request_repaint();
            }
            BatchOutcome { failed: true, broken: !server, database }
        }
    }
}

fn is_server_error(e: &tiberius::error::Error) -> bool {
    matches!(e, tiberius::error::Error::Server(_))
}

/// Runs `sql` and collects every result set. Errors on a broken connection
/// start with `BROKEN`.
async fn fetch(
    client: &mut Conn,
    sql: &str,
    on_db_change: &mut (dyn FnMut(String) + Send),
) -> Result<Vec<Table>, String> {
    let wrap = |e: tiberius::error::Error| {
        if is_server_error(&e) { e.to_string() } else { format!("{BROKEN}{e}") }
    };
    let mut stream = client.simple_query(sql).await.map_err(wrap)?;
    let mut tables: Vec<Table> = Vec::new();
    let mut types = Vec::new();
    loop {
        let item = stream.try_next().await;
        for ev in stream.drain_events() {
            if let QueryEvent::DatabaseChanged(db) = ev {
                on_db_change(db);
            }
        }
        match item.map_err(wrap)? {
            Some(tiberius::QueryItem::Metadata(meta)) => {
                types = meta.columns().iter().map(|c| c.column_type()).collect();
                tables.push(Table {
                    columns: meta.columns().iter().map(|c| c.name().to_string()).collect(),
                    rows: Vec::new(),
                });
            }
            Some(tiberius::QueryItem::Row(row)) => {
                let cells = row
                    .cells()
                    .zip(types.iter())
                    .map(|((_, d), ty)| format::cell(d, *ty))
                    .collect();
                if let Some(t) = tables.last_mut() {
                    t.rows.push(cells);
                }
            }
            None => break,
        }
    }
    Ok(tables)
}

#[cfg(test)]
mod tests {
    use super::parse_server;

    #[test]
    fn server_names() {
        assert_eq!(parse_server("sql01"), ("sql01".into(), None, None));
        assert_eq!(parse_server("tcp:sql01,1444"), ("sql01".into(), Some(1444), None));
        assert_eq!(
            parse_server(r"sql01\SQLEXPRESS"),
            ("sql01".into(), None, Some("SQLEXPRESS".into()))
        );
        assert_eq!(parse_server("."), ("localhost".into(), None, None));
        assert_eq!(parse_server("(local),1433"), ("localhost".into(), Some(1433), None));
    }
}
