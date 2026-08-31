//! Abstract Syntax Tree (AST) types for WAASH.
//!
//! These types represent parsed shell constructs — commands, pipelines,
//! redirections, heredocs, control flow, etc.

use std::fmt;

/// A complete shell script / input — a list of commands.
#[derive(Debug, Clone, PartialEq)]
pub struct Script {
    pub commands: Vec<Command>,
}

/// A single command, which may be part of a pipeline or logical chain.
#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    /// A simple command: cmd arg1 arg2 ...
    Simple(SimpleCommand),
    /// A pipeline: cmd1 | cmd2 | cmd3 ...
    Pipeline(Pipeline),
    /// Logical AND: cmd1 && cmd2
    And(Box<Command>, Box<Command>),
    /// Logical OR: cmd1 || cmd2
    Or(Box<Command>, Box<Command>),
    /// Background job: cmd &
    Background(Box<Command>),
    /// Subshell: ( cmd1; cmd2 )
    Subshell(Script),
    /// Group command: { cmd1; cmd2; }
    Group(Script),
    /// No-op (reserved; matched by the executor but not produced by the parser)
    #[allow(dead_code)]
    Noop,
}

/// A simple command with arguments, redirections, and optional heredoc.
#[derive(Debug, Clone, PartialEq)]
pub struct SimpleCommand {
    /// The command name / program to execute.
    pub program: String,
    /// Arguments to the command.
    pub args: Vec<String>,
    /// Input/output redirections.
    pub redirections: Vec<Redirection>,
    /// Heredoc body, if any (for << and <<-).
    pub heredoc: Option<Heredoc>,
    /// Herestring value, if any (for <<< word).
    pub herestring: Option<String>,
    /// Environment variable assignments preceding the command: VAR=val cmd
    pub env_vars: Vec<(String, String)>,
}

/// A pipeline of commands connected by pipes.
#[derive(Debug, Clone, PartialEq)]
pub struct Pipeline {
    pub commands: Vec<SimpleCommand>,
    /// Whether stderr is also piped (|&).
    pub pipe_stderr: bool,
}

/// A redirection specification.
#[derive(Debug, Clone, PartialEq)]
pub enum Redirection {
    /// < file — read stdin from file
    Input(String),
    /// > file — write stdout to file (truncate)
    Output(String),
    /// >> file — append stdout to file
    Append(String),
    /// <> file — read-write on stdin
    ReadWrite(String),
    /// >& fd — duplicate output fd (fd 1 becomes a dup of `fd`)
    DupOutput(i32),
    /// <& fd — duplicate input fd (fd 0 becomes a dup of `fd`)
    DupInput(i32),
    /// &> file — redirect both stdout and stderr
    Both(String),
    /// N> file — redirect fd N to file (truncate)
    FdOutput(i32, String),
    /// N>> file — redirect fd N to file (append)
    FdAppend(i32, String),
    /// N< file — fd N reads from file
    FdInput(i32, String),
    /// N<> file — fd N reads/writes file
    FdReadWrite(i32, String),
    /// N>&M or N<&M — fd N becomes a dup of fd M
    FdDup(i32, i32),
}

/// A heredoc specification (the AST-side of what was parsed).
#[derive(Debug, Clone, PartialEq)]
pub struct Heredoc {
    /// The delimiter word (e.g., "EOF")
    pub delimiter: String,
    /// Whether to strip leading tabs (<<-)
    pub strip_tabs: bool,
    /// Whether to expand variables in the body (<<'EOF' disables expansion)
    pub expand: bool,
    /// The body content from the input.
    pub body: String,
}

/// Quote a word for display if it contains shell metacharacters or whitespace.
fn quote_word(w: &str) -> String {
    if w.is_empty() {
        return "''".to_string();
    }
    let special = w
        .chars()
        .any(|c| c.is_whitespace() || "\\'\"$`&|;()<>*?[]".contains(c));
    if !special {
        return w.to_string();
    }
    let mut out = String::with_capacity(w.len() + 2);
    out.push('\'');
    for c in w.chars() {
        if c == '\'' {
            out.push_str("'\\''");
        } else {
            out.push(c);
        }
    }
    out.push('\'');
    out
}

impl Script {
    /// Render the script back to readable shell syntax.
    pub fn render_script(&self) -> String {
        self.commands
            .iter()
            .map(|c| c.render())
            .collect::<Vec<_>>()
            .join("; ")
    }
}

