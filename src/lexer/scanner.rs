//! Lexer scanner — converts raw input into a stream of `SpannedToken`s.
//!
//! Uses nom for robust parsing of the shell grammar at the token level.

use nom::{
    branch::alt,
    bytes::complete::{tag, take_while, take_while1, take_until},
    character::complete::char,
    combinator::map,
    error::{ParseError, VerboseError},
    IResult,
};

use super::token::{SpannedToken, Token};

/// Type alias to simplify nom return types in this module.
type Res<T, U> = IResult<T, U, VerboseError<T>>;

/// The scanner holds input and position, producing tokens on demand.
pub struct Scanner {
    input: String,
    pos: usize,
    /// Tokens that have been lexed but not yet consumed.
    buffer: Vec<SpannedToken>,
    /// Heredoc delimiters waiting for their body (FIFO order).
    pending_heredocs: Vec<PendingHeredoc>,
    /// Whether the next word is expected to be a heredoc delimiter.
    expect_heredoc_delim: bool,
    /// Strip tabs for the currently-expected heredoc delimiter.
    expect_heredoc_strip: bool,
    /// Whether the last emitted token was a newline (or body end).
    last_was_newline: bool,
}

/// A heredoc that has been started (`<<DELIM`) and is waiting for its body.
struct PendingHeredoc {
    delimiter: String,
    strip_tabs: bool,
    expand: bool,
}

impl Scanner {
    pub fn new(input: String) -> Self {
        Self {
            input,
            pos: 0,
            buffer: Vec::new(),
            pending_heredocs: Vec::new(),
            expect_heredoc_delim: false,
            expect_heredoc_strip: false,
            last_was_newline: false,
        }
    }

    /// Lex all remaining tokens. Call this once, then iterate with `next`.
    pub fn lex_all(&mut self) -> Result<(), String> {
        loop {
            // If a heredoc body is pending and we just hit a newline,
            // collect the raw body lines FIRST (before skipping whitespace,
            // otherwise the body's leading tabs/spaces would be eaten).
            if !self.pending_heredocs.is_empty() && self.last_was_newline {
                self.collect_heredoc_body();
                continue;
            }

            // Skip horizontal whitespace (spaces/tabs), not newlines.
            while self.pos < self.input.len() {
                let c = self.input.as_bytes()[self.pos] as char;
                if c.is_whitespace() && c != '\n' {
                    self.pos += 1;
                } else {
                    break;
                }
            }

            if self.pos >= self.input.len() {
                break;
            }

            let start = self.pos;
            let remaining = &self.input[self.pos..];
            let (remaining_str, sp_tok) = token(remaining)
                .map_err(|e| format!("Lexer error: {:?}", e))?;
            let consumed = remaining.len() - remaining_str.len();
            self.pos += consumed;
            let tok = sp_tok.token;

            // Track heredoc delimiter expectation.
            match &tok {
                Token::HeredocStart => {
                    self.expect_heredoc_delim = true;
                    self.expect_heredoc_strip = false;
                    // The delimiter word is the NEXT token — capture it below.
                }
                Token::HeredocStartStrip => {
                    self.expect_heredoc_delim = true;
                    self.expect_heredoc_strip = true;
                }
                _ => {}
            }

            // Capture the delimiter word if we're expecting one. This runs on
            // the token AFTER `<<`/`<<-` (never on the marker itself).
            if self.expect_heredoc_delim
                && !matches!(tok, Token::HeredocStart | Token::HeredocStartStrip)
            {
                let (delim, expand) = match &tok {
                    Token::Word(w) => (w.clone(), true),
                    Token::DoubleQuoted(w) => (w.clone(), true),
                    Token::SingleQuoted(w) => (w.clone(), false),
                    _ => {
                        // Missing delimiter — give up on heredoc tracking.
                        self.expect_heredoc_delim = false;
                        (String::new(), true)
                    }
                };
                if !delim.is_empty() {
                    self.pending_heredocs.push(PendingHeredoc {
                        delimiter: delim,
                        strip_tabs: self.expect_heredoc_strip,
                        expand,
                    });
                }
                self.expect_heredoc_delim = false;
                self.expect_heredoc_strip = false;
            }

            self.last_was_newline = matches!(tok, Token::Newline);
            self.buffer.push(SpannedToken::new(tok, start, self.pos));
        }

        let final_pos = self.input.len();
        self.buffer.push(SpannedToken::new(Token::Eof, final_pos, final_pos));
        Ok(())
    }

