//! WAASH REPL — the interactive shell experience.
//!
//! Built on `rustyline` for line editing with:
//! - FISH-style autosuggestions from history
//! - Syntax highlighting (commands, strings, variables, operators)
//! - Tab completion (commands, files, builtins)
//! - Heredoc continuation prompts

mod completer;
mod highlighter;
mod hinter;
mod prompt;
mod welcome;
mod bashblock;

use rustyline::error::ReadlineError;
use rustyline::ExternalPrinter as _;
use rustyline::{At, Cmd, Config, EditMode, Editor, Event, KeyCode, KeyEvent, Modifiers, Movement, Word};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU64, Ordering};

use crate::config::{KeyBinding, WaashConfig};
use crate::executor::Executor;
use crate::lexer::Scanner;
use crate::parser::Parser;

use completer::WaashCompleter;
use highlighter::WaashHighlighter;
use hinter::WaashHinter;
use prompt::{PromptStyle, WaashHelper};

/// Parse a config key string (e.g. `Ctrl+R`, `Alt+Left`, `Up`, `Ctrl+Shift+F`)
/// into a rustyline `KeyEvent`. Returns `None` if unrecognized.
fn parse_key_event(s: &str) -> Option<KeyEvent> {
    let mut mods = Modifiers::NONE;
    let mut name = s;
    loop {
        if let Some(rest) = name
            .strip_prefix("Ctrl+")
            .or_else(|| name.strip_prefix("C-"))
        {
            mods |= Modifiers::CTRL;
            name = rest;
        } else if let Some(rest) = name
            .strip_prefix("Alt+")
            .or_else(|| name.strip_prefix("M-"))
        {
            mods |= Modifiers::ALT;
            name = rest;
        } else if let Some(rest) = name.strip_prefix("Shift+") {
            mods |= Modifiers::SHIFT;
            name = rest;
        } else {
            break;
        }
    }
    let code = match name {
        "Up" => KeyCode::Up,
        "Down" => KeyCode::Down,
        "Left" => KeyCode::Left,
        "Right" => KeyCode::Right,
        "Home" => KeyCode::Home,
        "End" => KeyCode::End,
        "Tab" => KeyCode::Tab,
        "Enter" => KeyCode::Enter,
        "Backspace" | "BS" => KeyCode::Backspace,
        "Delete" | "Del" => KeyCode::Delete,
        "Esc" | "Escape" => KeyCode::Esc,
        "PageUp" => KeyCode::PageUp,
        "PageDown" => KeyCode::PageDown,
        _ => {
            let mut chars = name.chars();
            let c = chars.next()?;
            if chars.next().is_some() {
                return None; // multi-character unknown key name
            }
            KeyCode::Char(c)
        }
    };
    Some(KeyEvent(code, mods))
}

/// Map a config action name to a rustyline `Cmd`. Returns `None` if unknown.
fn action_cmd(action: &str) -> Option<Cmd> {
    Some(match action {
        "history_search_backward" => Cmd::HistorySearchBackward,
        "history_search_forward" => Cmd::HistorySearchForward,
        "reverse_search_history" => Cmd::ReverseSearchHistory,
        "forward_search_history" => Cmd::ForwardSearchHistory,
        "beginning_of_line" => Cmd::Move(Movement::BeginningOfLine),
        "end_of_line" => Cmd::Move(Movement::EndOfLine),
        "forward_char" => Cmd::Move(Movement::ForwardChar(1)),
        "backward_char" => Cmd::Move(Movement::BackwardChar(1)),
        "forward_word" => Cmd::Move(Movement::ForwardWord(1, At::AfterEnd, Word::Big)),
        "backward_word" => Cmd::Move(Movement::BackwardWord(1, Word::Big)),
        "previous_history" => Cmd::PreviousHistory,
        "next_history" => Cmd::NextHistory,
        "beginning_of_history" => Cmd::BeginningOfHistory,
        "end_of_history" => Cmd::EndOfHistory,
        "clear_screen" => Cmd::ClearScreen,
        "accept_line" => Cmd::AcceptLine,
        "undo" => Cmd::Undo(1),
        "complete" => Cmd::Complete,
        "interrupt" => Cmd::Interrupt,
        "kill_line" => Cmd::Kill(Movement::WholeLine),
        "transpose_chars" => Cmd::TransposeChars,
        "revert_line" => Cmd::Kill(Movement::WholeLine),
        "abort" => Cmd::Abort,
        _ => return None,
    })
}

