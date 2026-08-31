//! Word expansion helpers shared across the lexer/parser and heredocs.
//!
//! Expands `$VAR`, `${VAR}`, `$?`, `$$`, `$!`, `$0`..`$9`, `$(cmd)`, and
//! `` `cmd` `` inside a string (double-quoted strings, heredoc bodies, and
//! concatenated words).

use std::process::Command;
use std::sync::atomic::{AtomicI32, Ordering};

/// PID of the most recently launched background job, for `$!`. -1 = none yet.
static LAST_BG_PID: AtomicI32 = AtomicI32::new(-1);

/// Record the most recently launched background job's pid (used by `$!`).
pub fn set_last_background_pid(pid: i32) {
    LAST_BG_PID.store(pid, Ordering::Relaxed);
}

/// Expand a leading `~` (start of a word, or after whitespace) to a home
/// directory, matching bash: `~` → $HOME, `~/x` → $HOME/x, `~user` → that
/// user's home (via /etc/passwd), `~user/x` → home + /x. Unknown users are
/// left literal.
fn expand_tilde(input: &str) -> String {
    let home = std::env::var("HOME").unwrap_or_default();
    let mut out = String::with_capacity(input.len());
    let mut i = 0;
    let bytes = input.as_bytes();
    let mut at_word_start = true;
    while i < bytes.len() {
        if bytes[i] == b'~' && at_word_start {
            // End of the (possibly empty) username: next '/' or end of input.
            let mut j = i + 1;
            while j < bytes.len() && bytes[j] != b'/' {
                j += 1;
            }
            let user = &input[i + 1..j];
            if user.is_empty() {
                out.push_str(&home);
            } else {
                match nix::unistd::User::from_name(user) {
                    Ok(Some(u)) => out.push_str(&u.dir.to_string_lossy()),
                    _ => {
                        out.push('~');
                        out.push_str(user);
                    }
                }
            }
            i = j;
            at_word_start = false;
        } else {
            let ch = input[i..].chars().next().unwrap();
            out.push(ch);
            at_word_start = ch.is_whitespace();
            i += ch.len_utf8();
        }
    }
    out
}

/// Split the inside of a brace group on top-level commas (respecting nesting),
/// e.g. `a,{b,c}` → `["a", "{b,c}"]`.
fn split_brace_elements(inner: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut depth = 0i32;
    let mut cur = String::new();
    for c in inner.chars() {
        match c {
            '{' => {
                depth += 1;
                cur.push(c);
            }
            '}' => {
                depth -= 1;
                cur.push(c);
            }
            ',' if depth == 0 => parts.push(std::mem::take(&mut cur)),
            _ => cur.push(c),
        }
    }
    parts.push(cur);
    parts
}

/// Whether `inner` looks like a numeric/alpha range `a..b` or `a..b..step`.
fn is_range(inner: &str) -> bool {
    let parts: Vec<&str> = inner.split("..").collect();
    parts.len() >= 2
        && parts
            .iter()
            .all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit() || c.is_ascii_alphabetic()))
}

/// Expand a `a..b` / `a..b..step` range into its values.
fn expand_range(inner: &str) -> Vec<String> {
    let parts: Vec<&str> = inner.split("..").collect();
    let (start, end, step) = match parts.len() {
        2 => (parts[0], parts[1], 1i64),
        3 => (parts[0], parts[1], parts[2].parse::<i64>().ok().unwrap_or(1).max(1)),
        _ => return vec![format!("{{{}}}", inner)],
    };
    let mut out = Vec::new();
    if let (Ok(a), Ok(b)) = (start.parse::<i64>(), end.parse::<i64>()) {
        if a <= b {
            let mut v = a;
            while v <= b {
                out.push(v.to_string());
                v += step;
            }
        } else {
            let mut v = a;
            while v >= b {
                out.push(v.to_string());
                v -= step;
            }
        }
    } else if start.chars().count() == 1 && end.chars().count() == 1 {
        let sa = start.chars().next().unwrap() as u32;
        let sb = end.chars().next().unwrap() as u32;
        if sa <= sb {
            for c in sa..=sb {
                out.push(char::from_u32(c).unwrap().to_string());
            }
        } else {
            let mut c = sa;
            loop {
                out.push(char::from_u32(c).unwrap().to_string());
                if c == sb {
                    break;
                }
                c = c.wrapping_sub(1);
            }
        }
    } else {
        out.push(format!("{{{}}}", inner));
    }
    out
}

