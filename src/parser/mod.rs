//! Parser — converts a stream of tokens into an AST.
//!
//! Handles the full shell grammar: pipelines, logical operators,
//! redirections, heredocs, command substitution, etc.

pub mod ast;

use crate::lexer::token::{SpannedToken, Token};
use crate::lexer::Scanner;
use ast::*;

/// Parse error with source location.
#[derive(Debug, Clone)]
pub struct ParseError {
    pub message: String,
    /// Byte span in the input (reserved for future editor integration).
    #[allow(dead_code)]
    pub span: Option<(usize, usize)>,
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Parse error: {}", self.message)
    }
}

impl std::error::Error for ParseError {}

/// The parser consumes tokens from a Scanner and builds an AST.
pub struct Parser {
    scanner: Scanner,
    /// Lookahead token
    current: Option<SpannedToken>,
}

impl Parser {
    pub fn new(scanner: Scanner) -> Self {
        Self {
            scanner,
            current: None,
        }
    }

    /// Parse input into a complete script.
    pub fn parse(&mut self) -> Result<Script, ParseError> {
        self.advance()?;
        let mut commands = Vec::new();
        // Heredoc bodies appear as tokens after the line they belong to.
        // Collect them in source order, then attach to the AST at the end.
        let mut heredoc_bodies: Vec<String> = Vec::new();

        while !self.is_at_end() {
            if self.check(&Token::Newline)
                || self.check(&Token::Semicolon)
                || self.is_comment()
            {
                self.advance()?;
                continue;
            }
            if self.current_is_heredoc_body() {
                if let Some(t) = self.current.as_ref() {
                    if let Token::HeredocBody(b) = &t.token {
                        heredoc_bodies.push(b.clone());
                    }
                }
                self.advance()?;
                continue;
            }
            commands.push(self.parse_command()?);
        }

        let mut script = Script { commands };
        // Attach collected bodies to the heredocs in source order.
        crate::heredoc::assign_heredoc_bodies(&mut script, &heredoc_bodies);
        Ok(script)
    }

    // ── Command parsing ──

    fn parse_command(&mut self) -> Result<Command, ParseError> {
        let mut cmd = self.parse_simple_or_pipeline()?;

        // Handle logical operators (left-associative)
        loop {
            if self.check(&Token::AndAnd) {
                self.advance()?;
                let right = self.parse_simple_or_pipeline()?;
                cmd = Command::And(Box::new(cmd), Box::new(right));
            } else if self.check(&Token::OrOr) {
                self.advance()?;
                let right = self.parse_simple_or_pipeline()?;
                cmd = Command::Or(Box::new(cmd), Box::new(right));
            } else {
                break;
            }
        }

        // Handle background
        if self.check(&Token::Background) {
            self.advance()?;
            cmd = Command::Background(Box::new(cmd));
        }

        Ok(cmd)
    }

    fn parse_simple_or_pipeline(&mut self) -> Result<Command, ParseError> {
        // Check for subshell or group
        if self.check(&Token::LParen) {
            return self.parse_subshell();
        }
        if self.check(&Token::LBrace) {
            return self.parse_group();
        }

        let first = self.parse_simple_command()?;

        // Check if this begins a pipeline
        if self.check(&Token::Pipe) || self.check(&Token::PipeStderr) {
            let pipe_stderr = self.check(&Token::PipeStderr);
            self.advance()?;

            let mut commands = vec![first];

            // Parse rest of pipeline
            loop {
                commands.push(self.parse_simple_command()?);
                if self.check(&Token::Pipe) || self.check(&Token::PipeStderr) {
                    if self.check(&Token::PipeStderr) {
                        // |& only valid as first pipe operator conceptually,
                        // but we'll track it
                    }
                    self.advance()?;
                } else {
                    break;
                }
            }

            return Ok(Command::Pipeline(Pipeline {
                commands,
                pipe_stderr,
            }));
        }

        Ok(Command::Simple(first))
    }

