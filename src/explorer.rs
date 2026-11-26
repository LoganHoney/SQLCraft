//! Object Explorer tree: what each node is, the SQL that lists its
//! children, and how a fetched table turns into child nodes.

use crate::db::Table;
use crate::format::{lit, qi};

#[derive(Debug, Clone, PartialEq)]
pub enum Kind {
    Server,
    DatabasesFolder,
    SystemDatabasesFolder,
    Database { db: String },
    TablesFolder { db: String },
    ViewsFolder { db: String },
    ProgrammabilityFolder { db: String },
    ProceduresFolder { db: String },
    FunctionsFolder { db: String },
    DbSecurityFolder { db: String },
    UsersFolder { db: String },
    SchemasFolder { db: String },
    Table { db: String, schema: String, name: String },
    View { db: String, schema: String, name: String },
    Procedure { db: String, schema: String, name: String },
    Function { db: String, schema: String, name: String, kind: String },
    ColumnsFolder { db: String, schema: String, name: String },
    IndexesFolder { db: String, schema: String, name: String },
    KeysFolder { db: String, schema: String, name: String },
    TriggersFolder { db: String, schema: String, name: String },
    SecurityFolder,
    LoginsFolder,
    AgentFolder,
    JobsFolder,
    /// A node with nothing beneath it (columns, indexes, logins, users...).
    Leaf { icon: &'static str },
}

impl Kind {
    pub fn icon(&self) -> &'static str {
        match self {
            Kind::Server => "🖥",
            Kind::Database { .. } => "🗄",
            Kind::Table { .. } => "▦",
            Kind::View { .. } => "👁",
            Kind::Procedure { .. } => "⚙",
            Kind::Function { .. } => "ƒ",
            Kind::Leaf { icon } => icon,
            _ => "📁",
        }
    }

    pub fn expandable(&self) -> bool {
        !matches!(self, Kind::Leaf { .. } | Kind::Procedure { .. } | Kind::Function { .. })
    }

    /// The database a node belongs to, for "New Query" from that node.
    pub fn database(&self) -> Option<&str> {
        use Kind::*;
        match self {
            Database { db } | TablesFolder { db } | ViewsFolder { db }
            | ProgrammabilityFolder { db } | ProceduresFolder { db } | FunctionsFolder { db }
            | DbSecurityFolder { db } | UsersFolder { db } | SchemasFolder { db } => Some(db),
            Table { db, .. } | View { db, .. } | Procedure { db, .. } | Function { db, .. }
            | ColumnsFolder { db, .. } | IndexesFolder { db, .. } | KeysFolder { db, .. }
            | TriggersFolder { db, .. } => Some(db),
            _ => None,
        }
    }
}

pub struct Node {
    pub id: u64,
    pub label: String,
    pub kind: Kind,
    /// `None` until the node has been expanded once.
    pub children: Option<Vec<Node>>,
    pub loading: bool,
    pub error: Option<String>,
}

impl Node {
    pub fn new(id: u64, label: impl Into<String>, kind: Kind) -> Self {
        Self { id, label: label.into(), kind, children: None, loading: false, error: None }
    }

    pub fn find_mut(&mut self, id: u64) -> Option<&mut Node> {
        if self.id == id {
            return Some(self);
        }
        self.children.as_mut()?.iter_mut().find_map(|c| c.find_mut(id))
    }

    pub fn reset(&mut self) {
        self.children = None;
        self.loading = false;
        self.error = None;
    }
}

