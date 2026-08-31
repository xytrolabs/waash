//! Configuration system for WAASH.
//!
//! Reads from `~/.config/waash/config.toml` with reasonable defaults.
//! Controls: prompt style, colors, aliases, history, and behavior.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Top-level WAASH configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WaashConfig {
    /// Prompt customization
    #[serde(default)]
    pub prompt: PromptConfig,

    /// Syntax highlighting theme
    #[serde(default)]
    pub theme: ThemeConfig,

    /// Shell behavior
    #[serde(default)]
    pub shell: ShellConfig,

    /// Aliases (name → expansion)
    #[serde(default)]
    pub aliases: Vec<AliasEntry>,

    /// Key bindings
    #[serde(default)]
    pub keybindings: Vec<KeyBinding>,

    /// Path to the Indent runtime binary (for .waash scripts)
    /// If not set, auto-detected from PATH / common locations.
    #[serde(default)]
    pub indent_binary: Option<String>,

    /// Enable Indent-based scripting (if false, uses POSIX shell mode)
    #[serde(default = "default_true")]
    pub indent_scripting: bool,
}

/// How the prompt looks.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptConfig {
    /// Template string with substitutions:
    ///   {user} {host} {dir} {git_branch} {exit_code} {prompt_char}
    /// Default: "{user}@{host} {dir} {git} {prompt}"
    #[serde(default = "default_prompt_template")]
    pub template: String,

    /// Character shown in the prompt (default ❯ on success, ❯ on error)
    #[serde(default = "default_prompt_char_ok")]
    pub char_ok: String,

    /// Character shown when last command failed
    #[serde(default = "default_prompt_char_err")]
    pub char_err: String,

    /// Show exit code in prompt when non-zero
    #[serde(default = "default_true")]
    pub show_exit_code: bool,

    /// Show git branch/dirty status in prompt
    #[serde(default = "default_true")]
    pub show_git: bool,

    /// Shorten directory paths (show only first letter of parents)
    #[serde(default = "default_true")]
    pub shorten_path: bool,

    /// Show hostname in prompt
    #[serde(default = "default_true")]
    pub show_hostname: bool,

    /// Prompt on the right side (RPROMPT) — shown dimmed
    #[serde(default)]
    pub right_prompt: Option<String>,
}

/// Syntax highlighting color theme.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThemeConfig {
    /// Color for valid commands
    #[serde(default = "default_command_color")]
    pub command: String,

    /// Color for builtin commands
    #[serde(default = "default_builtin_color")]
    pub builtin: String,

    /// Color for invalid/unknown commands
    #[serde(default = "default_error_color")]
    pub error_command: String,

    /// Color for strings (single and double quoted)
    #[serde(default = "default_string_color")]
    pub string: String,

    /// Color for variable references ($VAR)
    #[serde(default = "default_variable_color")]
    pub variable: String,

    /// Color for operators (|, ;, &&, etc.)
    #[serde(default = "default_operator_color")]
    pub operator: String,

    /// Color for options/flags (-f, --flag)
    #[serde(default = "default_flag_color")]
    pub flag: String,

    /// Color for file paths
    #[serde(default = "default_path_color")]
    pub path: String,

    /// Color for comments (# ...)
    #[serde(default = "default_comment_color")]
    pub comment: String,

    /// Dimmed hint text (autosuggestions)
    #[serde(default = "default_hint_color")]
    pub hint: String,
}

