//! T-SQL colouring in SSMS's scheme, and the editor commands that work on
//! lines of text (comment, indent, change case).

use eframe::egui::{Color32, FontId, text::LayoutJob, text::TextFormat};

use crate::theme::Palette;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Text,
    Keyword,
    String,
    Comment,
    Function,
    SystemObject,
    SystemProc,
    Operator,
    Number,
}

const KEYWORDS: &[&str] = &[
    "ADD", "ALTER", "AS", "ASC", "AUTHORIZATION", "BACKUP", "BEGIN", "BREAK", "BROWSE", "BULK", "BY", "CASCADE", "CASE",
    "CATCH", "CHECK", "CHECKPOINT", "CLOSE", "CLUSTERED", "COLLATE", "COLUMN", "COMMIT", "COMPUTE", "CONSTRAINT",
    "CONTAINS", "CONTAINSTABLE", "CONTINUE", "CREATE", "CURRENT", "CURSOR", "DATABASE", "DBCC", "DEALLOCATE", "DECLARE",
    "DEFAULT", "DELETE", "DENY", "DESC", "DISK", "DISTINCT", "DISTRIBUTED", "DOUBLE", "DROP", "DUMP", "ELSE", "END",
    "ERRLVL", "ESCAPE", "EXCEPT", "EXEC", "EXECUTE", "EXIT", "EXTERNAL", "FETCH", "FILE", "FILLFACTOR", "FOR", "FOREIGN",
    "FREETEXT", "FREETEXTTABLE", "FROM", "FUNCTION", "GO", "GOTO", "GRANT", "GROUP", "HAVING", "HOLDLOCK", "IDENTITY",
    "IDENTITY_INSERT", "IDENTITYCOL", "IF", "INDEX", "INSERT", "INTERSECT", "INTO", "KEY", "KILL", "LINENO", "LOAD",
    "MERGE", "NATIONAL", "NOCHECK", "NOCOUNT", "NONCLUSTERED", "OF", "OFF", "OFFSETS", "ON", "OPEN", "OPENDATASOURCE",
    "OPENQUERY", "OPENROWSET", "OPENXML", "OPTION", "ORDER", "OVER", "PERCENT", "PIVOT", "PLAN", "PRECISION", "PRIMARY",
    "PRINT", "PROC", "PROCEDURE", "PUBLIC", "RAISERROR", "READ", "READTEXT", "RECONFIGURE", "REFERENCES", "REPLICATION",
    "RESTORE", "RESTRICT", "RETURN", "RETURNS", "REVERT", "REVOKE", "ROLLBACK", "ROWCOUNT", "ROWGUIDCOL", "RULE", "SAVE",
    "SCHEMA", "SECURITYAUDIT", "SELECT", "SET", "SETUSER", "SHUTDOWN", "STATISTICS", "TABLE", "TABLESAMPLE", "TEXTSIZE",
    "THEN", "THROW", "TO", "TOP", "TRAN", "TRANSACTION", "TRIGGER", "TRUNCATE", "TRY", "TSEQUAL", "UNION", "UNIQUE",
    "UNPIVOT", "UPDATE", "UPDATETEXT", "USE", "USER", "VALUES", "VARYING", "VIEW", "WAITFOR", "WHEN", "WHERE", "WHILE",
    "WITH", "WITHIN", "WRITETEXT", "OUTPUT", "MAX", "PARTITION", "ROWS", "RANGE", "UNBOUNDED", "PRECEDING", "FOLLOWING",
    "OFFSET", "NEXT", "ONLY", "FIRST", "LAST", "MATCHED", "SOURCE", "TARGET", "APPLY", "READONLY", "RECOMPILE",
    "ENCRYPTION", "SCHEMABINDING", "NOLOCK", "READUNCOMMITTED", "XACT_ABORT", "ANSI_NULLS", "QUOTED_IDENTIFIER",
    "PARSEONLY", "SHOWPLAN_XML", "LOGIN", "PASSWORD", "ROLE", "WINDOWS", "PERSISTED", "INCLUDE", "LOCAL", "GLOBAL",
    "STATIC", "FAST_FORWARD", "FORWARD_ONLY", "SCROLL", "DYNAMIC", "KEYSET", "TIES", "AFTER", "INSTEAD", "ENABLE",
    "DISABLE", "REBUILD", "REORGANIZE", "ONLINE", "SEQUENCE", "SYNONYM", "TYPE", "ASSEMBLY", "CERTIFICATE", "BINARY",
    "VARBINARY", "CHAR", "VARCHAR", "NCHAR", "NVARCHAR", "TEXT", "NTEXT", "INT", "BIGINT", "SMALLINT", "TINYINT", "BIT",
    "DECIMAL", "NUMERIC", "MONEY", "SMALLMONEY", "FLOAT", "REAL", "DATE", "TIME", "DATETIME", "DATETIME2",
    "SMALLDATETIME", "DATETIMEOFFSET", "UNIQUEIDENTIFIER", "XML", "SQL_VARIANT", "IMAGE", "TIMESTAMP", "ROWVERSION",
    "HIERARCHYID", "GEOGRAPHY", "GEOMETRY", "SYSNAME",
];