/// Children that need no query: folders under a database or object.
pub fn static_children(kind: &Kind, next_id: &mut dyn FnMut() -> u64) -> Option<Vec<Node>> {
    let mut n = |label: &str, kind: Kind| Node::new(next_id(), label, kind);
    Some(match kind {
        Kind::Server => vec![
            n("Databases", Kind::DatabasesFolder),
            n("Security", Kind::SecurityFolder),
            n("SQL Server Agent", Kind::AgentFolder),
        ],
        Kind::SecurityFolder => vec![n("Logins", Kind::LoginsFolder)],
        Kind::AgentFolder => vec![n("Jobs", Kind::JobsFolder)],
        Kind::Database { db } => vec![
            n("Tables", Kind::TablesFolder { db: db.clone() }),
            n("Views", Kind::ViewsFolder { db: db.clone() }),
            n("Programmability", Kind::ProgrammabilityFolder { db: db.clone() }),
            n("Security", Kind::DbSecurityFolder { db: db.clone() }),
        ],
        Kind::ProgrammabilityFolder { db } => vec![
            n("Stored Procedures", Kind::ProceduresFolder { db: db.clone() }),
            n("Functions", Kind::FunctionsFolder { db: db.clone() }),
        ],
        Kind::DbSecurityFolder { db } => vec![
            n("Users", Kind::UsersFolder { db: db.clone() }),
            n("Schemas", Kind::SchemasFolder { db: db.clone() }),
        ],
        Kind::Table { db, schema, name } => {
            let o = (db.clone(), schema.clone(), name.clone());
            vec![
                n("Columns", Kind::ColumnsFolder { db: o.0.clone(), schema: o.1.clone(), name: o.2.clone() }),
                n("Keys", Kind::KeysFolder { db: o.0.clone(), schema: o.1.clone(), name: o.2.clone() }),
                n("Indexes", Kind::IndexesFolder { db: o.0.clone(), schema: o.1.clone(), name: o.2.clone() }),
                n("Triggers", Kind::TriggersFolder { db: o.0, schema: o.1, name: o.2 }),
            ]
        }
        Kind::View { db, schema, name } => vec![n(
            "Columns",
            Kind::ColumnsFolder { db: db.clone(), schema: schema.clone(), name: name.clone() },
        )],
        _ => return None,
    })
}

fn object(db: &str, schema: &str, name: &str) -> String {
    lit(&format!("{}.{}.{}", qi(db), qi(schema), qi(name)))
}