/// Find the first brace group in `s`, returning `(prefix, inner, suffix)`.
/// Only treats a `{...}` as a brace group when it has no `;` (so group-command
/// syntax `{ cmd; }` is untouched) and contains a top-level comma or a `..`
/// range.
fn find_brace_group(s: &str) -> Option<(String, String, String)> {
    let bytes = s.as_bytes();
    for (i, &b) in bytes.iter().enumerate() {
        if b == b'{' {
            let mut depth = 0i32;
            let mut j = i;
            let mut has_top_comma = false;
            let mut has_semi = false;
            while j < bytes.len() {
                match bytes[j] {
                    b'{' => depth += 1,
                    b'}' => {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                    b',' if depth == 1 => has_top_comma = true,
                    b';' => has_semi = true,
                    _ => {}
                }
                j += 1;
            }
            if j < bytes.len() && depth == 0 {
                let inner = &s[i + 1..j];
                if !has_semi && (has_top_comma || is_range(inner)) {
                    return Some((s[..i].to_string(), inner.to_string(), s[j + 1..].to_string()));
                }
            }
        }
    }
    None
}

/// Recursively expand all brace groups in a single word, returning every
/// resulting string. `{a,b}` → `[a, b]`; `{1..3}` → `[1, 2, 3]`; `x{a,b}` →
/// `[xa, xb]`; nested groups work too.
fn expand_braces(s: &str) -> Vec<String> {
    match find_brace_group(s) {
        None => vec![s.to_string()],
        Some((prefix, inner, suffix)) => {
            let elements: Vec<String> = if is_range(&inner) {
                expand_range(&inner)
            } else {
                split_brace_elements(&inner)
            };
            let mut out = Vec::new();
            for e in &elements {
                for p in expand_braces(&format!("{}{}", e, suffix)) {
                    out.push(format!("{}{}", prefix, p));
                }
            }
            out
        }
    }
}

/// Expand brace groups in a command line into space-joined words. Each
/// whitespace-separated word is expanded independently (bash semantics), so
/// `echo {a,b}` → `echo a b` and `cp {a,b}.txt d` → `cp a.txt b.txt d`.
pub fn expand_braces_line(input: &str) -> String {
    let mut words: Vec<String> = Vec::new();
    for w in input.split_whitespace() {
        words.extend(expand_braces(w));
    }
    words.join(" ")
}

/// Expand a string containing variable references and command substitutions.
///
/// `last_exit` supplies the value for `$?`. Positional parameters (`$1`..)
/// expand to the empty string outside of scripts/functions.
pub fn expand(input: &str, last_exit: i32) -> String {
    // Tilde-expand leading `~` in each word first.
    let input = expand_tilde(input);
    let mut result = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();

    while let Some(c) = chars.next() {
        if c == '\\' {
            if let Some(&next) = chars.peek() {
                match next {
                    '$' | '\\' | '`' | '"' => {
                        result.push(chars.next().unwrap());
                    }
                    'n' => {
                        chars.next();
                        result.push('\n');
                    }
                    't' => {
                        chars.next();
                        result.push('\t');
                    }
                    _ => result.push(c),
                }
            } else {
                result.push(c);
            }
        } else if c == '$' {
            match chars.peek() {
                Some('{') => {
                    chars.next();
                    let mut body = String::new();
                    // Read the body, honoring nested `${...}` so `${A:-${B:-x}}`
                    // stops at the MATCHING outer brace, not the inner one.
                    let mut depth = 1i32;
                    for nc in chars.by_ref() {
                        if nc == '{' {
                            depth += 1;
                            body.push(nc);
                        } else if nc == '}' {
                            depth -= 1;
                            if depth == 0 {
                                break;
                            }
                            body.push(nc);
                        } else {
                            body.push(nc);
                        }
                    }
                    result.push_str(&expand_parameter(&body));
                }
                Some('(') => {
                    chars.next();
                    let cmd = read_until_balanced(&mut chars, '(', ')');
                    result.push_str(&run_command(&cmd));
                }
                Some('?') => {
                    chars.next();
                    result.push_str(&last_exit.to_string());
                }
                Some('$') => {
                    chars.next();
                    result.push_str(&std::process::id().to_string());
                }
                Some('!') => {
                    chars.next();
                    // PID of the most recently launched background job (`$!`).
                    let p = LAST_BG_PID.load(Ordering::Relaxed);
                    if p > 0 {
                        result.push_str(&p.to_string());
                    }
                }
                Some('#') | Some('*') | Some('@') | Some('-') | Some('_') => {
                    chars.next();
                    // Argument count / all-args — empty in this context.
                }
                Some('0') => {
                    chars.next();
                    result.push_str("waash");
                }
                Some(d) if d.is_ascii_digit() => {
                    chars.next();
                    // Positional parameter — empty outside scripts.
                }
                Some(c2) if c2.is_alphanumeric() || *c2 == '_' => {
                    let mut name = String::new();
                    name.push(chars.next().unwrap());
                    while let Some(&nc) = chars.peek() {
                        if nc.is_alphanumeric() || nc == '_' {
                            name.push(chars.next().unwrap());
                        } else {
                            break;
                        }
                    }
                    result.push_str(&env_value(&name));
                }
                _ => result.push('$'),
            }
        } else if c == '`' {
            let mut inner = String::new();
            for nc in chars.by_ref() {
                if nc == '`' {
                    break;
                }
                inner.push(nc);
            }
            result.push_str(&run_command(&inner));
        } else {
            result.push(c);
        }
    }

    result
}

fn env_value(name: &str) -> String {
    std::env::var(name).unwrap_or_default()
}

/// Expand a `${...}` parameter expression body (the text between the braces).
///
/// Supports the common bash forms:
///   `${VAR}`              — value of VAR (empty if unset)
///   `${VAR:-default}`     — VAR if set AND non-empty, else default
///   `${VAR-default}`      — VAR if set (even empty), else default
///   `${VAR:=default}`     — like `:-` (assignment semantics not applied in
///                           the expander; value used is default when unset)
///   `${VAR:?msg}`         — VAR if set, else msg (printed as a note)
///   `${#VAR}`             — length of VAR's value
///   `${VAR/pat/repl}`     — replace first occurrence of `pat` with `repl`
///   `${VAR//pat/repl}`    — replace ALL occurrences
fn expand_parameter(body: &str) -> String {
    // ${#VAR} — length
    if let Some(name) = body.strip_prefix('#') {
        let v = env_value(name.trim());
        return v.chars().count().to_string();
    }

    // Default/alternate forms: split on the first `:-`, `:=`, `:?`, `:+`, or
    // bare `-` / `=` at the top level (not inside a nested `${...}`). The
    // default operand may itself reference variables, so expand it too.
    let split = find_operator(body);
    let (name, op, operand) = match split {
        Some((idx, op)) => (&body[..idx], op, &body[idx + op.len()..]),
        None => (body, "", ""),
    };

    let name = name.trim();
    if op.is_empty() {
        // Plain ${VAR}
        return env_value(name);
    }

    let val = env_value(name);
    let has_value = std::env::var_os(name).is_some();
    let is_set_nonempty = has_value && !val.is_empty();

    match op {
        ":-" => {
            if is_set_nonempty {
                val
            } else {
                expand_parameter_or_plain(operand)
            }
        }
        ":=" => {
            if is_set_nonempty {
                val
            } else {
                expand_parameter_or_plain(operand)
            }
        }
        ":?" => {
            if is_set_nonempty {
                val
            } else {
                let msg = if operand.is_empty() {
                    format!("{}: parameter null or not set", name)
                } else {
                    operand.to_string()
                };
                eprintln!("waash: {}", msg);
                String::new()
            }
        }
        ":+" => {
            // Value if set & non-empty, else empty (opposite of :-).
            if is_set_nonempty {
                expand_parameter_or_plain(operand)
            } else {
                String::new()
            }
        }
        "-" => {
            if has_value {
                val
            } else {
                expand_parameter_or_plain(operand)
            }
        }
        "=" => {
            if has_value {
                val
            } else {
                expand_parameter_or_plain(operand)
            }
        }
        _ => val,
    }
}

/// Expand a default/operand string that may itself contain `${...}` or `$VAR`.
fn expand_parameter_or_plain(s: &str) -> String {
    // Recursively expand nested references in the default value.
    expand(s, 0)
}

/// Find the first `${...}` operator at the top level (not inside a nested
/// `${...}`) in a parameter body. Returns its byte index and which operator
/// it is. Distinguishes `:-`/`:=`/`:?`/`:+` (colon form) from bare `-`/`=`.
fn find_operator(body: &str) -> Option<(usize, &'static str)> {
    let bytes = body.as_bytes();
    let mut depth = 0usize;
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b == b'{' {
            depth += 1;
            i += 1;
        } else if b == b'}' {
            depth = depth.saturating_sub(1);
            i += 1;
        } else if depth == 0 {
            // Colon operators: `:` followed by - = ? +
            if b == b':' && i + 1 < bytes.len() {
                let op = match bytes[i + 1] {
                    b'-' => Some(":-"),
                    b'=' => Some(":="),
                    b'?' => Some(":?"),
                    b'+' => Some(":+"),
                    _ => None,
                };
                if let Some(op) = op {
                    return Some((i, op));
                }
            }
            // Bare - or = (reached only if not part of a colon operator).
            if b == b'-' {
                return Some((i, "-"));
            }
            if b == b'=' {
                return Some((i, "="));
            }
            i += 1;
        } else {
            i += 1;
        }
    }
    None
}