/// Apply the user's configured custom key bindings to the editor.
fn apply_keybindings(
    editor: &mut Editor<WaashHelper, rustyline::history::DefaultHistory>,
    keybindings: &[KeyBinding],
) {
    for kb in keybindings {
        match (parse_key_event(&kb.key), action_cmd(&kb.action)) {
            (Some(ev), Some(cmd)) => {
                editor.bind_sequence(Event::from(ev), cmd);
            }
            _ => {
                eprintln!(
                    "waash: ignoring unknown keybinding '{}' -> '{}'",
                    kb.key, kb.action
                );
            }
        }
    }
}

/// Whether the current terminal supports WAASH's live-refresh prompt.
///
/// The live prompt injects escape sequences into the output every second.
/// VS Code's integrated terminal (xterm.js + shell integration) chokes on
/// these: the terminal can appear to hang and shell-integration/agent output
/// parsing breaks. We detect it via `TERM_PROGRAM` and fall back to the
/// normal per-command prompt there.
fn live_refresh_supported() -> bool {
    live_refresh_supported_for(std::env::var("TERM_PROGRAM").ok())
}

/// Pure version of [`live_refresh_supported`], testable without touching env.
fn live_refresh_supported_for(term_program: Option<String>) -> bool {
    match term_program.as_deref() {
        Some(p) if p.eq_ignore_ascii_case("vscode") || p.eq_ignore_ascii_case("code-oss") => false,
        _ => true,
    }
}

/// The main WAASH REPL.
pub struct WaashRepl {
    editor: Editor<WaashHelper, rustyline::history::DefaultHistory>,
    executor: Executor,
    history_file: PathBuf,
    /// User configuration
    config: WaashConfig,
    /// Current prompt style
    prompt_style: PromptStyle,
    /// Last exit code for prompt display
    last_exit: i32,
    /// Duration of last command execution (for {duration} in prompt)
    last_duration: std::time::Duration,
    /// Skip the welcome banner
    no_banner: bool,
    /// Shared with the helper so the prompt can rebuild live (exit code).
    live_exit: Arc<AtomicI32>,
    /// Shared with the helper so the prompt can rebuild live (duration in ns).
    live_duration_ns: Arc<AtomicU64>,
    /// Stop flag for the live-refresh ticker thread.
    live_stop: Arc<AtomicBool>,
    /// True while rustyline is waiting at the prompt (ticker only fires then).
    live_active: Arc<AtomicBool>,
    /// Monotonic ns timestamp of the last keystroke. The ticker skips while
    /// a keystroke happened within a cooldown, so the injected repaint can't
    /// race a typed character / Enter.
    last_keystroke: Arc<AtomicU64>,
}