/// Shown grey in SSMS, like operators.
const OPERATOR_WORDS: &[&str] = &[
    "ALL", "AND", "ANY", "BETWEEN", "CROSS", "EXISTS", "FULL", "IN", "INNER", "IS", "JOIN", "LEFT", "LIKE", "NOT",
    "NULL", "OR", "OUTER", "RIGHT", "SOME", "PIVOT",
];

/// Shown magenta in SSMS.
const FUNCTIONS: &[&str] = &[
    "ABS", "APP_NAME", "ASCII", "AVG", "CAST", "CEILING", "CHARINDEX", "CHECKSUM", "CHOOSE", "COALESCE", "COL_NAME",
    "CONCAT", "CONCAT_WS", "CONVERT", "COUNT", "COUNT_BIG", "CUME_DIST", "CURRENT_TIMESTAMP", "CURRENT_USER",
    "DATABASEPROPERTYEX", "DATALENGTH", "DATEADD", "DATEDIFF", "DATEDIFF_BIG", "DATEFROMPARTS", "DATENAME", "DATEPART",
    "DAY", "DB_ID", "DB_NAME", "DENSE_RANK", "EOMONTH", "ERROR_LINE", "ERROR_MESSAGE", "ERROR_NUMBER",
    "ERROR_PROCEDURE", "ERROR_SEVERITY", "ERROR_STATE", "FIRST_VALUE", "FLOOR", "FORMAT", "GETDATE", "GETUTCDATE",
    "GREATEST", "GROUPING", "HAS_DBACCESS", "HASHBYTES", "HOST_NAME", "IIF", "ISDATE", "ISJSON", "ISNULL", "ISNUMERIC",
    "JSON_MODIFY", "JSON_QUERY", "JSON_VALUE", "LAG", "LAST_VALUE", "LEAD", "LEAST", "LEN", "LOWER", "LTRIM", "MONTH",
    "NEWID", "NEWSEQUENTIALID", "NTILE", "NULLIF", "OBJECT_ID", "OBJECT_NAME", "OBJECT_SCHEMA_NAME",
    "OBJECTPROPERTY", "OPENJSON", "PATINDEX", "PERCENT_RANK", "QUOTENAME", "RAND", "RANK", "REPLACE", "REPLICATE",
    "REVERSE", "ROUND", "ROW_NUMBER", "RTRIM", "SCHEMA_ID", "SCHEMA_NAME", "SCOPE_IDENTITY", "SERVERPROPERTY",
    "SESSION_USER", "SIGN", "SPACE", "SQRT", "STR", "STRING_AGG", "STRING_ESCAPE", "STRING_SPLIT", "STUFF",
    "SUBSTRING", "SUM", "SUSER_NAME", "SUSER_SNAME", "SYSDATETIME", "SYSDATETIMEOFFSET", "SYSTEM_USER", "SYSUTCDATETIME",
    "TRANSLATE", "TRIM", "TRY_CAST", "TRY_CONVERT", "TRY_PARSE", "TYPE_NAME", "UPPER", "USER_ID", "USER_NAME", "YEAR",
    "MIN", "STDEV", "VAR",
];

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '@' || c == '#' || c == '$'
}

