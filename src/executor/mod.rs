//! Command executor for WAASH.
//!
//! Executes parsed AST commands: forks child processes, sets up pipes,
//! applies redirections, handles heredocs, manages job control.

use nix::errno::Errno;
use nix::fcntl;
use nix::sys::signal::{self, SaFlags, SigAction, SigHandler, SigSet, Signal};
use nix::sys::wait::{waitpid, WaitPidFlag, WaitStatus};
use nix::unistd::{self, dup, dup2, fork, pipe, setpgid, ForkResult, Pid};
use std::collections::HashMap;
use std::env;
use std::ffi::CString;
use std::io::{self, Write};
use std::os::unix::io::AsRawFd;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use crate::builtins;
use crate::parser::ast::*;

/// Exit status of a command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExitStatus {
    Code(i32),
    Exit(i32),
    Signal(i32),
}

impl ExitStatus {
    pub fn is_success(&self) -> bool {
        matches!(self, ExitStatus::Code(0))
    }

    pub fn code(&self) -> i32 {
        match self {
            ExitStatus::Code(c) => *c,
            ExitStatus::Exit(c) => *c,
            ExitStatus::Signal(s) => 128 + s,
        }
    }
}

/// The executor manages job state and runs command trees.
pub struct Executor {
    /// Last exit code ($?)
    last_exit: i32,
    /// Background jobs
    jobs: Vec<Job>,
    /// Directory stack for `pushd`/`popd` (entries below the current dir).
    dir_stack: Vec<String>,
    /// Aliases
    aliases: HashMap<String, String>,
    /// History file used by the `history` builtin
    history_file: PathBuf,
    /// Job-control notifications (e.g. "[1] 1234" on background launch, or
    /// "[2] foo exited with status 1"). Pushed here instead of writing
    /// directly to stdout, because printing mid-edit corrupts rustyline's
    /// terminal state and causes subsequent Enter presses to be swallowed.
    /// The REPL drains these at a safe point (just before drawing the next
    /// prompt) and prints them there.
    pub notifications: Arc<Mutex<Vec<String>>>,
    /// Number of tracked background jobs. The live-refresh ticker in the REPL
    /// checks this and suppresses itself while >0, because the ticker's
    /// injected repaint races with the Enter that submits the next command
    /// and swallows it — exactly the failure users hit when backgrounding
    /// many jobs at once. With jobs active we stop repainting so input is
    /// never dropped.
    pub background_jobs: Arc<AtomicUsize>,
    /// Enable Ctrl+Z job control for foreground commands (per-command process
    /// group + terminal handoff) so a running task can be suspended and moved
    /// to the background. Off keeps the old always-foreground behavior.
    job_control: bool,
    /// True while running inside a background (`&`) child. Such children must
    /// NOT hand the terminal to their own children or act as a foreground job
    /// — they run detached from the terminal.
    in_background_child: bool,
    /// Terminal suspend-char byte used as the single-key "move to background"
    /// shortcut. Default Ctrl+B (0x02); configurable via `[shell] bg_shortcut`.
    bg_key: u8,
}

#[derive(Debug, Clone)]
struct Job {
    pid: Pid,
    /// Process group id. For a single command this equals `pid`; for a
    /// background *pipeline* all children share this pgid. Signals for the
    /// whole job (fg/bg/disown) must target the *pgid*, not just `pid`,
    /// otherwise sibling pipeline members go unmanaged.
    pgid: Pid,
    command: String,
    state: JobState,
    /// Exit code once the job has finished (used by `wait`).
    code: Option<i32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum JobState {
    Running,
    Stopped,
    Done,
}

/// Configure the shell process's signal handling.
///
/// The shell ignores SIGINT/SIGQUIT so that pressing Ctrl+C while a
/// foreground child is running kills the CHILD but not the shell.
/// It also ignores the job-control stop signals (SIGTSTP/SIGTTIN/SIGTTOU)
/// so that Ctrl+Z stops a foreground CHILD (which resets them to default)
/// rather than stopping the shell itself.
/// Child processes reset these to default before exec.
pub fn setup_shell_signals() {
    unsafe {
        let ignore = SigAction::new(SigHandler::SigIgn, SaFlags::empty(), SigSet::empty());
        for sig in [Signal::SIGINT, Signal::SIGQUIT, Signal::SIGTSTP, Signal::SIGTTIN, Signal::SIGTTOU] {
            let _ = signal::sigaction(sig, &ignore);
        }
    }
}

/// Reset signal dispositions to default in a child before exec.
fn reset_child_signals() {
    unsafe {
        let dfl = SigAction::new(SigHandler::SigDfl, SaFlags::empty(), SigSet::empty());
        for sig in [
            Signal::SIGINT,
            Signal::SIGQUIT,
            Signal::SIGTERM,
            Signal::SIGHUP,
            Signal::SIGPIPE,
            Signal::SIGTSTP,
            Signal::SIGTTIN,
            Signal::SIGTTOU,
        ] {
            let _ = signal::sigaction(sig, &dfl);
        }
    }
}

/// Wait for a child, retrying on EINTR (e.g. from SIGWINCH on resize).
/// WUNTRACED reports a Ctrl+Z stop (`WaitStatus::Stopped`) so the caller can
/// move the job to the background instead of treating it as an exit.
fn wait_for_child(pid: Pid) -> Result<WaitStatus, String> {
    loop {
        match waitpid(pid, Some(WaitPidFlag::WUNTRACED)) {
            Ok(status) => return Ok(status),
            Err(Errno::EINTR) => continue,
            Err(e) => return Err(format!("waitpid: {}", e)),
        }
    }
}

/// Open a file for writing (truncating), returning the raw fd.
fn open_output(path: &str, _append: bool) -> Result<i32, String> {
    fcntl::open(
        path,
        fcntl::OFlag::O_WRONLY | fcntl::OFlag::O_CREAT | fcntl::OFlag::O_TRUNC,
        nix::sys::stat::Mode::S_IRUSR
            | nix::sys::stat::Mode::S_IWUSR
            | nix::sys::stat::Mode::S_IRGRP
            | nix::sys::stat::Mode::S_IROTH,
    )
    .map_err(|e| format!("open {}: {}", path, e))
}

/// Open a file for appending, returning the raw fd.
fn open_append(path: &str) -> Result<i32, String> {
    fcntl::open(
        path,
        fcntl::OFlag::O_WRONLY | fcntl::OFlag::O_CREAT | fcntl::OFlag::O_APPEND,
        nix::sys::stat::Mode::S_IRUSR
            | nix::sys::stat::Mode::S_IWUSR
            | nix::sys::stat::Mode::S_IRGRP
            | nix::sys::stat::Mode::S_IROTH,
    )
    .map_err(|e| format!("open {}: {}", path, e))
}

/// Resolve a signal name (with or without a `SIG` prefix) to a `Signal`.
/// Used by the `kill` builtin for `-TERM`, `-SIGKILL`, `-s USR1`, etc.
fn signal_from_name(name: &str) -> Option<Signal> {
    let up = name.to_uppercase();
    let up = up.strip_prefix("SIG").unwrap_or(&up);
    Some(match up {
        "HUP" => Signal::SIGHUP,
        "INT" => Signal::SIGINT,
        "QUIT" => Signal::SIGQUIT,
        "KILL" => Signal::SIGKILL,
        "TERM" => Signal::SIGTERM,
        "CONT" => Signal::SIGCONT,
        "STOP" => Signal::SIGSTOP,
        "TSTP" => Signal::SIGTSTP,
        "USR1" => Signal::SIGUSR1,
        "USR2" => Signal::SIGUSR2,
        "PIPE" => Signal::SIGPIPE,
        "ALRM" => Signal::SIGALRM,
        "CHLD" => Signal::SIGCHLD,
        "WINCH" => Signal::SIGWINCH,
        "ABRT" => Signal::SIGABRT,
        "SEGV" => Signal::SIGSEGV,
        "FPE" => Signal::SIGFPE,
        "ILL" => Signal::SIGILL,
        "BUS" => Signal::SIGBUS,
        _ => return None,
    })
}

/// The terminal's original suspend character, remembered so we can restore it
/// after temporarily remapping it for the "move to background" shortcut.
static ORIG_SUSP: Mutex<Option<u8>> = Mutex::new(None);

impl Executor {
    pub fn new() -> Self {
        let history_file = dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("waash")
            .join("history");
        Self {
            last_exit: 0,
            jobs: Vec::new(),
            dir_stack: Vec::new(),
            aliases: HashMap::new(),
            history_file,
            notifications: Arc::new(Mutex::new(Vec::new())),
            background_jobs: Arc::new(AtomicUsize::new(0)),
            job_control: true,
            in_background_child: false,
            bg_key: 0x02, // Ctrl+B
        }
    }

