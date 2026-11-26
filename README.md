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

Laid out like SSMS 22:

- **Window**: Visual Studio-style docking. Object Explorer, Properties (F4),
  Output and Template Browser are tool windows you can drag, dock, tab together
  or float; query windows are document tabs captioned
  `SQLQuery1.sql - SQL01.master (CORP\you (57))`. Dark, Light and Blue themes
  (Tools → Options). View → Reset Window Layout restores the default.
- **Menus and toolbars** follow SSMS: File, Edit, View, Query, Tools, Window and
  Help; the Standard and SQL Editor toolbars (database picker, Execute, Cancel,
  Parse, Results to Text/Grid/File, comment/uncomment, indent, template
  parameters). Features not built yet are shown greyed out.
- **Connect to Server**: `host`, `host,port`, `host\instance` (via SQL Browser)
  or `tcp:host,port`; Windows Authentication against Active Directory
  (Kerberos, see below), Windows NTLM (`DOMAIN\user` + password) or SQL Server
  logins; Encryption, Trust server certificate, and **Use custom color** to tint
  a connection's status bar (red for production). Passwords can be kept in the
  system keyring.
- **Object Explorer**: Databases (user and system) → Tables / Views /
  Programmability / Security; table Columns, Keys, Indexes and Triggers; server
  Logins and Agent Jobs. Right-click for Select Top 1000 Rows, Script Table as
  CREATE/SELECT/INSERT/UPDATE/DELETE/DROP, Modify, Execute Stored Procedure,
  New Query, Refresh. The Properties window shows the selected server, database,
  table, view, procedure or function.
- **Query editor**: SSMS's T-SQL colours (blue keywords, red strings, green
  comments, magenta functions, grey operators, maroon system procedures), line
  numbers, `GO` / `GO n`, F5 runs the selection if there is one, Parse, Cancel
  (stops the query on the server), Quick Find / Replace, Go To Line, comment and
  indent commands, upper/lower case, Alt+F1 / Ctrl+1 / Ctrl+2, Template Browser
  with Ctrl+Shift+M parameters, and the Ln/Col/Ch status bar.
- **Results**: Results to Grid (one grid per result set, stacked like SSMS;
  NULLs highlighted; copy cell/row/column/all with headers; save as CSV),
  Results to Text (fixed-width, messages in line) and Results to File (`.rpt`),
  plus the Messages tab with `PRINT` output, `(n rows affected)` and
  `Msg n, Level n, State n, Line n` errors mapped to editor lines.
- Open/save `.sql` files (SSMS's UTF-16 files are read correctly), Recent
  Files, unsaved scripts restored on the next start.

### Keyboard shortcuts (SSMS defaults)

| Keys | Command |
|---|---|
| F5, Ctrl+E / Ctrl+F5 / Alt+Break | Execute / Parse / Cancel |
| Ctrl+N, Ctrl+O, Ctrl+S, Ctrl+Shift+S, Ctrl+F4 | New query, Open, Save, Save All, Close |
| Ctrl+R / Ctrl+D / Ctrl+T / Ctrl+Shift+F | Results pane / to Grid / to Text / to File |
| Ctrl+F, Ctrl+H, F3, Ctrl+G | Find, Replace, Find Next, Go To Line |
| Ctrl+K Ctrl+C / Ctrl+K Ctrl+U | Comment / Uncomment |
| Ctrl+Shift+U / Ctrl+Shift+L | Uppercase / Lowercase |
| Tab / Shift+Tab on a selection | Indent / Outdent |
| Ctrl+Shift+M | Specify values for template parameters |
| Alt+F1, Ctrl+1, Ctrl+2 | sp_help, sp_who, sp_lock |
| F8, F4, Ctrl+Alt+T, Ctrl+Alt+O | Object Explorer, Properties, Template Browser, Output |
| Ctrl+Tab, Shift+Alt+Enter | Next document, Full screen |

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

IntelliSense, graphical execution plans and client statistics, the table
designer and Edit Top 200 Rows, database/login properties dialogs, Generate
Scripts, Backup/Restore dialogs, Activity Monitor, Agent job management,
XEvent Profiler, Registered Servers, Microsoft Entra ID (Azure AD) sign-in, and
the AD user/group picker on the New Login dialog.

## Layout

| Path | What |
|---|---|
| `src/db.rs` | Connections: one tokio task per session, streaming results to the UI |
| `src/explorer.rs` | Object Explorer queries and scripting |
| `src/app.rs` | Application state and commands |
| `src/ui.rs` | Shell: docking, menus, toolbars, tool windows |
| `src/query_ui.rs` | Query window: editor, results, status bar |
| `src/dialogs.rs` | Connect, Options, Go To Line, template parameters |
| `src/tsql.rs` | T-SQL colouring and editor line commands |
| `src/theme.rs` | Dark / Light / Blue themes |
| `src/templates.rs` | Template Browser contents |
| `src/batch.rs` | `GO` batch splitter |
| `src/kerberos.rs` | Active Directory sign-in, SPN lookup, sandboxed krb5.conf |
| `vendor/tiberius` | tiberius 0.13 with a small patch for messages and row counts (see `PATCHED.md`) |
| `flatpak/` | Manifest, desktop file, AppStream metadata, icon, build script |

After changing dependencies, `flatpak/build.sh` regenerates
`flatpak/cargo-sources.json` from `Cargo.lock` (via `flatpak/gen-cargo-sources.py`)
so the Flatpak build stays offline.
