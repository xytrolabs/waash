//! Indent runtime integration for WAASH.
//!
//! WAASH scripts are written in Indent syntax (.waash or .ind files).
//! This module handles discovering the Indent runtime, setting up the
//! WAASH helper library, and executing Indent-based scripts.

use std::path::{Path, PathBuf};
use std::process::Command;

/// Find the Indent runtime binary.
///
/// Search order:
/// 1. WAASH_INDENT_BINARY env var
/// 2. Config value (indent_binary)
/// 3. `which indent` on PATH
/// 4. ~/.local/bin/indent
/// 5. Hardcoded path (your development build)
pub fn find_indent_binary(config_path: Option<&str>) -> Option<PathBuf> {
    // 1. Environment variable override
    if let Ok(path) = std::env::var("WAASH_INDENT_BINARY") {
        let p = PathBuf::from(&path);
        if p.exists() {
            log::info!("Using indent from WAASH_INDENT_BINARY: {}", p.display());
            return Some(p);
        }
    }

    // 2. Config value
    if let Some(path) = config_path {
        let p = PathBuf::from(path);
        if p.exists() {
            log::info!("Using indent from config: {}", p.display());
            return Some(p);
        }
    }

    // 3. PATH search
    if let Ok(output) = Command::new("which").arg("indent").output() {
        if output.status.success() {
            let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
            let p = PathBuf::from(&path);
            if p.exists() {
                log::info!("Found indent on PATH: {}", p.display());
                return Some(p);
            }
        }
    }

    // 4. Common install location
    let home_local = dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("/home/user"))
        .join(".local/bin/indent");
    if home_local.exists() {
        log::info!("Found indent at: {}", home_local.display());
        return Some(home_local);
    }

    // 5. Hardcoded development path
    let dev_path = PathBuf::from(
        "/run/media/raf/Z/Aether.ath/indent-native/target/release/indent",
    );
    if dev_path.exists() {
        log::info!("Found indent at dev path: {}", dev_path.display());
        return Some(dev_path);
    }

    None
}

/// Find the WAASH helper library directory (contains waash.ind).
pub fn find_waash_lib() -> Option<PathBuf> {
    // Check relative to the binary
    if let Ok(exe) = std::env::current_exe() {
        let share_dir = exe
            .parent()
            .unwrap_or(Path::new("."))
            .parent()
            .unwrap_or(Path::new("."))
            .join("share/waash");

        let lib = share_dir.join("waash.ind");
        if lib.exists() {
            return Some(share_dir);
        }
    }

    // Check relative to current directory (dev mode)
    let dev_share = PathBuf::from("share/waash");
    let dev_lib = dev_share.join("waash.ind");
    if dev_lib.exists() {
        return Some(dev_share);
    }

    // Check installed location
    let installed = dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("waash/lib");
    let installed_lib = installed.join("waash.ind");
    if installed_lib.exists() {
        return Some(installed);
    }

    None
}

/// Run an Indent script file through the Indent runtime.
/// Reads the file, preprocesses it (auto-imports, wraps bare commands),
/// then executes via indent.
/// Returns the exit code.
pub fn run_indent_script(
    indent_binary: &Path,
    script_path: &Path,
    waash_lib_dir: &Path,
) -> anyhow::Result<i32> {
    // Read the original script
    let code = std::fs::read_to_string(script_path)
        .map_err(|e| anyhow::anyhow!("Failed to read {}: {}", script_path.display(), e))?;

    // Preprocess (auto-import waash helpers, wrap bare commands)
    let processed = preprocess_waash_script(&code);

    // Write to temp file
    let tmp_dir = std::env::temp_dir();
    let tmp_file = tmp_dir.join(format!(
        "waash_script_{}.ind",
        std::process::id()
    ));

    std::fs::write(&tmp_file, &processed)
        .map_err(|e| anyhow::anyhow!("Failed to write temp script: {}", e))?;

    // Set up INDENT_PATH so the script can import waash
    let indent_path = waash_lib_dir.to_string_lossy().to_string();
    let current_path = std::env::var("INDENT_PATH").unwrap_or_default();
    let combined_path = if current_path.is_empty() {
        indent_path
    } else {
        format!("{}:{}", indent_path, current_path)
    };

    let output = Command::new(indent_binary)
        .arg("run")
        .arg(&tmp_file)
        .env("INDENT_PATH", &combined_path)
        .output()
        .map_err(|e| anyhow::anyhow!("Failed to run indent: {}", e))?;

    // Clean up temp file
    let _ = std::fs::remove_file(&tmp_file);

    // Forward stdout/stderr
    if !output.stdout.is_empty() {
        use std::io::Write;
        let _ = std::io::stdout().write_all(&output.stdout);
    }
    if !output.stderr.is_empty() {
        use std::io::Write;
        let _ = std::io::stderr().write_all(&output.stderr);
    }

    Ok(output.status.code().unwrap_or(1))
}

