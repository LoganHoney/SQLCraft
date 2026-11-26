# SQL Studio

A SQL Server management tool for Linux in the style of SSMS, written in Rust
(egui + tiberius) and packaged as a Flatpak.

## Install as a Flatpak

On the Linux desktop where you want it (needs `flatpak` and `flatpak-builder`):

```sh
sudo apt install flatpak flatpak-builder   # once
flatpak/build.sh                           # build + install for your user
flatpak run io.github.sqlstudio.SqlStudio  # or launch "SQL Studio" from the app menu
```

`flatpak/build.sh --bundle` also writes `SqlStudio.flatpak`, a single file you
can copy to another machine and install with `flatpak install --user SqlStudio.flatpak`.

The first build downloads the freedesktop 25.08 runtime and the Rust SDK
extension from Flathub (about 1 GB), then builds offline.

## Run from source

```sh
cargo run --release
```

## What it does

- **Connect to Server**: `host`, `host,port`, `host\instance` (via SQL Browser)
  or `tcp:host,port`; SQL Server or Windows (NTLM, `DOMAIN\user`) logins;
  Encrypt Mandatory/Optional/Strict/Off and Trust Server Certificate; recent
  connections, with passwords kept in the system keyring if you ask. Azure SQL
  redirects are followed.
- **Object Explorer**: Databases (user and system) → Tables / Views /
  Programmability (Stored Procedures, Functions) / Security (Users, Schemas);
  each table has Columns (with PK/FK, type and nullability), Keys, Indexes and
  Triggers. Server Security → Logins, SQL Server Agent → Jobs. Items load when
  expanded; Refresh reloads them.
- **Right-click scripting**: Select Top 1000 Rows; Script Table as
  CREATE/SELECT/INSERT/UPDATE/DELETE/DROP; Modify and Script as CREATE/DROP for
  views, procedures and functions; Execute Stored Procedure (an EXEC template
  with the parameters filled in).
- **Query windows**: each window has its own connection, as in SSMS. T-SQL
  highlighting, line numbers, `GO` / `GO n` batches, F5 runs the selection if
  there is one, a database picker, and Cancel (sends a real TDS attention, so
  the server stops too).
- **Results**: a grid per result set (NULLs highlighted, right-aligned numbers,
  resizable columns, copy cell/row/column/all), plus a Messages tab with
  `PRINT`/`RAISERROR` output, `(n rows affected)` and `Msg n, Level n, State n,
  Line n` errors mapped to editor lines. Save Results As CSV.
- Open/save `.sql` files (SSMS's UTF-16 files are read correctly), unsaved
  scripts restored on the next start, dark/light theme, font size.

Shortcuts: F5/Ctrl+E execute, Alt+Break cancel, Ctrl+N new query, Ctrl+O/Ctrl+S
open/save, Ctrl+W close, Ctrl+Tab next window, Ctrl+R results pane, F8 Object
Explorer.

## Not yet

Graphical execution plans, the table designer and Edit Top 200 Rows, Activity
Monitor, Agent job management, IntelliSense, and Kerberos (integrated)
authentication.

## Layout

| Path | What |
|---|---|
| `src/db.rs` | Connections: one tokio task per session, streaming results to the UI |
| `src/explorer.rs` | Object Explorer queries and scripting |
| `src/app.rs` | The UI |
| `src/batch.rs` | `GO` batch splitter |
| `vendor/tiberius` | tiberius 0.13 with a small patch for messages and row counts (see `PATCHED.md`) |
| `flatpak/` | Manifest, desktop file, AppStream metadata, icon, build script |

After changing dependencies, `flatpak/build.sh` regenerates
`flatpak/cargo-sources.json` from `Cargo.lock` (via `flatpak/gen-cargo-sources.py`)
so the Flatpak build stays offline.