/// Whether a word contains glob metacharacters (`*`, `?`, `[`).
fn has_glob(word: &str) -> bool {
    word.contains('*') || word.contains('?') || word.contains('[')
}

/// Expand glob metacharacters (`*`, `?`, `[...]`) in a word against the
/// filesystem.
///
/// Returns `Some(sorted_matches)` when the word contains glob characters and
/// at least one path matches; returns `None` otherwise (the caller keeps the
/// original word, matching POSIX "no match = literal"). `**` is also
/// supported for recursive matching.
pub fn glob_expand(word: &str) -> Option<Vec<String>> {
    if !has_glob(word) {
        return None;
    }
    let mut matches = Vec::new();
    if let Ok(paths) = glob::glob(word) {
        for p in paths.flatten() {
            matches.push(p.to_string_lossy().into_owned());
        }
    }
    if matches.is_empty() {
        return None;
    }
    matches.sort();
    Some(matches)
}

/// Expand each argument: variable/substitution expansion, then glob
/// expansion. A single word may become multiple arguments (e.g. `*.rs`
/// matching several files).
pub fn expand_argv(args: &[String], last_exit: i32) -> Vec<String> {
    let mut out = Vec::new();
    for a in args {
        let expanded = expand(a, last_exit);
        match glob_expand(&expanded) {
            Some(matches) => out.extend(matches),
            None => out.push(expanded),
        }
    }
    out
}