    /// Set the single-key "move to background" shortcut. Accepts `Ctrl+X`
    /// (maps to the control byte); anything else falls back to Ctrl+B.
    pub fn set_bg_shortcut(&mut self, s: &str) {
        if let Some(ch) = s.strip_prefix("Ctrl+") {
            if let Some(c) = ch.chars().next() {
                let lower = c.to_ascii_lowercase();
                if ('a'..='z').contains(&lower) {
                    self.bg_key = (lower as u8) - b'a' + 1;
                    return;
                }
            }
        }
        self.bg_key = 0x02;
    }

    /// Set the terminal's suspend character (VSUSP) to `byte`, so a foreground
    /// child with the terminal sends SIGTSTP when that key is pressed. Only
    /// applies when stdin is a tty; the original char is remembered for
    /// [`Self::restore_susp_char`].
    fn set_susp_char(&self, byte: u8) {
        use nix::sys::termios::{SetArg, SpecialCharacterIndices, tcgetattr, tcsetattr};
        if !nix::unistd::isatty(io::stdin().as_raw_fd()).unwrap_or(false) {
            return;
        }
        let mut t = match tcgetattr(&io::stdin()) {
            Ok(t) => t,
            Err(_) => return,
        };
        {
            let mut guard = ORIG_SUSP.lock().unwrap();
            if guard.is_none() {
                *guard = Some(t.control_chars[SpecialCharacterIndices::VSUSP as usize]);
            }
        }
        t.control_chars[SpecialCharacterIndices::VSUSP as usize] = byte;
        let _ = tcsetattr(&io::stdin(), SetArg::TCSANOW, &t);
    }

    /// Restore the terminal's original suspend character.
    fn restore_susp_char(&self) {
        let orig = *ORIG_SUSP.lock().unwrap();
        if let Some(orig) = orig {
            self.set_susp_char(orig);
        }
    }

    /// Enable/disable Ctrl+Z job control (see the `job_control` field).
    pub fn set_job_control(&mut self, enabled: bool) {
        self.job_control = enabled;
    }

    /// Take and return any pending job-control notifications, clearing the
    /// queue. The REPL calls this at a safe point (before drawing the next
    /// prompt) and prints the lines — never print these mid-edit.
    pub fn take_notifications(&self) -> Vec<String> {
        let mut q = self.notifications.lock().unwrap();
        std::mem::take(&mut *q)
    }

    fn notify(&self, line: String) {
        self.notifications.lock().unwrap().push(line);
    }

    /// Remove the job at `idx`. Decrements the active-background-job counter
    /// only if the job wasn't already Done (a Done job already decremented it
    /// in [`Self::mark_job_done`]) — otherwise the counter would double-count.
    fn remove_job(&mut self, idx: usize) {
        let was_active = self.jobs[idx].state != JobState::Done;
        self.jobs.remove(idx);
        if was_active {
            self.background_jobs.fetch_sub(1, Ordering::Relaxed);
        }
    }

    /// Load aliases from the user config so they apply to every command.
    pub fn load_config_aliases(&mut self, entries: &[crate::config::AliasEntry]) {
        for entry in entries {
            self.aliases.insert(entry.name.clone(), entry.value.clone());
        }
    }

    /// Override the history file used by the `history` builtin.
    pub fn set_history_file(&mut self, path: PathBuf) {
        self.history_file = path;
    }

    /// Reap any finished background jobs (avoids zombie processes).
    pub fn reap_jobs(&mut self) {
        loop {
            match waitpid(Pid::from_raw(-1), Some(WaitPidFlag::WNOHANG)) {
                Ok(WaitStatus::Exited(pid, code)) => {
                    self.mark_job_done(pid, code);
                }
                Ok(WaitStatus::Signaled(pid, sig, _)) => {
                    self.mark_job_done(pid, 128 + sig as i32);
                }
                Ok(WaitStatus::Stopped(pid, _)) => {
                    if let Some(job) = self.jobs.iter_mut().find(|j| j.pid == pid) {
                        job.state = JobState::Stopped;
                    }
                }
                // StillAlive = a child is running but none have changed state.
                // That's NOT a reapable event — break out so the shell returns
                // to the prompt immediately instead of spinning here until the
                // background job finishes (which made `cmd &` block the shell).
                Ok(WaitStatus::StillAlive) => break,
                Ok(_) => continue,
                Err(Errno::EINTR) => continue,
                _ => break, // ECHILD or error — no more children
            }
        }
    }

    fn mark_job_done(&mut self, pid: Pid, code: i32) {
        // Find the index first (immutable), then update the job
        let idx = self.jobs.iter().position(|j| j.pid == pid);
        if let Some(i) = idx {
            let command = self.jobs[i].command.clone();
            // A job leaving Running/Stopped is no longer an *active* background
            // task, so the prompt indicator and ticker gate reflect reality.
            let was_active = self.jobs[i].state != JobState::Done;
            self.jobs[i].state = JobState::Done;
            self.jobs[i].code = Some(code);
            if was_active {
                self.background_jobs.fetch_sub(1, Ordering::Relaxed);
            }
            self.notify(format!("[{}] {} exited with status {}", i + 1, command, code));
        }
    }

    /// Execute a parsed script, returning the exit status of the last command.
    pub fn execute(&mut self, script: &Script, heredoc_inputs: &mut dyn Iterator<Item = String>) -> Result<ExitStatus, String> {
        let mut last_status = ExitStatus::Code(0);

        for cmd in &script.commands {
            last_status = self.execute_command(cmd, heredoc_inputs)?;

            // Handle Exit status (from `exit` builtin)
            if let ExitStatus::Exit(code) = last_status {
                return Ok(ExitStatus::Exit(code));
            }
        }

        self.last_exit = last_status.code();
        Ok(last_status)
    }

