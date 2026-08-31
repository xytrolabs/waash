//! Token types emitted by the WAASH lexer.

use std::fmt;

/// A token with its source location information.
#[derive(Debug, Clone, PartialEq)]
pub struct SpannedToken {
    pub token: Token,
    pub span: Span,
}

impl SpannedToken {
    pub fn new(token: Token, start: usize, end: usize) -> Self {
        Self {
            token,
            span: Span { start, end },
        }
    }
}

/// Source location (byte offsets into the input).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}

/// All token types the WAASH lexer can produce.
#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    // ── Words & strings ──
    /// Unquoted word (command name, argument, etc.)
    Word(String),
    /// Single-quoted string: 'like this' — no expansion
    SingleQuoted(String),
    /// Double-quoted string: "like $this" — expansion allowed
    DoubleQuoted(String),

    // ── Variables ──
    /// $VAR or ${VAR}
    Variable(String),
    /// $? $! $$ $0..$9 etc.
    SpecialVar(String),

    // ── Operators ──
    /// | pipe
    Pipe,
    /// |& pipe with stderr
    PipeStderr,
    /// ; command separator
    Semicolon,
    /// && logical AND
    AndAnd,
    /// || logical OR
    OrOr,
    /// & background
    Background,

    // ── Redirections ──
    /// < file
    RedirectInput,
    /// > file
    RedirectOutput,
    /// >> file
    RedirectAppend,
    /// << marker (heredoc start)
    HeredocStart,
    /// <<- marker (heredoc strip tabs)
    HeredocStartStrip,
    /// <<< word (herestring)
    HereString,
    /// >& fd
    RedirectDupOutput,
    /// <& fd
    RedirectDupInput,
    /// <> file (read-write)
    RedirectReadWrite,
    /// &> file (redirect stdout+stderr)
    RedirectBoth,

    // ── Grouping ──
    /// (
    LParen,
    /// )
    RParen,
    /// {
    LBrace,
    /// }
    RBrace,

    // ── Heredoc content ──
    /// The body of a heredoc (already processed for expansion)
    HeredocBody(String),

    // ── Command substitution ──
    /// $(command) or `command`
    CmdSubstitution(String),

    // ── Process substitution (reserved for future use) ──
    /// <(command)
    #[allow(dead_code)]
    ProcessSubInput(String),
    /// >(command)
    #[allow(dead_code)]
    ProcessSubOutput(String),

    // ── Special ──
    /// End of input
    Eof,
    /// A newline (significant for command termination)
    Newline,
    /// A comment: # ... until newline
    Comment(String),
}

impl fmt::Display for Token {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Token::Word(s) => write!(f, "{}", s),
            Token::SingleQuoted(s) => write!(f, "'{}'", s),
            Token::DoubleQuoted(s) => write!(f, "\"{}\"", s),
            Token::Variable(s) => write!(f, "${}", s),
            Token::SpecialVar(s) => write!(f, "{}", s),
            Token::Pipe => write!(f, "|"),
            Token::PipeStderr => write!(f, "|&"),
            Token::Semicolon => write!(f, ";"),
            Token::AndAnd => write!(f, "&&"),
            Token::OrOr => write!(f, "||"),
            Token::Background => write!(f, "&"),
            Token::RedirectInput => write!(f, "<"),
            Token::RedirectOutput => write!(f, ">"),
            Token::RedirectAppend => write!(f, ">>"),
            Token::HeredocStart => write!(f, "<<"),
            Token::HeredocStartStrip => write!(f, "<<-"),
            Token::HereString => write!(f, "<<<"),
            Token::RedirectDupOutput => write!(f, ">&"),
            Token::RedirectDupInput => write!(f, "<&"),
            Token::RedirectReadWrite => write!(f, "<>"),
            Token::RedirectBoth => write!(f, "&>"),
            Token::LParen => write!(f, "("),
            Token::RParen => write!(f, ")"),
            Token::LBrace => write!(f, "{{"),
            Token::RBrace => write!(f, "}}"),
            Token::HeredocBody(s) => write!(f, "<<{}>>", s),
            Token::CmdSubstitution(_) => write!(f, "$(...)"),
            Token::ProcessSubInput(_) => write!(f, "<(...)"),
            Token::ProcessSubOutput(_) => write!(f, ">(...)"),
            Token::Eof => write!(f, "<EOF>"),
            Token::Newline => write!(f, "\\n"),
            Token::Comment(s) => write!(f, "#{}", s),
        }
    }
}