/// Shell behavior settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShellConfig {
    /// Show welcome banner on startup
    #[serde(default = "default_true")]
    pub show_banner: bool,

    /// Maximum history entries
    #[serde(default = "default_history_size")]
    pub history_size: usize,

    /// History file path (relative to config dir)
    #[serde(default = "default_history_file")]
    pub history_file: String,

    /// Enable FISH-style autosuggestions
    #[serde(default = "default_true")]
    pub autosuggestions: bool,

    /// Enable syntax highlighting
    #[serde(default = "default_true")]
    pub syntax_highlighting: bool,

    /// Case-insensitive tab completion
    #[serde(default)]
    pub case_insensitive_completion: bool,

    /// Show completion list automatically
    #[serde(default = "default_true")]
    pub auto_completion: bool,

    /// Number of spaces to use for tabs
    #[serde(default = "default_tab_width")]
    pub tab_width: usize,

    /// Edit mode: "emacs" or "vi"
    #[serde(default = "default_edit_mode")]
    pub edit_mode: String,

    /// Keep the prompt live-updating (time, CPU load, sudo badge) while
    /// waiting for input. Uses a background thread + rustyline's external
    /// printer, so the prompt re-renders ~every second.
    ///
    /// WARNING: the background repaint races with the Enter key and can
    /// SWALLOW the command you type right after launching a background job
    /// (e.g. `sleep 3 &` then `echo hi` — the echo never runs). This is a
    /// known limitation of rustyline's external-printer path. Defaults to
    /// OFF for reliability; enable only if you accept the risk.
    #[serde(default = "default_false")]
    pub live_refresh: bool,

    /// WAASH commands to run once at interactive startup — acts like a
    /// `~/.waashrc`. Runs after login profiles are sourced and aliases are
    /// loaded, so you can `alias`/`export`/`source` here.
    #[serde(default)]
    pub startup_commands: Vec<String>,

    /// Ctrl+Z job control: run each foreground command in its own process
    /// group with the terminal handed to it, so pressing Ctrl+Z suspends the
    /// command instead of the whole shell. You can then resume it in the
    /// background with `bg` (or `fg`). Disable only if you hit terminal
    /// oddities.
    #[serde(default = "default_true")]
    pub job_control: bool,

    /// Single-key "move to background" shortcut. While a foreground command
    /// runs, pressing this key backgrounds it (it keeps running in the
    /// background and the prompt returns). Default `Ctrl+B`.
    #[serde(default = "default_bg_shortcut")]
    pub bg_shortcut: String,

    /// When you background a foreground-started command with the shortcut, its
    /// output stays on the terminal (a running process can't have its output
    /// redirected). When enabled, WAASH prints a one-line hint reminding you
    /// how to redirect it to a log instead.
    #[serde(default = "default_true")]
    pub bg_hint: bool,
}

/// A single alias entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AliasEntry {
    pub name: String,
    pub value: String,
}

/// A custom key binding.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeyBinding {
    /// Key sequence (e.g., "Ctrl+N", "Alt+Left")
    pub key: String,
    /// Action: "history_search_forward", "history_search_backward", etc.
    pub action: String,
}

// ── Default value helpers ──

fn default_prompt_template() -> String {
    // macOS Terminal-inspired: user@host dir (git) on line 1, `%` on line 2.
    // The inline right-info (battery/load/time/duration) is added by the
    // prompt renderer on the first line automatically.
    "{separator} {user}@{host} {dir} {git}{venv}{newline}{sudo}{exit_code}{prompt} ".into()
}

fn default_prompt_char_ok() -> String {
    "%".into()
}

fn default_prompt_char_err() -> String {
    "%".into()
}

fn default_command_color() -> String {
    "blue".into()
}

fn default_builtin_color() -> String {
    "bright blue".into()
}

fn default_error_color() -> String {
    "red".into()
}

fn default_string_color() -> String {
    "yellow".into()
}

fn default_variable_color() -> String {
    "cyan".into()
}

fn default_operator_color() -> String {
    "bright magenta".into()
}

fn default_flag_color() -> String {
    "green".into()
}

fn default_path_color() -> String {
    "cyan".into()
}

fn default_comment_color() -> String {
    "bright black".into()
}

fn default_hint_color() -> String {
    "bright black".into()
}

fn default_true() -> bool {
    true
}

fn default_false() -> bool {
    false
}

fn default_history_size() -> usize {
    100_000
}

fn default_history_file() -> String {
    "history".into()
}

fn default_tab_width() -> usize {
    4
}

fn default_edit_mode() -> String {
    "emacs".into()
}

fn default_bg_shortcut() -> String {
    "Ctrl+B".into()
}

// ── Config loading ──

impl Default for WaashConfig {
    fn default() -> Self {
        Self {
            prompt: PromptConfig::default(),
            theme: ThemeConfig::default(),
            shell: ShellConfig::default(),
            aliases: Vec::new(),
            keybindings: Vec::new(),
            indent_binary: None,
            indent_scripting: true,
        }
    }
}

impl Default for PromptConfig {
    fn default() -> Self {
        Self {
            template: default_prompt_template(),
            char_ok: default_prompt_char_ok(),
            char_err: default_prompt_char_err(),
            show_exit_code: true,
            show_git: true,
            shorten_path: true,
            show_hostname: true,
            right_prompt: None,
        }
    }
}

impl Default for ThemeConfig {
    fn default() -> Self {
        Self {
            command: default_command_color(),
            builtin: default_builtin_color(),
            error_command: default_error_color(),
            string: default_string_color(),
            variable: default_variable_color(),
            operator: default_operator_color(),
            flag: default_flag_color(),
            path: default_path_color(),
            comment: default_comment_color(),
            hint: default_hint_color(),
        }
    }
}