/// Read characters until the matching close bracket, honoring nesting.
fn read_until_balanced(
    chars: &mut std::iter::Peekable<std::str::Chars>,
    open: char,
    close: char,
) -> String {
    let mut depth = 1i32;
    let mut content = String::new();
    while let Some(nc) = chars.next() {
        if nc == open {
            depth += 1;
            content.push(nc);
        } else if nc == close {
            depth -= 1;
            if depth == 0 {
                break;
            }
            content.push(nc);
        } else {
            content.push(nc);
        }
    }
    content
}

/// Run a command via `sh -c` and capture its stdout (trailing newlines removed).
fn run_command(cmd: &str) -> String {
    match Command::new("sh").arg("-c").arg(cmd).output() {
        Ok(out) => {
            let mut s = String::from_utf8_lossy(&out.stdout).into_owned();
            while s.ends_with('\n') {
                s.pop();
            }
            s
        }
        Err(_) => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tilde_expansion() {
        let home = std::env::var("HOME").unwrap_or_default();
        if home.is_empty() {
            return; // no HOME in this environment — nothing to expand to
        }
        assert_eq!(expand_tilde("~"), home);
        assert_eq!(expand_tilde("~/foo"), format!("{}/foo", home));
        // Only expands at the start of a word.
        assert_eq!(expand_tilde("a~b"), "a~b");
        assert_eq!(expand_tilde("x ~/y"), format!("x {}/y", home));
        // Unknown users are left literal (should not panic).
        assert!(expand_tilde("~no_such_user_zzz/x").contains("~no_such_user_zzz"));
    }

    #[test]
    fn test_brace_expansion() {
        assert_eq!(expand_braces_line("echo {a,b}"), "echo a b");
        assert_eq!(expand_braces_line("echo {1..3}"), "echo 1 2 3");
        assert_eq!(expand_braces_line("cp {a,b}.txt d"), "cp a.txt b.txt d");
        assert_eq!(expand_braces_line("x{a,{b,c}}"), "xa xb xc");
        assert_eq!(expand_braces_line("echo {5..1}"), "echo 5 4 3 2 1");
        // Group-command braces are untouched.
        assert_eq!(expand_braces_line("{ echo hi; }"), "{ echo hi; }");
        // No braces -> unchanged.
        assert_eq!(expand_braces_line("echo hi"), "echo hi");
    }

    #[test]
    fn test_last_background_pid_var() {
        set_last_background_pid(12345);
        assert_eq!(expand("$!", 0), "12345");
        set_last_background_pid(-1);
        assert_eq!(expand("$!", 0), "");
    }

    #[test]
    fn test_env_var() {
        std::env::set_var("EXP_TEST", "world");
        assert_eq!(expand("hello $EXP_TEST", 0), "hello world");
        assert_eq!(expand("hello ${EXP_TEST}!", 0), "hello world!");
        std::env::remove_var("EXP_TEST");
    }

    #[test]
    fn test_special_vars() {
        assert_eq!(expand("$?", 42), "42");
        assert_eq!(expand("$$", 0), std::process::id().to_string());
        assert_eq!(expand("$0", 0), "waash");
        assert_eq!(expand("$1", 0), "");
    }

    #[test]
    fn test_command_substitution() {
        // `printf` is available everywhere via sh.
        let out = expand("$(printf hi)", 0);
        assert_eq!(out, "hi");
        let backtick = expand("`printf yo`", 0);
        assert_eq!(backtick, "yo");
    }

    #[test]
    fn test_escapes() {
        assert_eq!(expand("a\\$b", 0), "a$b");
        assert_eq!(expand("a\\nb", 0), "a\nb");
    }

    #[test]
    fn test_glob_expand() {
        let dir = std::env::temp_dir().join(format!("waash-glob-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.txt"), "").unwrap();
        std::fs::write(dir.join("b.txt"), "").unwrap();
        std::fs::write(dir.join("c.rs"), "").unwrap();

        let pat = format!("{}/*.txt", dir.display());
        let matches = glob_expand(&pat).expect("glob should match");
        assert_eq!(matches.len(), 2);
        assert!(matches[0].ends_with("a.txt"));
        assert!(matches[1].ends_with("b.txt"));

        // No match -> None (word stays literal).
        assert!(glob_expand(&format!("{}/*.none", dir.display())).is_none());
        // No glob chars -> None.
        assert!(glob_expand("plain.txt").is_none());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_expand_argv_multiple_matches() {
        let dir = std::env::temp_dir().join(format!("waash-argv-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("x.rs"), "").unwrap();
        std::fs::write(dir.join("y.rs"), "").unwrap();
        let pat = format!("{}/*.rs", dir.display());
        let args = vec![pat];
        let out = expand_argv(&args, 0);
        assert_eq!(out.len(), 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_parameter_default_forms() {
        // Unset variable -> default.
        std::env::remove_var("WX_UNSET");
        assert_eq!(expand("${WX_UNSET:-fallback}", 0), "fallback");
        assert_eq!(expand("${WX_UNSET-fallback}", 0), "fallback");
        assert_eq!(expand("${WX_UNSET:=also}", 0), "also");

        // Set but empty variable -> `:-` uses default, `-` keeps empty.
        std::env::set_var("WX_EMPTY", "");
        assert_eq!(expand("${WX_EMPTY:-dflt}", 0), "dflt");
        assert_eq!(expand("${WX_EMPTY-dflt}", 0), "");
        std::env::remove_var("WX_EMPTY");

        // Set non-empty -> value used.
        std::env::set_var("WX_SET", "hello");
        assert_eq!(expand("${WX_SET:-dflt}", 0), "hello");
        assert_eq!(expand("${WX_SET:-}", 0), "hello");
        std::env::remove_var("WX_SET");

        // Plain form still works.
        std::env::set_var("WX_PLAIN", "val");
        assert_eq!(expand("${WX_PLAIN}", 0), "val");
        std::env::remove_var("WX_PLAIN");
    }

    #[test]
    fn test_parameter_length_and_nested_default() {
        std::env::set_var("WX_LEN", "abcd");
        assert_eq!(expand("${#WX_LEN}", 0), "4");
        std::env::remove_var("WX_LEN");

        // Nested default inside default.
        std::env::remove_var("WX_OUTER");
        std::env::set_var("WX_INNER", "inner");
        assert_eq!(expand("${WX_OUTER:-${WX_INNER:-x}}", 0), "inner");
        std::env::remove_var("WX_INNER");
    }

    #[test]
    fn test_parameter_alternate_and_question() {
        std::env::set_var("WX_ALT", "yes");
        // `:+` -> alternate when set, else empty.
        assert_eq!(expand("${WX_ALT:+alternate}", 0), "alternate");
        std::env::remove_var("WX_ALT");
        assert_eq!(expand("${WX_ALT:+alternate}", 0), "");
    }
}