    /// Collect the raw body lines for the next pending heredoc, consuming
    /// input up to and including the delimiter line. Emits a HeredocBody.
    fn collect_heredoc_body(&mut self) {
        let marker = match self.pending_heredocs.first() {
            Some(m) => PendingHeredoc {
                delimiter: m.delimiter.clone(),
                strip_tabs: m.strip_tabs,
                expand: m.expand,
            },
            None => return,
        };
        self.pending_heredocs.remove(0);

        let start = self.pos;
        let mut body = String::new();
        let mut first = true;

        loop {
            let line: String = {
                let rest = &self.input[self.pos..];
                if rest.is_empty() {
                    // End of input without finding the delimiter.
                    break;
                }
                let line_end = rest.find('\n').map(|i| i + 1).unwrap_or(rest.len());
                let line = &rest[..line_end];
                self.pos += line_end;
                line.to_string()
            };

            let content = line.strip_suffix('\n').unwrap_or(&line).to_string();
            let check = if marker.strip_tabs {
                content.strip_prefix('\t').unwrap_or(&content)
            } else {
                &content
            };

            if check == marker.delimiter {
                break;
            }

            if !first {
                body.push('\n');
            }
            first = false;
            body.push_str(&content);
        }

        let end = self.pos;
        self.buffer.push(SpannedToken::new(Token::HeredocBody(body), start, end));
        // A heredoc body ends at a line boundary, so the next pending
        // heredoc (if any) can start collecting on the next iteration.
        self.last_was_newline = true;
    }

    /// Return the next token without consuming it.
    pub fn peek(&self) -> Option<&SpannedToken> {
        self.buffer.first()
    }

    /// Consume and return the next token.
    pub fn next(&mut self) -> Option<SpannedToken> {
        if self.buffer.is_empty() {
            None
        } else {
            Some(self.buffer.remove(0))
        }
    }
}

/// Parse a single token.
fn token(input: &str) -> Res<&str, SpannedToken> {
    let original_input = input;
    let start_offset = original_input.as_ptr() as usize;

    let (input, tok) = alt((
        // Group 1: multi-char operators MUST come before single-char
        alt((
            map(tag("<<<"), |_| Token::HereString),
            map(tag("<<-"), |_| Token::HeredocStartStrip),
            map(tag("<<" ), |_| Token::HeredocStart),
            map(tag(">>" ), |_| Token::RedirectAppend),
            map(tag("&&" ), |_| Token::AndAnd),
            map(tag("||" ), |_| Token::OrOr),
            map(tag("|&" ), |_| Token::PipeStderr),
            map(tag("&>"), |_| Token::RedirectBoth),
            map(tag(">&"), |_| Token::RedirectDupOutput),
            map(tag("<&"), |_| Token::RedirectDupInput),
            map(tag("<>"), |_| Token::RedirectReadWrite),
        )),
        // Group 2: single-char tokens and newline
        alt((
            map(char('\n'), |_| Token::Newline),
            map(char('|'), |_| Token::Pipe),
            map(char(';'), |_| Token::Semicolon),
            map(char('&'), |_| Token::Background),
            map(char('<'), |_| Token::RedirectInput),
            map(char('>'), |_| Token::RedirectOutput),
            map(char('('), |_| Token::LParen),
            map(char(')'), |_| Token::RParen),
            map(char('{'), |_| Token::LBrace),
            map(char('}'), |_| Token::RBrace),
        )),
        // Group 3: strings and comments
        alt((
            comment_token,
            double_quoted_string,
            single_quoted_string,
        )),
        // Group 4: variable references, command substitution, and words
        alt((
            command_substitution,
            backtick_command,
            variable_reference,
            unquoted_word,
        )),
    ))(input)?;

    let end_offset = input.as_ptr() as usize;
    let start = start_offset - original_input.as_ptr() as usize;
    let end = end_offset - original_input.as_ptr() as usize;

    Ok((input, SpannedToken::new(tok, start, end)))
}