/// SQL that lists a node's children, for nodes that need a query.
pub fn children_sql(kind: &Kind) -> Option<String> {
    Some(match kind {
        Kind::DatabasesFolder | Kind::SystemDatabasesFolder => {
            let system = matches!(kind, Kind::SystemDatabasesFolder);
            format!(
                "SELECT name, state_desc FROM sys.databases WHERE {} database_id <= 4 ORDER BY name",
                if system { "" } else { "NOT" }
            )
        }
        Kind::TablesFolder { db } => format!(
            "SELECT s.name AS [schema], t.name FROM {d}.sys.tables t JOIN {d}.sys.schemas s ON s.schema_id = t.schema_id ORDER BY s.name, t.name",
            d = qi(db)
        ),
        Kind::ViewsFolder { db } => format!(
            "SELECT s.name AS [schema], v.name FROM {d}.sys.views v JOIN {d}.sys.schemas s ON s.schema_id = v.schema_id ORDER BY s.name, v.name",
            d = qi(db)
        ),
        Kind::ProceduresFolder { db } => format!(
            "SELECT s.name AS [schema], p.name FROM {d}.sys.procedures p JOIN {d}.sys.schemas s ON s.schema_id = p.schema_id ORDER BY s.name, p.name",
            d = qi(db)
        ),
        Kind::FunctionsFolder { db } => format!(
            "SELECT s.name AS [schema], o.name, o.type_desc FROM {d}.sys.objects o JOIN {d}.sys.schemas s ON s.schema_id = o.schema_id \
             WHERE o.type IN ('FN','IF','TF','FS','FT','AF') ORDER BY s.name, o.name",
            d = qi(db)
        ),
        Kind::UsersFolder { db } => format!(
            "SELECT name, type_desc FROM {d}.sys.database_principals WHERE type IN ('S','U','G','E','X','C','K') ORDER BY name",
            d = qi(db)
        ),
        Kind::SchemasFolder { db } => format!("SELECT name FROM {}.sys.schemas ORDER BY name", qi(db)),
        Kind::ColumnsFolder { db, schema, name } => format!(
            "SELECT c.name, t.name AS type_name, c.max_length, c.precision, c.scale, c.is_nullable, c.is_identity, c.is_computed, \
             CASE WHEN EXISTS (SELECT 1 FROM {d}.sys.index_columns ic JOIN {d}.sys.indexes i ON i.object_id = ic.object_id AND i.index_id = ic.index_id \
               WHERE i.is_primary_key = 1 AND ic.object_id = c.object_id AND ic.column_id = c.column_id) THEN 1 ELSE 0 END AS is_pk, \
             CASE WHEN EXISTS (SELECT 1 FROM {d}.sys.foreign_key_columns f WHERE f.parent_object_id = c.object_id AND f.parent_column_id = c.column_id) THEN 1 ELSE 0 END AS is_fk \
             FROM {d}.sys.columns c JOIN {d}.sys.types t ON t.user_type_id = c.user_type_id \
             WHERE c.object_id = OBJECT_ID({o}) ORDER BY c.column_id",
            d = qi(db),
            o = object(db, schema, name)
        ),
        Kind::KeysFolder { db, schema, name } => format!(
            "SELECT name, type_desc FROM {d}.sys.key_constraints WHERE parent_object_id = OBJECT_ID({o}) \
             UNION ALL SELECT name, 'FOREIGN_KEY_CONSTRAINT' FROM {d}.sys.foreign_keys WHERE parent_object_id = OBJECT_ID({o}) ORDER BY 1",
            d = qi(db),
            o = object(db, schema, name)
        ),
        Kind::IndexesFolder { db, schema, name } => format!(
            "SELECT name, type_desc, is_unique, is_primary_key FROM {d}.sys.indexes WHERE object_id = OBJECT_ID({o}) AND name IS NOT NULL ORDER BY name",
            d = qi(db),
            o = object(db, schema, name)
        ),
        Kind::TriggersFolder { db, schema, name } => format!(
            "SELECT name, is_disabled FROM {d}.sys.triggers WHERE parent_id = OBJECT_ID({o}) ORDER BY name",
            d = qi(db),
            o = object(db, schema, name)
        ),
        Kind::LoginsFolder => "SELECT name, type_desc, is_disabled FROM sys.server_principals \
             WHERE type IN ('S','U','G','E','X','C','K') AND name NOT LIKE '##%' ORDER BY name"
            .into(),
        Kind::JobsFolder => "SELECT name, enabled FROM msdb.dbo.sysjobs ORDER BY name".into(),
        _ => return None,
    })
}

/// Formats a column's type the way SSMS does: `nvarchar(50)`, `decimal(18,2)`, `varchar(max)`.
pub fn column_type(type_name: &str, max_length: &str, precision: &str, scale: &str) -> String {
    let len: i32 = max_length.parse().unwrap_or(0);
    let t = type_name.to_lowercase();
    match t.as_str() {
        "varchar" | "char" | "varbinary" | "binary" => {
            if len == -1 { format!("{t}(max)") } else { format!("{t}({len})") }
        }
        "nvarchar" | "nchar" => {
            if len == -1 { format!("{t}(max)") } else { format!("{t}({})", len / 2) }
        }
        "decimal" | "numeric" => format!("{t}({precision},{scale})"),
        "datetime2" | "time" | "datetimeoffset" => format!("{t}({scale})"),
        _ => t,
    }
}