    fn parse_subshell(&mut self) -> Result<Command, ParseError> {
        self.expect(&Token::LParen)?;
        let mut commands = Vec::new();

        while !self.check(&Token::RParen) && !self.is_at_end() {
            if self.check(&Token::Newline)
                || self.check(&Token::Semicolon)
                || self.is_comment()
            {
                self.advance()?;
                continue;
            }
            commands.push(self.parse_command()?);
        }

        self.expect(&Token::RParen)?;
        Ok(Command::Subshell(Script { commands }))
    }

    fn parse_group(&mut self) -> Result<Command, ParseError> {
        self.expect(&Token::LBrace)?;
        let mut commands = Vec::new();

        while !self.check(&Token::RBrace) && !self.is_at_end() {
            if self.check(&Token::Newline)
                || self.check(&Token::Semicolon)
                || self.is_comment()
            {
                self.advance()?;
                continue;
            }
            commands.push(self.parse_command()?);
        }

        self.expect(&Token::RBrace)?;
        Ok(Command::Group(Script { commands }))
    }

    fn parse_simple_command(&mut self) -> Result<SimpleCommand, ParseError> {
        let mut env_vars = Vec::new();
        let mut args = Vec::new();
        let mut redirections = Vec::new();
        let mut heredoc: Option<Heredoc> = None;
        let mut herestring: Option<String> = None;

        // Skip leading comments (e.g. `#!` shebangs at the top of a file).
        while self.is_comment() {
            self.advance()?;
        }

        // Parse optional env vars: VAR=val
        while self.peek_token_is_assignment() {
            let (name, value) = self.parse_env_assignment()?;
            env_vars.push((name, value));
        }

        // Parse program name (first word)
        let program = if let Some(tok) = self.consume_word_sequence() {
            tok
        } else if !env_vars.is_empty() {
            // Only env vars, no command — valid in bash
            String::new()
        } else {
            return Err(ParseError {
                message: "Expected a command".to_string(),
                span: None,
            });
        };
        loop {
            if self.is_at_end() || self.check(&Token::Newline) || self.check(&Token::Semicolon)
                || self.check(&Token::Pipe) || self.check(&Token::PipeStderr)
                || self.check(&Token::AndAnd) || self.check(&Token::OrOr)
                || self.check(&Token::Background) || self.check(&Token::RParen)
                || self.check(&Token::RBrace) || self.is_comment()
            {
                break;
            }

            if self.is_fd_prefixed_redirection() {
                let fd = self.consume_fd_number()?;
                let redir = self.parse_redirection_with_fd(fd)?;
                redirections.push(redir);
            } else if self.is_redirection() {
                let redir = self.parse_redirection()?;
                redirections.push(redir);
            } else if self.check(&Token::HeredocStart) || self.check(&Token::HeredocStartStrip) {
                let hd = self.parse_heredoc()?;
                heredoc = Some(hd);
            } else if self.check(&Token::HereString) {
                self.advance()?;
                if let Some(word) = self.consume_word_sequence() {
                    herestring = Some(word);
                }
            } else if let Some(word) = self.consume_word_sequence() {
                args.push(word);
            } else {
                break;
            }
        }

        Ok(SimpleCommand {
            program,
            args,
            redirections,
            heredoc,
            herestring,
            env_vars,
        })
    }

    // ── Helpers ──

    /// Check if the current token could be a VAR=val assignment.
    fn peek_token_is_assignment(&self) -> bool {
        // Look at the first word-part of the upcoming sequence: it must look
        // like NAME=... with a valid identifier as the name.
        if let Some(ref t) = self.current {
            let text = match &t.token {
                Token::Word(s) => s.as_str(),
                Token::SingleQuoted(s) => s.as_str(),
                Token::DoubleQuoted(s) => s.as_str(),
                _ => return false,
            };
            if let Some((name, _)) = text.split_once('=') {
                return !name.is_empty()
                    && name.chars().next().map_or(false, |c| c.is_alphabetic() || c == '_')
                    && name.chars().all(|c| c.is_alphanumeric() || c == '_');
            }
        }
        false
    }

