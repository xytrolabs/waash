//! Prompt styles for the WAASH REPL.
//!
//! Features:
//! - Right-side prompt (RPROMPT) with date/time/battery
//! - Rainbow-cycling prompt arrow that changes color each prompt
//! - Powerline-style separator lines
//! - Full template substitution with 20+ variables

use crate::config::PromptConfig;
use ansi_term::Colour::{Blue, Cyan, Green, Purple, Red, Yellow};
use ansi_term::Style;
use std::sync::Arc;
use std::sync::atomic::{AtomicI32, AtomicU64, AtomicUsize, Ordering};

/// Configurable prompt formatter.
#[derive(Clone)]
pub struct PromptStyle {
    config: PromptConfig,
    user: String,
    hostname: String,
    /// Cycles for {rainbow} — each prompt shifts the color
    rainbow_idx: usize,
    /// Number of active (running/stopped) background jobs, shown by {jobs}.
    /// Shares the counter with the executor so the indicator stays live.
    background_jobs: Arc<AtomicUsize>,
}

/// The rainbow palette — 12 colors that cycle
const RAINBOW_COLORS: &[u8] = &[1, 3, 2, 6, 4, 5, 9, 11, 10, 14, 12, 13];

impl Default for PromptStyle {
    fn default() -> Self {
        Self::from_config(&PromptConfig::default())
    }
}

impl PromptStyle {
    pub fn from_config(config: &PromptConfig) -> Self {
        let user = std::env::var("USER").unwrap_or_else(|_| "user".to_string());
        let hostname = std::process::Command::new("hostname")
            .output()
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .map(|s| s.trim().to_string())
            .unwrap_or_else(|| "localhost".to_string());

        Self {
            config: config.clone(),
            user,
            hostname,
            rainbow_idx: 0,
            background_jobs: Arc::new(AtomicUsize::new(0)),
        }
    }

    /// Attach the executor's live background-job counter so {jobs} reflects
    /// the actual number of running/stopped background tasks.
    pub fn set_background_jobs(&mut self, counter: Arc<AtomicUsize>) {
        self.background_jobs = counter;
    }

    /// Build the full prompt, cycling the rainbow index each call.
    pub fn format_prompt(
        &mut self,
        last_exit: i32,
        last_duration: std::time::Duration,
    ) -> String {
        // Cycle the rainbow color for this prompt
        self.rainbow_idx = (self.rainbow_idx + 1) % RAINBOW_COLORS.len();
        self.format_prompt_static(last_exit, last_duration)
    }

