//! Syntax highlighting for WAASH — like FISH's real-time syntax coloring.
//!
//! Colors commands, strings, variables, operators, and other shell constructs
//! as the user types, giving immediate visual feedback.
//! Colors are fully customizable via the theme config.

use crate::builtins;
use crate::config::ThemeConfig;
use ansi_term::{Colour, Style};

/// Syntax highlighter with customizable theme.
pub struct WaashHighlighter {
    theme: ThemeConfig,
}

impl WaashHighlighter {
    pub fn with_theme(theme: ThemeConfig) -> Self {
        Self { theme }
    }

    /// Highlight a line of shell input with FISH-like colors.
    pub fn highlight<'l>(&self, line: &'l str, _pos: usize) -> std::borrow::Cow<'l, str> {
        if line.is_empty() {
            return std::borrow::Cow::Borrowed(line);
        }

        let highlighted = self.colorize(line);
        std::borrow::Cow::Owned(highlighted)
    }

    /// Parse a color name from the config into an ansi_term Colour.
    fn parse_color(&self, name: &str) -> Colour {
        match name.to_lowercase().as_str() {
            "black" => Colour::Black,
            "red" => Colour::Red,
            "green" => Colour::Green,
            "yellow" => Colour::Yellow,
            "blue" => Colour::Blue,
            "magenta" | "purple" => Colour::Purple,
            "cyan" => Colour::Cyan,
            "white" => Colour::White,
            "bright black" | "grey" | "gray" => Colour::Fixed(8),
            "bright red" => Colour::Fixed(9),
            "bright green" => Colour::Fixed(10),
            "bright yellow" => Colour::Fixed(11),
            "bright blue" => Colour::Fixed(12),
            "bright magenta" | "bright purple" => Colour::Fixed(13),
            "bright cyan" => Colour::Fixed(14),
            "bright white" => Colour::Fixed(15),
            _ => Colour::White,
        }
    }

    /// Style a word based on its position and content.
    fn style_word(&self, word: &str, is_command: bool) -> String {
        if word.starts_with('$') {
            return self.parse_color(&self.theme.variable).paint(word).to_string();
        }

        if word.starts_with('-') && word.len() > 1 && !word.starts_with("--") {
            return self.parse_color(&self.theme.flag).paint(word).to_string();
        }

        if word.starts_with("--") {
            return self.parse_color(&self.theme.flag).paint(word).to_string();
        }

        if is_command {
            if builtins::is_builtin(word) {
                return self.parse_color(&self.theme.builtin).bold().paint(word).to_string();
            } else if self.is_external_command(word) {
                return self.parse_color(&self.theme.command).paint(word).to_string();
            } else {
                return self.parse_color(&self.theme.error_command).paint(word).to_string();
            }
        }

        if word.contains('/') || word.starts_with('.') {
            return self.parse_color(&self.theme.path).underline().paint(word).to_string();
        }

        word.to_string()
    }

    fn colorize(&self, line: &str) -> String {
        let mut result = String::with_capacity(line.len() * 2);
        let mut chars = line.chars().peekable();
        let mut in_single_quote = false;
        let mut in_double_quote = false;
        let mut in_comment = false;
        let mut word_start = true; // True at the start of a new word
        let mut current_word = String::new();

        while let Some(c) = chars.next() {
            match c {
                '#' if !in_single_quote && !in_double_quote => {
                    if !current_word.is_empty() {
                        let styled = self.style_word(&current_word, word_start);
                        result.push_str(&styled);
                        current_word.clear();
                    }
                    in_comment = true;
                    result.push_str(
                        &self.parse_color(&self.theme.comment)
                            .paint("#")
                            .to_string(),
                    );
                }
                '\'' if !in_double_quote && !in_comment => {
                    if !current_word.is_empty() {
                        let styled = self.style_word(&current_word, word_start);
                        result.push_str(&styled);
                        current_word.clear();
                    }
                    in_single_quote = !in_single_quote;
                    result.push_str(
                        &self.parse_color(&self.theme.string)
                            .paint("'")
                            .to_string(),
                    );
                }
                '"' if !in_single_quote && !in_comment => {
                    if !current_word.is_empty() {
                        let styled = self.style_word(&current_word, word_start);
                        result.push_str(&styled);
                        current_word.clear();
                    }
                    in_double_quote = !in_double_quote;
                    result.push_str(
                        &self.parse_color(&self.theme.string)
                            .paint("\"")
                            .to_string(),
                    );
                }
                '$' if !in_single_quote && !in_comment => {
                    current_word.push(c);
                }
                c if c.is_whitespace() && !in_single_quote && !in_double_quote => {
                    if !current_word.is_empty() {
                        let styled = self.style_word(&current_word, word_start);
                        result.push_str(&styled);
                        current_word.clear();
                    }
                    result.push(c);
                    word_start = true;
                    in_comment = false;
                }
                '|' | ';' | '&' | '<' | '>' if !in_single_quote && !in_double_quote && !in_comment =>
                {
                    if !current_word.is_empty() {
                        let styled = self.style_word(&current_word, word_start);
                        result.push_str(&styled);
                        current_word.clear();
                    }
                    result.push_str(
                        &self.parse_color(&self.theme.operator)
                            .bold()
                            .paint(c.to_string())
                            .to_string(),
                    );
                    word_start = true;
                }
                '(' | ')' | '{' | '}' if !in_single_quote && !in_double_quote && !in_comment => {
                    if !current_word.is_empty() {
                        let styled = self.style_word(&current_word, word_start);
                        result.push_str(&styled);
                        current_word.clear();
                    }
                    result.push_str(
                        &self.parse_color(&self.theme.operator)
                            .paint(c.to_string())
                            .to_string(),
                    );
                    word_start = true;
                }
                _ => {
                    current_word.push(c);
                    if !c.is_alphanumeric()
                        && c != '_'
                        && c != '-'
                        && c != '.'
                        && c != '/'
                        && c != '$'
                    {
                        word_start = false;
                    }
                }
            }
        }

        if !current_word.is_empty() {
            let styled = self.style_word(&current_word, word_start);
            result.push_str(&styled);
        }

        result
    }

    fn is_external_command(&self, name: &str) -> bool {
        if name.contains('/') {
            return std::path::Path::new(name).exists();
        }
        if let Ok(path) = std::env::var("PATH") {
            for dir in path.split(':') {
                if std::path::Path::new(dir).join(name).exists() {
                    return true;
                }
            }
        }
        false
    }

    // ── Rustyline trait methods ──

    pub fn highlight_hint<'h>(&self, hint: &'h str) -> std::borrow::Cow<'h, str> {
        std::borrow::Cow::Owned(
            Style::new()
                .dimmed()
                .fg(self.parse_color(&self.theme.hint))
                .paint(hint)
                .to_string(),
        )
    }

    pub fn highlight_candidate<'c>(
        &self,
        candidate: &'c str,
        _completion: rustyline::CompletionType,
    ) -> std::borrow::Cow<'c, str> {
        std::borrow::Cow::Borrowed(candidate)
    }

    pub fn highlight_char(&self, _line: &str, _pos: usize, _kind: rustyline::highlight::CmdKind) -> bool {
        false
    }

    pub fn validate(
        &self,
        _ctx: &mut rustyline::validate::ValidationContext<'_>,
    ) -> rustyline::validate::ValidationResult {
        rustyline::validate::ValidationResult::Valid(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_highlight_simple_command() {
        let h = WaashHighlighter::with_theme(crate::config::ThemeConfig::default());
        let result = h.highlight("echo hello", 0);
        // Should contain ANSI codes
        assert!(!result.is_empty());
        assert!(result.contains("echo"));
    }

    #[test]
    fn test_highlight_variable() {
        let h = WaashHighlighter::with_theme(crate::config::ThemeConfig::default());
        let result = h.highlight("echo $HOME", 0);
        assert!(result.contains("$HOME"));
    }

    #[test]
    fn test_highlight_string() {
        let h = WaashHighlighter::with_theme(crate::config::ThemeConfig::default());
        let result = h.highlight("echo \"hello world\"", 0);
        assert!(result.contains("hello world"));
    }

    #[test]
    fn test_highlight_comment() {
        let h = WaashHighlighter::with_theme(crate::config::ThemeConfig::default());
        let result = h.highlight("echo foo # comment", 0);
        assert!(result.contains("comment"));
    }
}
