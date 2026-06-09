//! Port of al-sem `src/digest/diff-parser.ts`.
//!
//! Minimal unified diff parser: produces a list of DiffFile (with hunks) from
//! `git diff` / `diff -u` output. Only the new-side line ranges are extracted.
//!
//! Handles the 9 spec edge cases: a/b/ prefix stripping, quoted paths, /dev/null,
//! renames, omitted hunk count (defaults to 1), zero-length ranges, CRLF,
//! "\ No newline" marker, and always returns {files, errors}.

#[derive(Debug, Clone)]
pub struct DiffHunk {
    /// 1-based first line of the new file covered by this hunk.
    pub new_start: i32,
    /// Number of new-file lines (may be 0 for pure deletions).
    pub new_count: i32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiffFileKind {
    Modified,
    Added,
    Deleted,
    Renamed,
}

#[derive(Debug, Clone)]
pub struct DiffFile {
    /// The NEW path (b/ stripped, unquoted, forward-slash normalized).
    pub path: String,
    pub kind: DiffFileKind,
    pub old_path: Option<String>,
    pub hunks: Vec<DiffHunk>,
}

#[derive(Debug, Clone)]
pub struct DiffParseResult {
    pub files: Vec<DiffFile>,
    pub errors: Vec<String>,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn strip_ab_prefix(p: &str) -> &str {
    if p.starts_with("a/") || p.starts_with("b/") {
        &p[2..]
    } else {
        p
    }
}

fn unquote_path(s: &str) -> String {
    let trimmed = s.trim();
    if !trimmed.starts_with('"') || !trimmed.ends_with('"') {
        return trimmed.to_string();
    }
    let inner = &trimmed[1..trimmed.len() - 1];
    let mut result = String::with_capacity(inner.len());
    let mut chars = inner.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\\' {
            result.push(c);
            continue;
        }
        match chars.next() {
            Some('"') => result.push('"'),
            Some('\\') => result.push('\\'),
            Some('n') => result.push('\n'),
            Some('t') => result.push('\t'),
            Some('r') => result.push('\r'),
            Some(o1) if o1.is_ascii_digit() => {
                // Octal escape \NNN
                let mut oct = String::new();
                oct.push(o1);
                for _ in 0..2 {
                    if chars.peek().map(|c| c.is_ascii_digit()).unwrap_or(false) {
                        oct.push(chars.next().unwrap());
                    }
                }
                if let Ok(n) = u8::from_str_radix(&oct, 8) {
                    result.push(n as char);
                }
            }
            Some(other) => {
                result.push('\\');
                result.push(other);
            }
            None => result.push('\\'),
        }
    }
    result
}

fn extract_path(line: &str) -> Option<String> {
    // Remove leading --- / +++ and one space (line[4..])
    let rest = line[4..].trim_start();
    if rest == "/dev/null" {
        return None;
    }
    let unquoted = unquote_path(rest);
    let normalized = unquoted.replace('\\', "/");
    Some(strip_ab_prefix(&normalized).to_string())
}

fn parse_hunk_header(line: &str) -> Option<DiffHunk> {
    // @@ -old[,oldCount] +new[,newCount] @@
    let line = line.trim_start_matches('@').trim();
    // Expect "-N[,N] +N[,N]"
    let re_new_start;
    let re_new_count;
    // Simple manual parser
    let rest = line;
    // Find +N portion
    let plus_pos = rest.find('+')?;
    let after_plus = &rest[plus_pos + 1..];
    let space_or_end = after_plus.find(|c: char| !c.is_ascii_digit() && c != ',');
    let new_part = match space_or_end {
        Some(end) => &after_plus[..end],
        None => after_plus,
    };
    let comma_pos = new_part.find(',');
    if let Some(cp) = comma_pos {
        re_new_start = new_part[..cp].parse::<i32>().ok()?;
        re_new_count = new_part[cp + 1..].parse::<i32>().ok()?;
    } else {
        re_new_start = new_part.parse::<i32>().ok()?;
        re_new_count = 1; // omitted count → 1
    }
    Some(DiffHunk {
        new_start: re_new_start,
        new_count: re_new_count,
    })
}

// ---------------------------------------------------------------------------
// Parser
// ---------------------------------------------------------------------------