impl WaashRepl {
    /// Create a new REPL with user configuration.
    pub fn with_config(config: WaashConfig, no_banner: bool) -> anyhow::Result<Self> {
        let mut builder = Config::builder()
            .auto_add_history(true)
            .history_ignore_space(true);
        builder = builder.history_ignore_dups(true)
            .map_err(|e| anyhow::anyhow!("config: {:?}", e))?;
        builder = builder.max_history_size(config.shell.history_size)
            .map_err(|e| anyhow::anyhow!("config: {:?}", e))?;
        let rusty_config = builder
            .completion_type(rustyline::CompletionType::List)
            .edit_mode(match config.shell.edit_mode.as_str() {
                "vi" => EditMode::Vi,
                _ => EditMode::Emacs,
            })
            .build();

        let mut editor = Editor::with_config(rusty_config)?;

        // Apply the user's custom key bindings from the config.
        apply_keybindings(&mut editor, &config.keybindings);

        let highlight_theme = config.theme.clone();
        let hinter_enabled = config.shell.autosuggestions;

        // History file
        let config_dir = dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("waash");
        let history_file = config_dir.join(&config.shell.history_file);

        // Executor configured from user config: aliases + history file.
        let mut executor = Executor::new();
        executor.load_config_aliases(&config.aliases);
        executor.set_history_file(history_file.clone());
        executor.set_job_control(config.shell.job_control);
        executor.set_bg_shortcut(&config.shell.bg_shortcut);
        executor.set_bg_hint(config.shell.bg_hint);
        executor.set_auto_bg_seconds(config.shell.auto_bg_seconds);

        // The prompt shares the executor's live background-job counter so the
        // `{jobs}` indicator reflects how many tasks are running/stopped.
        let mut prompt_style = PromptStyle::from_config(&config.prompt);
        prompt_style.set_background_jobs(executor.background_jobs.clone());

        // Shared live-refresh state (the helper rebuilds the prompt on render).
        let live_exit = Arc::new(AtomicI32::new(0));
        let live_duration_ns = Arc::new(AtomicU64::new(0));
        let last_keystroke = Arc::new(AtomicU64::new(0));

        let helper = WaashHelper {
            completer: WaashCompleter::new(),
            highlighter: WaashHighlighter::with_theme(highlight_theme),
            hinter: WaashHinter::with_enabled(hinter_enabled),
            prompt_style: prompt_style.clone(),
            last_exit: live_exit.clone(),
            last_duration_ns: live_duration_ns.clone(),
            last_keystroke: last_keystroke.clone(),
        };

        editor.set_helper(Some(helper));

        // Load history
        if history_file.exists() {
            let _ = editor.load_history(&history_file);
        }

        Ok(Self {
            editor,
            executor,
            history_file,
            config,
            prompt_style,
            last_exit: 0,
            last_duration: std::time::Duration::ZERO,
            no_banner,
            live_exit,
            live_duration_ns,
            live_stop: Arc::new(AtomicBool::new(false)),
            live_active: Arc::new(AtomicBool::new(false)),
            last_keystroke,
        })
    }

