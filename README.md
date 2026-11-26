# SQLCraft

A SQL Server management tool for Linux in the style of SSMS, written in Rust
(egui + tiberius) and packaged as a Flatpak.

## Install on Bazzite / Fedora Atomic / any Flatpak desktop

Download `SQLCraft.flatpak` from the latest
[release](https://github.com/LoganHoney/SQLCraft/releases/latest) and run:

```sh
flatpak install --user SQLCraft.flatpak
flatpak run io.github.loganhoney.SQLCraft    # or "SQLCraft" in the app menu
```

The freedesktop 25.08 runtime comes from Flathub automatically (Bazzite already
has Flathub set up).

## Build the Flatpak yourself

Needs `flatpak` and `flatpak-builder` (Bazzite: `flatpak install flathub
org.flatpak.Builder` and use `flatpak run org.flatpak.Builder` in place of
`flatpak-builder`; Ubuntu: `sudo apt install flatpak flatpak-builder`).

```sh
flatpak/build.sh            # build + install for your user
flatpak/build.sh --bundle   # also write SQLCraft.flatpak
```

Every push to `main` builds the bundle in GitHub Actions; pushing a `v*` tag
publishes it as a release.

## Run from source

```sh
cargo run --release
```

## What it does

- **Connect to Server**: `host`, `host,port`, `host\instance` (via SQL Browser)
  or `tcp:host,port`; Windows Authentication against Active Directory
  (Kerberos, see below), Windows NTLM (`DOMAIN\user` + password) or SQL Server logins;
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

## Active Directory

Choose **Windows Authentication (Active Directory)**. Then either:

- leave User name and Password blank to use the Kerberos ticket you already
  have: you signed in to a domain-joined machine, or ran `kinit user@CORP.EXAMPLE.COM`; or
- type `user@CORP.EXAMPLE.COM` (or `CORP\user`) and your password, and SQLCraft
  gets a ticket for you, like `kinit` does.

The dialog shows which domain account you are signed in as. Use the server's
full DNS name (`sql01.corp.example.com`); short names are expanded through DNS.
If the domain registered a different service principal, set it under
Options → Service principal (SPN).

The Flatpak reads the host's `/etc/krb5.conf` (read-only) and uses the KCM
ticket cache that Fedora-based systems such as Bazzite use by default, so
tickets from `kinit` on the host and from SQLCraft are shared. For the domain
to be found, the machine needs either `realm join`, or a `krb5.conf` with
`default_realm` (DNS SRV records locate the domain controllers).

## Not yet

Graphical execution plans, the table designer and Edit Top 200 Rows, Activity
Monitor, Agent job management, IntelliSense, Microsoft Entra ID (Azure AD)
sign-in, and the AD user/group picker on the New Login dialog.

## Layout

| Path | What |
|---|---|
| `src/db.rs` | Connections: one tokio task per session, streaming results to the UI |
| `src/explorer.rs` | Object Explorer queries and scripting |
| `src/app.rs` | The UI |
| `src/batch.rs` | `GO` batch splitter |
| `src/kerberos.rs` | Active Directory sign-in, SPN lookup, sandboxed krb5.conf |
| `vendor/tiberius` | tiberius 0.13 with a small patch for messages and row counts (see `PATCHED.md`) |
| `flatpak/` | Manifest, desktop file, AppStream metadata, icon, build script |

After changing dependencies, `flatpak/build.sh` regenerates
`flatpak/cargo-sources.json` from `Cargo.lock` (via `flatpak/gen-cargo-sources.py`)
so the Flatpak build stays offline.