/// Splits `src` into (kind, byte range) spans covering every byte.
pub fn tokenize(src: &str) -> Vec<(Kind, std::ops::Range<usize>)> {
    let b = src.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    let mut prev_dot = false;
    while i < b.len() {
        let start = i;
        let c = src[i..].chars().next().unwrap();
        let kind = if c == '-' && b.get(i + 1) == Some(&b'-') {
            i = src[i..].find('\n').map_or(b.len(), |p| i + p);
            Kind::Comment
        } else if c == '/' && b.get(i + 1) == Some(&b'*') {
            let mut depth = 0;
            while i < b.len() {
                if b[i] == b'/' && b.get(i + 1) == Some(&b'*') {
                    depth += 1;
                    i += 2;
                } else if b[i] == b'*' && b.get(i + 1) == Some(&b'/') {
                    depth -= 1;
                    i += 2;
                    if depth == 0 {
                        break;
                    }
                } else {
                    i += 1;
                }
            }
            Kind::Comment
        } else if c == '\'' || ((c == 'N' || c == 'n') && b.get(i + 1) == Some(&b'\'')) {
            i += if c == '\'' { 1 } else { 2 };
            while i < b.len() {
                if b[i] == b'\'' {
                    if b.get(i + 1) == Some(&b'\'') {
                        i += 2;
                        continue;
                    }
                    i += 1;
                    break;
                }
                i += 1;
            }
            Kind::String
        } else if c == '[' {
            while i < b.len() && b[i] != b']' {
                i += 1;
            }
            i = (i + 1).min(b.len());
            Kind::Text
        } else if c.is_ascii_digit() {
            while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'.') {
                i += 1;
            }
            Kind::Number
        } else if is_word_char(c) {
            while i < b.len() {
                let ch = src[i..].chars().next().unwrap();
                if !is_word_char(ch) {
                    break;
                }
                i += ch.len_utf8();
            }
            classify(&src[start..i], prev_dot, src[i..].trim_start().starts_with('('))
        } else if c.is_whitespace() {
            i += c.len_utf8();
            Kind::Text
        } else {
            i += c.len_utf8();
            if c == '.' { Kind::Text } else { Kind::Operator }
        };
        if !src[start..i].trim().is_empty() {
            prev_dot = src[start..i] == *".";
        }
        out.push((kind, start..i));
    }
    out
}

fn classify(word: &str, after_dot: bool, before_paren: bool) -> Kind {
    let upper = word.to_ascii_uppercase();
    if upper.starts_with("@@") {
        return Kind::Function;
    }
    if word.starts_with('@') || word.starts_with('#') {
        return Kind::Text;
    }
    if !after_dot && (upper == "SYS" || upper == "INFORMATION_SCHEMA") {
        return Kind::SystemObject;
    }
    if upper.starts_with("SP_") || upper.starts_with("XP_") {
        return Kind::SystemProc;
    }
    if after_dot {
        return Kind::Text;
    }
    if FUNCTIONS.contains(&upper.as_str()) && (before_paren || matches!(upper.as_str(), "CURRENT_TIMESTAMP" | "CURRENT_USER" | "SESSION_USER" | "SYSTEM_USER")) {
        return Kind::Function;
    }
    if OPERATOR_WORDS.contains(&upper.as_str()) {
        return Kind::Operator;
    }
    if KEYWORDS.contains(&upper.as_str()) {
        return Kind::Keyword;
    }
    Kind::Text
}

/// Builds the coloured layout for the editor.
pub fn highlight(src: &str, p: &Palette, font: FontId) -> LayoutJob {
    let mut job = LayoutJob::default();
    let mut prev_sys = false;
    for (kind, range) in tokenize(src) {
        let text = &src[range];
        // Names after sys. / INFORMATION_SCHEMA. are green too.
        let kind = if prev_sys && kind == Kind::Text && text != "." && !text.trim().is_empty() {
            prev_sys = false;
            Kind::SystemObject
        } else {
            if kind == Kind::SystemObject {
                prev_sys = true;
            } else if !(text == "." || text.trim().is_empty()) {
                prev_sys = false;
            }
            kind
        };
        let color: Color32 = match kind {
            Kind::Text => p.text,
            Kind::Keyword => p.keyword,
            Kind::String => p.string,
            Kind::Comment => p.comment,
            Kind::Function => p.function,
            Kind::SystemObject => p.system_object,
            Kind::SystemProc => p.system_proc,
            Kind::Operator => p.operator,
            Kind::Number => p.number,
        };
        job.append(text, 0.0, TextFormat { font_id: font.clone(), color, ..Default::default() });
    }
    job
}