    /// Run the REPL loop.
    pub fn run(&mut self) -> anyhow::Result<()> {
        // Show the neofetch-style welcome screen unless disabled via -q/--no-banner
        // or `show_banner = false` in the config.
        if !self.no_banner && self.config.shell.show_banner {
            self.print_welcome();
        }

        // Run the user's configured startup commands (acts like a ~/.waashrc):
        // aliases/config are already loaded, so these can alias/export/source.
        let startup_commands: Vec<String> = self.config.shell.startup_commands.clone();
        for cmd in &startup_commands {
            let _ = self.parse_and_execute(cmd);
        }

        // Live-refresh ticker: while we're waiting at the prompt, nudge rustyline
        // to re-render every second so the time / CPU / sudo badge stay live.
        // The message "\x1b[1A" (cursor up 1) offsets external_print()'s forced
        // trailing newline, so the rebuilt prompt stays in the same place.
        //
        // This is auto-disabled in terminals that can't handle the injected
        // escape sequences (notably VS Code's integrated terminal — it makes
        // the terminal appear to hang and breaks shell-integration/agent
        // output parsing).
        let live_stop = self.live_stop.clone();
        let live_active = self.live_active.clone();
        let last_keystroke = self.last_keystroke.clone();
        let bg_jobs = self.executor.background_jobs.clone();
        let ticker = if self.config.shell.live_refresh && live_refresh_supported() {
            self.editor
                .create_external_printer()
                .ok()
                .map(|mut printer| {
                    std::thread::spawn(move || {
                        // Cooldown: don't fire within this many ns of any
                        // keystroke, so the repaint can't race an Enter.
                        const COOLDOWN_NS: u64 = 400_000_000; // 400ms
                        while !live_stop.load(Ordering::Relaxed) {
                            std::thread::sleep(std::time::Duration::from_millis(250));
                            if live_active.load(Ordering::Relaxed)
                                && !live_stop.load(Ordering::Relaxed)
                                && bg_jobs.load(Ordering::Relaxed) == 0
                            {
                                let last = last_keystroke.load(Ordering::Relaxed);
                                let now = std::time::SystemTime::now()
                                    .duration_since(std::time::UNIX_EPOCH)
                                    .map(|d| d.as_nanos() as u64)
                                    .unwrap_or(0);
                                if now.saturating_sub(last) >= COOLDOWN_NS {
                                    let _ = printer.print("\u{1b}[1A".to_string());
                                }
                            }
                        }
                    })
                })
        } else {
            None
        };

        loop {
            // Reap finished background jobs to avoid zombies. This may also
            // queue "[N] ... exited" notices.
            self.executor.reap_jobs();

            // Print any queued job-control notifications NOW — just before the
            // prompt is drawn — rather than mid-edit. Writing them to stdout
            // while rustyline owns the terminal desyncs its cursor state and
            // swallows the next Enter keystroke. Printing here lets rustyline
            // paint a fresh prompt below them.
            for line in self.executor.take_notifications() {
                println!("{}", line);
            }

            let prompt = self.build_prompt();

            self.live_active.store(true, Ordering::Relaxed);
            let readline = self.editor.readline(&prompt);
            self.live_active.store(false, Ordering::Relaxed);

            match readline {
                Ok(line) => {
                    let line = line.trim().to_string();
                    if line.is_empty() {
                        continue;
                    }

                    // Time the command execution
                    let start = std::time::Instant::now();

                    // If the line starts a multi-line bash control-flow block,
                    // collect it and run via bash (hybrid). Otherwise parse
                    // and execute normally.
                    let status = match self.try_bash_block(&line) {
                        Some(status) => status,
                        None => {
                            match self.parse_and_execute(&line) {
                                Ok(status) => status,
                                Err(e) => {
                                    eprintln!("waash: {}", e);
                                    crate::executor::ExitStatus::Code(1)
                                }
                            }
                        }
                    };

                    self.last_exit = status.code();
                    // `exit` builtin → leave the shell
                    if let crate::executor::ExitStatus::Exit(_) = status {
                        break;
                    }

                    self.last_duration = start.elapsed();
                }
                Err(ReadlineError::Interrupted) => {
                    println!("^C");
                    self.last_exit = 130;
                }
                Err(ReadlineError::Eof) => {
                    println!("exit");
                    break;
                }
                Err(err) => {
                    eprintln!("waash: readline error: {:?}", err);
                    break;
                }
            }

            // Publish the latest result for the live-refresh prompt.
            self.live_exit.store(self.last_exit, Ordering::Relaxed);
            self.live_duration_ns
                .store(self.last_duration.as_nanos() as u64, Ordering::Relaxed);
        }

        // Shut the ticker down and let the thread finish.
        self.live_stop.store(true, Ordering::Relaxed);
        if let Some(handle) = ticker {
            let _ = handle.join();
        }

        if let Some(parent) = self.history_file.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = self.editor.save_history(&self.history_file);

        Ok(())
    }

    fn print_welcome(&self) {
        // Neofetch-style startup screen with the Xytro logo + system info.
        welcome::print_welcome_screen(env!("CARGO_PKG_VERSION"), self.config.aliases.len());
    }

    fn build_prompt(&mut self) -> String {
        self.prompt_style.format_prompt(self.last_exit, self.last_duration)
    }

    fn parse_and_execute(&mut self, input: &str) -> Result<crate::executor::ExitStatus, String> {
        // Expand `{a,b}` / `{1..3}` before lexing (per-word, bash semantics).
        let input = crate::wordexp::expand_braces_line(input);
        let mut scanner = Scanner::new(input);
        scanner.lex_all().map_err(|e| format!("lexer error: {}", e))?;

        let mut parser = Parser::new(scanner);
        let mut script = parser.parse().map_err(|e| format!("parse error: {}", e))?;

        // Fill any pending heredoc bodies by reading continuation lines
        // from the editor. This is safe: if there are no heredocs it's a no-op.
        let cont_prompt = self.prompt_style.heredoc_prompt();
        let editor = &mut self.editor;
        crate::heredoc::fill_heredoc_bodies(&mut script, &mut |_delim| {
            editor.readline(&cont_prompt).ok()
        });

        let no_lines: Vec<String> = Vec::new();
        self.executor.execute(&script, &mut no_lines.into_iter())
    }