pub fn parse_unified_diff(text: &str) -> DiffParseResult {
    if text.trim().is_empty() {
        return DiffParseResult {
            files: Vec::new(),
            errors: Vec::new(),
        };
    }

    // Normalize CRLF → LF
    let normalized = text.replace("\r\n", "\n");
    let lines: Vec<&str> = normalized.split('\n').collect();

    let mut files: Vec<DiffFile> = Vec::new();
    let errors: Vec<String> = Vec::new();

    // State
    let mut current_file: Option<DiffFile> = None;
    let mut pending_rename_from: Option<String> = None;
    let mut pending_rename_to: Option<String> = None;
    let mut pending_old_path: Option<Option<String>> = None; // None means "not seen"; Some(None) means /dev/null
    let mut in_file = false;

    let finalize = |current: &mut Option<DiffFile>, files: &mut Vec<DiffFile>| {
        if let Some(f) = current.take() {
            files.push(f);
        }
    };

    for raw_line in &lines {
        // Strip trailing CR
        let line = raw_line.trim_end_matches('\r');

        // Edge case 8: "\ No newline at end of file" — skip
        if line.starts_with("\\ ") {
            continue;
        }

        // --- line (old-side path header)
        if line.starts_with("--- ") {
            finalize(&mut current_file, &mut files);
            pending_old_path = Some(extract_path(line));
            in_file = true;
            pending_rename_from = None;
            pending_rename_to = None;
            continue;
        }

        // +++ line (new-side path header)
        if line.starts_with("+++ ") && in_file {
            let new_path = extract_path(line);
            current_file = match (pending_old_path.take(), new_path) {
                (Some(None), Some(new_p)) => {
                    // --- /dev/null → added
                    Some(DiffFile {
                        path: new_p,
                        kind: DiffFileKind::Added,
                        old_path: None,
                        hunks: Vec::new(),
                    })
                }
                (Some(_old), None) => {
                    // +++ /dev/null → deleted
                    let path = pending_old_path
                        .take()
                        .flatten()
                        .unwrap_or_else(|| "unknown".to_string());
                    Some(DiffFile {
                        path: path.clone(),
                        kind: DiffFileKind::Deleted,
                        old_path: Some(path),
                        hunks: Vec::new(),
                    })
                }
                (Some(Some(old_p)), Some(new_p)) => {
                    let is_rename =
                        pending_rename_from.is_some() || pending_rename_to.is_some();
                    let kind = if is_rename {
                        DiffFileKind::Renamed
                    } else {
                        DiffFileKind::Modified
                    };
                    let old_path = if is_rename {
                        pending_rename_from.clone()
                    } else if old_p != new_p {
                        Some(old_p)
                    } else {
                        None
                    };
                    Some(DiffFile {
                        path: new_p,
                        kind,
                        old_path,
                        hunks: Vec::new(),
                    })
                }
                _ => None,
            };
            pending_rename_from = None;
            pending_rename_to = None;
            continue;
        }

        // Rename headers
        if let Some(from) = line.strip_prefix("rename from ") {
            pending_rename_from = Some(unquote_path(from.trim()).replace('\\', "/"));
            continue;
        }
        if let Some(to) = line.strip_prefix("rename to ") {
            pending_rename_to = Some(unquote_path(to.trim()).replace('\\', "/"));
            continue;
        }

        // diff --git header → signals end of previous file
        if line.starts_with("diff --git ") {
            // 100% rename with no hunks
            if pending_rename_from.is_some()
                && pending_rename_to.is_some()
                && current_file.is_none()
            {
                files.push(DiffFile {
                    path: pending_rename_to.take().unwrap(),
                    kind: DiffFileKind::Renamed,
                    old_path: pending_rename_from.take(),
                    hunks: Vec::new(),
                });
            } else {
                finalize(&mut current_file, &mut files);
            }
            pending_rename_from = None;
            pending_rename_to = None;
            pending_old_path = None;
            in_file = false;
            continue;
        }

        // Hunk header
        if line.starts_with("@@") {
            if let Some(hunk) = parse_hunk_header(&line[2..]) {
                if let Some(ref mut f) = current_file {
                    f.hunks.push(hunk);
                }
            }
        }
        // All other lines ignored
    }

    // End-of-input
    if pending_rename_from.is_some() && pending_rename_to.is_some() && current_file.is_none() {
        files.push(DiffFile {
            path: pending_rename_to.take().unwrap(),
            kind: DiffFileKind::Renamed,
            old_path: pending_rename_from.take(),
            hunks: Vec::new(),
        });
    } else {
        finalize(&mut current_file, &mut files);
    }

    DiffParseResult { files, errors }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_diff_returns_empty() {
        let r = parse_unified_diff("  ");
        assert!(r.files.is_empty());
        assert!(r.errors.is_empty());
    }

    #[test]
    fn simple_modified_file() {
        let diff = "diff --git a/src/Foo.al b/src/Foo.al\n\
            --- a/src/Foo.al\n\
            +++ b/src/Foo.al\n\
            @@ -10,3 +10,4 @@ SomeContext\n\
            -old\n\
            +new\n";
        let r = parse_unified_diff(diff);
        assert_eq!(r.files.len(), 1);
        let f = &r.files[0];
        assert_eq!(f.path, "src/Foo.al");
        assert_eq!(f.kind, DiffFileKind::Modified);
        assert_eq!(f.hunks.len(), 1);
        assert_eq!(f.hunks[0].new_start, 10);
        assert_eq!(f.hunks[0].new_count, 4);
    }

    #[test]
    fn omitted_hunk_count_defaults_to_1() {
        let diff = "diff --git a/F.al b/F.al\n--- a/F.al\n+++ b/F.al\n@@ -5 +5 @@\n-a\n+b\n";
        let r = parse_unified_diff(diff);
        assert_eq!(r.files[0].hunks[0].new_count, 1);
    }

    #[test]
    fn deleted_file_is_kind_deleted() {
        let diff =
            "diff --git a/Old.al b/Old.al\n--- a/Old.al\n+++ /dev/null\n@@ -1,2 +0,0 @@\n-x\n-y\n";
        let r = parse_unified_diff(diff);
        assert_eq!(r.files[0].kind, DiffFileKind::Deleted);
    }

    #[test]
    fn added_file_is_kind_added() {
        let diff =
            "diff --git a/New.al b/New.al\n--- /dev/null\n+++ b/New.al\n@@ -0,0 +1,2 @@\n+x\n+y\n";
        let r = parse_unified_diff(diff);
        assert_eq!(r.files[0].kind, DiffFileKind::Added);
    }
}