pub fn build_children(kind: &Kind, table: &Table, next_id: &mut dyn FnMut() -> u64) -> Vec<Node> {
    let rows = 0..table.rows.len();
    let g = |r: usize, c: &str| table.get(r, c).to_string();
    match kind {
        Kind::DatabasesFolder => {
            let mut v = vec![Node::new(next_id(), "System Databases", Kind::SystemDatabasesFolder)];
            v.extend(rows.map(|r| database_node(next_id(), &g(r, "name"), &g(r, "state_desc"))));
            v
        }
        Kind::SystemDatabasesFolder => {
            rows.map(|r| database_node(next_id(), &g(r, "name"), &g(r, "state_desc"))).collect()
        }
        Kind::TablesFolder { db } | Kind::ViewsFolder { db } | Kind::ProceduresFolder { db } => rows
            .map(|r| {
                let (schema, name) = (g(r, "schema"), g(r, "name"));
                let label = format!("{schema}.{name}");
                let (db, schema2, name2) = (db.clone(), schema, name);
                let k = match kind {
                    Kind::TablesFolder { .. } => Kind::Table { db, schema: schema2, name: name2 },
                    Kind::ViewsFolder { .. } => Kind::View { db, schema: schema2, name: name2 },
                    _ => Kind::Procedure { db, schema: schema2, name: name2 },
                };
                Node::new(next_id(), label, k)
            })
            .collect(),
        Kind::FunctionsFolder { db } => rows
            .map(|r| {
                let (schema, name) = (g(r, "schema"), g(r, "name"));
                Node::new(
                    next_id(),
                    format!("{schema}.{name}"),
                    Kind::Function { db: db.clone(), schema, name, kind: g(r, "type_desc") },
                )
            })
            .collect(),
        Kind::ColumnsFolder { .. } => rows
            .map(|r| {
                let mut tags = Vec::new();
                if g(r, "is_pk") == "1" {
                    tags.push("PK".to_string());
                }
                if g(r, "is_fk") == "1" {
                    tags.push("FK".to_string());
                }
                if g(r, "is_computed") == "1" {
                    tags.push("Computed".to_string());
                }
                tags.push(column_type(&g(r, "type_name"), &g(r, "max_length"), &g(r, "precision"), &g(r, "scale")));
                tags.push(if g(r, "is_nullable") == "1" { "null" } else { "not null" }.to_string());
                let icon = if g(r, "is_pk") == "1" { "🔑" } else { "▫" };
                Node::new(next_id(), format!("{} ({})", g(r, "name"), tags.join(", ")), Kind::Leaf { icon })
            })
            .collect(),
        Kind::KeysFolder { .. } => rows
            .map(|r| Node::new(next_id(), g(r, "name"), Kind::Leaf { icon: "🔑" }))
            .collect(),
        Kind::IndexesFolder { .. } => rows
            .map(|r| {
                let unique = if g(r, "is_unique") == "1" { "Unique, " } else { "" };
                let ty = g(r, "type_desc").to_lowercase().replace('_', " ");
                Node::new(next_id(), format!("{} ({unique}{ty})", g(r, "name")), Kind::Leaf { icon: "⚡" })
            })
            .collect(),
        Kind::TriggersFolder { .. } => rows
            .map(|r| {
                let off = if g(r, "is_disabled") == "1" { " (Disabled)" } else { "" };
                Node::new(next_id(), format!("{}{off}", g(r, "name")), Kind::Leaf { icon: "⚡" })
            })
            .collect(),
        Kind::LoginsFolder => rows
            .map(|r| {
                let off = if g(r, "is_disabled") == "1" { " (disabled)" } else { "" };
                Node::new(next_id(), format!("{}{off}", g(r, "name")), Kind::Leaf { icon: "👤" })
            })
            .collect(),
        Kind::UsersFolder { .. } => rows
            .map(|r| Node::new(next_id(), g(r, "name"), Kind::Leaf { icon: "👤" }))
            .collect(),
        Kind::SchemasFolder { .. } => rows
            .map(|r| Node::new(next_id(), g(r, "name"), Kind::Leaf { icon: "📂" }))
            .collect(),
        Kind::JobsFolder => rows
            .map(|r| {
                let off = if g(r, "enabled") == "0" { " (Disabled)" } else { "" };
                Node::new(next_id(), format!("{}{off}", g(r, "name")), Kind::Leaf { icon: "⏱" })
            })
            .collect(),
        _ => Vec::new(),
    }
}