    fn execute_command(
        &mut self,
        cmd: &Command,
        heredoc_inputs: &mut dyn Iterator<Item = String>,
    ) -> Result<ExitStatus, String> {
        match cmd {
            Command::Simple(sc) => self.execute_simple(sc, heredoc_inputs),
            Command::Pipeline(p) => self.execute_pipeline(p, heredoc_inputs),
            Command::And(left, right) => {
                let status = self.execute_command(left, heredoc_inputs)?;
                if status.is_success() {
                    self.execute_command(right, heredoc_inputs)
                } else {
                    Ok(status)
                }
            }
            Command::Or(left, right) => {
                let status = self.execute_command(left, heredoc_inputs)?;
                if !status.is_success() {
                    self.execute_command(right, heredoc_inputs)
                } else {
                    Ok(status)
                }
            }
            Command::Background(cmd) => self.execute_background(cmd, heredoc_inputs),
            Command::Subshell(script) => {
                // Fork and execute in child
                self.execute_script_in_child(script, heredoc_inputs)
            }
            Command::Group(script) => self.execute(script, heredoc_inputs),
            Command::Noop => Ok(ExitStatus::Code(0)),
        }
    }

    fn execute_simple(
        &mut self,
        cmd: &SimpleCommand,
        heredoc_inputs: &mut dyn Iterator<Item = String>,
    ) -> Result<ExitStatus, String> {
        // Resolve alias, splitting the expansion into program + prefix args
        // (e.g. `ll` → program "ls", prefix args ["-la"]).
        let resolved = self.resolve_alias(&cmd.program);
        let (program, prefix_args) = if resolved != cmd.program {
            let mut words = resolved.split_whitespace();
            let prog = words.next().unwrap_or("").to_string();
            let rest: Vec<String> = words.map(|s| s.to_string()).collect();
            (prog, rest)
        } else {
            (resolved, Vec::new())
        };
        let mut args = prefix_args;
        args.extend(cmd.args.iter().cloned());

        // Expand variables / special vars / command substitution NOW (at
        // execution time), so `export X=1; echo $X` and `false; echo $?`
        // reflect the current shell state. Also glob-expand `*`/`?`/`[...]`
        // so `ls *.rs` works.
        let program = self.expand_word(&program);
        let args = crate::wordexp::expand_argv(&args, self.last_exit);

        // Bare assignment: `FOO=bar` alone sets the variable in the shell.
        if program.is_empty() && !cmd.env_vars.is_empty() {
            for (name, value) in &cmd.env_vars {
                env::set_var(name, self.expand_word(value));
            }
            self.last_exit = 0;
            return Ok(ExitStatus::Code(0));
        }

        // Stateful builtins (need executor state: aliases, jobs, history).
        // Builtins run in-process, so apply redirections to the shell's own
        // fds temporarily (then restore) — otherwise `echo > file` would leak
        // to the terminal.
        if self.is_stateful_builtin(&program) {
            let status = self.run_builtin_with_redirections(&cmd.redirections, |s| {
                s.try_stateful_builtin(&program, &args)
                    .map(|opt| opt.unwrap_or(ExitStatus::Code(0)))
            })?;
            self.last_exit = status.code();
            return Ok(status);
        }

        // Check for builtins
        if builtins::is_builtin(&program) {
            let status = self.run_builtin_with_redirections(&cmd.redirections, |_s| {
                builtins::try_builtin(&program, &args)
                    .unwrap_or_else(|| Ok(ExitStatus::Code(1)))
            })?;
            self.last_exit = status.code();
            return Ok(status);
        }

        // Resolve heredoc body if needed
        let heredoc_body = if let Some(ref hd) = cmd.heredoc {
            if hd.body.is_empty() {
                // Read from input lines
                Some(crate::heredoc::read_heredoc_lines(
                    &hd.delimiter,
                    hd.strip_tabs,
                    heredoc_inputs,
                ))
            } else {
                Some(hd.body.clone())
            }
        } else {
            None
        };

        // Fork and exec
        match unsafe { fork() }.map_err(|e| format!("fork: {}", e))? {
            ForkResult::Parent { child } => {
                // Job control: run the child in its own process group and hand
                // it the terminal, so Ctrl+Z (SIGTSTP) stops the child — not
                // the shell — and we can move it to the background. We hand the
                // terminal back to the shell as soon as the child exits/stops.
                if self.job_control && !self.in_background_child {
                    let _ = setpgid(child, child);
                    let tty = io::stdin().as_raw_fd();
                    unsafe { nix::libc::tcsetpgrp(tty, child.as_raw()) };
                    // Remap the suspend key so Ctrl+B stops the child, which
                    // the shell then moves to the background.
                    self.set_susp_char(self.bg_key);

                    let waited = wait_for_child(child);

                    // Give the terminal back to the shell before touching the
                    // terminal again (rustyline redraws the next prompt).
                    unsafe { nix::libc::tcsetpgrp(tty, unistd::getpgrp().as_raw()) };
                    self.restore_susp_char();

                    match waited.map_err(|e| e)? {
                        WaitStatus::Exited(_, code) => {
                            self.last_exit = code;
                            Ok(ExitStatus::Code(code))
                        }
                        WaitStatus::Signaled(_, sig, _) => {
                            let code = 128 + sig as i32;
                            self.last_exit = code;
                            Ok(ExitStatus::Signal(sig as i32))
                        }
                        WaitStatus::Stopped(_, _) => {
                            // Move-to-background shortcut (Ctrl+B): continue
                            // the job in the background so it keeps running,
                            // and add it to the job table.
                            let _ = signal::killpg(child, Signal::SIGCONT);
                            let js = self.jobs.len() + 1;
                            let command = cmd.render();
                            self.jobs.push(Job {
                                pid: child,
                                pgid: child,
                                command: command.clone(),
                                state: JobState::Running,
                                code: None,
                            });
                            self.background_jobs.fetch_add(1, Ordering::Relaxed);
                            self.notify(format!("[{}] {} moved to background", js, command));
                            self.last_exit = 0;
                            Ok(ExitStatus::Code(0))
                        }
                        _ => Ok(ExitStatus::Code(1)),
                    }
                } else {
                    // Wait for child (retrying on EINTR)
                    match wait_for_child(child).map_err(|e| e)? {
                        WaitStatus::Exited(_, code) => {
                            self.last_exit = code;
                            Ok(ExitStatus::Code(code))
                        }
                        WaitStatus::Signaled(_, sig, _) => {
                            let code = 128 + sig as i32;
                            self.last_exit = code;
                            Ok(ExitStatus::Signal(sig as i32))
                        }
                        _ => Ok(ExitStatus::Code(1)),
                    }
                }
            }
            ForkResult::Child => {
                // Reset signals so the child dies on Ctrl+C (shell ignores them)
                reset_child_signals();

                // Join our own process group (mirror of the parent's setpgid,
                // avoids a race) so the child owns its job-control identity.
                if self.job_control && !self.in_background_child {
                    let _ = setpgid(Pid::from_raw(0), Pid::from_raw(0));
                }

                // Apply command-local env vars ONLY in the child
                // (prevents `FOO=bar cmd` from leaking into the shell)
                for (name, value) in &cmd.env_vars {
                    env::set_var(name, self.expand_word(value));
                }

                // Apply redirections
                self.apply_redirections(&cmd.redirections)?;

                // Handle heredoc: pipe it to stdin
                if let Some(body) = heredoc_body {
                    self.setup_heredoc_stdin(&body);
                }

                // Handle herestring
                if let Some(ref hs) = cmd.herestring {
                    self.setup_herestring_stdin(&self.expand_word(hs));
                }

                // Exec the command
                self.exec_program(&program, &args);
                // exec_program doesn't return on success
                std::process::exit(127);
            }
        }
    }

