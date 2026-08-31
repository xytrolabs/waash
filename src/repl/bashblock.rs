//! Detection and balancing of multi-line bash control-flow blocks typed in
//! the REPL.
//!
//! WAASH's own parser handles simple commands (fast path). Complex bash
//! constructs — `if`/`for`/`while`/`case`, function definitions, `[[ ]]`,
//! brace groups — are collected as a multi-line block and run through the
//! real `bash` interpreter for correctness. This is the "hybrid" approach:
//! WAASH keeps its FISH-style UX for ordinary commands, and delegates the
//! bash grammar it doesn't parse to bash itself, so nothing ever breaks.

/// Keywords that OPEN a bash control-flow block (increase nesting depth).
/// `then`/`do`/`else`/`elif` are NOT counted — they're continuations inside
/// an `if`/`for` block and don't require their own closer. `function` is
/// handled via its `{`, and `[[`/`{` are handled separately.
const BLOCK_OPENERS: &[&str] = &["if", "for", "while", "until", "case", "select"];

/// Keywords that CLOSE a bash control-flow block (decrease nesting depth).
const BLOCK_CLOSERS: &[&str] = &["fi", "done", "esac"];

/// Whether a line looks like a function definition: `name() {` or `function name`.
fn looks_like_function_def(trimmed: &str) -> bool {
    if trimmed.starts_with("function ") {
        return true;
    }
    // name() { ... }
    if let Some(open) = trimmed.find("()") {
        // The part before `()` must be a valid identifier, and there must be a
        // `{` after it (the function body).
        let name = trimmed[..open].trim();
        if !name.is_empty()
            && name.chars().all(|c| c.is_alphanumeric() || c == '_')
            && trimmed[open + 2..].contains('{')
        {
            return true;
        }
    }
    false
}

/// Does `line` start a bash control-flow block that needs continuation?
pub fn starts_bash_block(line: &str) -> bool {
    let t = line.trim_start();
    if looks_like_function_def(t) {
        return true;
    }
    // `[[ ... ]]` is a single-line test expression handled natively by WAASH's
    // `[[` builtin, so it is NOT a multi-line bash block.
    if t.starts_with("function ") || t.starts_with('{') {
        return true;
    }
    let first = t.split_whitespace().next().unwrap_or("");
    BLOCK_OPENERS.contains(&first)
}

/// Compute the control-flow nesting depth of a single line using a lightweight
/// word scan. Positive = more openers than closers (block not yet complete).
/// This is a heuristic: it intentionally over-counts rather than under-counts,
/// so a real block is never split prematurely. Over-collecting an extra line
/// is harmless (bash just runs it); splitting a block early would break it.
fn line_depth(line: &str) -> i32 {
    let mut depth = 0i32;
    for tok in line.split_whitespace() {
        match tok {
            t if BLOCK_OPENERS.contains(&t) => depth += 1,
            t if BLOCK_CLOSERS.contains(&t) => depth -= 1,
            "{" => depth += 1,
            "}" => depth -= 1,
            "[[" => depth += 1,
            "]]" => depth -= 1,
            _ => {}
        }
    }
    depth
}

/// Is the collected block of lines balanced (complete)? `lines` must be
/// non-empty. A single self-contained line (e.g. `if x; then y; fi`) returns
/// true; a block missing its terminator returns false.
pub fn bash_block_done(lines: &[String]) -> bool {
    if lines.is_empty() {
        return false;
    }
    let mut depth = 0i32;
    for l in lines {
        depth += line_depth(l);
    }
    depth <= 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_openers() {
        assert!(starts_bash_block("if [ -f x ]; then"));
        assert!(starts_bash_block("for i in 1 2 3; do"));
        assert!(starts_bash_block("while true; do"));
        assert!(starts_bash_block("until false; do"));
        assert!(starts_bash_block("case $x in"));
        assert!(starts_bash_block("function foo {"));
        assert!(starts_bash_block("myfunc() {"));
        assert!(starts_bash_block("{ echo hi;"));
        // `[[ ]]` is now handled natively by the `[[` builtin (single line).
        assert!(!starts_bash_block("[[ -f x ]]"));
        // Non-openers stay on the normal path.
        assert!(!starts_bash_block("echo hello"));
        assert!(!starts_bash_block("ls -la"));
        assert!(!starts_bash_block("git status"));
    }

    #[test]
    fn one_line_complete() {
        // A self-contained one-liner is "done".
        let l = vec!["if true; then echo hi; fi".to_string()];
        assert!(bash_block_done(&l));
        let l = vec!["for i in 1 2 3; do echo $i; done".to_string()];
        assert!(bash_block_done(&l));
    }

    #[test]
    fn multi_line_needs_terminator() {
        // Opening if alone is not done.
        let l = vec!["if [ -f x ]; then".to_string()];
        assert!(!bash_block_done(&l));
        // Adding the fi completes it.
        let l = vec!["if [ -f x ]; then".to_string(), "  echo hi".to_string(), "fi".to_string()];
        assert!(bash_block_done(&l));
        // for ... done
        let l = vec!["for i in 1 2 3; do".to_string(), "  echo $i".to_string(), "done".to_string()];
        assert!(bash_block_done(&l));
        // Nested if/for: open 2, close 2.
        let l = vec![
            "if true; then".to_string(),
            "  for i in 1 2; do".to_string(),
            "    echo $i".to_string(),
            "  done".to_string(),
            "fi".to_string(),
        ];
        assert!(bash_block_done(&l));
    }

    #[test]
    fn function_def() {
        let l = vec!["greet() {".to_string(), "  echo hi".to_string(), "}".to_string()];
        assert!(bash_block_done(&l));
        assert!(starts_bash_block("greet() {"));
    }
}