fn database_node(id: u64, name: &str, state: &str) -> Node {
    let label = if state.is_empty() || state == "ONLINE" {
        name.to_string()
    } else {
        format!("{name} ({})", state.to_lowercase())
    };
    Node::new(id, label, Kind::Database { db: name.to_string() })
}

/// One row whose columns are the Properties window entries for a node.
pub fn properties_sql(kind: &Kind) -> Option<String> {
    Some(match kind {
        Kind::Server => "SELECT CAST(SERVERPROPERTY('ServerName') AS nvarchar(256)) AS [Name], \
             CAST(SERVERPROPERTY('ProductVersion') AS nvarchar(128)) AS [Version], \
             CAST(SERVERPROPERTY('Edition') AS nvarchar(128)) AS [Edition], \
             CAST(SERVERPROPERTY('ProductLevel') AS nvarchar(128)) AS [Product Level], \
             CAST(SERVERPROPERTY('ProductUpdateLevel') AS nvarchar(128)) AS [Update Level], \
             CAST(SERVERPROPERTY('Collation') AS nvarchar(128)) AS [Server Collation], \
             CAST(SERVERPROPERTY('IsClustered') AS nvarchar(10)) AS [Is Clustered], \
             CAST(SERVERPROPERTY('IsHadrEnabled') AS nvarchar(10)) AS [Is HADR Enabled], \
             CAST(SERVERPROPERTY('IsIntegratedSecurityOnly') AS nvarchar(10)) AS [Windows Authentication Only], \
             (SELECT CAST(cpu_count AS nvarchar(10)) FROM sys.dm_os_sys_info) AS [Processors], \
             (SELECT CAST(physical_memory_kb / 1024 AS nvarchar(20)) + ' MB' FROM sys.dm_os_sys_info) AS [Memory], \
             (SELECT CONVERT(nvarchar(30), sqlserver_start_time, 120) FROM sys.dm_os_sys_info) AS [Started], \
             @@VERSION AS [Full Version]"
            .into(),
        Kind::Database { db } => format!(
            "SELECT d.name AS [Name], d.state_desc AS [Status], SUSER_SNAME(d.owner_sid) AS [Owner], \
             CONVERT(nvarchar(30), d.create_date, 120) AS [Date Created], d.recovery_model_desc AS [Recovery Model], \
             d.collation_name AS [Collation], CAST(d.compatibility_level AS nvarchar(10)) AS [Compatibility Level], \
             d.user_access_desc AS [Restrict Access], CAST(d.is_read_only AS nvarchar(5)) AS [Read Only], \
             (SELECT CAST(CAST(SUM(size) * 8 / 1024.0 AS decimal(18,2)) AS nvarchar(30)) + ' MB' FROM sys.master_files f WHERE f.database_id = d.database_id) AS [Size], \
             (SELECT CONVERT(nvarchar(30), MAX(backup_finish_date), 120) FROM msdb.dbo.backupset b WHERE b.database_name = d.name AND b.type = 'D') AS [Last Database Backup] \
             FROM sys.databases d WHERE d.name = {}",
            lit(db)
        ),
        Kind::Table { db, schema, name } => format!(
            "SELECT t.name AS [Name], s.name AS [Schema], CONVERT(nvarchar(30), t.create_date, 120) AS [Created Date], \
             CONVERT(nvarchar(30), t.modify_date, 120) AS [Modified Date], \
             (SELECT CAST(SUM(p.rows) AS nvarchar(30)) FROM {d}.sys.partitions p WHERE p.object_id = t.object_id AND p.index_id IN (0, 1)) AS [Row Count], \
             (SELECT CAST(CAST(SUM(a.used_pages) * 8 / 1024.0 AS decimal(18,2)) AS nvarchar(30)) + ' MB' FROM {d}.sys.partitions p JOIN {d}.sys.allocation_units a ON a.container_id = p.partition_id WHERE p.object_id = t.object_id) AS [Data Space Used], \
             t.lock_escalation_desc AS [Lock Escalation], CAST(t.is_memory_optimized AS nvarchar(5)) AS [Memory Optimized], \
             t.temporal_type_desc AS [Temporal Type] \
             FROM {d}.sys.tables t JOIN {d}.sys.schemas s ON s.schema_id = t.schema_id WHERE t.object_id = OBJECT_ID({o})",
            d = qi(db),
            o = object(db, schema, name)
        ),
        Kind::View { db, schema, name } | Kind::Procedure { db, schema, name } | Kind::Function { db, schema, name, .. } => format!(
            "SELECT o.name AS [Name], s.name AS [Schema], o.type_desc AS [Type], CONVERT(nvarchar(30), o.create_date, 120) AS [Created Date], \
             CONVERT(nvarchar(30), o.modify_date, 120) AS [Modified Date], \
             CAST(OBJECTPROPERTY(o.object_id, 'IsEncrypted') AS nvarchar(5)) AS [Encrypted], \
             CAST(OBJECTPROPERTY(o.object_id, 'IsSchemaBound') AS nvarchar(5)) AS [Schema Bound] \
             FROM {d}.sys.objects o JOIN {d}.sys.schemas s ON s.schema_id = o.schema_id WHERE o.object_id = OBJECT_ID({o})",
            d = qi(db),
            o = object(db, schema, name)
        ),
        _ => return None,
    })
}