/// Parse a comment: # ... until newline or EOF
fn comment_token(input: &str) -> Res<&str, Token> {
    let (input, _) = char('#')(input)?;
    let (input, comment) = take_while(|c: char| c != '\n')(input)?;
    Ok((input, Token::Comment(comment.to_string())))
}

/// Parse a double-quoted string: "like $this"
fn double_quoted_string(input: &str) -> Res<&str, Token> {
    let (input, _) = char('"')(input)?;
    let (input, content) = take_until("\"")(input)?;
    let (input, _) = char('"')(input)?;
    Ok((input, Token::DoubleQuoted(content.to_string())))
}

/// Parse a single-quoted string: 'no expansion here'
fn single_quoted_string(input: &str) -> Res<&str, Token> {
    let (input, _) = char('\'')(input)?;
    let (input, content) = take_until("'")(input)?;
    let (input, _) = char('\'')(input)?;
    Ok((input, Token::SingleQuoted(content.to_string())))
}

/// Parse variable references: $VAR, ${VAR}, $?, $!, $$, $0..$9, $_
fn variable_reference(input: &str) -> Res<&str, Token> {
    let (input, _) = char('$')(input)?;

    // Check for special vars ($?, $$, $!, $#, $*, $@, $-, $0..$9, $_)
    let special_chars = "?!#$*@-0123456789_";
    if input.starts_with(|c: char| special_chars.contains(c)) {
        let (input, special) = take_while1(|c: char| c.is_alphanumeric() || "_?!#$*@-".contains(c))(input)?;
        return Ok((input, Token::SpecialVar(format!("${}", special))));
    }

    // ${VAR} form
    if input.starts_with('{') {
        let (input, _) = char('{')(input)?;
        let (input, var) = take_while1(|c: char| c.is_alphanumeric() || c == '_')(input)?;
        let (input, _) = char('}')(input)?;
        return Ok((input, Token::Variable(var.to_string())));
    }

    // $VAR form
    let (input, var) = take_while1(|c: char| c.is_alphanumeric() || c == '_')(input)?;
    if var.is_empty() {
        return Ok((input, Token::Word("$".to_string())));
    }
    Ok((input, Token::Variable(var.to_string())))
}

/// Parse command substitution: $(command) — balanced parentheses.
fn command_substitution(input: &str) -> Res<&str, Token> {
    let (input, _) = tag("$(")(input)?;
    let mut depth = 1i32;
    let mut end = None;
    for (idx, c) in input.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    end = Some(idx);
                    break;
                }
            }
            _ => {}
        }
    }
    let idx = match end {
        Some(i) => i,
        None => {
            return Err(nom::Err::Error(VerboseError::from_error_kind(
                input,
                nom::error::ErrorKind::Tag,
            )))
        }
    };
    let content = input[..idx].to_string();
    Ok((&input[idx + 1..], Token::CmdSubstitution(content)))
}

/// Parse backtick command substitution: `command`.
fn backtick_command(input: &str) -> Res<&str, Token> {
    let (input, _) = char('`')(input)?;
    let (input, content) = take_until("`")(input)?;
    let (input, _) = char('`')(input)?;
    Ok((input, Token::CmdSubstitution(content.to_string())))
}

