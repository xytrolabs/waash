//! Login-shell support.
//!
//! When WAASH is installed as a user's login shell (`chsh -s .../waash`), the
//! login process invokes it with a leading `-` in `argv[0]` (e.g. `-waash`).
//! A proper login shell is expected to source the system profile files
//! (`/etc/profile` and `~/.profile`) so the user's environment (PATH, exports,
//! umask, …) is set up.
//!
//! WAASH's own parser intentionally implements the *interactive* shell subset
//! (simple commands, pipelines, redirections, heredocs, `&&`/`||`), not full
//! POSIX control flow (`if`/`for`/`while`/`case`/functions). To stay fully
//! compatible with arbitrary `.profile` scripts, we evaluate the profiles with
//! the system POSIX `sh` and then **import the resulting environment** into
//! WAASH's process, so `export`s persist exactly as they would in bash.

use std::env;
use std::process::{Command, Stdio};

/// Detect whether WAASH was invoked as a login shell (`argv[0]` starts with `-`).
pub fn is_login_shell() -> bool {
    env::args().next().map_or(false, |a| a.starts_with('-'))
}

/// Parse a NUL-separated `env -0` dump into `(name, value)` pairs.
///
/// Split into its own function so it can be unit-tested without touching the
/// real process environment.
pub fn parse_env_dump(dump: &[u8]) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for entry in dump.split(|&b| b == 0) {
        if entry.is_empty() {
            continue;
        }
        if let Some(eq) = entry.iter().position(|&b| b == b'=') {
            let name = String::from_utf8_lossy(&entry[..eq]);
            let value = String::from_utf8_lossy(&entry[eq + 1..]);
            if !name.is_empty() {
                out.push((name.into_owned(), value.into_owned()));
            }
        }
    }
    out
}

/// Source `/etc/profile` then `~/.profile` and import the resulting
/// environment into WAASH's process.
///
/// The profiles are evaluated by the system POSIX `sh` (which understands all
/// control flow), with their normal output left to the terminal. The final
/// environment is dumped NUL-separated to a temp file and re-applied, so any
/// `export`s (and `set -a`-exported assignments) survive.
///
/// # Safety
/// A login shell must NEVER hang on startup, or the user could get locked out.
/// So the profile `sh` runs with stdin from `/dev/null` (any `read` gets EOF
/// instead of blocking) and is killed after a hard 5-second timeout.
pub fn source_login_profiles() {
    let dump_path = env::temp_dir().join(format!("waash-login-env-{}", std::process::id()));
    let quoted = shell_quote(&dump_path.to_string_lossy());

    // `set -a` marks assignments for export, so even plain `FOO=bar` lines in
    // a profile end up in the environment. Profile output goes to the terminal
    // (matches normal login behaviour); only the env dump is redirected.
    let script = format!(
        "set -a\n\
         . /etc/profile 2>/dev/null\n\
         . \"$HOME/.profile\" 2>/dev/null\n\
         set +a\n\
         env -0 > {quoted}"
    );

    let Ok(mut child) = Command::new("sh")
        .arg("-c")
        .arg(&script)
        .stdin(Stdio::null())
        .spawn()
    else {
        return;
    };

    // Poll with a deadline; kill if the profile is still running after 5s.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) => {
                if std::time::Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            Err(_) => break,
        }
    }

    if let Ok(dump) = std::fs::read(&dump_path) {
        for (name, value) in parse_env_dump(&dump) {
            // Prefer the freshly-sourced value; these come from the profile.
            env::set_var(name, value);
        }
    }
    let _ = std::fs::remove_file(&dump_path);
}

/// Single-quote a string for use inside a `sh -c` script.
fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_env_dump_basic() {
        let dump = b"PATH=/usr/bin:/bin\0HOME=/home/raf\0SHELL=/usr/bin/zsh\0";
        let pairs = parse_env_dump(dump);
        assert_eq!(
            pairs,
            vec![
                ("PATH".to_string(), "/usr/bin:/bin".to_string()),
                ("HOME".to_string(), "/home/raf".to_string()),
                ("SHELL".to_string(), "/usr/bin/zsh".to_string()),
            ]
        );
    }

    #[test]
    fn test_parse_env_dump_handles_empty_and_malformed() {
        // Trailing NUL, empty entries, and entries without '=' are skipped.
        let dump = b"FOO=1\0\0BAR\0BAZ=with=equals\0";
        let pairs = parse_env_dump(dump);
        assert_eq!(
            pairs,
            vec![
                ("FOO".to_string(), "1".to_string()),
                ("BAZ".to_string(), "with=equals".to_string()),
            ]
        );
    }

    #[test]
    fn test_parse_env_dump_unicode_value() {
        let dump = "NAME=Wåsh ⚡\0".as_bytes();
        let pairs = parse_env_dump(dump);
        assert_eq!(pairs, vec![("NAME".to_string(), "Wåsh ⚡".to_string())]);
    }

    #[test]
    fn test_shell_quote() {
        assert_eq!(shell_quote("plain"), "'plain'");
        assert_eq!(shell_quote("has 'quotes'"), "'has '\\''quotes'\\'''");
    }
}