    /// Build the prompt without advancing the rainbow index. Used both by
    /// `format_prompt` and by the live-refresh path (the prompt is rebuilt on
    /// every render so time/CPU/sudo stay current).
    pub(crate) fn format_prompt_static(
        &self,
        last_exit: i32,
        last_duration: std::time::Duration,
    ) -> String {
        let rainbow_color = RAINBOW_COLORS[self.rainbow_idx];

        let cwd = std::env::current_dir()
            .unwrap_or_else(|_| std::path::PathBuf::from("/"))
            .to_string_lossy()
            .to_string();

        let home = std::env::var("HOME").unwrap_or_else(|_| "/home/user".to_string());
        let cwd_display = if cwd.starts_with(&home) {
            format!("~{}", &cwd[home.len()..])
        } else {
            cwd.clone()
        };

        // ── Segments ──

        let exit_segment = if self.config.show_exit_code && last_exit != 0 {
            format!("{} ", Red.bold().paint(format!("✗{}", last_exit)))
        } else {
            String::new()
        };

        let duration_segment = format_duration(last_duration);

        // Rainbow prompt arrow
        let prompt_char = if last_exit == 0 {
            ansi_term::Colour::Fixed(rainbow_color)
                .bold()
                .paint(&self.config.char_ok)
                .to_string()
        } else {
            Red.bold().paint(&self.config.char_err).to_string()
        };

        let user_colored = Cyan.bold().paint(&self.user).to_string();
        let host_colored = if self.config.show_hostname {
            Green.bold().paint(&self.hostname).to_string()
        } else {
            String::new()
        };

        let dir_target = if self.config.shorten_path {
            self.shorten_path(&cwd_display)
        } else {
            cwd_display.clone()
        };
        let dir_colored = Blue.bold().paint(&dir_target).to_string();
        let full_dir_colored = Blue.bold().paint(&cwd_display).to_string();

        let git_segment = if self.config.show_git {
            self.git_segment()
        } else {
            String::new()
        };

        let time_icon = time_of_day_icon();
        let time_str = chrono_prompt_time();

        let shlvl = std::env::var("SHLVL")
            .unwrap_or_else(|_| "1".into())
            .parse::<u32>()
            .unwrap_or(1);

        let venv = std::env::var("VIRTUAL_ENV")
            .ok()
            .and_then(|p| {
                std::path::Path::new(&p)
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
            })
            .unwrap_or_default();

        let venv_segment = if !venv.is_empty() {
            format!("{} ", Purple.paint(format!("🐍{}", venv)))
        } else {
            String::new()
        };

        let jobs_segment = self.jobs_segment();

        let prompt_str = prompt_char;
        let shlvl_str = if shlvl > 1 {
            format!("{}", Cyan.dimmed().paint(format!("+{}", shlvl - 1)))
        } else {
            String::new()
        };

        // ── New expressive segments ──

        // Random flair emoji (changes every second!)
        let flair = random_flair();

        // Battery
        let battery_segment = if let Some(pct) = battery_pct() {
            let icon = match pct {
                0..=10 => "🪫",
                11..=30 => "🔋",
                31..=60 => "🔋",
                61..=90 => "🔋",
                _ => "🔋",
            };
            let color = match pct {
                0..=20 => Red,
                21..=50 => Yellow,
                _ => Green,
            };
            format!("{} ", color.paint(format!("{}{}%", icon, pct)))
        } else {
            String::new()
        };

        // CPU load
        let load_segment = if let Some(load) = load_avg() {
            let color = if load > 4.0 {
                Style::new().fg(Red)
            } else if load > 2.0 {
                Style::new().fg(Yellow)
            } else {
                Style::new().fg(Cyan)
            };
            format!("{} ", color.paint(format!("⚙{:.1}", load)))
        } else {
            String::new()
        };

        // ── Right-side info (inline, no cursor jumps) ──
        let rprompt = self.build_right_prompt(last_duration);

        // ── Powerline separator ──
        let separator = format!(
            "{}",
            ansi_term::Colour::Fixed(rainbow_color)
                .dimmed()
                .paint("╭─")
        );

        // ── Template substitution ──
        let mut result = self.config.template
            .replace("{separator}", &separator)
            .replace("{user}", &user_colored)
            .replace("{host}", &host_colored)
            .replace("{dir}", &dir_colored)
            .replace("{full_dir}", &full_dir_colored)
            .replace("{git}", &git_segment)
            .replace("{exit_code}", &exit_segment)
            .replace("{duration}", &duration_segment)
            .replace("{prompt}", &prompt_str)
            .replace("{time_icon}", time_icon)
            .replace("{time}", &time_str)
            .replace("{date}", "")
            .replace("{shlvl}", &shlvl_str)
            .replace("{venv}", &venv_segment)
            .replace("{sudo}", &sudo_segment())
            .replace("{jobs}", &jobs_segment)
            .replace("{flair}", flair)
            .replace("{battery}", &battery_segment)
            .replace("{load}", &load_segment)
            .replace("{shell}", "waash")
            .replace("{version}", env!("CARGO_PKG_VERSION"))
            .replace("\\n", "\n")
            .replace("{newline}", "\n");

        // Insert right-side info inline on the FIRST line (before the newline),
        // so the prompt is stable and the cursor lands right after {prompt}.
        if !rprompt.is_empty() {
            let sep = format!("   {}", Style::new().dimmed().paint(&rprompt));
            if let Some(pos) = result.find('\n') {
                result.insert_str(pos, &sep);
            } else {
                result.push_str(&sep);
            }
        }

        result = result.replace("  ", " ");
        result = result.replace(" \n", "\n");

        result
    }

