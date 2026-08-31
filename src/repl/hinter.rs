//! Autosuggestions (hints) for WAASH — FISH-style ghost text.
//!
//! Shows dimmed predictions as the user types. Like FISH, the hint
//! appears as gray text that can be accepted with Right Arrow or Ctrl+F.
//!
//! Prediction sources (in order of preference):
//!   1. Command history — most recent matching command wins.
//!   2. Command-name completion — finish a partially-typed command from PATH.
//!   3. Next-argument hints — e.g. `git ` → `status`, `systemctl ` → `status`.
//!   4. Path completion — if the last token looks like a path prefix.
//!   5. Common idioms — e.g. `cd` alone hints at `cd ~`.

use rustyline::hint::Hinter;
use rustyline::Context;
use rustyline::history::{History, SearchDirection};
use std::cell::RefCell;

/// Hint provider that predicts the rest of a command.
pub struct WaashHinter {
    /// Whether autosuggestions are enabled
    enabled: bool,
    /// Lazily-built, cached list of executable names on PATH (for command hints).
    commands: RefCell<Option<Vec<String>>>,
}

impl WaashHinter {
    pub fn with_enabled(enabled: bool) -> Self {
        Self {
            enabled,
            commands: RefCell::new(None),
        }
    }

    /// Cache the list of executable names found on PATH (built once).
    fn command_list(&self) -> std::cell::Ref<'_, Option<Vec<String>>> {
        if self.commands.borrow().is_none() {
            let mut names = std::collections::HashSet::new();
            if let Ok(path_var) = std::env::var("PATH") {
                for dir in path_var.split(':') {
                    if dir.is_empty() {
                        continue;
                    }
                    if let Ok(rd) = std::fs::read_dir(dir) {
                        for e in rd.flatten() {
                            if let Ok(ft) = e.file_type() {
                                if ft.is_file() {
                                    names.insert(e.file_name().to_string_lossy().to_string());
                                }
                            }
                        }
                    }
                }
            }
            let mut sorted: Vec<String> = names.into_iter().collect();
            sorted.sort();
            *self.commands.borrow_mut() = Some(sorted);
        }
        self.commands.borrow()
    }

    /// Search history for a command matching the current prefix.
    fn search_history(&self, line: &str, history: &dyn History) -> Option<String> {
        if line.is_empty() {
            return None;
        }

        let trimmed = line.trim_end();

        // Search history in reverse order (most recent first)
        for idx in (0..history.len()).rev() {
            if let Ok(Some(entry)) = history.get(idx, SearchDirection::Forward) {
                let entry = entry.entry.trim();
                if entry.starts_with(trimmed) && entry.len() > trimmed.len() {
                    // Return the suffix after the typed prefix
                    return Some(entry[trimmed.len()..].to_string());
                }
            }
        }

        None
    }

    /// Complete a partially-typed command name from PATH, e.g. `sys` → `temctl`.
    /// Skips builtins and exact matches so `ls` doesn't suggest `lsblk`.
    fn complete_command_name(&self, line: &str) -> Option<String> {
        let word = line.trim();
        if word.len() < 2
            || word.contains(' ')
            || word.contains('/')
            || word.contains('~')
            || word.contains('.')
        {
            return None;
        }
        if crate::builtins::is_builtin(word) {
            return None;
        }

        let commands = self.command_list();
        let commands = commands.as_ref()?;
        let exact = commands.binary_search(&word.to_string()).is_ok();
        if exact {
            return None;
        }

        let matches: Vec<String> = commands
            .iter()
            .filter(|c| c.starts_with(word) && c.len() > word.len())
            .take(64)
            .cloned()
            .collect();

        if matches.is_empty() {
            return None;
        }
        let common = longest_common_prefix(&matches);
        if common.len() > word.len() {
            Some(common[word.len()..].to_string())
        } else {
            None
        }
    }

    /// Suggest a common next argument when a well-known command is complete,
    /// e.g. `git ` → `status`, `systemctl ` → `status`, `cargo ` → `build`.
    fn suggest_arguments(&self, line: &str) -> Option<String> {
        if !line.ends_with(' ') {
            return None;
        }
        let words: Vec<&str> = line.trim_end().split_whitespace().collect();
        let cmd = *words.first()?;
        let arg = match cmd {
            "git" => "status",
            "systemctl" => "status",
            "docker" => "ps",
            "podman" => "ps",
            "npm" => "run",
            "cargo" => "build",
            "apt" | "apt-get" | "dnf" => "install",
            "pacman" | "yay" | "paru" => "-S",
            "zypper" => "install",
            "flatpak" => "list",
            "ls" => "-la",
            "mkdir" => "-p",
            "grep" => "-r",
            "curl" => "-L",
            "ssh" => "-p 22",
            _ => return None,
        };
        if arg.is_empty() {
            None
        } else {
            Some(format!("{} ", arg))
        }
    }

    /// If the line ends with a path prefix, complete it from the filesystem.
    /// e.g. "cd ~/De" → hints "sktop"
    fn complete_path(&self, line: &str) -> Option<String> {
        let trimmed = line.trim_end();
        if trimmed.is_empty() {
            return None;
        }

        let words: Vec<&str> = trimmed.split_whitespace().collect();
        let last = *words.last()?;
        let first = *words.first()?;

        let is_path_cmd = [
            "cd", "ls", "cat", "vim", "nano", "open", "cp", "mv", "rm",
            "less", "more", "grep", "find", "touch", "mkdir", "source",
        ]
        .contains(&first);

        if !is_path_cmd && !last.contains('/') && !last.starts_with('.') && !last.starts_with('~') {
            return None;
        }

        // Expand a leading "~" so we can read the filesystem.
        let home = std::env::var("HOME").ok()?;
        let expanded = if last == "~" {
            home.clone()
        } else if let Some(rest) = last.strip_prefix("~/") {
            format!("{}/{}", home, rest)
        } else {
            last.to_string()
        };

        // Split into directory + prefix.
        let (dir, prefix) = match expanded.rfind('/') {
            Some(i) => {
                let d = if i == 0 { "/".to_string() } else { expanded[..i].to_string() };
                (d, expanded[i + 1..].to_string())
            }
            None => (".".to_string(), expanded.clone()),
        };

        let mut names: Vec<(String, bool)> = std::fs::read_dir(&dir)
            .ok()?
            .filter_map(|e| e.ok())
            .filter_map(|e| {
                let name = e.file_name().to_string_lossy().to_string();
                if name.starts_with(&prefix) {
                    Some((name, e.file_type().map(|t| t.is_dir()).unwrap_or(false)))
                } else {
                    None
                }
            })
            .collect();

        if names.is_empty() {
            return None;
        }
        // Directories first, then alphabetically.
        names.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));

        let best = if names.len() == 1 {
            names[0].clone()
        } else {
            let common = longest_common_prefix(
                &names.iter().map(|(n, _)| n.clone()).collect::<Vec<_>>(),
            );
            (common, false)
        };

        // Build the completed last token (keeping "~" as the user typed it).
        let completed_full = format!("{}/{}", dir.trim_end_matches('/'), best.0);
        let completed = if last == "~" || last.starts_with("~/") {
            let rel = completed_full
                .trim_start_matches(&home)
                .trim_start_matches('/');
            format!("~/{}", rel)
        } else {
            completed_full
        };

        if completed.starts_with(last) && completed.len() > last.len() {
            Some(completed[last.len()..].to_string())
        } else {
            None
        }
    }

    /// Hint common idioms for a bare command word.
    fn common_idioms(&self, line: &str) -> Option<String> {
        match line.trim_end() {
            "cd" => Some(" ~".to_string()),
            "ls" => Some(" -la".to_string()),
            "git" => Some(" status".to_string()),
            "mkdir" => Some(" -p".to_string()),
            "source" | "." => Some(" ./".to_string()),
            "sudo" => Some(" ".to_string()),
            _ => None,
        }
    }
}

