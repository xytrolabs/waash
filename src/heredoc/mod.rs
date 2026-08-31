//! Heredoc processing for WAASH.
//!
//! Handles BASH-style heredocs:
//! - `<<EOF`  — basic heredoc with variable expansion
//! - `<<-EOF` — strip leading tabs
//! - `<<'EOF'` — no expansion (literal)
//! - `<<<word` — herestring
//! - Multiple heredocs on one command line

use crate::parser::ast::{Command, Heredoc, Script, SimpleCommand};

/// Walk a parsed script and fill in any heredoc bodies that were left empty
/// (i.e. the parser saw `<<DELIM` but the body lines weren't in the input).
///
/// `read_line` is called with the delimiter and should return the next line
/// of body input, or `None` to abort (Ctrl+D / error). Lines are collected
/// until the delimiter is seen. After the body is read, expansion and
/// tab-stripping are applied according to the heredoc's flags.
pub fn fill_heredoc_bodies(
    script: &mut Script,
    read_line: &mut dyn FnMut(&str) -> Option<String>,
) {
    for cmd in &mut script.commands {
        fill_heredoc_in_command(cmd, read_line);
    }
}

/// Attach pre-collected body strings (from the lexer's HeredocBody tokens)
/// to the script's heredocs in source order. Then apply expansion/strip.
pub fn assign_heredoc_bodies(script: &mut Script, bodies: &[String]) {
    let mut idx = 0;
    for cmd in &mut script.commands {
        assign_in_command(cmd, bodies, &mut idx);
    }
}

fn assign_in_command(cmd: &mut Command, bodies: &[String], idx: &mut usize) {
    match cmd {
        Command::Simple(sc) => assign_in_simple(sc, bodies, idx),
        Command::Pipeline(p) => {
            for c in &mut p.commands {
                assign_in_simple(c, bodies, idx);
            }
        }
        Command::And(a, b) | Command::Or(a, b) => {
            assign_in_command(a, bodies, idx);
            assign_in_command(b, bodies, idx);
        }
        Command::Background(inner) => assign_in_command(inner, bodies, idx),
        Command::Subshell(s) | Command::Group(s) => {
            for c in &mut s.commands {
                assign_in_command(c, bodies, idx);
            }
        }
        Command::Noop => {}
    }
}

fn assign_in_simple(sc: &mut SimpleCommand, bodies: &[String], idx: &mut usize) {
    if let Some(hd) = &mut sc.heredoc {
        if hd.body.is_empty() && *idx < bodies.len() {
            hd.body = bodies[*idx].clone();
            *idx += 1;
        }
        process_heredoc_body(hd);
    }
}

fn fill_heredoc_in_command(
    cmd: &mut Command,
    read_line: &mut dyn FnMut(&str) -> Option<String>,
) {
    match cmd {
        Command::Simple(sc) => fill_heredoc_in_simple(sc, read_line),
        Command::Pipeline(p) => {
            for c in &mut p.commands {
                fill_heredoc_in_simple(c, read_line);
            }
        }
        Command::And(a, b) | Command::Or(a, b) => {
            fill_heredoc_in_command(a, read_line);
            fill_heredoc_in_command(b, read_line);
        }
        Command::Background(inner) => fill_heredoc_in_command(inner, read_line),
        Command::Subshell(s) | Command::Group(s) => {
            for c in &mut s.commands {
                fill_heredoc_in_command(c, read_line);
            }
        }
        Command::Noop => {}
    }
}

fn fill_heredoc_in_simple(
    sc: &mut SimpleCommand,
    read_line: &mut dyn FnMut(&str) -> Option<String>,
) {
    if let Some(hd) = &mut sc.heredoc {
        if hd.body.is_empty() {
            let mut body = String::new();
            loop {
                let line = match read_line(&hd.delimiter) {
                    Some(l) => l,
                    None => break,
                };
                let check = if hd.strip_tabs {
                    line.strip_prefix('\t').unwrap_or(&line)
                } else {
                    &line
                };
                if check == hd.delimiter {
                    break;
                }
                if !body.is_empty() {
                    body.push('\n');
                }
                body.push_str(&line);
            }
            hd.body = body;
        }
        process_heredoc_body(hd);
    }
}

/// Process a heredoc body: strip tabs and/or expand variables as configured.
pub fn process_heredoc_body(heredoc: &mut Heredoc) {
    let body = std::mem::take(&mut heredoc.body);

    let body = if heredoc.strip_tabs {
        strip_leading_tabs(&body)
    } else {
        body
    };

    let body = if heredoc.expand {
        crate::wordexp::expand(&body, 0)
    } else {
        body
    };

    heredoc.body = body;
}

/// Strip leading tab characters from each line (<<- behavior).
fn strip_leading_tabs(body: &str) -> String {
    body.lines()
        .map(|line| line.strip_prefix('\t').unwrap_or(line))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Read heredoc body from input lines (used by the REPL in interactive mode).
pub fn read_heredoc_lines(
    delimiter: &str,
    strip_tabs: bool,
    lines: &mut dyn Iterator<Item = String>,
) -> String {
    let mut body = String::new();

    for line in lines {
        let check = if strip_tabs {
            line.strip_prefix('\t').unwrap_or(&line)
        } else {
            &line
        };

        if check == delimiter {
            break;
        }

        if !body.is_empty() {
            body.push('\n');
        }
        body.push_str(&line);
    }

    body
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_strip_tabs() {
        let input = "\thello\n\tworld";
        assert_eq!(strip_leading_tabs(input), "hello\nworld");
    }

    #[test]
    fn test_expand_variables() {
        std::env::set_var("TEST_VAR", "expanded_value");
        let result = crate::wordexp::expand("Hello $TEST_VAR!", 0);
        assert_eq!(result, "Hello expanded_value!");
        std::env::remove_var("TEST_VAR");
    }

    #[test]
    fn test_read_heredoc_lines() {
        let lines = vec![
            "line one".to_string(),
            "line two".to_string(),
            "EOF".to_string(),
            "not included".to_string(),
        ];
        let body = read_heredoc_lines("EOF", false, &mut lines.into_iter());
        assert_eq!(body, "line one\nline two");
    }
}