    fn execute_pipeline(
        &mut self,
        pipeline: &Pipeline,
        heredoc_inputs: &mut dyn Iterator<Item = String>,
    ) -> Result<ExitStatus, String> {
        let n = pipeline.commands.len();
        if n == 0 {
            return Ok(ExitStatus::Code(0));
        }
        if n == 1 {
            return self.execute_simple(&pipeline.commands[0], heredoc_inputs);
        }

        let mut pipes = Vec::new();
        let mut children = Vec::new();

        // Create n-1 pipes
        for _ in 0..n - 1 {
            let (r, w) = pipe().map_err(|e| format!("pipe: {}", e))?;
            pipes.push((r, w));
        }

        for (i, cmd) in pipeline.commands.iter().enumerate() {
            match unsafe { fork() }.map_err(|e| format!("fork: {}", e))? {
                ForkResult::Parent { child } => {
                    // Group all pipeline children into one process group so a
                    // background pipeline can be signaled/managed as a unit.
                    if i == 0 {
                        let _ = setpgid(child, child); // first child is leader
                    } else {
                        let _ = setpgid(child, children[0]); // join leader's group
                    }
                    children.push(child);
                }
                ForkResult::Child => {
                    // Reset signals so children die on Ctrl+C
                    reset_child_signals();

                    // Join the pipeline's process group (leader = first child).
                    // In the background case the intermediate shell already
                    // created the group; in the foreground case this is a no-op
                    // setpgid that avoids a race with the parent.
                    let leader = children.first().copied().unwrap_or(Pid::from_raw(0));
                    let target = if i == 0 { Pid::from_raw(0) } else { leader };
                    let _ = setpgid(Pid::from_raw(0), target);

                    // Setup pipe connections
                    if i > 0 {
                        // Connect stdin from previous pipe (read end)
                        let (r, _) = &pipes[i - 1];
                        dup2(r.as_raw_fd(), 0).map_err(|e| format!("dup2: {}", e))?;
                    }
                    if i < n - 1 {
                        // Connect stdout to next pipe (write end)
                        let (_, w) = &pipes[i];
                        dup2(w.as_raw_fd(), 1).map_err(|e| format!("dup2: {}", e))?;
                    }
                    if pipeline.pipe_stderr && i < n - 1 {
                        // Also pipe stderr
                        let (_, w) = &pipes[i];
                        dup2(w.as_raw_fd(), 2).map_err(|e| format!("dup2: {}", e))?;
                    }

                    // Close all pipe fds in child
                    drop(pipes);

                    // Apply command-local env vars ONLY in the child
                    for (name, value) in &cmd.env_vars {
                        env::set_var(name, self.expand_word(value));
                    }

                    // Apply redirections for this command
                    self.apply_redirections(&cmd.redirections)?;

                    // Resolve alias, expand, and check for builtins
                    let resolved = self.resolve_alias(&cmd.program);
                    let (program, prefix_args) = if resolved != cmd.program {
                        let mut words = resolved.split_whitespace();
                        let prog = words.next().unwrap_or("").to_string();
                        let rest: Vec<String> = words.map(|s| s.to_string()).collect();
                        (prog, rest)
                    } else {
                        (resolved, Vec::new())
                    };
                    let mut args = prefix_args;
                    args.extend(cmd.args.iter().cloned());
                    let program = self.expand_word(&program);
                    let args = crate::wordexp::expand_argv(&args, self.last_exit);

                    if let Some(result) = builtins::try_builtin(&program, &args) {
                        let status = result.unwrap_or(ExitStatus::Code(1));
                        std::process::exit(status.code());
                    }

                    self.exec_program(&program, &args);
                    std::process::exit(127);
                }
            }
        }

        // Close all pipe fds in parent
        drop(pipes);

        // Job control: hand the terminal to the pipeline's process group
        // (leader = first child) so Ctrl+Z stops the pipeline, not the shell.
        let tty = io::stdin().as_raw_fd();
        let pgid = children[0];
        let leader_pid = children[0];
        if self.job_control && !self.in_background_child {
            unsafe { nix::libc::tcsetpgrp(tty, pgid.as_raw()) };
            self.set_susp_char(self.bg_key);
        }

        // Wait for all children (retrying on EINTR)
        let mut last_status = ExitStatus::Code(0);
        let mut stopped = false;
        for child in children {
            match wait_for_child(child) {
                Ok(WaitStatus::Exited(_, code)) => {
                    last_status = ExitStatus::Code(code);
                }
                Ok(WaitStatus::Signaled(_, sig, _)) => {
                    last_status = ExitStatus::Signal(sig as i32);
                }
                Ok(WaitStatus::Stopped(_, _)) => {
                    stopped = true;
                    break; // a member was Ctrl+B'd — background the pipeline
                }
                Ok(_) => {}
                Err(_) => {}
            }
        }

        // Give the terminal back to the shell.
        if self.job_control && !self.in_background_child {
            unsafe { nix::libc::tcsetpgrp(tty, unistd::getpgrp().as_raw()) };
            self.restore_susp_char();
        }

        if stopped {
            let js = self.jobs.len() + 1;
            let sep = if pipeline.pipe_stderr { " |& " } else { " | " };
            let command = pipeline
                .commands
                .iter()
                .map(|c| c.render())
                .collect::<Vec<_>>()
                .join(sep);
            // Move-to-background shortcut (Ctrl+B): resume the pipeline in the
            // background so it keeps running.
            let _ = signal::killpg(pgid, Signal::SIGCONT);
            self.jobs.push(Job {
                pid: leader_pid,
                pgid,
                command: command.clone(),
                state: JobState::Running,
                code: None,
            });
            self.background_jobs.fetch_add(1, Ordering::Relaxed);
            self.notify(format!("[{}] {} moved to background", js, command));
            self.last_exit = 0;
            return Ok(ExitStatus::Code(0));
        }

        self.last_exit = last_status.code();
        Ok(last_status)
    }

    /// Launch a command (`cmd &`) as a background job.
    ///
    /// Proper job-control setup:
    ///   * the job gets its OWN process group (setpgid) so signals to it
    ///     (fg/bg/disown, Ctrl+C) hit the whole job and never the shell or
    ///     other jobs;
    ///   * the job's stdin is redirected from /dev/null so a background
    ///     process can't steal keystrokes from the terminal (bash does this);
    ///   * stdout/stderr stay attached to the terminal so the user still sees
    ///     background output.
    ///
    /// A background *pipeline* (`cmd1 | cmd2 &`) is forked once here as an
    /// intermediate shell child that then launches the pipeline children into
    /// the same process group, so `fg`/`bg` manage the entire pipeline.
    /// Launch a command (`cmd &`) as a background job.
    ///
    /// Proper job-control setup:
    ///   * the job gets its OWN process group (setpgid) so signals to it
    ///     (fg/bg/disown) hit the whole job and never the shell or other jobs;
    ///   * the job's stdin is redirected from /dev/null so a background
    ///     process can't steal keystrokes from the terminal (bash does this);
    ///   * stdout/stderr stay attached to the terminal so the user still sees
    ///     background output.
    ///
    /// The "[N] pid" launch notice is pushed onto [`Self::notifications`] and
    /// printed by the REPL just before the next prompt — never written directly
    /// to stdout, which would desync rustyline and swallow the next Enter.
    fn execute_background(
        &mut self,
        cmd: &Command,
        heredoc_inputs: &mut dyn Iterator<Item = String>,
    ) -> Result<ExitStatus, String> {
        let js = self.jobs.len() + 1;

        match unsafe { fork() }.map_err(|e| format!("fork: {}", e))? {
            ForkResult::Parent { child } => {
                // Parent: put the child into its own process group (it is the
                // group leader because its pid == the pgid we assign it).
                let _ = setpgid(child, child);
                // Record it for `$!` (most recently launched background job).
                crate::wordexp::set_last_background_pid(child.as_raw());
                let job = Job {
                    pid: child,
                    pgid: child,
                    command: cmd.render(),
                    state: JobState::Running,
                    code: None,
                };
                self.notify(format!("[{}] {}", js, child));
                self.jobs.push(job);
                self.background_jobs.fetch_add(1, Ordering::Relaxed);
                Ok(ExitStatus::Code(0))
            }
            ForkResult::Child => {
                reset_child_signals();

                // Make ourselves the leader of a new process group.
                let _ = setpgid(Pid::from_raw(0), Pid::from_raw(0));

                // Detach stdin from the terminal so background jobs can't
                // consume keystrokes meant for the shell.
                let null = fcntl::open(
                    "/dev/null",
                    fcntl::OFlag::O_RDONLY,
                    nix::sys::stat::Mode::empty(),
                );
                if let Ok(fd) = null {
                    let _ = dup2(fd.as_raw_fd(), 0);
                    let _ = nix::unistd::close(fd.as_raw_fd());
                }

                // Run the command subtree and exit with its status. This child
                // is detached from the terminal, so it must not hand the
                // terminal to its own children or act as a foreground job.
                self.in_background_child = true;
                let status = self.execute_command(cmd, heredoc_inputs).unwrap_or(ExitStatus::Code(1));
                std::process::exit(status.code());
            }
        }
    }