    fn parse_env_assignment(&mut self) -> Result<(String, String), ParseError> {
        let word = self
            .consume_word_sequence()
            .ok_or_else(|| ParseError {
                message: "Expected VAR=value assignment".to_string(),
                span: None,
            })?;
        if let Some((name, value)) = word.split_once('=') {
            Ok((name.to_string(), value.to_string()))
        } else {
            Err(ParseError {
                message: "Expected VAR=value assignment".to_string(),
                span: None,
            })
        }
    }

    fn parse_redirection(&mut self) -> Result<Redirection, ParseError> {
        self.parse_redirection_with_fd(0)
    }

    /// Whether the current token is an all-digit word immediately followed by
    /// a redirection operator (e.g. `2>`, `2>&1`, `2>>`, `3<`). Such digits
    /// are a file-descriptor prefix, not an argument.
    ///
    /// IMPORTANT: this only applies when the digit is *adjacent* to the
    /// operator (no whitespace), matching bash. `ls 2>err.txt` redirects fd 2,
    /// but `seq 1 3 > out.txt` keeps `3` as an argument — `3 >` has a space.
    /// Without this rule, numeric arguments before `>` were swallowed as fds.
    fn is_fd_prefixed_redirection(&self) -> bool {
        let current = match self.current.as_ref() {
            Some(t) => t,
            None => return false,
        };
        let digit = match &current.token {
            Token::Word(s) if !s.is_empty() && s.chars().all(|c| c.is_ascii_digit()) => true,
            _ => return false,
        };
        let _ = digit;
        let peek = match self.scanner.peek() {
            Some(t) => t,
            None => return false,
        };
        let is_redir = matches!(
            &peek.token,
            Token::RedirectInput
                | Token::RedirectOutput
                | Token::RedirectAppend
                | Token::RedirectDupOutput
                | Token::RedirectDupInput
                | Token::RedirectReadWrite
                | Token::RedirectBoth
        );
        is_redir && current.span.end == peek.span.start
    }

    fn consume_fd_number(&mut self) -> Result<i32, ParseError> {
        if let Some(ref t) = self.current {
            if let Token::Word(s) = &t.token {
                if let Ok(n) = s.parse::<i32>() {
                    self.advance()?;
                    return Ok(n);
                }
            }
        }
        Err(ParseError {
            message: "Expected file descriptor".to_string(),
            span: None,
        })
    }