    /// Build the right-side info: battery + load + time + duration.
    /// Returns plain dimmed text (NO ANSI cursor control — those break
    /// rustyline's cursor tracking and cause a visible gap before input).
    ///
    /// IMPORTANT: every segment here is re-rendered every second by the live
    /// refresh, and the rebuilt prompt must have the SAME width as the stored
    /// one or rustyline's cursor math breaks (a "random space" gap appears on
    /// the second line). So all variable segments are right-padded to a fixed
    /// width (load can cross 10/100, battery % can cross 10, etc.).
    fn build_right_prompt(&self, last_duration: std::time::Duration) -> String {
        let time_str = chrono_prompt_time();
        let mut parts = Vec::new();

        // Duration (bold if slow) — fixed width so it never shifts the line.
        let dur_text = format_duration_simple(last_duration);
        if !dur_text.is_empty() {
            let dur_fixed = format!("{:>6}", dur_text);
            if last_duration.as_secs() >= 1 {
                parts.push(Style::new().bold().paint(&dur_fixed).to_string());
            } else {
                parts.push(Style::new().dimmed().paint(&dur_fixed).to_string());
            }
        }

        // Battery — fixed width (icon + 3-digit %).
        if let Some(pct) = battery_pct() {
            let icon = if pct <= 20 { "🪫" } else { "🔋" };
            parts.push(format!("{}{:>3}%", icon, pct));
        }

        // Load — fixed width: `⚙` + number right-aligned to 4 (⚙12.2 / ⚙ 1.2).
        if let Some(load) = load_avg() {
            parts.push(format!("⚙{:>4}", format!("{:.1}", load)));
        }

        // Background tasks — always visible, right next to the CPU load so it
        // stays on the status line (not mixed with sudo/error on line 2).
        // Fixed 3 cells (⏳ + 2 digits) so live refresh keeps a stable width.
        let bg = self.background_jobs.load(Ordering::Relaxed);
        parts.push(format!("⏳{:>2}", bg));

        // Always show time (HH:MM:SS — fixed 8).
        parts.push(time_str);

        parts.join(" │ ")
    }

    pub fn heredoc_prompt(&self) -> String {
        format!("{} ", Yellow.paint("> "))
    }

    fn jobs_segment(&self) -> String {
        // The background-task count now lives on the right-side status line
        // (see [`Self::build_right_prompt`]), so the `{jobs}` template
        // placeholder renders nothing to avoid double-display.
        String::new()
    }

    fn git_segment(&self) -> String {
        if let Some(branch) = self.git_branch() {
            let dirty = self.git_dirty();
            let ahead_behind = self.git_ahead_behind();
            let branch_color = if dirty {
                Style::new().fg(Yellow).bold()
            } else {
                Style::new().fg(Purple)
            };
            let dirty_mark = if dirty { " ✗" } else { "" };
            format!(
                "{} ",
                branch_color.paint(format!("({}{}{})", branch, ahead_behind, dirty_mark))
            )
        } else {
            String::new()
        }
    }

    fn shorten_path(&self, path: &str) -> String {
        if path == "~" || path == "/" { return path.to_string(); }
        let parts: Vec<&str> = path.split('/').collect();
        if parts.len() <= 3 { return path.to_string(); }
        let mut short = String::new();
        for (i, part) in parts.iter().enumerate() {
            if i == parts.len() - 1 {
                if !short.is_empty() { short.push('/'); }
                short.push_str(part);
            } else if i == 0 && *part == "~" {
                short.push('~');
            } else if !part.is_empty() {
                short.push('/');
                short.push(part.chars().next().unwrap());
            }
        }
        short
    }

    fn git_branch(&self) -> Option<String> {
        std::process::Command::new("git")
            .args(["rev-parse", "--abbrev-ref", "HEAD"])
            .output().ok()
            .and_then(|o| {
                let s = String::from_utf8_lossy(&o.stdout).trim().to_string();
                if s.is_empty() { None } else { Some(s) }
            })
    }

    fn git_dirty(&self) -> bool {
        std::process::Command::new("git")
            .args(["diff", "--quiet"])
            .status().map(|s| !s.success()).unwrap_or(false)
    }

    fn git_ahead_behind(&self) -> String {
        let ahead = std::process::Command::new("git")
            .args(["rev-list", "--count", "@{u}..HEAD"])
            .output().ok()
            .and_then(|o| String::from_utf8_lossy(&o.stdout).trim().parse::<u32>().ok())
            .unwrap_or(0);
        let behind = std::process::Command::new("git")
            .args(["rev-list", "--count", "HEAD..@{u}"])
            .output().ok()
            .and_then(|o| String::from_utf8_lossy(&o.stdout).trim().parse::<u32>().ok())
            .unwrap_or(0);
        match (ahead, behind) {
            (0, 0) => String::new(),
            (a, 0) => format!(" ↑{}", a),
            (0, b) => format!(" ↓{}", b),
            (a, b) => format!(" ↑{}↓{}", a, b),
        }
    }
}

// ── Helper functions ──

fn format_duration(d: std::time::Duration) -> String {
    let millis = d.as_millis();
    let secs = d.as_secs_f64();
    if millis < 10 { return String::new(); }
    let (text, style): (String, Style) = if millis < 1000 {
        (format!("{}ms", millis), Style::new().fg(Yellow).dimmed())
    } else if secs < 60.0 {
        (format!("{:.1}s", secs), Style::new().dimmed())
    } else if secs < 3600.0 {
        let m = (secs / 60.0) as u64;
        let s = (secs % 60.0) as u64;
        (format!("{}m {}s", m, s), Style::new().fg(Yellow).dimmed())
    } else {
        let h = (secs / 3600.0) as u64;
        let m = ((secs % 3600.0) / 60.0) as u64;
        (format!("{}h {}m", h, m), Style::new().fg(Red).dimmed())
    };
    format!("{} ", style.paint(format!("~{}", text)))
}