    fn execute_script_in_child(
        &self,
        script: &Script,
        heredoc_inputs: &mut dyn Iterator<Item = String>,
    ) -> Result<ExitStatus, String> {
        match unsafe { fork() }.map_err(|e| format!("fork: {}", e))? {
            ForkResult::Parent { child } => {
                match wait_for_child(child) {
                    Ok(WaitStatus::Exited(_, code)) => Ok(ExitStatus::Code(code)),
                    Ok(WaitStatus::Signaled(_, sig, _)) => Ok(ExitStatus::Signal(sig as i32)),
                    Ok(_) => Ok(ExitStatus::Code(1)),
                    Err(e) => Err(e),
                }
            }
            ForkResult::Child => {
                reset_child_signals();
                // We need a mutable executor in child — create a temp one
                let mut child_exec = Executor::new();
                let status = child_exec.execute(script, heredoc_inputs).unwrap_or(ExitStatus::Code(1));
                std::process::exit(status.code());
            }
        }
    }

    // ── Redirection helpers ──

    /// Expand variables / special vars / command substitution in a word,
    /// using the executor's current state (`$?` = last exit code).
    fn expand_word(&self, s: &str) -> String {
        crate::wordexp::expand(s, self.last_exit)
    }

    fn apply_redirections(&self, redirs: &[Redirection]) -> Result<(), String> {
        for redir in redirs {
            match redir {
                Redirection::Input(file) => {
                    let file = self.expand_word(file);
                    let fd = fcntl::open(
                        file.as_str(),
                        fcntl::OFlag::O_RDONLY,
                        nix::sys::stat::Mode::empty(),
                    )
                    .map_err(|e| format!("open {}: {}", file, e))?;
                    dup2(fd.as_raw_fd(), 0).map_err(|e| format!("dup2: {}", e))?;
                    let _ = nix::unistd::close(fd.as_raw_fd());
                }
                Redirection::Output(file) => {
                    let file = self.expand_word(file);
                    let fd = open_output(&file, false).map_err(|e| e)?;
                    dup2(fd.as_raw_fd(), 1).map_err(|e| format!("dup2: {}", e))?;
                    let _ = nix::unistd::close(fd.as_raw_fd());
                }
                Redirection::Append(file) => {
                    let file = self.expand_word(file);
                    let fd = open_append(&file).map_err(|e| e)?;
                    dup2(fd.as_raw_fd(), 1).map_err(|e| format!("dup2: {}", e))?;
                    let _ = nix::unistd::close(fd.as_raw_fd());
                }
                Redirection::ReadWrite(file) => {
                    let file = self.expand_word(file);
                    let fd = fcntl::open(
                        file.as_str(),
                        fcntl::OFlag::O_RDWR | fcntl::OFlag::O_CREAT,
                        nix::sys::stat::Mode::S_IRUSR
                            | nix::sys::stat::Mode::S_IWUSR
                            | nix::sys::stat::Mode::S_IRGRP
                            | nix::sys::stat::Mode::S_IROTH,
                    )
                    .map_err(|e| format!("open {}: {}", file, e))?;
                    dup2(fd.as_raw_fd(), 0).map_err(|e| format!("dup2: {}", e))?;
                    let _ = nix::unistd::close(fd.as_raw_fd());
                }
                Redirection::Both(file) => {
                    let file = self.expand_word(file);
                    let fd = open_output(&file, false).map_err(|e| e)?;
                    dup2(fd.as_raw_fd(), 1).map_err(|e| format!("dup2: {}", e))?;
                    dup2(fd.as_raw_fd(), 2).map_err(|e| format!("dup2: {}", e))?;
                    let _ = nix::unistd::close(fd.as_raw_fd());
                }
                Redirection::DupOutput(target) => {
                    dup2(*target, 1).map_err(|e| format!("dup2: {}", e))?;
                }
                Redirection::DupInput(target) => {
                    dup2(*target, 0).map_err(|e| format!("dup2: {}", e))?;
                }
                Redirection::FdOutput(fd_num, file) => {
                    let file = self.expand_word(file);
                    let fd = open_output(&file, false).map_err(|e| e)?;
                    dup2(fd.as_raw_fd(), *fd_num).map_err(|e| format!("dup2: {}", e))?;
                    let _ = nix::unistd::close(fd.as_raw_fd());
                }
                Redirection::FdAppend(fd_num, file) => {
                    let file = self.expand_word(file);
                    let fd = open_append(&file).map_err(|e| e)?;
                    dup2(fd.as_raw_fd(), *fd_num).map_err(|e| format!("dup2: {}", e))?;
                    let _ = nix::unistd::close(fd.as_raw_fd());
                }
                Redirection::FdInput(fd_num, file) => {
                    let file = self.expand_word(file);
                    let fd = fcntl::open(
                        file.as_str(),
                        fcntl::OFlag::O_RDONLY,
                        nix::sys::stat::Mode::empty(),
                    )
                    .map_err(|e| format!("open {}: {}", file, e))?;
                    dup2(fd.as_raw_fd(), *fd_num).map_err(|e| format!("dup2: {}", e))?;
                    let _ = nix::unistd::close(fd.as_raw_fd());
                }
                Redirection::FdReadWrite(fd_num, file) => {
                    let file = self.expand_word(file);
                    let fd = fcntl::open(
                        file.as_str(),
                        fcntl::OFlag::O_RDWR | fcntl::OFlag::O_CREAT,
                        nix::sys::stat::Mode::S_IRUSR
                            | nix::sys::stat::Mode::S_IWUSR
                            | nix::sys::stat::Mode::S_IRGRP
                            | nix::sys::stat::Mode::S_IROTH,
                    )
                    .map_err(|e| format!("open {}: {}", file, e))?;
                    dup2(fd.as_raw_fd(), *fd_num).map_err(|e| format!("dup2: {}", e))?;
                    let _ = nix::unistd::close(fd.as_raw_fd());
                }
                Redirection::FdDup(from, to) => {
                    dup2(*from, *to).map_err(|e| format!("dup2: {}", e))?;
                }
            }
        }
        Ok(())
    }

