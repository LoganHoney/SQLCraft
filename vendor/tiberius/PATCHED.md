# Local patch to tiberius 0.13.0

Upstream tiberius parses INFO, DONE and ENVCHANGE tokens but does not hand
them to callers, so a query tool cannot show `PRINT` output, "(N rows
affected)" or follow `USE` statements. This copy adds, without changing any
existing API:

- `QueryStream::drain_events()` returning `QueryEvent`s in arrival order:
  `Message(ServerMessage)` for INFO and ERROR tokens, `RowCount(u64)` for
  DONE/DONEINPROC/DONEPROC tokens that carry a valid count, and
  `DatabaseChanged(String)`.
- `TokenDone::row_count()`.

Files touched: `src/tds/stream/query.rs`, `src/tds/codec/token/token_done.rs`,
`src/lib.rs`. Everything else is the crates.io 0.13.0 release
(MIT OR Apache-2.0, see the LICENSE files).

Also added `Config::service_principal_name()` to override the Kerberos SPN
(the default `MSSQLSvc/<host>:<port>` is wrong for short host names and for
named instances reached through SQL Browser). Touches `src/client/config.rs`,
`src/client/connection.rs` and `src/tds/context.rs`.