fn format_duration_simple(d: std::time::Duration) -> String {
    let millis = d.as_millis();
    if millis < 10 { return String::new(); }
    let secs = d.as_secs_f64();
    if millis < 1000 { format!("~{}ms", millis) }
    else if secs < 60.0 { format!("~{:.1}s", secs) }
    else if secs < 3600.0 { format!("~{}m{}s", (secs/60.0) as u64, (secs%60.0) as u64) }
    else { format!("~{}h{}m", (secs/3600.0) as u64, ((secs%3600.0)/60.0) as u64) }
}

fn time_of_day_icon() -> &'static str {
    let hour = local_hour();
    match hour {
        5..=7 => "🌅",  8..=11 => "☀️",
        12..=14 => "🌤", 15..=17 => "🌥",
        18..=19 => "🌅", 20..=21 => "🌙",
        22..=23 | 0..=4 => "🌃",
        _ => "🌤",
    }
}

/// Get local timezone offset in seconds (east of UTC).
fn local_tz_offset() -> i64 {
    if let Ok(output) = std::process::Command::new("date").arg("+%z").output() {
        let s = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if s.len() == 5 && (s.starts_with('+') || s.starts_with('-')) {
            let sign: i64 = if s.starts_with('-') { -1 } else { 1 };
            let hours: i64 = s[1..3].parse().unwrap_or(0);
            let mins: i64 = s[3..5].parse().unwrap_or(0);
            return sign * (hours * 3600 + mins * 60);
        }
    }
    0
}

fn local_hour() -> u32 {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = now.as_secs() as i64 + local_tz_offset();
    (((secs % 86400 + 86400) % 86400) / 3600) as u32
}

fn chrono_prompt_time() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = now.as_secs() as i64 + local_tz_offset();
    let h = ((secs % 86400 + 86400) % 86400 / 3600) as u32;
    let m = ((secs % 3600 + 3600) % 3600 / 60) as u32;
    let s = ((secs % 60 + 60) % 60) as u32;
    format!("{:02}:{:02}:{:02}", h, m, s)
}

/// Random emoji for the prompt — changes every second.
fn random_flair() -> &'static str {
    let flairs = ["🚀","✨","💫","🌟","⚡","🔥","💪","🎯","🌈","🦄","🎸","🎨","🧠","💡","🔮","🍀","🎪","🎭","🪄","💎","🌻","🦋","🎋","🏆","🧩"];
    let idx = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| (d.as_secs() as usize) % flairs.len())
        .unwrap_or(0);
    flairs[idx]
}

/// Get battery percentage (Linux).
fn battery_pct() -> Option<u8> {
    let base = "/sys/class/power_supply";
    if let Ok(entries) = std::fs::read_dir(base) {
        for entry in entries.flatten() {
            let cap = entry.path().join("capacity");
            if cap.exists() {
                if let Ok(c) = std::fs::read_to_string(&cap) {
                    return c.trim().parse::<u8>().ok();
                }
            }
        }
    }
    None
}

/// Get 1-min load average.
fn load_avg() -> Option<f64> {
    std::fs::read_to_string("/proc/loadavg")
        .ok()
        .and_then(|s| s.split_whitespace().next()?.parse::<f64>().ok())
}