    fn setup_heredoc_stdin(&self, body: &str) {
        // Create a pipe and write heredoc content to it
        match pipe() {
            Ok((r, w)) => {
                // Write body to write end — OwnedFd implements AsFd
                let _ = nix::unistd::write(&w, body.as_bytes());
                let _ = nix::unistd::write(&w, b"\n");
                let _ = nix::unistd::close(w.as_raw_fd());

                // Redirect stdin from read end
                let _ = dup2(r.as_raw_fd(), 0);
                let _ = nix::unistd::close(r.as_raw_fd());
            }
            Err(_) => {}
        }
    }

    fn setup_herestring_stdin(&self, content: &str) {
        match pipe() {
            Ok((r, w)) => {
                let _ = nix::unistd::write(&w, content.as_bytes());
                let _ = nix::unistd::close(w.as_raw_fd());
                let _ = dup2(r.as_raw_fd(), 0);
                let _ = nix::unistd::close(r.as_raw_fd());
            }
            Err(_) => {}
        }
    }

    // ── Program execution ──

    fn resolve_alias(&self, program: &str) -> String {
        self.aliases.get(program).cloned().unwrap_or_else(|| program.to_string())
    }

    // ── Stateful builtins (need executor state) ──

    /// Whether `program` is a stateful builtin (handled by the executor).
    fn is_stateful_builtin(&self, program: &str) -> bool {
        matches!(
            program,
            "alias" | "unalias" | "source" | "." | "jobs" | "fg" | "bg"
                | "disown" | "wait" | "history" | "kill" | "pushd" | "popd" | "dirs"
        )
    }

    /// Run an in-process builtin with redirections applied to the shell's own
    /// fds temporarily.
    ///
    /// External commands are forked and redirect their fds in the child, but
    /// builtins execute inside the shell process — so we save fd 0/1/2, apply
    /// the redirections, run the builtin, then restore the originals.
    fn run_builtin_with_redirections(
        &mut self,
        redirections: &[Redirection],
        f: impl FnOnce(&mut Self) -> Result<ExitStatus, String>,
    ) -> Result<ExitStatus, String> {
        let saved_stdin = dup(0).map_err(|e| format!("dup: {}", e))?;
        let saved_stdout = dup(1).map_err(|e| format!("dup: {}", e))?;
        let saved_stderr = dup(2).map_err(|e| format!("dup: {}", e))?;

        let result = match self.apply_redirections(redirections) {
            Ok(()) => f(self),
            Err(e) => Err(e),
        };

        // If the builtin returned an error, print it to the STILL-redirected
        // stderr so `2>` / `2>&1` capture it (otherwise it would be printed
        // by the REPL after fds are restored and leak to the terminal), and
        // treat it as exit code 1.
        let result = match result {
            Err(e) => {
                let _ = writeln!(io::stderr(), "waash: {}", e);
                Ok(ExitStatus::Code(1))
            }
            ok => ok,
        };

        // Restore the shell's original stdin/stdout/stderr.
        let _ = dup2(saved_stdin.as_raw_fd(), 0);
        let _ = dup2(saved_stdout.as_raw_fd(), 1);
        let _ = dup2(saved_stderr.as_raw_fd(), 2);
        let _ = nix::unistd::close(saved_stdin.as_raw_fd());
        let _ = nix::unistd::close(saved_stdout.as_raw_fd());
        let _ = nix::unistd::close(saved_stderr.as_raw_fd());

        result
    }

    /// Handle builtins that require shell state (aliases, jobs, history).
    /// Returns Ok(None) if `program` isn't one of these builtins.
    fn try_stateful_builtin(
        &mut self,
        program: &str,
        args: &[String],
    ) -> Result<Option<ExitStatus>, String> {
        match program {
            "alias" => Ok(Some(self.builtin_alias(args))),
            "unalias" => Ok(Some(self.builtin_unalias(args))),
            "source" | "." => self.builtin_source(args).map(Some),
            "jobs" => Ok(Some(self.builtin_jobs(args))),
            "fg" => self.builtin_fg(args).map(Some),
            "bg" => self.builtin_bg(args).map(Some),
            "disown" => Ok(Some(self.builtin_disown(args))),
            "wait" => self.builtin_wait(args).map(Some),
            "kill" => self.builtin_kill(args).map(Some),
            "pushd" => self.builtin_pushd(args).map(Some),
            "popd" => self.builtin_popd(args).map(Some),
            "dirs" => Ok(Some(self.builtin_dirs(args))),
            "history" => Ok(Some(self.builtin_history(args))),
            _ => Ok(None),
        }
    }

    fn builtin_alias(&mut self, args: &[String]) -> ExitStatus {
        if args.is_empty() {
            // List all aliases, sorted by name.
            let mut names: Vec<&String> = self.aliases.keys().collect();
            names.sort();
            for name in names {
                println!("alias {}={}", name, self.aliases[name]);
            }
            return ExitStatus::Code(0);
        }
        for arg in args {
            if let Some((name, value)) = arg.split_once('=') {
                self.aliases.insert(name.to_string(), value.to_string());
            } else if let Some(v) = self.aliases.get(arg) {
                println!("alias {}={}", arg, v);
            } else {
                eprintln!("alias: {}: not found", arg);
            }
        }
        ExitStatus::Code(0)
    }

    fn builtin_unalias(&mut self, args: &[String]) -> ExitStatus {
        for arg in args {
            self.aliases.remove(arg);
        }
        ExitStatus::Code(0)
    }

    fn builtin_source(&mut self, args: &[String]) -> Result<ExitStatus, String> {
        if args.is_empty() {
            return Err("source: missing filename".to_string());
        }
        let path = &args[0];
        let p = std::path::Path::new(path);

        // .waash / .ind files are WAASH/Indent scripts — parse & run in-process
        // so their variables/aliases persist in the shell.
        if crate::indent::is_waash_script(p) {
            let contents = std::fs::read_to_string(path)
                .map_err(|e| format!("source: {}: {}", path, e))?;
            let mut scanner = crate::lexer::Scanner::new(contents);
            scanner.lex_all().map_err(|e| format!("lexer error: {}", e))?;
            let mut parser = crate::parser::Parser::new(scanner);
            let script = parser.parse().map_err(|e| format!("parse error: {}", e))?;
            let no_lines: Vec<String> = Vec::new();
            return self.execute(&script, &mut no_lines.into_iter());
        }

        // Everything else is treated as a shell script and delegated to the
        // real shell (bash by default) so POSIX/bash syntax works. Note: shell
        // variables set this way won't persist into WAASH (separate process),
        // but the script runs correctly and won't error on bash-isms.
        let status = std::process::Command::new("bash")
            .arg(path)
            .status()
            .map_err(|e| format!("source: failed to run bash: {}", e))?;
        Ok(ExitStatus::Code(status.code().unwrap_or(1)))
    }
    fn builtin_jobs(&mut self, _args: &[String]) -> ExitStatus {
        // Reap finished jobs first so states are current.
        self.reap_jobs();
        if self.jobs.is_empty() {
            self.notify("(no jobs)".to_string());
            return ExitStatus::Code(0);
        }
        for (i, job) in self.jobs.iter().enumerate() {
            let state = match job.state {
                JobState::Running => "Running",
                JobState::Stopped => "Stopped",
                JobState::Done => "Done",
            };
            // `+` marks the most recent job (the default target of fg/bg).
            let marker = if i == self.jobs.len() - 1 { "+" } else { "-" };
            let line = if job.pgid == job.pid {
                format!("[{}] {} {}  pid={}  {}", i + 1, marker, state, job.pid, job.command)
            } else {
                format!("[{}] {} {}  pid={} pgid={}  {}", i + 1, marker, state, job.pid, job.pgid, job.command)
            };
            self.notify(line);
        }
        ExitStatus::Code(0)
    }