/// Execute a string of Indent code via the Indent runtime.
/// Also preprocesses the code: auto-imports waash helpers and
/// wraps bare command lines as sh() calls.
pub fn run_indent_string(
    indent_binary: &Path,
    code: &str,
    waash_lib_dir: &Path,
) -> anyhow::Result<i32> {
    let processed = preprocess_waash_script(code);

    // Write code to a temp file
    let tmp_dir = std::env::temp_dir();
    let tmp_file = tmp_dir.join(format!(
        "waash_script_{}.ind",
        std::process::id()
    ));

    std::fs::write(&tmp_file, &processed)
        .map_err(|e| anyhow::anyhow!("Failed to write temp script: {}", e))?;

    let result = run_indent_script(indent_binary, &tmp_file, waash_lib_dir);

    // Clean up
    let _ = std::fs::remove_file(&tmp_file);

    result
}

/// Keywords and helper names that mark a line as Indent (not a bare shell
/// command). Used both for preprocessing scripts and for deciding whether a
/// `-c` string is Indent or plain shell.
///
/// Kept in sync with the current Indent runtime (v1.4.0) — includes the
/// newer `set <var> <type>` type-conversion syntax and the `contains` set
/// builtin.
pub const INDENT_KEYWORDS: &[&str] = &[
    "fun ", "var ", "if ", "or ", "otherwise", "repeat ", "for ",
    "say ", "give ", "get ", "import ", "class ", "match ",
    "do:", "catch ", "lastly:", "flag ", "while ", "stop", "next",
    "reset", "open ", "#!", "include ", "assert ",
    // Indent 1.4.0: set type-conversion + set builtins
    "set ", "contains ",
    // Indent builtins (so bare calls aren't mistaken for shell commands)
    "process_exit ", "os_system ", "os_exists ", "os_getenv ",
    "os_setenv ", "os_remove ", "file_read_text ", "file_write_text ",
    "file_append_text ", "time_perf_counter ", "time_now ", "time_sleep ",
    "json_loads ", "json_dumps ", "keys ", "has_key ", "slice ",
    "trim ", "starts_with ", "ends_with ", "string ", "int ",
    "float ", "boolean ", "ws_connect ", "ws_send_text ", "ws_recv_text ",
    "ws_close ", "http_get_json ", "http_post_json ", "call_func ",
    // WAASH helper functions (auto-imported)
    "header ", "success ", "error ", "info ",
    "sh ", "sh_capture ", "has_command ",
];

/// Whether a single trimmed line is an Indent construct (vs. a bare command).
pub fn is_indent_line(trimmed: &str) -> bool {
    INDENT_KEYWORDS
        .iter()
        .any(|kw| trimmed.starts_with(kw))
        || is_comprehension(trimmed)
}

/// Indent 1.4.0 list/dict comprehensions start with `[` or `{` and contain a
/// `for ... in` clause, e.g. `[x * 2 for x in nums]` or
/// `{k: v * 2 for k, v in pairs}`. Shell `[ ... ]` tests and `{ ... }` groups
/// almost never contain `for ... in`, so this is a safe discriminator.
fn is_comprehension(line: &str) -> bool {
    let l = line.trim_start();
    (l.starts_with('[') || l.starts_with('{'))
        && l.contains(" for ")
        && l.contains(" in ")
}

/// Preprocess a WAASH script: auto-import waash helpers, wrap bare commands.
///
/// Transforms this:
///   cargo build --release
///   echo hello
/// Into:
///   get sh from waash
///   sh("cargo build --release")
///   sh("echo hello")
///
/// Lines starting with Indent keywords (fun, var, if, say, get, etc.)
/// or comments (#!) are left untouched.
pub fn preprocess_waash_script(code: &str) -> String {
    // Collect user-defined function names (`fun NAME ...`) so that bare calls
    // to them (e.g. a top-level `main`) aren't mistaken for shell commands.
    let mut func_names: Vec<String> = Vec::new();
    for line in code.lines() {
        if let Some(rest) = line.trim().strip_prefix("fun ") {
            if let Some(name) = rest.split_whitespace().next() {
                if !name.is_empty() {
                    func_names.push(name.to_string());
                }
            }
        }
    }

    let mut result = String::new();

    // Auto-import all waash helpers
    result.push_str("#! Auto-imported by WAASH\n");
    result.push_str("get sh from waash\n");
    result.push_str("get sh_capture from waash\n");
    result.push_str("get has_command from waash\n");
    result.push_str("get header from waash\n");
    result.push_str("get success from waash\n");
    result.push_str("get error from waash\n");
    result.push_str("get info from waash\n");
    result.push_str("\n");

    for line in code.lines() {
        let trimmed = line.trim();

        // Skip empty lines
        if trimmed.is_empty() {
            result.push_str("\n");
            continue;
        }

        // Skip lines that are Indent constructs or calls to functions defined
        // in this script.
        let is_indent = is_indent_line(trimmed)
            || func_names.iter().any(|name| {
                trimmed == name || trimmed.starts_with(&format!("{} ", name))
            });

        if is_indent {
            result.push_str(line);
            result.push_str("\n");
        } else {
            // Treat as a shell command — wrap in sh()
            // Escape quotes in the command
            let escaped = trimmed.replace('"', "\\\"");
            result.push_str(&format!("sh(\"{}\")\n", escaped));
        }
    }

    result
}