/// Parse an unquoted word.
fn unquoted_word(input: &str) -> Res<&str, Token> {
    let (input, word) = take_while1(|c: char| {
        !c.is_whitespace()
            && !matches!(
                c,
                '|' | ';' | '&' | '<' | '>' | '(' | ')' | '{' | '}' | '\'' | '"' | '$' | '#' | '`'
            )
    })(input)?;

    if word.is_empty() {
        return Err(nom::Err::Error(VerboseError::from_error_kind(
            input,
            nom::error::ErrorKind::TakeWhile1,
        )));
    }
    Ok((input, Token::Word(word.to_string())))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lex(input: &str) -> Vec<Token> {
        let mut scanner = Scanner::new(input.to_string());
        scanner.lex_all().unwrap();
        let mut tokens = Vec::new();
        while let Some(t) = scanner.next() {
            tokens.push(t.token);
        }
        tokens
    }

    #[test]
    fn test_simple_command() {
        let tokens = lex("echo hello world\n");
        assert_eq!(tokens[0], Token::Word("echo".into()));
        assert_eq!(tokens[1], Token::Word("hello".into()));
        assert_eq!(tokens[2], Token::Word("world".into()));
        assert_eq!(tokens[3], Token::Newline);
        assert_eq!(tokens[4], Token::Eof);
    }

    #[test]
    fn test_pipeline() {
        let tokens = lex("cat file.txt | grep foo\n");
        let pipe_pos = tokens.iter().position(|t| *t == Token::Pipe).unwrap();
        assert!(pipe_pos > 0);
    }

    #[test]
    fn test_variables() {
        let tokens = lex("echo $HOME ${PATH} $?\n");
        assert!(tokens.iter().any(|t| matches!(t, Token::Variable(ref v) if v == "HOME")));
        assert!(tokens.iter().any(|t| matches!(t, Token::Variable(ref v) if v == "PATH")));
        assert!(tokens.iter().any(|t| matches!(t, Token::SpecialVar(ref v) if v == "$?")));
    }

    #[test]
    fn test_quoted_strings() {
        let tokens = lex("echo \"hello world\" 'no $expansion'\n");
        assert_eq!(tokens[1], Token::DoubleQuoted("hello world".into()));
        assert_eq!(tokens[2], Token::SingleQuoted("no $expansion".into()));
    }

    #[test]
    fn test_heredoc_start() {
        let tokens = lex("cat <<EOF\n");
        assert_eq!(tokens[1], Token::HeredocStart);
        assert_eq!(tokens[2], Token::Word("EOF".into()));
    }

    #[test]
    fn test_heredoc_strip() {
        let tokens = lex("cat <<-EOF\n");
        assert_eq!(tokens[1], Token::HeredocStartStrip);
        assert_eq!(tokens[2], Token::Word("EOF".into()));
    }

    #[test]
    fn test_heredoc_body() {
        let tokens = lex("cat <<EOF\nhello\nworld\nEOF\n");
        // cat, <<, EOF, \n, HeredocBody, Eof
        assert_eq!(tokens[4], Token::HeredocBody("hello\nworld".into()));
        assert!(matches!(tokens[5], Token::Eof));
    }

    #[test]
    fn test_heredoc_body_strip_tabs() {
        let tokens = lex("cat <<-EOF\n\thello\n\tworld\nEOF\n");
        assert_eq!(tokens[4], Token::HeredocBody("\thello\n\tworld".into()));
    }

    #[test]
    fn test_heredoc_no_expand_delim() {
        let tokens = lex("cat <<'EOF'\n$HOME\nEOF\n");
        // <<, SingleQuoted(EOF), \n, HeredocBody("$HOME"), Eof
        assert_eq!(tokens[2], Token::SingleQuoted("EOF".into()));
        assert_eq!(tokens[4], Token::HeredocBody("$HOME".into()));
    }

    #[test]
    fn test_herestring() {
        let tokens = lex("read <<< \"hello world\"\n");
        assert_eq!(tokens[1], Token::HereString);
    }

    #[test]
    fn test_logical_operators() {
        let tokens = lex("make && ./run || echo fail\n");
        assert!(tokens.iter().any(|t| *t == Token::AndAnd));
        assert!(tokens.iter().any(|t| *t == Token::OrOr));
    }

    #[test]
    fn test_redirects() {
        let tokens = lex("cmd >out 2>&1 <in >>app\n");
        assert!(tokens.iter().any(|t| *t == Token::RedirectOutput));
        assert!(tokens.iter().any(|t| *t == Token::RedirectDupOutput));
        assert!(tokens.iter().any(|t| *t == Token::RedirectInput));
        assert!(tokens.iter().any(|t| *t == Token::RedirectAppend));
    }

    #[test]
    fn test_background() {
        let tokens = lex("sleep 10 &\n");
        assert!(tokens.iter().any(|t| *t == Token::Background));
    }

    #[test]
    fn test_comment() {
        let tokens = lex("echo foo # this is a comment\n");
        assert!(tokens.iter().any(|t| matches!(t, Token::Comment(_))));
    }
}