    fn parse_redirection_with_fd(&mut self, fd: i32) -> Result<Redirection, ParseError> {
        let redir = match self.current.as_ref().map(|t| &t.token) {
            Some(Token::RedirectInput) => {
                self.advance()?;
                if fd != 0 {
                    if let Some(file) = self.consume_word() {
                        Redirection::FdInput(fd, file)
                    } else {
                        return Err(ParseError {
                            message: "Expected filename after <".to_string(),
                            span: None,
                        });
                    }
                } else if let Some(file) = self.consume_word() {
                    Redirection::Input(file)
                } else {
                    return Err(ParseError {
                        message: "Expected filename after <".to_string(),
                        span: None,
                    });
                }
            }
            Some(Token::RedirectOutput) => {
                self.advance()?;
                if let Some(file) = self.consume_word() {
                    if fd != 0 && fd != 1 {
                        Redirection::FdOutput(fd, file)
                    } else {
                        Redirection::Output(file)
                    }
                } else {
                    return Err(ParseError {
                        message: "Expected filename after >".to_string(),
                        span: None,
                    });
                }
            }
            Some(Token::RedirectAppend) => {
                self.advance()?;
                if let Some(file) = self.consume_word() {
                    if fd != 0 && fd != 1 {
                        Redirection::FdAppend(fd, file)
                    } else {
                        Redirection::Append(file)
                    }
                } else {
                    return Err(ParseError {
                        message: "Expected filename after >>".to_string(),
                        span: None,
                    });
                }
            }
            Some(Token::RedirectReadWrite) => {
                self.advance()?;
                if let Some(file) = self.consume_word() {
                    if fd != 0 {
                        Redirection::FdReadWrite(fd, file)
                    } else {
                        Redirection::ReadWrite(file)
                    }
                } else {
                    return Err(ParseError {
                        message: "Expected filename after <>".to_string(),
                        span: None,
                    });
                }
            }
            Some(Token::RedirectDupOutput) => {
                self.advance()?;
                if let Some(word) = self.consume_digit() {
                    let target = word.parse::<i32>().unwrap_or(1);
                    if fd != 0 && fd != 1 {
                        Redirection::FdDup(target, fd)
                    } else {
                        Redirection::DupOutput(target)
                    }
                } else if fd != 0 {
                    Redirection::FdDup(fd, fd)
                } else {
                    return Err(ParseError {
                        message: "Expected fd after >&".to_string(),
                        span: None,
                    });
                }
            }
            Some(Token::RedirectDupInput) => {
                self.advance()?;
                if let Some(word) = self.consume_digit() {
                    let target = word.parse::<i32>().unwrap_or(0);
                    if fd != 0 {
                        Redirection::FdDup(target, fd)
                    } else {
                        Redirection::DupInput(target)
                    }
                } else {
                    return Err(ParseError {
                        message: "Expected fd after <&".to_string(),
                        span: None,
                    });
                }
            }
            Some(Token::RedirectBoth) => {
                self.advance()?;
                if let Some(file) = self.consume_word() {
                    Redirection::Both(file)
                } else {
                    return Err(ParseError {
                        message: "Expected filename after &>".to_string(),
                        span: None,
                    });
                }
            }
            _ => {
                return Err(ParseError {
                    message: "Expected redirection operator".to_string(),
                    span: None,
                });
            }
        };

        Ok(redir)
    }

    fn parse_heredoc(&mut self) -> Result<Heredoc, ParseError> {
        let strip_tabs = self.check(&Token::HeredocStartStrip);
        self.advance()?;

        // Get delimiter word (maybe quoted to prevent expansion)
        let (delimiter, expand) = if let Some(ref t) = self.current {
            match &t.token {
                Token::Word(s) => {
                    let delim = s.clone();
                    let expand = true;
                    self.advance()?;
                    (delim, expand)
                }
                Token::SingleQuoted(s) => {
                    let delim = s.clone();
                    let expand = false;
                    self.advance()?;
                    (delim, expand)
                }
                Token::DoubleQuoted(s) => {
                    let delim = s.clone();
                    let expand = true;
                    self.advance()?;
                    (delim, expand)
                }
                _ => {
                    return Err(ParseError {
                        message: "Expected heredoc delimiter".to_string(),
                        span: None,
                    });
                }
            }
        } else {
            return Err(ParseError {
                message: "Expected heredoc delimiter".to_string(),
                span: None,
            });
        };

        // Body is collected by the lexer as a HeredocBody token that appears
        // after the command's terminating newline. The top-level parse()
        // collects these and attaches them to the AST in source order.
        Ok(Heredoc {
            delimiter,
            strip_tabs,
            expand,
            body: String::new(),
        })
    }

    fn is_redirection(&self) -> bool {
        matches!(
            self.current.as_ref().map(|t| &t.token),
            Some(
                Token::RedirectInput
                    | Token::RedirectOutput
                    | Token::RedirectAppend
                    | Token::RedirectDupOutput
                    | Token::RedirectDupInput
                    | Token::RedirectBoth
                    | Token::RedirectReadWrite
            )
        )
    }