// ----- Line editing -------------------------------------------------------

/// Byte offsets of the lines touched by the char range `[a, b)`.
fn line_span(text: &str, a: usize, b: usize) -> (usize, usize) {
    let ba = char_to_byte(text, a);
    let mut bb = char_to_byte(text, b);
    // A selection ending at the very start of a line does not include that line.
    if b > a && bb > 0 && text.as_bytes()[bb - 1] == b'\n' {
        bb -= 1;
    }
    let start = text[..ba].rfind('\n').map_or(0, |p| p + 1);
    let end = text[bb..].find('\n').map_or(text.len(), |p| bb + p);
    (start, end)
}

pub fn char_to_byte(text: &str, chars: usize) -> usize {
    text.char_indices().nth(chars).map_or(text.len(), |(i, _)| i)
}

fn byte_to_char(text: &str, byte: usize) -> usize {
    text[..byte].chars().count()
}

/// Applies `f` to each line touched by the selection. Returns the new
/// selection (char range covering the edited lines).
fn map_lines(text: &mut String, a: usize, b: usize, f: impl Fn(&str) -> String) -> (usize, usize) {
    let (start, end) = line_span(text, a, b);
    let new: Vec<String> = text[start..end].split('\n').map(f).collect();
    let new = new.join("\n");
    text.replace_range(start..end, &new);
    (byte_to_char(text, start), byte_to_char(text, start + new.len()))
}

pub fn comment(text: &mut String, a: usize, b: usize) -> (usize, usize) {
    map_lines(text, a, b, |l| if l.trim().is_empty() { l.to_string() } else { format!("--{l}") })
}

pub fn uncomment(text: &mut String, a: usize, b: usize) -> (usize, usize) {
    map_lines(text, a, b, |l| {
        let indent = l.len() - l.trim_start().len();
        match l[indent..].strip_prefix("--") {
            Some(rest) => format!("{}{rest}", &l[..indent]),
            None => l.to_string(),
        }
    })
}

pub fn indent(text: &mut String, a: usize, b: usize) -> (usize, usize) {
    map_lines(text, a, b, |l| if l.is_empty() { String::new() } else { format!("\t{l}") })
}

pub fn unindent(text: &mut String, a: usize, b: usize) -> (usize, usize) {
    map_lines(text, a, b, |l| {
        if let Some(rest) = l.strip_prefix('\t') {
            rest.to_string()
        } else {
            let spaces = l.len() - l.trim_start_matches(' ').len();
            l[spaces.min(4)..].to_string()
        }
    })
}

/// Upper- or lower-cases the selection (or nothing when empty).
pub fn change_case(text: &mut String, a: usize, b: usize, upper: bool) {
    if a == b {
        return;
    }
    let (ba, bb) = (char_to_byte(text, a), char_to_byte(text, b));
    let s = if upper { text[ba..bb].to_uppercase() } else { text[ba..bb].to_lowercase() };
    text.replace_range(ba..bb, &s);
}

/// 1-based line and column of a char offset.
pub fn line_col(text: &str, chars: usize) -> (usize, usize) {
    let byte = char_to_byte(text, chars);
    let before = &text[..byte];
    let line = before.matches('\n').count() + 1;
    let col = before.rsplit('\n').next().map_or(0, |l| l.chars().count()) + 1;
    (line, col)
}

/// Char offset of the start of a 1-based line.
pub fn line_start(text: &str, line: usize) -> usize {
    if line <= 1 {
        return 0;
    }
    let mut seen = 1;
    for (i, ch) in text.chars().enumerate() {
        if ch == '\n' {
            seen += 1;
            if seen == line {
                return i + 1;
            }
        }
    }
    text.chars().count()
}