// ----- Scripting --------------------------------------------------------

pub fn select_top(db: &str, schema: &str, name: &str, columns: &[String], n: u32) -> String {
    let cols = if columns.is_empty() {
        "*".to_string()
    } else {
        columns.iter().map(|c| qi(c)).collect::<Vec<_>>().join("\n      ,")
    };
    format!(
        "/****** Script for SelectTopNRows command ******/\nSELECT TOP ({n}) {cols}\n  FROM {}.{}.{}\n",
        qi(db),
        qi(schema),
        qi(name)
    )
}

/// SQL that fetches what `script_create_table` needs: columns, then the primary key.
pub fn table_definition_sql(db: &str, schema: &str, name: &str) -> String {
    let o = object(db, schema, name);
    format!(
        "SELECT c.name, t.name AS type_name, c.max_length, c.precision, c.scale, c.is_nullable, c.is_identity, \
         CAST(ic.seed_value AS nvarchar(40)) AS seed, CAST(ic.increment_value AS nvarchar(40)) AS incr, \
         dc.name AS default_name, dc.definition AS default_def, cc.definition AS computed, cc.is_persisted \
         FROM {d}.sys.columns c JOIN {d}.sys.types t ON t.user_type_id = c.user_type_id \
         LEFT JOIN {d}.sys.identity_columns ic ON ic.object_id = c.object_id AND ic.column_id = c.column_id \
         LEFT JOIN {d}.sys.default_constraints dc ON dc.object_id = c.default_object_id \
         LEFT JOIN {d}.sys.computed_columns cc ON cc.object_id = c.object_id AND cc.column_id = c.column_id \
         WHERE c.object_id = OBJECT_ID({o}) ORDER BY c.column_id;\n\
         SELECT k.name AS constraint_name, k.type, col.name AS column_name, ixc.is_descending_key, i.type_desc \
         FROM {d}.sys.key_constraints k \
         JOIN {d}.sys.indexes i ON i.object_id = k.parent_object_id AND i.index_id = k.unique_index_id \
         JOIN {d}.sys.index_columns ixc ON ixc.object_id = i.object_id AND ixc.index_id = i.index_id \
         JOIN {d}.sys.columns col ON col.object_id = ixc.object_id AND col.column_id = ixc.column_id \
         WHERE k.parent_object_id = OBJECT_ID({o}) ORDER BY k.type, k.name, ixc.key_ordinal;",
        d = qi(db)
    )
}