    /// Consume a single word-part token, returning its RAW text.
    ///
    /// Variable references, special vars, and command substitutions are kept
    /// unexpanded (`$HOME`, `$?`, `$(cmd)`) so the executor can expand them
    /// per-command at execution time (which is required for things like
    /// `export X=1; echo $X` and `false; echo $?`).
    ///
    /// Single-quoted content is escaped so the exec-time expander won't treat
    /// `$`, `\`, or backticks as special.
    fn consume_word(&mut self) -> Option<String> {
        match self.current.as_ref().map(|t| &t.token) {
            Some(Token::Word(s)) => {
                let s = s.clone();
                self.advance().ok()?;
                Some(s)
            }
            Some(Token::SingleQuoted(s)) => {
                let s = s.clone();
                self.advance().ok()?;
                // Single quotes = literal: escape for the exec-time expander.
                Some(escape_literal(&s))
            }
            Some(Token::DoubleQuoted(s)) => {
                let s = s.clone();
                self.advance().ok()?;
                // Keep raw; executor expands variables/substitutions.
                Some(s)
            }
            Some(Token::Variable(s)) => {
                let s = s.clone();
                self.advance().ok()?;
                Some(format!("${}", s))
            }
            Some(Token::CmdSubstitution(cmd)) => {
                let cmd = cmd.clone();
                self.advance().ok()?;
                Some(format!("$({})", cmd))
            }
            Some(Token::SpecialVar(s)) => {
                let s = s.clone();
                self.advance().ok()?;
                Some(s)
            }
            _ => None,
        }
    }

    /// Consume a run of adjacent word-parts, concatenating them into one word.
    ///
    /// Shell concatenates adjacent parts with no whitespace between them:
    /// `a"b"c`, `FOO="$HOME/x"`, `pre$VARpost`, `x$(cmd)y`. Each part is
    /// expanded appropriately (quoted vs. variable vs. command substitution).
    fn consume_word_sequence(&mut self) -> Option<String> {
        if !is_word_part(&self.current.as_ref()?.token) {
            return None;
        }
        let mut result = String::new();
        loop {
            let end = self.current.as_ref()?.span.end;
            let seg = self.consume_word()?;
            result.push_str(&seg);
            // Continue only if the next token is BOTH adjacent (no gap) AND
            // a word part. A trailing newline/operator that abuts the word
            // (e.g. "world\n") must not be consumed — otherwise we'd fail
            // and drop the word we already built.
            let adjacent = match &self.current {
                Some(n) => n.span.start == end && is_word_part(&n.token),
                None => false,
            };
            if !adjacent {
                break;
            }
        }
        Some(result)
    }

    fn consume_digit(&mut self) -> Option<String> {
        match self.current.as_ref().map(|t| &t.token) {
            Some(Token::Word(s)) if s.chars().all(|c| c.is_ascii_digit()) => {
                let s = s.clone();
                self.advance().ok()?;
                Some(s)
            }
            _ => None,
        }
    }

    fn check(&self, expected: &Token) -> bool {
        self.current.as_ref().map(|t| &t.token == expected).unwrap_or(false)
    }

    /// Whether the current token is a comment (`# ...`).
    fn is_comment(&self) -> bool {
        matches!(self.current.as_ref().map(|t| &t.token), Some(Token::Comment(_)))
    }

    /// Whether the current token is a heredoc body.
    fn current_is_heredoc_body(&self) -> bool {
        matches!(self.current.as_ref().map(|t| &t.token), Some(Token::HeredocBody(_)))
    }

    fn is_at_end(&self) -> bool {
        matches!(self.current.as_ref().map(|t| &t.token), Some(Token::Eof) | None)
    }

    fn advance(&mut self) -> Result<(), ParseError> {
        self.current = self.scanner.next();
        Ok(())
    }

