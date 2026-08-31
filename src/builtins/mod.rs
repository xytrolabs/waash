//! Built-in commands for WAASH.
//!
//! Shell builtins that execute within the shell process itself
//! (no fork/exec needed). These are the standard POSIX + FISH-like builtins.

use std::env;
use std::io::{self, BufRead, Write};
use std::path::PathBuf;

use crate::executor::ExitStatus;

/// Result of executing a builtin command.
pub type BuiltinResult = Result<ExitStatus, String>;

/// Try to execute a builtin command. Returns None if the command is not a builtin.
pub fn try_builtin(program: &str, args: &[String]) -> Option<BuiltinResult> {
    match program {
        "cd" => Some(cd(args)),
        "pwd" => Some(pwd(args)),
        "exit" => Some(exit(args)),
        "export" => Some(export(args)),
        "unset" => Some(unset(args)),
        "echo" => Some(echo(args)),
        "read" => Some(read(args)),
        "type" => Some(type_cmd(args)),
        "set" => Some(set_cmd(args)),
        "test" | "[" | "[[" => Some(test_cmd(args)),
        "true" => Some(Ok(ExitStatus::Code(0))),
        "false" => Some(Ok(ExitStatus::Code(1))),
        ":" => Some(Ok(ExitStatus::Code(0))), // no-op
        // alias, unalias, source, jobs, fg, bg, history are handled by the
        // executor (they need shell state), not here.
        _ => None,
    }
}

/// Return whether a command name is a builtin.
pub fn is_builtin(name: &str) -> bool {
    matches!(
        name,
        "cd" | "pwd" | "exit" | "export" | "unset" | "alias" | "unalias"
            | "echo" | "type" | "source" | "." | "jobs" | "fg" | "bg"
            | "history" | "set" | "test" | "[" | "[[" | "true" | "false" | ":"
            | "kill" | "wait" | "disown" | "read" | "pushd" | "popd" | "dirs"
    )
}

// ── Individual builtin implementations ──

fn cd(args: &[String]) -> BuiltinResult {
    let target = if args.is_empty() {
        env::var("HOME").unwrap_or_else(|_| "/".to_string())
    } else if args[0] == "-" {
        env::var("OLDPWD").unwrap_or_else(|_| {
            eprintln!("cd: OLDPWD not set");
            "/".to_string()
        })
    } else {
        args[0].clone()
    };

    let old_pwd = env::current_dir().map_err(|e| format!("cd: {}", e))?;

    let path = PathBuf::from(&target);
    env::set_current_dir(&path).map_err(|e| format!("cd: {}: {}", target, e))?;

    env::set_var("OLDPWD", old_pwd.to_string_lossy().as_ref());
    env::set_var(
        "PWD",
        env::current_dir()
            .unwrap()
            .to_string_lossy()
            .as_ref(),
    );

    Ok(ExitStatus::Code(0))
}

fn pwd(_args: &[String]) -> BuiltinResult {
    let cwd = env::current_dir().map_err(|e| format!("pwd: {}", e))?;
    println!("{}", cwd.display());
    Ok(ExitStatus::Code(0))
}

fn exit(args: &[String]) -> BuiltinResult {
    let code = args
        .first()
        .and_then(|s| s.parse::<i32>().ok())
        .unwrap_or(0);
    Ok(ExitStatus::Exit(code))
}

fn export(args: &[String]) -> BuiltinResult {
    if args.is_empty() {
        // Print all exported variables
        for (key, value) in env::vars() {
            println!("export {}={}", key, value);
        }
        return Ok(ExitStatus::Code(0));
    }

    for arg in args {
        if let Some((name, value)) = arg.split_once('=') {
            env::set_var(name, value);
        } else {
            // Just mark as exported (already in env)
            eprintln!("export: {} not in format NAME=VALUE", arg);
        }
    }

    Ok(ExitStatus::Code(0))
}

fn unset(args: &[String]) -> BuiltinResult {
    for arg in args {
        env::remove_var(arg);
    }
    Ok(ExitStatus::Code(0))
}