pub fn script_create_table(db: &str, schema: &str, name: &str, tables: &[Table]) -> String {
    let Some(cols) = tables.first() else {
        return String::new();
    };
    let mut lines = Vec::new();
    for r in 0..cols.rows.len() {
        let g = |c: &str| cols.get(r, c);
        let mut line = format!("\t{} ", qi(g("name")));
        if !g("computed").is_empty() {
            line = format!("\t{} AS {}", qi(g("name")), g("computed"));
            if g("is_persisted") == "1" {
                line.push_str(" PERSISTED");
            }
            lines.push(line);
            continue;
        }
        let ty = column_type(g("type_name"), g("max_length"), g("precision"), g("scale"));
        line.push_str(&format!("[{}]{}", g("type_name"), &ty[g("type_name").len()..]));
        if g("is_identity") == "1" {
            line.push_str(&format!(" IDENTITY({},{})", g("seed"), g("incr")));
        }
        line.push_str(if g("is_nullable") == "1" { " NULL" } else { " NOT NULL" });
        if !g("default_def").is_empty() {
            line.push_str(&format!(" CONSTRAINT {} DEFAULT {}", qi(g("default_name")), g("default_def")));
        }
        lines.push(line);
    }
    // Primary key and unique constraints, grouped by name.
    if let Some(keys) = tables.get(1) {
        let mut i = 0;
        while i < keys.rows.len() {
            let cname = keys.get(i, "constraint_name").to_string();
            let kind = if keys.get(i, "type") == "PK" { "PRIMARY KEY" } else { "UNIQUE" };
            let clustered = keys.get(i, "type_desc").to_string();
            let mut cols_in = Vec::new();
            while i < keys.rows.len() && keys.get(i, "constraint_name") == cname {
                let dir = if keys.get(i, "is_descending_key") == "1" { "DESC" } else { "ASC" };
                cols_in.push(format!("\t{} {dir}", qi(keys.get(i, "column_name"))));
                i += 1;
            }
            lines.push(format!(
                " CONSTRAINT {} {kind} {}\n(\n{}\n)",
                qi(&cname),
                clustered,
                cols_in.join(",\n")
            ));
        }
    }
    format!(
        "USE {}\nGO\n\nSET ANSI_NULLS ON\nGO\n\nSET QUOTED_IDENTIFIER ON\nGO\n\nCREATE TABLE {}.{}(\n{}\n)\nGO\n",
        qi(db),
        qi(schema),
        qi(name),
        lines.join(",\n")
    )
}

/// SQL returning an object's source (views, procedures, functions, triggers).
pub fn definition_sql(db: &str, schema: &str, name: &str) -> String {
    format!(
        "SELECT m.definition FROM {d}.sys.sql_modules m WHERE m.object_id = OBJECT_ID({o})",
        d = qi(db),
        o = object(db, schema, name)
    )
}

/// Turns `CREATE PROCEDURE` into `CREATE OR ALTER PROCEDURE` for "Modify".
pub fn as_alter(definition: &str) -> String {
    let lower = definition.to_lowercase();
    if let Some(pos) = lower.find("create") {
        let after = &lower[pos + 6..];
        if !after.trim_start().starts_with("or ") {
            return format!("{}CREATE OR ALTER{}", &definition[..pos], &definition[pos + 6..]);
        }
    }
    definition.to_string()
}

pub fn exec_sql(db: &str, schema: &str, name: &str) -> String {
    format!(
        "SELECT p.name, TYPE_NAME(p.user_type_id) AS type_name, p.is_output FROM {d}.sys.parameters p \
         WHERE p.object_id = OBJECT_ID({o}) ORDER BY p.parameter_id",
        d = qi(db),
        o = object(db, schema, name)
    )
}

pub fn script_exec(db: &str, schema: &str, name: &str, params: &Table) -> String {
    let mut decl = Vec::new();
    let mut args = Vec::new();
    for r in 0..params.rows.len() {
        let p = params.get(r, "name");
        if p.is_empty() {
            continue;
        }
        decl.push(format!("DECLARE {p} {}", params.get(r, "type_name")));
        let out = if params.get(r, "is_output") == "1" { " OUTPUT" } else { "" };
        args.push(format!("{p} = {p}{out}"));
    }
    let mut s = format!("USE {}\nGO\n\nDECLARE @return_value int\n", qi(db));
    for d in &decl {
        s.push_str(d);
        s.push('\n');
    }
    s.push_str(&format!("\nEXEC @return_value = {}.{}", qi(schema), qi(name)));
    if !args.is_empty() {
        s.push_str("\n\t\t");
        s.push_str(&args.join(",\n\t\t"));
    }
    s.push_str("\n\nSELECT 'Return Value' = @return_value\nGO\n");
    s
}

