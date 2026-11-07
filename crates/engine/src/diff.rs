//! Unified diffs for display: files, hunks and lines with their line numbers.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineKind {
    Context,
    Removed,
    Added,
    /// "\ No newline at end of file"
    Note,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiffLine {
    pub kind: LineKind,
    pub text: String,
    pub old: Option<u32>,
    pub new: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hunk {
    /// The @@ line.
    pub header: String,
    pub lines: Vec<DiffLine>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiffFile {
    pub path: String,
    pub old_path: Option<String>,
    /// A, D, R, C or M.
    pub status: char,
    pub binary: bool,
    /// Mode changes and other header lines worth showing.
    pub notes: Vec<String>,
    pub added: usize,
    pub removed: usize,
    pub hunks: Vec<Hunk>,
}

/// git's C-style quoting of unusual paths ("a/t\303\251st") back to text.
fn unquote(raw: &str) -> String {
    let Some(inner) = raw.strip_prefix('"').and_then(|r| r.strip_suffix('"')) else { return raw.to_owned() };
    let bytes = inner.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        if c != b'\\' || i + 1 >= bytes.len() {
            out.push(c);
            i += 1;
            continue;
        }
        let next = bytes[i + 1];
        if (b'0'..=b'7').contains(&next) && i + 4 <= bytes.len() {
            let octal = std::str::from_utf8(&bytes[i + 1..i + 4]).ok().and_then(|s| u8::from_str_radix(s, 8).ok());
            if let Some(b) = octal {
                out.push(b);
                i += 4;
                continue;
            }
        }
        out.push(match next {
            b'n' => b'\n',
            b't' => b'\t',
            b'r' => b'\r',
            b'a' => 7,
            b'b' => 8,
            b'f' => 12,
            b'v' => 11,
            other => other,
        });
        i += 2;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The path from a ---/+++ line without its a/ or b/ prefix; None for /dev/null.
fn side_path(line: &str) -> Option<String> {
    let raw = line[4..].split('\t').next().unwrap_or("");
    let raw = unquote(raw);
    if raw == "/dev/null" {
        return None;
    }
    Some(raw.strip_prefix("a/").or_else(|| raw.strip_prefix("b/")).unwrap_or(&raw).to_owned())
}

fn parse_hunk_header(line: &str) -> Option<(u32, u32)> {
    let rest = line.strip_prefix("@@ -")?;
    let (old, rest) = rest.split_once(' ')?;
    let new = rest.strip_prefix('+')?.split(' ').next()?;
    let start = |s: &str| s.split(',').next()?.parse().ok();
    Some((start(old)?, start(new)?))
}

/// Fallback paths from "diff --git a/x b/y", for diffs without ---/+++ lines (binary files, mode-only changes).
fn header_paths(line: &str) -> (String, Option<String>) {
    let rest = &line["diff --git ".len()..];
    if rest.starts_with('"')
        && let Some(end) = rest[1..].find("\" ") {
            let a = unquote(&rest[..end + 2]);
            let b = unquote(rest[end + 3..].trim());
            let strip = |p: &str| p.strip_prefix("a/").or_else(|| p.strip_prefix("b/")).unwrap_or(p).to_owned();
            return (strip(&b), Some(strip(&a)));
        }
    // Unquoted: the two halves are the same length when the path didn't change.
    let half = rest.len() / 2;
    if rest.len() % 2 == 1 && rest[..half].strip_prefix("a/") == rest[half + 1..].strip_prefix("b/") {
        let path = rest[half + 1..].strip_prefix("b/").unwrap_or("").to_owned();
        return (path, None);
    }
    match rest.find(" b/") {
        Some(i) => (rest[i + 3..].to_owned(), Some(rest[..i].trim_start_matches("a/").to_owned())),
        None => (rest.to_owned(), None),
    }
}

/// Parses `git diff` output with any number of files.
pub fn parse_diff(text: &str) -> Vec<DiffFile> {
    let mut files: Vec<DiffFile> = Vec::new();
    let mut in_hunk = false;
    let (mut old_line, mut new_line) = (0u32, 0u32);
    for line in text.split('\n') {
        let line = line.strip_suffix('\r').unwrap_or(line);
        if line.starts_with("diff --git ") {
            let (path, old_path) = header_paths(line);
            files.push(DiffFile { path, old_path, status: 'M', binary: false, notes: Vec::new(), added: 0, removed: 0, hunks: Vec::new() });
            in_hunk = false;
            continue;
        }
        let Some(file) = files.last_mut() else { continue };
        if let Some((old, new)) = line.starts_with("@@ ").then(|| parse_hunk_header(line)).flatten() {
            file.hunks.push(Hunk { header: line.to_owned(), lines: Vec::new() });
            in_hunk = true;
            old_line = old;
            new_line = new;
            continue;
        }
        if !in_hunk {
            if line.starts_with("new file mode") {
                file.status = 'A';
            } else if line.starts_with("deleted file mode") {
                file.status = 'D';
            } else if let Some(p) = line.strip_prefix("rename from ") {
                file.status = 'R';
                file.old_path = Some(unquote(p));
            } else if let Some(p) = line.strip_prefix("rename to ") {
                file.path = unquote(p);
            } else if let Some(p) = line.strip_prefix("copy from ") {
                file.status = 'C';
                file.old_path = Some(unquote(p));
            } else if let Some(p) = line.strip_prefix("copy to ") {
                file.path = unquote(p);
            } else if line.starts_with("old mode ") || line.starts_with("new mode ") {
                file.notes.push(line.to_owned());
            } else if line.starts_with("Binary files") || line.starts_with("GIT binary patch") {
                file.binary = true;
            } else if line.starts_with("--- ") {
                if let Some(old) = side_path(line)
                    && old != file.path && file.status != 'A' && file.old_path.is_none() {
                        file.old_path = Some(old);
                    }
            } else if line.starts_with("+++ ")
                && let Some(now) = side_path(line) {
                    file.path = now;
                }
            continue;
        }
        let hunk = file.hunks.last_mut().unwrap();
        let Some(kind) = line.chars().next() else { continue };
        let text = line[kind.len_utf8()..].to_owned();
        match kind {
            ' ' => {
                hunk.lines.push(DiffLine { kind: LineKind::Context, text, old: Some(old_line), new: Some(new_line) });
                old_line += 1;
                new_line += 1;
            }
            '-' => {
                hunk.lines.push(DiffLine { kind: LineKind::Removed, text, old: Some(old_line), new: None });
                old_line += 1;
                file.removed += 1;
            }
            '+' => {
                hunk.lines.push(DiffLine { kind: LineKind::Added, text, old: None, new: Some(new_line) });
                new_line += 1;
                file.added += 1;
            }
            '\\' => hunk.lines.push(DiffLine { kind: LineKind::Note, text: text.trim_start().to_owned(), old: None, new: None }),
            _ => in_hunk = false,
        }
    }
    for f in &mut files {
        if f.old_path.as_deref() == Some(f.path.as_str()) {
            f.old_path = None;
        }
        if f.status == 'M' && f.old_path.is_some() {
            f.status = 'R';
        }
    }
    files
}

/// Cuts `files` down to about `max_lines` diff lines, so a huge commit stays responsive. Whole hunks are kept while
/// they fit; returns how many lines were left out.
pub fn limit_diff(files: &mut [DiffFile], max_lines: usize) -> usize {
    let mut budget = max_lines as isize;
    let mut omitted = 0;
    for f in files {
        let mut kept = Vec::new();
        for h in f.hunks.drain(..) {
            if budget > 0 {
                budget -= h.lines.len() as isize + 1;
                kept.push(h);
            } else {
                omitted += h.lines.len();
            }
        }
        f.hunks = kept;
    }
    omitted
}

#[cfg(test)]
mod tests {
    use super::*;

    const DIFF: &str = "diff --git a/src/a.rs b/src/a.rs
index 111..222 100644
--- a/src/a.rs
+++ b/src/a.rs
@@ -1,3 +1,4 @@ fn main
 one
-two
+deux
+drei
 three
\\ No newline at end of file
diff --git a/new.txt b/new.txt
new file mode 100644
--- /dev/null
+++ b/new.txt
@@ -0,0 +1 @@
+hello
diff --git a/old name.txt b/new name.txt
similarity index 100%
rename from old name.txt
rename to new name.txt
diff --git a/img.png b/img.png
Binary files a/img.png and b/img.png differ
diff --git \"a/t\\303\\251st\" \"b/t\\303\\251st\"
--- \"a/t\\303\\251st\"
+++ \"b/t\\303\\251st\"
@@ -1 +1 @@
-a
+b
";

    #[test]
    fn parses_files_hunks_and_lines() {
        let files = parse_diff(DIFF);
        assert_eq!(files.len(), 5);
        let a = &files[0];
        assert_eq!((a.path.as_str(), a.status, a.added, a.removed), ("src/a.rs", 'M', 2, 1));
        let lines = &a.hunks[0].lines;
        assert_eq!(lines[0], DiffLine { kind: LineKind::Context, text: "one".into(), old: Some(1), new: Some(1) });
        assert_eq!(lines[1].old, Some(2));
        assert_eq!(lines[2].new, Some(2));
        assert_eq!(lines[4].new, Some(4));
        assert_eq!(lines[5].kind, LineKind::Note);
        assert_eq!(files[1].status, 'A');
        assert_eq!(files[2].status, 'R');
        assert_eq!(files[2].path, "new name.txt");
        assert_eq!(files[2].old_path.as_deref(), Some("old name.txt"));
        assert!(files[3].binary);
        assert_eq!(files[3].path, "img.png");
        assert_eq!(files[4].path, "tést");
        assert_eq!(files[4].old_path, None);
    }

    #[test]
    fn limits_long_diffs() {
        let mut files = parse_diff(DIFF);
        let omitted = limit_diff(&mut files, 3);
        assert!(files[0].hunks.len() == 1);
        assert!(files[1].hunks.is_empty());
        assert_eq!(omitted, 1 + 2);
    }
}
