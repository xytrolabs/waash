# Changelog

## [0.3.0] — In progress — Background, expansion, jobs & builtins

### Added
- **Brace expansion** — `{a,b}`, `{1..5}`, `x{a,b}` and nested/range forms
  expand per word (`echo {a,b}` → `echo a b`, `cp {a,b}.txt d` →
  `cp a.txt b.txt d`). Applied before lexing; group-command braces
  (`{ cmd; }`) are left untouched. `src/wordexp.rs`
- **Native `[[ ]]` test** — `[[ -f file ]]`, `[[ a == b ]]`, `[[ 1 -gt 2 ]]`,
  `-d -e -x -w -r -s -z -n -eq -ne -lt -le -gt -ge`, `!`, `=`, `!=`. Now a
  WAASH builtin (no longer delegated to bash for the single-line case).
  `src/builtins/mod.rs`, `src/repl/bashblock.rs`
- **`read` builtin** — `read [-p prompt] var...` reads a line and assigns
  words to variables (last var gets the remainder; defaults to `REPLY`).
  `src/builtins/mod.rs`
- **`pushd`/`popd`/`dirs`** — directory stack. `src/executor/mod.rs`
- **Custom key bindings** — `[[keybindings]]` in `config.toml` now actually
  wire into rustyline (`key = "Ctrl+R"`, `action = "history_search_backward"`,
  etc.). `src/repl/mod.rs`, `src/config/mod.rs`

### Fixed
- **Tilde `~` expansion** — `~`, `~/Downloads`, `~user/x` expand to home dirs.
  `src/wordexp.rs`
- **`$!`** — expands to the PID of the most recently launched background job.
- **Builtin errors honor `2>` / `2>&1`** — written to the redirected stderr and
  reported as exit 1 (previously leaked to the terminal).
- **`kill` builtin** — `kill [-SIGNAL] <pid>|<%job>` signals a job's whole
  process group.

### Added (from earlier 0.3.0 work)
- **Move-to-background + `⏳N` background-task indicator** (Ctrl+Z → `bg`).
- **Background jobs no longer block the shell** (`reap_jobs` no longer spins).

## [0.2.0] — 2026-08-06 — Battle-tested for programming

This release fixes several bugs that bit real programming workflows, adds
FISH-style tab completion and job-control upgrades, and is the first
"battle-tested" release intended for everyday development use.

### Fixed — things getting swallowed
- **Glob expansion** (`*`, `?`, `[...]`, `**`) now works: `ls *.rs`, `rm *.tmp`,
  `git add *.py`. Previously `*` was passed literally to commands, so globbing
  silently failed. No-match globs stay literal (POSIX). (`src/wordexp.rs`)
- **Builtins now honor redirections.** `echo "x" > file` previously wrote to
  the terminal and never created the file (builtins ran in-process before the
  redirection code). Now `>`, `>>`, `2>`, `2>&1` work for builtins exactly like
  external commands. (`src/executor/mod.rs` — `run_builtin_with_redirections`)
- **Numeric arguments are no longer swallowed as fd numbers.** `seq 1 3 > out.txt`
  was parsed as `seq 1` + `3>out.txt` (fd-3 redirect), silently eating the `3`.
  Now, matching bash, only an *adjacent* `3>file` is an fd redirect; `3 > file`
  keeps `3` as an argument. (`src/parser/mod.rs` — `is_fd_prefixed_redirection`)
- **`jobs` shows readable commands**, not debug structs (`Simple(SimpleCommand{...})`
  → `sleep 5`). Added `render()` to the AST. (`src/parser/ast.rs`)

### Added
- **FISH-style tab completion**: the candidate list now appears on the **first**
  Tab (was bash-style double-Tab). (`vendor/rustyline` patch)
- **`disown [N]`** — stop tracking a background job (no zombies).
- **`wait [N]`** — block until a background job (or all) finishes; returns its
  exit status.
- **`jobs`** now shows index, `+`/`-` marker, state, PID and command.
- **git completion**: `git che<Tab>` → subcommands; `git checkout <Tab>` →
  branch names (local + remote).

### Fixed (previous)
- Live prompt flicker (vendored rustyline renders atomically).
- Live prompt auto-disabled in VS Code's integrated terminal.
- Login-shell support: sources `/etc/profile` + `~/.profile`, `startup_commands`.
- Safe login (5s profile timeout, stdin from /dev/null).

[0.2.0]: https://xytro.site/waash