impl Command {
    /// Render the command back to readable shell syntax (for `jobs` display).
    pub fn render(&self) -> String {
        match self {
            Command::Simple(sc) => sc.render(),
            Command::Pipeline(p) => {
                let sep = if p.pipe_stderr { " |& " } else { " | " };
                p.commands.iter().map(|c| c.render()).collect::<Vec<_>>().join(sep)
            }
            Command::And(a, b) => format!("{} && {}", a.render(), b.render()),
            Command::Or(a, b) => format!("{} || {}", a.render(), b.render()),
            Command::Background(c) => format!("{} &", c.render()),
            Command::Subshell(s) => format!("( {} )", s.render_script()),
            Command::Group(s) => format!("{{ {} }}", s.render_script()),
            Command::Noop => String::new(),
        }
    }
}

impl SimpleCommand {
    /// Render a simple command back to readable shell syntax.
    pub fn render(&self) -> String {
        let mut s = String::new();
        for (k, v) in &self.env_vars {
            s.push_str(&quote_word(k));
            s.push('=');
            s.push_str(&quote_word(v));
            s.push(' ');
        }
        s.push_str(&self.program);
        for a in &self.args {
            s.push(' ');
            s.push_str(&quote_word(a));
        }
        for r in &self.redirections {
            s.push(' ');
            s.push_str(&r.render());
        }
        if let Some(h) = &self.herestring {
            s.push_str(" <<< ");
            s.push_str(&quote_word(h));
        }
        if let Some(h) = &self.heredoc {
            let _ = h; // body omitted for display
            s.push_str(" <<<...>");
        }
        s
    }
}

impl Redirection {
    /// Render a redirection back to readable shell syntax.
    pub fn render(&self) -> String {
        match self {
            Redirection::Input(f) => format!("<{}", f),
            Redirection::Output(f) => format!(">{}", f),
            Redirection::Append(f) => format!(">>{}", f),
            Redirection::ReadWrite(f) => format!("<>{}", f),
            Redirection::DupOutput(n) => format!(">&{}", n),
            Redirection::DupInput(n) => format!("<&{}", n),
            Redirection::Both(f) => format!("&>{}", f),
            Redirection::FdOutput(n, f) => format!("{}>{}", n, f),
            Redirection::FdAppend(n, f) => format!("{}>>{}", n, f),
            Redirection::FdInput(n, f) => format!("{}<{}", n, f),
            Redirection::FdReadWrite(n, f) => format!("{}<>{}", n, f),
            Redirection::FdDup(n, m) => format!("{}>&{}", n, m),
        }
    }
}

impl fmt::Display for Script {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, cmd) in self.commands.iter().enumerate() {
            if i > 0 {
                writeln!(f, ";")?;
            }
            write!(f, "{}", cmd.render())?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_render_simple() {
        let cmd = Command::Simple(SimpleCommand {
            program: "ls".into(),
            args: vec!["-la".into(), "my dir".into()],
            redirections: vec![Redirection::Output("out.txt".into())],
            heredoc: None,
            herestring: None,
            env_vars: vec![],
        });
        // The arg with a space gets single-quoted.
        assert_eq!(cmd.render(), "ls -la 'my dir' >out.txt");
    }

    #[test]
    fn test_render_background_and_pipeline() {
        let bg = Command::Background(Box::new(Command::Simple(SimpleCommand {
            program: "sleep".into(),
            args: vec!["5".into()],
            redirections: vec![],
            heredoc: None,
            herestring: None,
            env_vars: vec![],
        })));
        assert_eq!(bg.render(), "sleep 5 &");

        let pipe = Command::Pipeline(Pipeline {
            commands: vec![
                SimpleCommand {
                    program: "ls".into(),
                    args: vec![],
                    redirections: vec![],
                    heredoc: None,
                    herestring: None,
                    env_vars: vec![],
                },
                SimpleCommand {
                    program: "grep".into(),
                    args: vec!["foo".into()],
                    redirections: vec![],
                    heredoc: None,
                    herestring: None,
                    env_vars: vec![],
                },
            ],
            pipe_stderr: false,
        });
        assert_eq!(pipe.render(), "ls | grep foo");
    }

    #[test]
    fn test_render_env_vars_and_quoting() {
        let cmd = Command::Simple(SimpleCommand {
            program: "echo".into(),
            args: vec!["$HOME".into()],
            redirections: vec![],
            heredoc: None,
            herestring: None,
            env_vars: vec![("FOO".into(), "a b".into())],
        });
        // env var value with a space is quoted; $HOME is quoted (metachar $).
        assert_eq!(cmd.render(), "FOO='a b' echo '$HOME'");
    }
}