    /// Remove a job from the table by pid (used by `wait`).

    /// `disown [job]` — stop tracking a background job so it is no longer
    /// reported by `jobs`/reaped into the shell. The process keeps running
    /// detached. (It is still reaped by the shell to avoid zombies, just
    /// silently, since it's no longer in the job table.)
    fn builtin_disown(&mut self, args: &[String]) -> ExitStatus {
        if args.is_empty() {
            if let Some(idx) = self.select_job(&[]) {
                let cmd = self.jobs[idx].command.clone();
                self.remove_job(idx);
                self.notify(format!("{} disowned", cmd));
            } else {
                self.notify("disown: no jobs".to_string());
            }
            return ExitStatus::Code(0);
        }
        for arg in args {
            match arg.parse::<usize>() {
                Ok(n) if n >= 1 && n <= self.jobs.len() => {
                    let cmd = self.jobs[n - 1].command.clone();
                    self.remove_job(n - 1);
                    self.notify(format!("{} disowned", cmd));
                }
                _ => self.notify(format!("disown: {}: no such job", arg)),
            }
        }
        ExitStatus::Code(0)
    }

    /// `wait [job]` — wait for a background job (or all of them) to finish and
    /// return its exit status.
    fn builtin_wait(&mut self, args: &[String]) -> Result<ExitStatus, String> {
        if let Some(arg) = args.first() {
            let idx = arg
                .parse::<usize>()
                .ok()
                .filter(|&n| n >= 1 && n <= self.jobs.len())
                .ok_or_else(|| format!("wait: {}: no such job", arg))?;
            let code = self.wait_for_job(idx - 1)?;
            self.remove_job(idx - 1);
            Ok(ExitStatus::Code(code))
        } else {
            // Wait for all background jobs, returning the last exit status seen
            // (0 if there were no jobs). Jobs that already finished use their
            // stored code; running ones are reaped here.
            let mut last = 0;
            let mut any = false;
            let idx = 0;
            while idx < self.jobs.len() {
                let code = self.wait_for_job(idx)?;
                self.remove_job(idx); // always remove the first job; list shrinks
                last = code;
                any = true;
            }            Ok(ExitStatus::Code(if any { last } else { 0 }))
        }
    }

    /// Wait for the job at `idx` and return its exit code. If the job already
    /// finished (its status was reaped), use the stored code instead of calling
    /// `waitpid` again (which would error with ECHILD).
    fn wait_for_job(&mut self, idx: usize) -> Result<i32, String> {
        if let Some(code) = self.jobs[idx].code {
            return Ok(code);
        }
        let pid = self.jobs[idx].pid;
        let command = self.jobs[idx].command.clone();
        let status = wait_for_child(pid).map_err(|e| format!("wait: {}", e))?;
        let code = match status {
            WaitStatus::Exited(_, c) => c,
            WaitStatus::Signaled(_, s, _) => 128 + s as i32,
            _ => 1,
        };
        self.notify(format!("{} done", command));
        Ok(code)
    }

    /// Pick a job index from `fg`/`bg` args; defaults to the most recent job.
    fn select_job(&self, args: &[String]) -> Option<usize> {
        if let Some(arg) = args.first() {
            arg.parse::<usize>().ok().and_then(|n| {
                if n >= 1 && n <= self.jobs.len() {
                    Some(n - 1)
                } else {
                    None
                }
            })
        } else if !self.jobs.is_empty() {
            Some(self.jobs.len() - 1)
        } else {
            None
        }
    }

    fn builtin_bg(&mut self, args: &[String]) -> Result<ExitStatus, String> {
        let idx = self
            .select_job(args)
            .ok_or_else(|| "bg: no such job".to_string())?;
        let pgid = self.jobs[idx].pgid;
        let command = self.jobs[idx].command.clone();
        // Signal the whole process group so a background pipeline resumes as
        // a unit (not just its leader).
        let _ = signal::killpg(pgid, Signal::SIGCONT);
        self.jobs[idx].state = JobState::Running;
        self.notify(format!("[{}] {} continued", idx + 1, command));
        Ok(ExitStatus::Code(0))
    }

    fn builtin_fg(&mut self, args: &[String]) -> Result<ExitStatus, String> {
        let idx = self
            .select_job(args)
            .ok_or_else(|| "fg: no such job".to_string())?;
        let pid = self.jobs[idx].pid;
        let pgid = self.jobs[idx].pgid;
        let command = self.jobs[idx].command.clone();

        // Give the terminal to the job's process group for the duration of
        // the wait, then hand it back to the shell. This lets the foreground
        // job read keystrokes and receive job control signals correctly.
        let tty = io::stdin().as_raw_fd();
        unsafe { nix::libc::tcsetpgrp(tty, pgid.as_raw()) };
        // Resume the whole group.
        let _ = signal::killpg(pgid, Signal::SIGCONT);
        self.jobs[idx].state = JobState::Running;

        let status = match wait_for_child(pid) {
            Ok(WaitStatus::Exited(_, code)) => ExitStatus::Code(code),
            Ok(WaitStatus::Signaled(_, sig, _)) => ExitStatus::Signal(sig as i32),
            _ => ExitStatus::Code(1),
        };
        // Restore the shell's terminal ownership.
        unsafe { nix::libc::tcsetpgrp(tty, unistd::getpgrp().as_raw()) };

        // Remove the job now that it has been brought to the foreground.
        self.remove_job(idx);
        self.notify(format!("[{}] {} done", idx + 1, command));
        Ok(status)
    }

    /// Resolve a job spec (`%N`, `%`, or `%+`/`%-`) to a job index. Returns
    /// None if the spec isn't a valid job reference.
    fn job_index_from_spec(&self, spec: &str) -> Option<usize> {
        let s = spec.strip_prefix('%')?;
        if s.is_empty() {
            // `%` / `%+` = most recent job.
            if !self.jobs.is_empty() {
                Some(self.jobs.len() - 1)
            } else {
                None
            }
        } else {
            s.parse::<usize>()
                .ok()
                .filter(|&n| n >= 1 && n <= self.jobs.len())
                .map(|n| n - 1)
        }
    }

    /// `kill [-SIGNAL] <pid>|<%job> ...` — send a signal to processes or jobs.
    /// Bare numbers are PIDs; `%N` / `%` are jobs and signal the job's whole
    /// process group (so a background pipeline is killed as a unit). The
    /// signal defaults to SIGTERM; `-9`, `-KILL`, `-s TERM`, etc. work.
    fn builtin_kill(&mut self, args: &[String]) -> Result<ExitStatus, String> {
        let mut sig = Signal::SIGTERM;
        let mut idx = 0;

        if let Some(first) = args.first() {
            if let Some(body) = first.strip_prefix('-') {
                if !body.is_empty() && body.chars().all(|c| c.is_ascii_digit()) {
                    if let Ok(n) = body.parse::<i32>() {
                        if let Some(s) = Signal::try_from(n).ok() {
                            sig = s;
                            idx = 1;
                        }
                    }
                } else if !body.is_empty() {
                    // -SIGNAME or -NAME (e.g. -KILL, -SIGTERM).
                    if let Some(s) = signal_from_name(body) {
                        sig = s;
                        idx = 1;
                    }
                }
            } else if first == "-s" {
                if let Some(name) = args.get(1) {
                    if let Some(s) = signal_from_name(name) {
                        sig = s;
                        idx = 2;
                    }
                }
            }
        }

        if idx >= args.len() {
            eprintln!("kill: usage: kill [-SIGNAL] <pid>|<%job> ...");
            return Ok(ExitStatus::Code(1));
        }

        let mut status = 0;
        for target in &args[idx..] {
            if let Some(job_idx) = self.job_index_from_spec(target) {
                let pgid = self.jobs[job_idx].pgid;
                let command = self.jobs[job_idx].command.clone();
                if signal::killpg(pgid, sig).is_ok() {
                    self.notify(format!("[{}] {} signalled", job_idx + 1, command));
                } else {
                    eprintln!("kill: {}: failed to signal job", target);
                    status = 1;
                }
            } else if let Ok(pid) = target.parse::<i32>() {
                if signal::kill(Pid::from_raw(pid), sig).is_err() {
                    eprintln!("kill: {}: no such process", target);
                    status = 1;
                }
            } else {
                eprintln!("kill: {}: invalid job or pid", target);
                status = 1;
            }
        }
        Ok(ExitStatus::Code(status))
    }