fn echo(args: &[String]) -> BuiltinResult {
    let mut first = true;
    let mut interpret_escapes = false;
    let mut no_newline = false;
    let mut idx = 0;

    // Parse echo flags: -n (no newline), -e (interpret escapes), -E (don't)
    for (i, arg) in args.iter().enumerate() {
        if arg == "-n" {
            no_newline = true;
            idx = i + 1;
        } else if arg == "-e" {
            interpret_escapes = true;
            idx = i + 1;
        } else if arg == "-E" {
            interpret_escapes = false;
            idx = i + 1;
        } else {
            break;
        }
    }

    for arg in &args[idx..] {
        if !first {
            print!(" ");
        }
        first = false;

        if interpret_escapes {
            print_escaped(arg);
        } else {
            print!("{}", arg);
        }
    }

    if !no_newline {
        println!();
    }
    io::stdout().flush().map_err(|e| format!("echo: {}", e))?;

    Ok(ExitStatus::Code(0))
}

fn print_escaped(s: &str) {
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('n') => print!("\n"),
                Some('t') => print!("\t"),
                Some('r') => print!("\r"),
                Some('\\') => print!("\\"),
                Some('a') => print!("\x07"),
                Some('b') => print!("\x08"),
                Some('e') => print!("\x1b"),
                Some('0') => {
                    // Octal: \0NNN
                    let mut octal = String::new();
                    for _ in 0..3 {
                        if let Some(c) = chars.clone().next() {
                            if c.is_ascii_digit() && c < '8' {
                                octal.push(chars.next().unwrap());
                            } else {
                                break;
                            }
                        }
                    }
                    if !octal.is_empty() {
                        if let Ok(n) = u8::from_str_radix(&octal, 8) {
                            print!("{}", n as char);
                        }
                    }
                }
                Some(other) => {
                    print!("\\{}", other);
                }
                None => print!("\\"),
            }
        } else {
            print!("{}", c);
        }
    }
}

/// `read [-p prompt] [-r] var...` — read a line from stdin and assign it to
/// the given variables. Words are split on whitespace; the last variable gets
/// the remainder of the line. With no variables, uses `REPLY`. Returns 1 on
/// EOF.
fn read(args: &[String]) -> BuiltinResult {
    let mut prompt: Option<String> = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "-p" => {
                i += 1;
                prompt = args.get(i).cloned();
                i += 1;
            }
            "-r" => {
                i += 1; // accepted; splitting is already literal here
            }
            _ => break,
        }
    }
    let vars: Vec<String> = if i < args.len() {
        args[i..].to_vec()
    } else {
        vec!["REPLY".to_string()]
    };

    if let Some(p) = &prompt {
        print!("{}", p);
        io::stdout().flush().ok();
    }

    let mut line = String::new();
    let n = io::stdin()
        .lock()
        .read_line(&mut line)
        .map_err(|e| format!("read: {}", e))?;
    if n == 0 {
        return Ok(ExitStatus::Code(1)); // EOF
    }
    let trimmed = line.trim_end_matches(|c| c == '\n' || c == '\r');
    let words: Vec<&str> = trimmed.split_whitespace().collect();

    for (idx, var) in vars.iter().enumerate() {
        if idx == vars.len() - 1 {
            let rest = words.get(idx).copied().unwrap_or("").to_string();
            env::set_var(var, rest);
        } else {
            env::set_var(var, words.get(idx).copied().unwrap_or(""));
        }
    }
    Ok(ExitStatus::Code(0))
}

fn type_cmd(args: &[String]) -> BuiltinResult {    for arg in args {
        if is_builtin(arg) {
            println!("{} is a shell builtin", arg);
        } else {
            // Search PATH
            if let Ok(paths) = env::var("PATH") {
                for dir in paths.split(':') {
                    let full = PathBuf::from(dir).join(arg);
                    if full.exists() && full.is_file() {
                        println!("{} is {}", arg, full.display());
                        return Ok(ExitStatus::Code(0));
                    }
                }
            }
            println!("{}: not found", arg);
        }
    }
    Ok(ExitStatus::Code(0))
}