/// The identifier around a char offset (for Alt+F1 with no selection).
pub fn word_at(text: &str, chars: usize) -> String {
    let byte = char_to_byte(text, chars);
    let ok = |c: char| c.is_alphanumeric() || matches!(c, '_' | '.' | '[' | ']' | '#' | '@' | '$');
    let start = text[..byte].char_indices().rev().take_while(|(_, c)| ok(*c)).last().map_or(byte, |(i, _)| i);
    let end = text[byte..].char_indices().find(|(_, c)| !ok(*c)).map_or(text.len(), |(i, _)| byte + i);
    text[start..end].to_string()
}

/// Finds `needle` after char offset `from`, wrapping around. Returns a char range.
pub fn find(text: &str, needle: &str, from: usize, match_case: bool) -> Option<(usize, usize)> {
    if needle.is_empty() {
        return None;
    }
    let (hay, pat) = if match_case { (text.to_string(), needle.to_string()) } else { (text.to_lowercase(), needle.to_lowercase()) };
    // Lowercasing can change byte lengths for some scripts; fall back to exact case then.
    let (hay, pat) = if hay.len() == text.len() { (hay, pat) } else { (text.to_string(), needle.to_string()) };
    let start = char_to_byte(text, from).min(hay.len());
    let pos = hay[start..].find(&pat).map(|p| p + start).or_else(|| hay[..start].find(&pat))?;
    let a = byte_to_char(text, pos);
    Some((a, a + needle.chars().count()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(s: &str) -> Vec<(Kind, String)> {
        tokenize(s).into_iter().filter(|(_, r)| !s[r.clone()].trim().is_empty()).map(|(k, r)| (k, s[r].to_string())).collect()
    }

    #[test]
    fn colours() {
        let k = kinds("SELECT COUNT(*) FROM sys.tables WHERE name LIKE N'x''y' -- hi\nEXEC sp_who");
        assert_eq!(k[0], (Kind::Keyword, "SELECT".into()));
        assert_eq!(k[1], (Kind::Function, "COUNT".into()));
        assert!(k.contains(&(Kind::SystemObject, "sys".into())));
        assert!(k.contains(&(Kind::Operator, "LIKE".into())));
        assert!(k.contains(&(Kind::String, "N'x''y'".into())));
        assert!(k.contains(&(Kind::Comment, "-- hi".into())));
        assert!(k.contains(&(Kind::SystemProc, "sp_who".into())));
        // A column called "count" is not a function.
        assert_eq!(kinds("select count from t")[1].0, Kind::Text);
        assert_eq!(kinds("select @@ROWCOUNT")[1].0, Kind::Function);
        assert_eq!(kinds("/* a /* b */ c */ x")[0].0, Kind::Comment);
    }

    #[test]
    fn line_edits() {
        let mut t = "select 1\nselect 2\n".to_string();
        let r = comment(&mut t, 0, 12);
        assert_eq!(t, "--select 1\n--select 2\n");
        uncomment(&mut t, r.0, r.1);
        assert_eq!(t, "select 1\nselect 2\n");
        indent(&mut t, 0, 3);
        assert_eq!(t, "\tselect 1\nselect 2\n");
        unindent(&mut t, 0, 3);
        assert_eq!(t, "select 1\nselect 2\n");
        // A selection ending at the start of line 2 only touches line 1.
        comment(&mut t, 0, 9);
        assert_eq!(t, "--select 1\nselect 2\n");
        change_case(&mut t, 2, 8, true);
        assert_eq!(t, "--SELECT 1\nselect 2\n");
    }

    #[test]
    fn positions() {
        let t = "ab\ncde\nf";
        assert_eq!(line_col(t, 4), (2, 2));
        assert_eq!(line_start(t, 3), 7);
        assert_eq!(word_at("exec sp_help dbo.Orders;", 15), "dbo.Orders");
        assert_eq!(find("Abc abc", "ABC", 1, false), Some((4, 7)));
        assert_eq!(find("Abc abc", "ABC", 5, false), Some((0, 3)));
        assert_eq!(find("Abc abc", "ABC", 0, true), None);
    }
}
