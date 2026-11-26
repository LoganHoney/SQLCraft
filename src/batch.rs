//! Splits a script into batches on `GO` lines, the way SSMS and sqlcmd do.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Batch {
    pub sql: String,
    /// 1-based line of the script where this batch starts, so server line
    /// numbers can be mapped back to the editor.
    pub start_line: u32,
    /// `GO 5` runs the batch five times.
    pub repeat: u32,
}

/// Returns `Some(count)` when the line is a batch separator.
fn separator(line: &str) -> Option<u32> {
    let t = line.trim();
    let t = t.split("--").next().unwrap_or("").trim();
    if t.len() < 2 || !t[..2].eq_ignore_ascii_case("go") {
        return None;
    }
    let rest = t[2..].trim();
    if rest.is_empty() {
        return Some(1);
    }
    if !t[2..].starts_with(char::is_whitespace) {
        return None;
    }
    rest.parse::<u32>().ok().filter(|n| *n > 0)
}

/// Tracks whether the scanner is inside a string or comment that spans lines,
/// where a `GO` line is ordinary text.
#[derive(Clone, Copy, PartialEq)]
enum State {
    Code,
    Str,
    Ident,
    Block(u32),
}

fn advance(mut state: State, line: &str) -> State {
    let b = line.as_bytes();
    let mut i = 0;
    while i < b.len() {
        match state {
            State::Code => match b[i] {
                b'\'' => state = State::Str,
                b'[' => state = State::Ident,
                b'-' if b.get(i + 1) == Some(&b'-') => return State::Code,
                b'/' if b.get(i + 1) == Some(&b'*') => {
                    state = State::Block(1);
                    i += 1;
                }
                _ => {}
            },
            State::Str => {
                if b[i] == b'\'' {
                    if b.get(i + 1) == Some(&b'\'') {
                        i += 1;
                    } else {
                        state = State::Code;
                    }
                }
            }
            State::Ident => {
                if b[i] == b']' {
                    if b.get(i + 1) == Some(&b']') {
                        i += 1;
                    } else {
                        state = State::Code;
                    }
                }
            }
            State::Block(depth) => {
                if b[i] == b'*' && b.get(i + 1) == Some(&b'/') {
                    state = if depth == 1 { State::Code } else { State::Block(depth - 1) };
                    i += 1;
                } else if b[i] == b'/' && b.get(i + 1) == Some(&b'*') {
                    state = State::Block(depth + 1);
                    i += 1;
                }
            }
        }
        i += 1;
    }
    state
}

pub fn split(script: &str) -> Vec<Batch> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut start = 1u32;
    let mut state = State::Code;
    for (idx, line) in script.split_inclusive('\n').enumerate() {
        let lineno = idx as u32 + 1;
        if state == State::Code {
            if let Some(repeat) = separator(line) {
                if !cur.trim().is_empty() {
                    out.push(Batch { sql: std::mem::take(&mut cur), start_line: start, repeat });
                }
                cur.clear();
                start = lineno + 1;
                continue;
            }
        }
        state = advance(state, line);
        cur.push_str(line);
    }
    if !cur.trim().is_empty() {
        out.push(Batch { sql: cur, start_line: start, repeat: 1 });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_on_go() {
        let b = split("select 1\nGO\nselect 2\ngo\n");
        assert_eq!(b.len(), 2);
        assert_eq!(b[0].sql, "select 1\n");
        assert_eq!(b[1].sql, "select 2\n");
        assert_eq!(b[1].start_line, 3);
    }

    #[test]
    fn go_with_count_and_comment() {
        let b = split("insert t values(1)\n  GO 3 -- thrice\nselect 1");
        assert_eq!(b[0].repeat, 3);
        assert_eq!(b[1].sql, "select 1");
    }

    #[test]
    fn ignores_go_inside_strings_and_comments() {
        let s = "select 'a\nGO\nb'\n/* x\nGO\n*/\nselect 2\nGO";
        let b = split(s);
        assert_eq!(b.len(), 1);
    }

    #[test]
    fn not_a_separator() {
        assert_eq!(separator("GOTO label"), None);
        assert_eq!(separator("go"), Some(1));
        assert_eq!(separator("GO 0"), None);
        assert_eq!(split("select 1 -- 'x\nGO\nselect 2").len(), 2);
    }

    #[test]
    fn empty_batches_skipped() {
        assert!(split("GO\n\nGO\n").is_empty());
    }
}