fn set_cmd(_args: &[String]) -> BuiltinResult {
    // Print all variables (like bash `set`)
    for (key, value) in env::vars() {
        println!("{}={}", key, value);
    }
    Ok(ExitStatus::Code(0))
}

/// Evaluate a `test`/`[`/`[[` expression. `args` are the already-expanded
/// arguments (the closing `]`/`]]` is stripped by the caller). Returns the
/// boolean result.
fn eval_test(args: &[&str]) -> bool {
    if args.is_empty() {
        return false;
    }
    match args[0] {
        "-f" => args.get(1).map(|p| PathBuf::from(p).is_file()).unwrap_or(false),
        "-d" => args.get(1).map(|p| PathBuf::from(p).is_dir()).unwrap_or(false),
        "-e" => args.get(1).map(|p| PathBuf::from(p).exists()).unwrap_or(false),
        "-s" => args
            .get(1)
            .and_then(|p| PathBuf::from(p).metadata().ok())
            .map(|m| m.len() > 0)
            .unwrap_or(false),
        "-r" => args
            .get(1)
            .map(|p| nix::unistd::access(std::path::Path::new(p), nix::unistd::AccessFlags::R_OK).is_ok())
            .unwrap_or(false),
        "-w" => args
            .get(1)
            .map(|p| nix::unistd::access(std::path::Path::new(p), nix::unistd::AccessFlags::W_OK).is_ok())
            .unwrap_or(false),
        "-x" => args
            .get(1)
            .map(|p| nix::unistd::access(std::path::Path::new(p), nix::unistd::AccessFlags::X_OK).is_ok())
            .unwrap_or(false),
        "-z" => args.get(1).map(|s| s.is_empty()).unwrap_or(true),
        "-n" => args.get(1).map(|s| !s.is_empty()).unwrap_or(false),
        "!" => args.len() >= 2 && !eval_test(&args[1..]),
        _ => {
            // Binary operators.
            if args.len() >= 3 {
                match args[1] {
                    "=" | "==" => args[0] == args[2],
                    "!=" => args[0] != args[2],
                    "-eq" => num(args[0]) == num(args[2]),
                    "-ne" => num(args[0]) != num(args[2]),
                    "-lt" => num(args[0]) < num(args[2]),
                    "-le" => num(args[0]) <= num(args[2]),
                    "-gt" => num(args[0]) > num(args[2]),
                    "-ge" => num(args[0]) >= num(args[2]),
                    _ => !args[0].is_empty(),
                }
            } else {
                !args[0].is_empty()
            }
        }
    }
}

fn num(s: &str) -> i64 {
    s.parse::<i64>().unwrap_or(0)
}

fn test_cmd(args: &[String]) -> BuiltinResult {
    // Strip the closing bracket for `[ ... ]` and `[[ ... ]]` forms.
    let mut args: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    if matches!(args.last(), Some(&"]") | Some(&"]]")) {
        args.pop();
    }
    let result = eval_test(&args);
    Ok(if result {
        ExitStatus::Code(0)
    } else {
        ExitStatus::Code(1)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bracket_forms() {
        // `[[ ... ]]` (trailing ]] stripped).
        assert!(eval_test(&["-e", "/tmp"]));
        assert!(!eval_test(&["-f", "/tmp"]));
        assert!(eval_test(&["-d", "/tmp"]));
        // String compare.
        assert!(eval_test(&["a", "=", "a"]));
        assert!(eval_test(&["a", "==", "a"]));
        assert!(eval_test(&["a", "!=", "b"]));
        // Numeric compare.
        assert!(eval_test(&["5", "-gt", "3"]));
        assert!(eval_test(&["2", "-le", "2"]));
        assert!(!eval_test(&["1", "-eq", "2"]));
        // -z / -n.
        assert!(eval_test(&["-z", ""]));
        assert!(eval_test(&["-n", "x"]));
        // Negation.
        assert!(eval_test(&["!", "-f", "/tmp"]));
        // Non-empty string is true.
        assert!(eval_test(&["hello"]));
    }
}