fn longest_common_prefix(strings: &[String]) -> String {
    if strings.is_empty() {
        return String::new();
    }
    let first = &strings[0];
    let mut end = first.len();
    for s in &strings[1..] {
        let mut e = 0;
        for (a, b) in first.bytes().zip(s.bytes()) {
            if a == b {
                e += 1;
            } else {
                break;
            }
        }
        end = end.min(e);
    }
    first[..end].to_string()
}

impl Hinter for WaashHinter {
    type Hint = String;

    fn hint(&self, line: &str, pos: usize, ctx: &Context<'_>) -> Option<String> {
        if !self.enabled {
            return None;
        }
        if pos < line.len() {
            // Cursor not at end — no hint
            return None;
        }
        if line.trim().is_empty() {
            return None;
        }

        // 1. History match (most fish-like)
        let history = ctx.history();
        if let Some(h) = self.search_history(line, history) {
            return Some(h);
        }

        // 2. Command-name completion from PATH (e.g. `sys` → `temctl`)
        if let Some(c) = self.complete_command_name(line) {
            return Some(c);
        }

        // 3. Next-argument hints for completed commands (`git ` → `status`)
        if let Some(a) = self.suggest_arguments(line) {
            return Some(a);
        }

        // 4. Path completion
        if let Some(p) = self.complete_path(line) {
            return Some(p);
        }

        // 5. Common idioms
        self.common_idioms(line)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_longest_common_prefix() {
        assert_eq!(
            longest_common_prefix(&["hello".into(), "help".into(), "hell".into()]),
            "hel"
        );
        assert_eq!(longest_common_prefix(&["abc".into()]), "abc");
        assert_eq!(longest_common_prefix(&[]), "");
    }

    #[test]
    fn test_suggest_arguments() {
        let h = WaashHinter::with_enabled(true);
        assert_eq!(h.suggest_arguments("git ").as_deref(), Some("status "));
        assert_eq!(h.suggest_arguments("cargo ").as_deref(), Some("build "));
        assert_eq!(h.suggest_arguments("ls ").as_deref(), Some("-la "));
        // Only fires when the line ends with a space.
        assert_eq!(h.suggest_arguments("git status"), None);
    }

    #[test]
    fn test_command_name_completion() {
        let h = WaashHinter::with_enabled(true);
        // `systemctl` is on PATH almost everywhere.
        let hint = h.complete_command_name("system");
        if let Some(suffix) = hint {
            assert_eq!(format!("system{}", suffix), "systemctl");
        }
        // Builtins are not "completed" via PATH.
        assert_eq!(h.complete_command_name("cd"), None);
        // Multi-word lines are not command-name completions.
        assert_eq!(h.complete_command_name("git status"), None);
    }

    #[test]
    fn test_common_idioms() {
        let h = WaashHinter::with_enabled(true);
        assert_eq!(h.common_idioms("cd").as_deref(), Some(" ~"));
        assert_eq!(h.common_idioms("ls").as_deref(), Some(" -la"));
    }
}