impl Default for ShellConfig {
    fn default() -> Self {
        Self {
            show_banner: true,
            history_size: default_history_size(),
            history_file: default_history_file(),
            autosuggestions: true,
            syntax_highlighting: true,
            case_insensitive_completion: false,
            auto_completion: true,
            tab_width: default_tab_width(),
            edit_mode: default_edit_mode(),
            live_refresh: false,
            startup_commands: Vec::new(),
            job_control: true,
            bg_shortcut: default_bg_shortcut(),
            bg_hint: true,
        }
    }
}

/// Load configuration from the standard location.
pub fn load_config() -> WaashConfig {
    let config_path = config_path();

    if config_path.exists() {
        match std::fs::read_to_string(&config_path) {
            Ok(contents) => match toml::from_str(&contents) {
                Ok(config) => {
                    log::info!("Loaded config from {}", config_path.display());
                    return config;
                }
                Err(e) => {
                    log::warn!("Failed to parse config: {}", e);
                }
            },
            Err(e) => {
                log::warn!("Failed to read config: {}", e);
            }
        }
    }

    WaashConfig::default()
}

/// Path to the config file: ~/.config/waash/config.toml
pub fn config_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("waash")
        .join("config.toml")
}

/// Generate and save a default config file.
pub fn init_config() -> std::io::Result<PathBuf> {
    let path = config_path();

    if path.exists() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            format!("Config already exists at {}", path.display()),
        ));
    }

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let default = WaashConfig::default();
    let toml_str = toml::to_string_pretty(&default)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?;

    // Add helpful comments (toml doesn't support comments natively, so we prepend)
    let header = r#"# WAASH Configuration
# Location: ~/.config/waash/config.toml
#
# This file controls everything about your WAASH experience.
#
# ═══════════════════════════════════════════════════════════
# PROMPT TEMPLATE VARIABLES
# ═══════════════════════════════════════════════════════════
#   {user}       — username (colored cyan)
#   {host}       — hostname (colored green, only if show_hostname)
#   {dir}        — current directory, shortened (colored blue)
#   {full_dir}   — current directory, full path (colored blue)
#   {git}        — git branch + ahead/behind/dirty (e.g. "(main ↑2 ✗)")
#   {venv}       — Python virtualenv name with 🐍 (e.g. "🐍myenv")
#   {duration}   — last command duration (~847ms, ~3.2s, ~2m 34s)
#   {exit_code}  — error code from last command (✗127, red, only non-zero)
#   {prompt}     — the prompt character (❯ or ❯ on error)
#   {time_icon}  — time-of-day emoji (☀️ 🌤 🌙 🌃)
#   {time}       — current time (HH:MM:SS)
#   {date}       — current date (YYYY-MM-DD)
#   {shlvl}      — shell nesting level (empty if level 1)
#   {jobs}       — background job count (empty if none)
#   {shell}      — "waash"
#   {version}    — shell version
#   {newline}    — explicit newline
#   \n           — also works as newline (backslash-n in toml)
#
# ═══════════════════════════════════════════════════════════
# AVAILABLE COLORS
# ═══════════════════════════════════════════════════════════
#   black, red, green, yellow, blue, magenta, cyan, white
#   bright black, bright red, bright green, bright yellow,
#   bright blue, bright magenta, bright cyan, bright white
#
# ═══════════════════════════════════════════════════════════
# EXAMPLE PROMPTS
# ═══════════════════════════════════════════════════════════
#   Simple FISH-like:
#     template = "{user}@{host} {dir} {git}{prompt} "
#
#   Powerline-style 2-line with timing:
#     template = "{time_icon} {time} {dir} {git}{venv}\n{exit_code}{prompt} "
#
#   Minimalist:
#     template = "{dir} {prompt} "
#
#   Info-rich developer prompt:
#     template = "{date} {time} {user}@{host} {dir} {git}{venv}\n{duration}{exit_code}{prompt} "

"#;

    let full_content = format!("{}{}", header, toml_str);
    std::fs::write(&path, full_content)?;

    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config_roundtrip() {
        let config = WaashConfig::default();
        let toml_str = toml::to_string_pretty(&config).unwrap();
        let parsed: WaashConfig = toml::from_str(&toml_str).unwrap();
        assert_eq!(parsed.prompt.template, config.prompt.template);
        assert_eq!(parsed.theme.command, config.theme.command);
    }

    #[test]
    fn test_config_path() {
        let path = config_path();
        assert!(path.ends_with("waash/config.toml"));
    }
}