    /// If `first` starts a multi-line bash control-flow block (if/for/while/
    /// case/function/[[ ]) that isn't complete on one line, collect the rest
    /// of the block from continuation prompts and run the whole thing through
    /// `bash -c`. Returns `None` if the line is NOT a bash block (caller
    /// should parse it normally), `Some(code)` if a bash block was collected
    /// and executed.
    ///
    /// On Ctrl+C during collection the block is aborted and `Some(130)` is
    /// returned so the caller records the interrupted status.
    fn try_bash_block(&mut self, first: &str) -> Option<crate::executor::ExitStatus> {
        use bashblock::{bash_block_done, starts_bash_block};

        if !starts_bash_block(first) {
            return None;
        }

        let mut block: Vec<String> = vec![first.to_string()];

        // A single self-contained line (e.g. `if x; then y; fi`) is already
        // complete — run it directly.
        if bash_block_done(&block) {
            return Some(run_bash(&block.join("\n")));
        }

        let cont_prompt = self.prompt_style.heredoc_prompt();
        loop {
            match self.editor.readline(&cont_prompt) {
                Ok(line) => {
                    let line = line.trim().to_string();
                    if line.is_empty() {
                        continue;
                    }
                    block.push(line);
                    if bash_block_done(&block) {
                        return Some(run_bash(&block.join("\n")));
                    }
                }
                Err(ReadlineError::Interrupted) | Err(ReadlineError::Eof) => {
                    // Abort the block.
                    self.last_exit = 130;
                    return Some(crate::executor::ExitStatus::Code(130));
                }
                Err(err) => {
                    eprintln!("waash: readline error: {:?}", err);
                    return Some(crate::executor::ExitStatus::Code(1));
                }
            }
        }
    }
}

/// Run a block of shell text through `bash -c`, inheriting stdio, and return
/// its exit status.
fn run_bash(code: &str) -> crate::executor::ExitStatus {
    match std::process::Command::new("bash").arg("-c").arg(code).status() {
        Ok(status) => crate::executor::ExitStatus::Code(status.code().unwrap_or(1)),
        Err(e) => {
            eprintln!("waash: failed to run bash: {}", e);
            crate::executor::ExitStatus::Code(1)
        }
    }
}

impl Drop for WaashRepl {
    fn drop(&mut self) {
        if let Some(parent) = self.history_file.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = self.editor.save_history(&self.history_file);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_key_event() {
        // Ctrl+letter.
        let e = parse_key_event("Ctrl+R").unwrap();
        assert_eq!(e.0, KeyCode::Char('R'));
        assert!(e.1.contains(Modifiers::CTRL));
        // Named keys with modifier.
        let e = parse_key_event("Alt+Left").unwrap();
        assert_eq!(e.0, KeyCode::Left);
        assert!(e.1.contains(Modifiers::ALT));
        // Bare named key.
        let e = parse_key_event("Up").unwrap();
        assert_eq!(e.0, KeyCode::Up);
        // Unknown -> None.
        assert!(parse_key_event("NotAKey").is_none());
    }

    #[test]
    fn test_live_refresh_disabled_in_vscode() {
        assert!(!live_refresh_supported_for(Some("vscode".into())));
        assert!(!live_refresh_supported_for(Some("code-oss".into())));
        assert!(!live_refresh_supported_for(Some("VSCODE".into())));
    }

    #[test]
    fn test_live_refresh_enabled_elsewhere() {
        assert!(live_refresh_supported_for(Some("kitty".into())));
        assert!(live_refresh_supported_for(Some("gnome-terminal".into())));
        assert!(live_refresh_supported_for(Some("tmux".into())));
        assert!(live_refresh_supported_for(None));
    }
}