/// Prompt indicator for root / verified sudo credentials (the `{sudo}` var).
///   - Running as root  → `#` (the classic root prompt char)
///   - `sudo` works right now without prompting (`sudo -n true`) → `⚡`
///   - otherwise → a dim `·`
///
/// Always exactly 1 column wide so the live-refresh prompt stays stable.
fn sudo_segment() -> String {
    use std::process::Stdio;

    if nix::unistd::geteuid().is_root() {
        return Red.bold().paint("#").to_string();
    }

    let verified = std::process::Command::new("sudo")
        .args(["-n", "true"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);

    if verified {
        // Gold — matches the Xytro brand.
        ansi_term::Colour::Fixed(220).bold().paint("⚡").to_string()
    } else {
        Style::new().dimmed().paint("·").to_string()
    }
}

// Helper wrapper for rustyline
pub struct WaashHelper {
    pub completer: super::completer::WaashCompleter,
    pub highlighter: super::highlighter::WaashHighlighter,
    pub hinter: super::hinter::WaashHinter,
    /// Prompt style used to rebuild the prompt on every render (live refresh).
    pub prompt_style: PromptStyle,
    /// Last exit code, shared with the REPL (for live-refresh prompt rebuilds).
    pub last_exit: Arc<AtomicI32>,
    /// Last command duration in ns, shared with the REPL.
    pub last_duration_ns: Arc<AtomicU64>,
    /// Monotonic timestamp (ns) of the last keystroke / buffer change. The
    /// live-refresh ticker skips if a keystroke happened recently, so the
    /// injected repaint can never race with a typed character or an Enter
    /// that submits a command.
    pub last_keystroke: Arc<AtomicU64>,
}

impl rustyline::Helper for WaashHelper {}

impl rustyline::completion::Completer for WaashHelper {
    type Candidate = String;

    fn complete(
        &self,
        line: &str,
        pos: usize,
        ctx: &rustyline::Context<'_>,
    ) -> Result<(usize, Vec<String>), rustyline::error::ReadlineError> {
        self.completer.complete(line, pos, ctx)
    }

    fn update(&self, line: &mut rustyline::line_buffer::LineBuffer, start: usize, elected: &str, cl: &mut rustyline::Changeset) {
        self.completer.update(line, start, elected, cl)
    }
}

impl rustyline::hint::Hinter for WaashHelper {
    type Hint = String;

    fn hint(&self, line: &str, pos: usize, ctx: &rustyline::Context<'_>) -> Option<String> {
        self.hinter.hint(line, pos, ctx)
    }
}

impl rustyline::highlight::Highlighter for WaashHelper {
    fn highlight<'l>(&self, line: &'l str, pos: usize) -> std::borrow::Cow<'l, str> {
        // Record that the user is actively touching the buffer (typing or
        // moving the cursor). The live-refresh ticker uses this timestamp to
        // avoid repainting right around a keystroke/Enter, which would
        // otherwise desync rustyline and swallow the submit.
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0);
        self.last_keystroke.store(now, Ordering::Relaxed);
        self.highlighter.highlight(line, pos)
    }

    fn highlight_prompt<'b, 's: 'b, 'p: 'b>(
        &'s self,
        _prompt: &'p str,
        _default: bool,
    ) -> std::borrow::Cow<'b, str> {
        // Rebuild the prompt on EVERY render with the current time/CPU/sudo,
        // so a periodic tick (via rustyline's ExternalPrinter) keeps it live.
        let exit = self.last_exit.load(Ordering::Relaxed);
        let dur = std::time::Duration::from_nanos(self.last_duration_ns.load(Ordering::Relaxed));
        std::borrow::Cow::Owned(self.prompt_style.format_prompt_static(exit, dur))
    }

    fn highlight_hint<'h>(&self, hint: &'h str) -> std::borrow::Cow<'h, str> {
        self.highlighter.highlight_hint(hint)
    }

    fn highlight_candidate<'c>(
        &self,
        candidate: &'c str,
        completion: rustyline::CompletionType,
    ) -> std::borrow::Cow<'c, str> {
        self.highlighter.highlight_candidate(candidate, completion)
    }

    fn highlight_char(&self, line: &str, pos: usize, kind: rustyline::highlight::CmdKind) -> bool {
        self.highlighter.highlight_char(line, pos, kind)
    }
}

impl rustyline::validate::Validator for WaashHelper {
    fn validate(
        &self,
        ctx: &mut rustyline::validate::ValidationContext<'_>,
    ) -> Result<rustyline::validate::ValidationResult, rustyline::error::ReadlineError> {
        Ok(self.highlighter.validate(ctx))
    }

    fn validate_while_typing(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::PromptConfig;
    use std::time::Duration;

    #[test]
    fn test_sudo_variable_substituted() {
        let mut config = PromptConfig::default();
        config.template = "{sudo}{prompt}".to_string();
        let mut style = PromptStyle::from_config(&config);
        let out = style.format_prompt(0, Duration::ZERO);
        // The {sudo} placeholder must be replaced (never left literal).
        assert!(!out.contains("{sudo}"));
    }

    #[test]
    fn test_sudo_segment_no_panic() {
        // Returns a String regardless of the current user's sudo state
        // (empty when not verified, "⚡ " when verified, "# " as root).
        let _: String = sudo_segment();
    }

    #[test]
    fn test_sudo_segment_root_branch() {
        // When running as root the segment must contain the `#` marker.
        if nix::unistd::geteuid().is_root() {
            assert!(sudo_segment().contains('#'));
        }
    }
}
