//! Tab completion for WAASH — FISH-like completions.
//!
//! Supports:
//! - Command completion (from PATH and builtins)
//! - File/directory completion
//! - Variable completion
//! - Option/flag completion for common commands

use rustyline::completion::Completer;
use rustyline::Context;
use std::collections::HashSet;
use std::env;

pub struct WaashCompleter {
    /// Cached PATH commands
    path_commands: Vec<String>,
}

impl WaashCompleter {
    pub fn new() -> Self {
        Self {
            path_commands: Self::scan_path_commands(),
        }
    }

    fn scan_path_commands() -> Vec<String> {
        let mut commands = HashSet::new();
        if let Ok(path) = env::var("PATH") {
            for dir in path.split(':') {
                if let Ok(entries) = std::fs::read_dir(dir) {
                    for entry in entries.flatten() {
                        if let Ok(meta) = entry.metadata() {
                            if meta.is_file() {
                                // Check if executable
                                use std::os::unix::fs::PermissionsExt;
                                let perms = meta.permissions();
                                if perms.mode() & 0o111 != 0 {
                                    commands.insert(entry.file_name().to_string_lossy().to_string());
                                }
                            }
                        }
                    }
                }
            }
        }
        let mut vec: Vec<String> = commands.into_iter().collect();
        vec.sort();
        vec
    }

    /// Get all builtin command names.
    fn builtin_commands() -> Vec<String> {
        vec![
            "cd", "pwd", "exit", "export", "unset", "alias", "unalias",
            "echo", "type", "source", ".", "jobs", "fg", "bg", "disown", "wait",
            "kill", "read", "pushd", "popd", "dirs", "history", "set", "test", "[", "[[", "true", "false", ":",
        ]
        .into_iter()
        .map(String::from)
        .collect()
    }

    /// Complete a partial command name.
    fn complete_command(&self, partial: &str) -> Vec<String> {
        let mut matches: Vec<String> = Vec::new();

        // Builtins
        for builtin in Self::builtin_commands() {
            if builtin.starts_with(partial) {
                matches.push(builtin);
            }
        }

        // PATH commands
        for cmd in &self.path_commands {
            if cmd.starts_with(partial) && !matches.contains(cmd) {
                matches.push(cmd.clone());
            }
        }

        matches.sort();
        matches
    }

    /// Complete file/directory paths.
    fn complete_path(&self, partial: &str) -> Vec<String> {
        let mut matches = Vec::new();

        // Determine the directory to search
        let (dir, file_prefix) = if partial.contains('/') {
            let last_slash = partial.rfind('/').unwrap();
            let dir = &partial[..=last_slash];
            let prefix = &partial[last_slash + 1..];
            (dir.to_string(), prefix.to_string())
        } else {
            (".".to_string(), partial.to_string())
        };

        if let Ok(entries) = std::fs::read_dir(&dir) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                if name.starts_with(&file_prefix) {
                    let mut full = if dir == "." {
                        name.clone()
                    } else {
                        format!("{}/{}", dir.trim_end_matches('/'), name)
                    };
                    // Add trailing slash for directories
                    if entry.path().is_dir() {
                        full.push('/');
                    }
                    matches.push(full);
                }
            }
        }

        matches.sort();
        matches
    }

    /// Complete git subcommands (`git che<TAB>` → `checkout`, `cherry-pick`, …).
    fn complete_git_subcommand(&self, partial: &str) -> Vec<String> {
        const SUBCOMMANDS: &[&str] = &[
            "add", "am", "archive", "bisect", "blame", "branch", "bundle", "cat-file",
            "checkout", "cherry-pick", "clean", "clone", "commit", "config", "describe",
            "diff", "fetch", "fsck", "gc", "grep", "init", "log", "maintenance", "merge",
            "mv", "notes", "pull", "push", "rebase", "reset", "restore", "revert", "rm",
            "shortlog", "show", "stash", "status", "submodule", "switch", "tag", "worktree",
        ];
        SUBCOMMANDS
            .iter()
            .filter(|s| s.starts_with(partial))
            .map(|s| s.to_string())
            .collect()
    }

    /// Complete git branch names (local + remote) for branch-taking subcommands.
    fn complete_git_branch(&self, partial: &str) -> Vec<String> {
        let mut branches = Vec::new();
        let out = std::process::Command::new("git")
            .args(["for-each-ref", "--format=%(refname:short)", "refs/heads", "refs/remotes"])
            .output();
        if let Ok(out) = out {
            if out.status.success() {
                for line in String::from_utf8_lossy(&out.stdout).lines() {
                    let line = line.trim();
                    if !line.is_empty() && line.starts_with(partial) {
                        branches.push(line.to_string());
                    }
                }
            }
        }
        branches.sort();
        branches.dedup();
        branches
    }

    /// Whether the previous word is a git subcommand that takes a branch name.
    fn is_git_branch_command(prev: &str) -> bool {
        matches!(
            prev,
            "checkout" | "switch" | "branch" | "merge" | "rebase" | "pull" | "push"
                | "log" | "show" | "diff" | "reset" | "restore" | "cherry-pick" | "revert"
        )
    }

    /// Complete variable names after $.
    fn complete_variable(&self, partial: &str) -> Vec<String> {
        let mut matches = Vec::new();

        for (key, _) in env::vars() {
            if key.starts_with(partial) {
                matches.push(format!("${}", key));
            }
        }

        // Common special vars
        let specials = ["$?", "$$", "$!", "$0", "$1", "$2", "$3",
                        "$HOME", "$PATH", "$USER", "$PWD", "$OLDPWD",
                        "$SHELL", "$EDITOR", "$TERM"];
        for s in &specials {
            if s.starts_with(&format!("${}", partial)) || s[1..].starts_with(partial) {
                matches.push(s.to_string());
            }
        }

        matches.sort();
        matches.dedup();
        matches
    }
}