pub fn script_drop(db: &str, what: &str, schema: &str, name: &str) -> String {
    format!("USE {}\nGO\n\nDROP {what} IF EXISTS {}.{}\nGO\n", qi(db), qi(schema), qi(name))
}

pub fn script_insert(db: &str, schema: &str, name: &str, columns: &[(String, String)]) -> String {
    let names: Vec<_> = columns.iter().map(|(n, _)| qi(n)).collect();
    let values: Vec<_> = columns.iter().map(|(n, t)| format!("<{n}, {t},>")).collect();
    format!(
        "INSERT INTO {}.{}.{}\n           ({})\n     VALUES\n           ({})\nGO\n",
        qi(db),
        qi(schema),
        qi(name),
        names.join("\n           ,"),
        values.join("\n           ,")
    )
}

pub fn script_update(db: &str, schema: &str, name: &str, columns: &[(String, String)]) -> String {
    let sets: Vec<_> = columns.iter().map(|(n, t)| format!("{} = <{n}, {t},>", qi(n))).collect();
    format!(
        "UPDATE {}.{}.{}\n   SET {}\n WHERE <Search Conditions,,>\nGO\n",
        qi(db),
        qi(schema),
        qi(name),
        sets.join("\n      ,")
    )
}

pub fn script_delete(db: &str, schema: &str, name: &str) -> String {
    format!("DELETE FROM {}.{}.{}\n      WHERE <Search Conditions,,>\nGO\n", qi(db), qi(schema), qi(name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn types() {
        assert_eq!(column_type("nvarchar", "100", "0", "0"), "nvarchar(50)");
        assert_eq!(column_type("varchar", "-1", "0", "0"), "varchar(max)");
        assert_eq!(column_type("decimal", "9", "18", "2"), "decimal(18,2)");
        assert_eq!(column_type("int", "4", "10", "0"), "int");
    }

    #[test]
    fn alter() {
        assert_eq!(as_alter("CREATE PROCEDURE x AS SELECT 1"), "CREATE OR ALTER PROCEDURE x AS SELECT 1");
        assert_eq!(as_alter("create or alter view v as select 1"), "create or alter view v as select 1");
    }

    #[test]
    fn create_table() {
        let cols = Table {
            columns: ["name", "type_name", "max_length", "precision", "scale", "is_nullable", "is_identity", "seed", "incr", "default_name", "default_def", "computed", "is_persisted"]
                .map(String::from)
                .to_vec(),
            rows: vec![
                ["Id", "int", "4", "10", "0", "0", "1", "1", "1", "", "", "", ""].map(|s| Some(s.to_string())).to_vec(),
                ["Name", "nvarchar", "100", "0", "0", "1", "0", "", "", "", "", "", ""].map(|s| Some(s.to_string())).to_vec(),
            ],
        };
        let keys = Table {
            columns: ["constraint_name", "type", "column_name", "is_descending_key", "type_desc"].map(String::from).to_vec(),
            rows: vec![["PK_T", "PK", "Id", "0", "CLUSTERED"].map(|s| Some(s.to_string())).to_vec()],
        };
        let s = script_create_table("db", "dbo", "T", &[cols, keys]);
        assert!(s.contains("[Id] [int] IDENTITY(1,1) NOT NULL"), "{s}");
        assert!(s.contains("[Name] [nvarchar](50) NULL"), "{s}");
        assert!(s.contains("CONSTRAINT [PK_T] PRIMARY KEY CLUSTERED"), "{s}");
    }
}
