//! Turns SQL Server values into the text SSMS shows in the grid.

use chrono::{DateTime, FixedOffset, NaiveDate, NaiveDateTime, NaiveTime};
use tiberius::{ColumnData, ColumnType, FromSql};

/// Binary values longer than this are cut off in the grid, as SSMS does.
const MAX_BINARY_BYTES: usize = 32 * 1024;

pub fn is_numeric(ty: ColumnType) -> bool {
    use ColumnType::*;
    matches!(
        ty,
        Int1 | Int2 | Int4 | Int8 | Intn | Float4 | Float8 | Floatn | Money | Money4 | Decimaln
            | Numericn
    )
}

pub fn type_name(ty: ColumnType) -> &'static str {
    use ColumnType::*;
    match ty {
        Null => "null",
        Bit | Bitn => "bit",
        Int1 => "tinyint",
        Int2 => "smallint",
        Int4 => "int",
        Int8 => "bigint",
        Intn => "int",
        Datetime4 => "smalldatetime",
        Datetime | Datetimen => "datetime",
        Float4 => "real",
        Float8 | Floatn => "float",
        Money => "money",
        Money4 => "smallmoney",
        Guid => "uniqueidentifier",
        Decimaln | Numericn => "decimal",
        Daten => "date",
        Timen => "time",
        Datetime2 => "datetime2",
        DatetimeOffsetn => "datetimeoffset",
        BigVarBin | BigBinary | Image => "varbinary",
        BigVarChar | BigChar | Text => "varchar",
        NVarchar | NChar | NText => "nvarchar",
        Xml => "xml",
        Udt => "udt",
        SSVariant => "sql_variant",
    }
}

fn hex(bytes: &[u8]) -> String {
    let shown = &bytes[..bytes.len().min(MAX_BINARY_BYTES)];
    let mut s = String::with_capacity(2 + shown.len() * 2);
    s.push_str("0x");
    for b in shown {
        s.push_str(&format!("{b:02X}"));
    }
    s
}

/// `None` means SQL NULL.
pub fn cell(data: &ColumnData<'static>, ty: ColumnType) -> Option<String> {
    match data {
        ColumnData::U8(v) => v.map(|v| v.to_string()),
        ColumnData::I16(v) => v.map(|v| v.to_string()),
        ColumnData::I32(v) => v.map(|v| v.to_string()),
        ColumnData::I64(v) => v.map(|v| v.to_string()),
        ColumnData::F32(v) => v.map(|v| v.to_string()),
        ColumnData::F64(v) => v.map(|v| match ty {
            ColumnType::Money | ColumnType::Money4 => format!("{v:.4}"),
            _ => v.to_string(),
        }),
        ColumnData::Bit(v) => v.map(|v| if v { "1" } else { "0" }.to_string()),
        ColumnData::String(v) => v.as_ref().map(|s| s.to_string()),
        ColumnData::Guid(v) => v.map(|g| g.to_string().to_uppercase()),
        ColumnData::Binary(v) => v.as_ref().map(|b| hex(b)),
        ColumnData::Numeric(v) => v.map(|n| n.to_string()),
        ColumnData::Xml(v) => v.as_ref().map(|x| x.to_string()),
        ColumnData::DateTime(_) => NaiveDateTime::from_sql(data)
            .ok()
            .flatten()
            .map(|d| d.format("%Y-%m-%d %H:%M:%S%.3f").to_string()),
        ColumnData::SmallDateTime(_) => NaiveDateTime::from_sql(data)
            .ok()
            .flatten()
            .map(|d| d.format("%Y-%m-%d %H:%M:%S").to_string()),
        ColumnData::DateTime2(_) => NaiveDateTime::from_sql(data)
            .ok()
            .flatten()
            .map(|d| d.format("%Y-%m-%d %H:%M:%S%.f").to_string()),
        ColumnData::Date(_) => NaiveDate::from_sql(data)
            .ok()
            .flatten()
            .map(|d| d.format("%Y-%m-%d").to_string()),
        ColumnData::Time(_) => NaiveTime::from_sql(data)
            .ok()
            .flatten()
            .map(|d| d.format("%H:%M:%S%.f").to_string()),
        ColumnData::DateTimeOffset(_) => DateTime::<FixedOffset>::from_sql(data)
            .ok()
            .flatten()
            .map(|d| d.format("%Y-%m-%d %H:%M:%S%.f %:z").to_string()),
    }
}

/// Quotes an identifier: `a]b` becomes `[a]]b]`.
pub fn qi(name: &str) -> String {
    format!("[{}]", name.replace(']', "]]"))
}

/// Quotes a string literal as N'...'.
pub fn lit(s: &str) -> String {
    format!("N'{}'", s.replace('\'', "''"))
}

/// One CSV field, quoted when needed (RFC 4180).
pub fn csv_field(s: &str) -> String {
    if s.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quoting() {
        assert_eq!(qi("a]b"), "[a]]b]");
        assert_eq!(lit("O'Neil"), "N'O''Neil'");
        assert_eq!(csv_field("a,b"), "\"a,b\"");
        assert_eq!(csv_field("say \"hi\""), "\"say \"\"hi\"\"\"");
        assert_eq!(csv_field("plain"), "plain");
    }

    #[test]
    fn values() {
        assert_eq!(cell(&ColumnData::I32(None), ColumnType::Int4), None);
        assert_eq!(cell(&ColumnData::Bit(Some(true)), ColumnType::Bit).unwrap(), "1");
        assert_eq!(
            cell(&ColumnData::F64(Some(1.5)), ColumnType::Money).unwrap(),
            "1.5000"
        );
        assert_eq!(
            cell(&ColumnData::Binary(Some(vec![0xde, 0xad].into())), ColumnType::BigVarBin)
                .unwrap(),
            "0xDEAD"
        );
    }
}