/// Check if a file is an Indent/WAASH script (by extension).
pub fn is_waash_script(path: &Path) -> bool {
    match path.extension().and_then(|e| e.to_str()) {
        Some("waash") | Some("ind") => true,
        _ => false,
    }
}

/// Install the WAASH helper library to the user's data directory.
pub fn install_waash_lib() -> std::io::Result<PathBuf> {
    let dest = dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("waash/lib");

    std::fs::create_dir_all(&dest)?;

    // Copy waash.ind
    let src = PathBuf::from("share/waash/waash.ind");
    if src.exists() {
        std::fs::copy(&src, dest.join("waash.ind"))?;
    }

    // Copy examples
    let examples_src = PathBuf::from("share/waash/examples");
    let examples_dest = dest.join("examples");
    if examples_src.exists() {
        std::fs::create_dir_all(&examples_dest)?;
        for entry in std::fs::read_dir(&examples_src)? {
            let entry = entry?;
            if entry.path().extension().map_or(false, |e| e == "waash") {
                std::fs::copy(entry.path(), examples_dest.join(entry.file_name()))?;
            }
        }
    }

    Ok(dest)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_indent_line_keywords() {
        // Classic Indent keywords.
        assert!(is_indent_line("fun main"));
        assert!(is_indent_line("var x = 1"));
        assert!(is_indent_line("if x > 0"));
        assert!(is_indent_line("say \"hi\""));
        assert!(is_indent_line("get sh from waash"));
        assert!(is_indent_line("#! comment"));
        assert!(is_indent_line("otherwise"));
        assert!(is_indent_line("repeat item in list"));
    }

    #[test]
    fn test_is_indent_line_1400_keywords() {
        // Indent 1.4.0 additions: `set <var> <type>` type conversion + `contains`.
        assert!(is_indent_line("set name string"));
        assert!(is_indent_line("set total int"));
        assert!(is_indent_line("contains s 2"));
    }

    #[test]
    fn test_is_indent_line_comprehensions() {
        // Indent 1.4.0 list/dict comprehensions must not be treated as shell.
        assert!(is_indent_line("[x * 2 for x in nums]"));
        assert!(is_indent_line("[x for x in nums if x > 5]"));
        assert!(is_indent_line("{x: x * 2 for x in nums}"));
        assert!(is_indent_line("[f(x) for x in items]"));
        // Shell `[ ... ]` tests and `{ ... }` groups are NOT comprehensions.
        assert!(!is_indent_line("[ -f file ]"));
        assert!(!is_indent_line("{ echo hi; }"));
    }

    #[test]
    fn test_preprocess_comprehensions() {
        let code = "var nums = [1, 2, 3]\nsay [x * 2 for x in nums]\n";
        let out = preprocess_waash_script(code);
        assert!(out.contains("[x * 2 for x in nums]"));
        assert!(!out.contains("sh(\"[x * 2"));
    }

    #[test]
    fn test_is_indent_line_bare_commands() {
        // Shell commands are NOT Indent lines.
        assert!(!is_indent_line("ls -la"));
        assert!(!is_indent_line("cargo build --release"));
        assert!(!is_indent_line("echo hello"));
        assert!(!is_indent_line("git status"));
    }

    #[test]
    fn test_preprocess_wraps_bare_commands() {
        let code = "echo hello\nvar x = 1\nsay x\n";
        let out = preprocess_waash_script(code);
        assert!(out.contains("sh(\"echo hello\")"));
        // Indent lines pass through untouched.
        assert!(out.contains("var x = 1"));
        assert!(out.contains("say x"));
        // Helpers auto-imported.
        assert!(out.contains("get sh from waash"));
    }

    #[test]
    fn test_preprocess_set_type_conversion() {
        // `set <var> <type>` must NOT be wrapped as a shell command.
        let code = "var n = 21\nset n string\nsay n\n";
        let out = preprocess_waash_script(code);
        assert!(out.contains("set n string"));
        assert!(!out.contains("sh(\"set n string\")"));
    }

    #[test]
    fn test_preprocess_user_function_calls() {
        // A top-level call to a function defined in the script must not be
        // wrapped as a shell command.
        let code = "fun build\n  say \"building\"\nbuild\n";
        let out = preprocess_waash_script(code);
        assert!(out.contains("fun build"));
        // The bare `build` call stays an Indent call.
        assert!(out.contains("\nbuild\n"));
        assert!(!out.contains("sh(\"build\")"));
    }

    #[test]
    fn test_preprocess_builtin_calls() {
        // Builtins used as bare statements stay Indent (not wrapped).
        let code = "if x\n  process_exit 1\n";
        let out = preprocess_waash_script(code);
        assert!(out.contains("process_exit 1"));
        assert!(!out.contains("sh(\"process_exit"));
    }
}