    /// Print the directory stack as a space-separated list (current dir first).
    fn print_dirs(&self) {
        let cwd = env::current_dir()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default();
        let mut parts = vec![cwd];
        parts.extend(self.dir_stack.iter().cloned());
        println!("{}", parts.join(" "));
    }

    /// `pushd [dir]` — save the current directory and `cd` to `dir` (or swap
    /// the top two stack entries when no dir is given). With `-n` it only
    /// manipulates the stack without changing directories.
    fn builtin_pushd(&mut self, args: &[String]) -> Result<ExitStatus, String> {
        let mut no_cd = false;
        let mut target: Option<String> = None;
        for a in args {
            if a == "-n" {
                no_cd = true;
            } else {
                target = Some(a.clone());
            }
        }

        let cwd = env::current_dir()
            .map(|p| p.to_string_lossy().to_string())
            .map_err(|e| format!("pushd: {}", e))?;

        let new_dir = match target {
            Some(dir) => {
                self.dir_stack.push(cwd);
                dir
            }
            None => {
                // No dir: rotate the top of the stack (if any) to the top.
                if self.dir_stack.is_empty() {
                    eprintln!("pushd: no other directory");
                    return Ok(ExitStatus::Code(1));
                }
                let top = self.dir_stack.pop().unwrap();
                self.dir_stack.push(cwd);
                top
            }
        };

        if !no_cd {
            std::env::set_current_dir(&new_dir)
                .map_err(|e| format!("pushd: {}: {}", new_dir, e))?;
        }
        self.print_dirs();
        Ok(ExitStatus::Code(0))
    }

    /// `popd [-n]` — `cd` back to the top of the stack and remove it.
    fn builtin_popd(&mut self, args: &[String]) -> Result<ExitStatus, String> {
        let no_cd = args.iter().any(|a| a == "-n");
        match self.dir_stack.pop() {
            Some(target) => {
                if !no_cd {
                    std::env::set_current_dir(&target)
                        .map_err(|e| format!("popd: {}: {}", target, e))?;
                }
                self.print_dirs();
                Ok(ExitStatus::Code(0))
            }
            None => {
                eprintln!("popd: directory stack empty");
                Ok(ExitStatus::Code(1))
            }
        }
    }

    /// `dirs` — print the directory stack (current dir first).
    fn builtin_dirs(&self, _args: &[String]) -> ExitStatus {
        self.print_dirs();
        ExitStatus::Code(0)
    }

    fn builtin_history(&mut self, args: &[String]) -> ExitStatus {
        let count = args
            .first()
            .and_then(|s| s.parse::<usize>().ok())
            .unwrap_or(50);        match std::fs::read_to_string(&self.history_file) {
            Ok(contents) => {
                let lines: Vec<&str> = contents.lines().collect();
                let start = lines.len().saturating_sub(count);
                for (i, line) in lines.iter().enumerate().skip(start) {
                    println!("{}  {}", i + 1, line);
                }
            }
            Err(_) => {
                println!("(no history yet)");
            }
        }
        ExitStatus::Code(0)
    }

    fn exec_program(&self, program: &str, args: &[String]) {
        let c_program = CString::new(program.as_bytes()).unwrap_or_default();
        let c_args: Vec<CString> = std::iter::once(program)
            .chain(args.iter().map(|s| s.as_str()))
            .map(|s| CString::new(s).unwrap_or_default())
            .collect();

        // Try direct path first, then PATH search
        if program.contains('/') {
            let _ = unistd::execv(&c_program, &c_args);
        } else {
            let _ = unistd::execvp(&c_program, &c_args);
        };

        // If we get here, exec failed
        let _ = writeln!(io::stderr(), "waash: {}: command not found", program);
    }
}

impl Default for Executor {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: push a job with the given pid/state directly into the table.
    fn push_job(ex: &mut Executor, pid: i32, state: JobState) {
        ex.jobs.push(Job {
            pid: Pid::from_raw(pid),
            pgid: Pid::from_raw(pid),
            command: "test".to_string(),
            state,
            code: None,
        });
        ex.background_jobs.fetch_add(1, Ordering::Relaxed);
    }

    #[test]
    fn background_jobs_counter_tracks_active_jobs() {
        let mut ex = Executor::new();
        assert_eq!(ex.background_jobs.load(Ordering::Relaxed), 0);

        // Two active (running) jobs.
        push_job(&mut ex, 100, JobState::Running);
        push_job(&mut ex, 101, JobState::Stopped);
        assert_eq!(ex.background_jobs.load(Ordering::Relaxed), 2);

        // One finishes -> active count drops, job is still in the table.
        ex.mark_job_done(Pid::from_raw(100), 0);
        assert_eq!(ex.background_jobs.load(Ordering::Relaxed), 1);

        // Removing a Done job must NOT double-decrement.
        ex.remove_job(0);
        assert_eq!(ex.background_jobs.load(Ordering::Relaxed), 1);

        // Removing a still-active job decrements.
        ex.remove_job(0);
        assert_eq!(ex.background_jobs.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn background_jobs_counter_remove_active_decrements() {
        let mut ex = Executor::new();
        push_job(&mut ex, 200, JobState::Running);
        assert_eq!(ex.background_jobs.load(Ordering::Relaxed), 1);
        // disown of a running job removes it from the active set.
        ex.remove_job(0);
        assert_eq!(ex.background_jobs.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn mark_job_done_is_idempotent_on_counter() {
        let mut ex = Executor::new();
        push_job(&mut ex, 300, JobState::Running);
        // Marking done twice shouldn't underflow the counter.
        ex.mark_job_done(Pid::from_raw(300), 0);
        ex.mark_job_done(Pid::from_raw(300), 0);
        assert_eq!(ex.background_jobs.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn job_index_from_spec_parses() {
        let mut ex = Executor::new();
        push_job(&mut ex, 100, JobState::Running);
        push_job(&mut ex, 101, JobState::Stopped);
        // `%` = most recent job.
        assert_eq!(ex.job_index_from_spec("%"), Some(1));
        assert_eq!(ex.job_index_from_spec("%2"), Some(1));
        assert_eq!(ex.job_index_from_spec("%1"), Some(0));
        // Out of range / non-job → None.
        assert_eq!(ex.job_index_from_spec("%3"), None);
        assert_eq!(ex.job_index_from_spec("%0"), None);
        // Bare numbers are PIDs, not job specs.
        assert_eq!(ex.job_index_from_spec("1"), None);
        assert_eq!(ex.job_index_from_spec("abc"), None);
    }
}