    fn expect(&mut self, expected: &Token) -> Result<(), ParseError> {
        if self.check(expected) {
            self.advance()
        } else {
            Err(ParseError {
                message: format!(
                    "Expected {:?}, got {:?}",
                    expected,
                    self.current.as_ref().map(|t| &t.token)
                ),
                span: None,
            })
        }
    }
}

/// Whether a token can be part of a (concatenated) word.
fn is_word_part(t: &Token) -> bool {
    matches!(
        t,
        Token::Word(_)
            | Token::SingleQuoted(_)
            | Token::DoubleQuoted(_)
            | Token::Variable(_)
            | Token::SpecialVar(_)
            | Token::CmdSubstitution(_)
    )
}

/// Escape single-quoted literal content so the exec-time word expander leaves
/// `$`, `\`, and backticks untouched.
fn escape_literal(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '$' => out.push_str("\\$"),
            '`' => out.push_str("\\`"),
            _ => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::Scanner;

    fn parse_input(input: &str) -> Result<Script, ParseError> {
        let mut scanner = Scanner::new(input.to_string());
        scanner.lex_all().map_err(|e| ParseError {
            message: e,
            span: None,
        })?;
        let mut parser = Parser::new(scanner);
        parser.parse()
    }

    #[test]
    fn test_simple_command() {
        let script = parse_input("echo hello world\n").unwrap();
        assert_eq!(script.commands.len(), 1);
        match &script.commands[0] {
            Command::Simple(cmd) => {
                assert_eq!(cmd.program, "echo");
                assert_eq!(cmd.args, vec!["hello", "world"]);
            }
            _ => panic!("Expected Simple command"),
        }
    }

    #[test]
    fn test_pipeline() {
        let script = parse_input("cat file | grep foo\n").unwrap();
        match &script.commands[0] {
            Command::Pipeline(p) => {
                assert_eq!(p.commands.len(), 2);
                assert_eq!(p.commands[0].program, "cat");
                assert_eq!(p.commands[1].program, "grep");
            }
            _ => panic!("Expected Pipeline"),
        }
    }

    #[test]
    fn test_logical_and() {
        let script = parse_input("make && ./run\n").unwrap();
        match &script.commands[0] {
            Command::And(_, _) => {}
            _ => panic!("Expected And"),
        }
    }

    #[test]
    fn test_background() {
        let script = parse_input("sleep 10 &\n").unwrap();
        match &script.commands[0] {
            Command::Background(_) => {}
            _ => panic!("Expected Background"),
        }
    }

    #[test]
    fn test_redirections() {
        let script = parse_input("cmd > out.txt 2>&1\n").unwrap();
        match &script.commands[0] {
            Command::Simple(cmd) => {
                assert!(!cmd.redirections.is_empty());
            }
            _ => panic!("Expected Simple command"),
        }
    }

    #[test]
    fn test_env_assignment() {
        let script = parse_input("FOO=bar env\n").unwrap();
        match &script.commands[0] {
            Command::Simple(cmd) => {
                assert_eq!(cmd.env_vars, vec![("FOO".to_string(), "bar".to_string())]);
                assert_eq!(cmd.program, "env");
            }
            _ => panic!("Expected Simple command"),
        }
    }

    #[test]
    fn test_semicolon_separator() {
        let script = parse_input("echo one; echo two; echo three\n").unwrap();
        assert_eq!(script.commands.len(), 3);
    }

    #[test]
    fn test_word_concatenation() {
        // Adjacent word parts join into a single word: pre"mid"post
        let script = parse_input("echo pre\"mid\"post\n").unwrap();
        match &script.commands[0] {
            Command::Simple(cmd) => {
                assert_eq!(cmd.args, vec!["premidpost".to_string()]);
            }
            _ => panic!("Expected Simple command"),
        }
    }

    #[test]
    fn test_variable_kept_raw_for_exec() {
        // The parser keeps $ references raw; the executor expands them.
        let script = parse_input("echo $HOME\n").unwrap();
        match &script.commands[0] {
            Command::Simple(cmd) => {
                assert_eq!(cmd.args, vec!["$HOME".to_string()]);
            }
            _ => panic!("Expected Simple command"),
        }
    }

    #[test]
    fn test_single_quoted_escaped() {
        // Single quotes are literal: content is escaped for the expander.
        let script = parse_input("echo 'a$b'\n").unwrap();
        match &script.commands[0] {
            Command::Simple(cmd) => {
                assert_eq!(cmd.args, vec!["a\\$b".to_string()]);
            }
            _ => panic!("Expected Simple command"),
        }
    }

    #[test]
    fn test_command_substitution_token() {
        let script = parse_input("echo $(date)\n").unwrap();
        match &script.commands[0] {
            Command::Simple(cmd) => {
                assert_eq!(cmd.args, vec!["$(date)".to_string()]);
            }
            _ => panic!("Expected Simple command"),
        }
    }

    #[test]
    fn test_fd_redirection_2_ampersand_1() {
        let script = parse_input("cmd 2>&1\n").unwrap();
        match &script.commands[0] {
            Command::Simple(cmd) => {
                assert_eq!(cmd.redirections, vec![Redirection::FdDup(1, 2)]);
            }
            _ => panic!("Expected Simple command"),
        }
    }

    #[test]
    fn test_fd_redirection_2_output() {
        let script = parse_input("cmd 2> err.txt\n").unwrap();
        match &script.commands[0] {
            Command::Simple(cmd) => {
                assert_eq!(cmd.redirections, vec![Redirection::FdOutput(2, "err.txt".to_string())]);
            }
            _ => panic!("Expected Simple command"),
        }
    }

    #[test]
    fn test_readwrite_redirection() {
        let script = parse_input("cat <> file.txt\n").unwrap();
        match &script.commands[0] {
            Command::Simple(cmd) => {
                assert_eq!(cmd.redirections, vec![Redirection::ReadWrite("file.txt".to_string())]);
            }
            _ => panic!("Expected Simple command"),
        }
    }

    #[test]
    fn test_numeric_arg_not_swallowed_by_redirect() {
        // `seq 1 3 > out.txt` — the `3` is an argument, not an fd prefix,
        // because there's a space before `>`.
        let script = parse_input("seq 1 3 > out.txt\n").unwrap();
        match &script.commands[0] {
            Command::Simple(cmd) => {
                assert_eq!(cmd.args, vec!["1".to_string(), "3".to_string()]);
                assert_eq!(
                    cmd.redirections,
                    vec![Redirection::Output("out.txt".to_string())]
                );
            }
            _ => panic!("Expected Simple command"),
        }
    }

    #[test]
    fn test_adjacent_fd_redirect_still_works() {
        // `seq 1 3>out.txt` — `3>` is adjacent, so it IS an fd-3 redirect.
        let script = parse_input("seq 1 3>out.txt\n").unwrap();
        match &script.commands[0] {
            Command::Simple(cmd) => {
                assert_eq!(cmd.args, vec!["1".to_string()]);
                assert_eq!(
                    cmd.redirections,
                    vec![Redirection::FdOutput(3, "out.txt".to_string())]
                );
            }
            _ => panic!("Expected Simple command"),
        }
    }

    #[test]
    fn test_comment_skipped() {
        let script = parse_input("#! shebang line\necho hi\n").unwrap();
        assert_eq!(script.commands.len(), 1);
    }

    #[test]
    fn test_assignment_with_quoted_value() {
        let script = parse_input("FOO=\"a b\" env\n").unwrap();
        match &script.commands[0] {
            Command::Simple(cmd) => {
                assert_eq!(cmd.env_vars, vec![("FOO".to_string(), "a b".to_string())]);
                assert_eq!(cmd.program, "env");
            }
            _ => panic!("Expected Simple command"),
        }
    }
}