impl Completer for WaashCompleter {
    type Candidate = String;

    fn complete(
        &self,
        line: &str,
        pos: usize,
        _ctx: &Context<'_>,
    ) -> Result<(usize, Vec<String>), rustyline::error::ReadlineError> {
        let line_up_to_cursor = &line[..pos];

        // Determine what we're completing
        let (start, partial) = if line_up_to_cursor.ends_with(' ') {
            // Completing a new word
            (pos, String::new())
        } else if let Some(last_space) = line_up_to_cursor.rfind(' ') {
            let partial = line_up_to_cursor[last_space + 1..].to_string();
            (last_space + 1, partial)
        } else {
            let partial = line_up_to_cursor.to_string();
            (0, partial)
        };

        // Context words: the command being typed and the word before the cursor.
        let words: Vec<&str> = line_up_to_cursor[..start].trim().split_whitespace().collect();
        let first_word = words.first().copied().unwrap_or("");
        let prev = words.last().copied().unwrap_or("");

        let candidates = if partial.starts_with('$') {
            self.complete_variable(&partial[1..])
        } else if first_word == "git" && prev == "git" {
            // `git <partial>` — subcommand completion
            self.complete_git_subcommand(&partial)
        } else if first_word == "git"
            && Self::is_git_branch_command(prev)
            && !partial.starts_with('-')
        {
            // `git checkout <partial>` — branch completion
            self.complete_git_branch(&partial)
        } else if start == 0 || line_up_to_cursor[..start].trim().is_empty() {
            // First word — command completion
            self.complete_command(&partial)
        } else {
            // Argument — file completion
            self.complete_path(&partial)
        };

        Ok((start, candidates))
    }

    fn update(
        &self,
        line: &mut rustyline::line_buffer::LineBuffer,
        start: usize,
        elected: &str,
        cl: &mut rustyline::Changeset,
    ) {
        let end = line.pos();
        line.replace(start..end, elected, cl);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_command_completion() {
        let c = WaashCompleter::new();
        let results = c.complete_command("ec");
        assert!(results.contains(&"echo".to_string()));
    }

    #[test]
    fn test_builtin_completion() {
        let c = WaashCompleter::new();
        let results = c.complete_command("ex");
        assert!(results.contains(&"exit".to_string()));
        assert!(results.contains(&"export".to_string()));
    }

    #[test]
    fn test_git_subcommand_completion() {
        let c = WaashCompleter::new();
        let results = c.complete_git_subcommand("che");
        assert!(results.contains(&"checkout".to_string()));
        assert!(results.contains(&"cherry-pick".to_string()));
    }

    #[test]
    fn test_git_branch_context_detection() {
        for cmd in ["checkout", "switch", "merge", "log", "restore"] {
            assert!(WaashCompleter::is_git_branch_command(cmd), "{}", cmd);
        }
        assert!(!WaashCompleter::is_git_branch_command("status"));
        assert!(!WaashCompleter::is_git_branch_command("commit"));
        assert!(!WaashCompleter::is_git_branch_command("--verbose"));
    }

    #[test]
    fn test_new_builtins_completable() {
        let c = WaashCompleter::new();
        let results = c.complete_command("dis");
        assert!(results.contains(&"disown".to_string()));
        let results = c.complete_command("wai");
        assert!(results.contains(&"wait".to_string()));
    }
}
