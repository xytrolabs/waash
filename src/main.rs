// WAASH - What An Amazing SHell
// A Linux shell combining FISH interactivity with BASH power (heredocs, etc.)

mod lexer;
mod parser;
mod executor;
mod builtins;
mod repl;
mod heredoc;
mod config;
mod indent;
mod wordexp;
mod login;

use std::io::{self, Read};
use std::path::PathBuf;

use config::WaashConfig;
use repl::WaashRepl;

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();

    // Parse flags
    let mut command_string: Option<String> = None;
    let mut no_banner = false;
    let mut use_indent_for_script = true; // default: use Indent for .waash files
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "-h" | "--help" => {
                print_help();
                return Ok(());
            }
            "-V" | "--version" => {
                println!("waash {}", env!("CARGO_PKG_VERSION"));
                return Ok(());
            }
            "--init" => {
                return init_config();
            }
            "--install-lib" => {
                return install_waash_library();
            }
            "--update" => {
                return update_waash();
            }
            "-c" => {
                i += 1;
                if i < args.len() {
                    command_string = Some(args[i].clone());
                } else {
                    eprintln!("waash: -c requires an argument");
                    std::process::exit(2);
                }
            }
            "--no-banner" | "-q" => {
                no_banner = true;
            }
            "--shell" => {
                // Force shell (POSIX) mode for -c even if config says Indent
                use_indent_for_script = false;
            }
            _ => {
                // Positional arg: run a script file
                let path = &args[i];
                run_script(path, use_indent_for_script)?;
                return Ok(());
            }
        }
        i += 1;
    }

    // Load config for REPL and script mode. Source login profiles first so
    // any PATH/HOME/XDG_* set by /etc/profile + ~/.profile is visible to the
    // config loader (and to every command that follows).
    if login::is_login_shell() {
        login::source_login_profiles();
    }
    let config = config::load_config();

    // Check if stdin is not a terminal (piped input)
    let is_piped = !atty::is(atty::Stream::Stdin);

    if let Some(cmd) = command_string {
        // -c mode: execute via Indent runtime (if available), or shell fallback
        if use_indent_for_script {
            run_command_via_indent(&config, &cmd)?;
        } else {
            run_command_string(&cmd)?;
        }
    } else if is_piped {
        // Piped stdin: run as a shell script via bash for full POSIX/bash
        // compatibility. This makes `echo 'for f in *.txt; do echo "$f"; done' |
        // waash` work even though WAASH's own REPL parser can't parse bash
        // control flow.
        let mut input = String::new();
        io::stdin().read_to_string(&mut input)?;
        let status = std::process::Command::new("bash")
            .arg("-c")
            .arg(&input)
            .status()
            .map_err(|e| anyhow::anyhow!("failed to run bash: {}", e))?;
        std::process::exit(status.code().unwrap_or(1));
    } else {
        // Interactive REPL
        env_logger::Builder::from_env(
            env_logger::Env::default().default_filter_or("warn"),
        )
        .init();

        log::info!("WAASH v{} starting...", env!("CARGO_PKG_VERSION"));

        // Shell must ignore SIGINT/SIGQUIT so Ctrl+C kills the child,
        // not WAASH itself. Children reset to default before exec.
        executor::setup_shell_signals();

        let mut repl = WaashRepl::with_config(config, no_banner)?;
        repl.run()?;
    }

    Ok(())
}

/// Run a command string via the Indent runtime, or directly if it's a bare command.
fn run_command_via_indent(config: &WaashConfig, code: &str) -> anyhow::Result<()> {
    // Check if this is a simple command (no Indent keywords) — run directly via sh
    if is_bare_command(code) {
        let status = std::process::Command::new("sh")
            .arg("-c")
            .arg(code)
            .status()
            .map_err(|e| anyhow::anyhow!("Failed to run command: {}", e))?;
        if !status.success() {
            std::process::exit(status.code().unwrap_or(1));
        }
        return Ok(());
    }

    // Otherwise, run through Indent with preprocessing
    let indent_bin = indent::find_indent_binary(config.indent_binary.as_deref())
        .ok_or_else(|| anyhow::anyhow!(
            "Indent runtime not found. Install indent or set WAASH_INDENT_BINARY.\n\
             Tip: run 'waash --install-lib' to set up WAASH helpers."
        ))?;

    let waash_lib = indent::find_waash_lib()
        .unwrap_or_else(|| PathBuf::from("share/waash"));

    let exit_code = indent::run_indent_string(&indent_bin, code, &waash_lib)?;
    if exit_code != 0 {
        std::process::exit(exit_code);
    }
    Ok(())
}

/// Check if a code string is a bare shell command (no Indent constructs).
fn is_bare_command(code: &str) -> bool {
    // If it's a single line with no Indent keywords, treat as bare command
    let lines: Vec<&str> = code.lines().collect();

    // Multi-line with mixed content = Indent script
    if lines.len() > 3 {
        return false;
    }

    for line in lines {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with("#!") {
            continue;
        }
        if indent::is_indent_line(trimmed) {
            return false;
        }
    }

    true
}

/// Install WAASH helper library for Indent.
fn install_waash_library() -> anyhow::Result<()> {
    match indent::install_waash_lib() {
        Ok(dest) => {
            println!("✅ WAASH helper library installed to {}", dest.display());
            println!("   Scripts can now use: import waash");
        }
        Err(e) => {
            anyhow::bail!("Failed to install WAASH library: {}", e);
        }
    }
    Ok(())
}

/// Upstream source used by the installer and `--update`.
const WAASH_REPO_URL: &str = "https://github.com/xytrolabs/waash";

/// Run a command, inheriting stdio, and fail if it exits non-zero.
fn run_ok(prog: &str, args: &[&str]) -> anyhow::Result<()> {
    let status = std::process::Command::new(prog)
        .args(args)
        .status()
        .map_err(|e| anyhow::anyhow!("failed to run {}: {}", prog, e))?;
    if !status.success() {
        anyhow::bail!("{} failed with {}", prog, status);
    }
    Ok(())
}

/// `waash --update` — rebuild the latest WAASH from source and replace the
/// currently running binary in place.
fn update_waash() -> anyhow::Result<()> {
    let cache = dirs::cache_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("waash")
        .join("waash-src");
    std::fs::create_dir_all(&cache)?;

    // Fetch or refresh the source.
    if cache.join("Cargo.toml").exists() {
        println!("Pulling latest source in {} ...", cache.display());
        run_ok("git", &["-C", cache.to_str().unwrap(), "pull", "--ff-only"])?;
    } else {
        println!("Cloning {} ...", WAASH_REPO_URL);
        run_ok(
            "git",
            &["clone", "--depth", "1", WAASH_REPO_URL, cache.to_str().unwrap()],
        )?;
    }

    // Build release.
    let manifest = cache.join("Cargo.toml");
    println!("Building WAASH (release)...");
    run_ok(
        "cargo",
        &["build", "--release", "--manifest-path", manifest.to_str().unwrap()],
    )?;

    let new_bin = cache.join("target/release/waash");
    if !new_bin.exists() {
        anyhow::bail!("Build produced no binary at {}", new_bin.display());
    }

    // Report versions (old from this build, new from the freshly built binary).
    let old_ver = env!("CARGO_PKG_VERSION");
    let new_ver = std::process::Command::new(&new_bin)
        .arg("--version")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "new build".to_string());
    println!("{} -> {}", old_ver, new_ver);

    // Replace the running binary (rename over it so a running process isn't
    // "text file busy"; the old process keeps its inode).
    let exe = std::env::current_exe()?;
    let tmp = exe.with_file_name(format!("waash.update.{}", std::process::id()));
    std::fs::copy(&new_bin, &tmp)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755))?;
    }
    std::fs::rename(&tmp, &exe)?;

    println!("✅ Updated WAASH at {}. Restart your shell to use it.", exe.display());
    Ok(())
}

fn init_config() -> anyhow::Result<()> {
    match config::init_config() {
        Ok(path) => {
            println!("✨ Created default config at {}", path.display());
            println!("Edit it to customize your WAASH experience!");
        }
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            let path = config::config_path();
            println!("Config already exists at {}", path.display());
        }
        Err(e) => {
            return Err(anyhow::anyhow!("Failed to create config: {}", e));
        }
    }
    Ok(())
}

fn print_help() {
    println!("WAASH v{} — What An Amazing SHell", env!("CARGO_PKG_VERSION"));
    println!("A Linux shell combining FISH interactivity with BASH power.");
    println!();
    println!("USAGE:");
    println!("  waash                  Start interactive shell");
    println!("  waash -c 'command'     Execute command string and exit");
    println!("  waash script.waash     Execute a script file");
    println!("  echo 'cmd' | waash     Execute commands from stdin");
    println!();
    println!("OPTIONS:");
    println!("  -h, --help             Show this help message");
    println!("  -V, --version          Print version and exit");
    println!("  -c CMD                 Execute CMD and exit");
    println!("  -q, --no-banner        Don't show the welcome banner");
    println!("  --init                 Generate default config at ~/.config/waash/config.toml");
    println!("  --install-lib          Install the WAASH helper library for scripts");
    println!("  --update               Rebuild & self-install the latest WAASH from GitHub");
    println!();
    println!("EXAMPLES:");
    println!("  waash                                        # interactive session");
    println!("  waash -c 'echo hello; ls -la'               # one-shot");
    println!("  echo 'for f in *.txt; do echo \"$f\"; done' | waash  # from pipe");
    println!("  waash -c 'cat <<EOF");
    println!("  line 1");
    println!("  line 2");
    println!("  EOF");
    println!("  '                                            # heredocs work too");
}

fn run_command_string(input: &str) -> anyhow::Result<()> {
    let mut executor = crate::executor::Executor::new();

    // Apply aliases from the user config so they work in piped/-c modes too.
    let config = config::load_config();
    executor.load_config_aliases(&config.aliases);

    // Expand `{a,b}` / `{1..3}` before lexing (per-word, bash semantics).
    let input = crate::wordexp::expand_braces_line(input);
    let mut scanner = crate::lexer::Scanner::new(input);
    scanner.lex_all().map_err(|e| anyhow::anyhow!("lexer error: {}", e))?;

    let mut parser = crate::parser::Parser::new(scanner);
    let script = parser.parse().map_err(|e| anyhow::anyhow!("parse error: {}", e))?;

    let no_lines: Vec<String> = Vec::new();
    let status = executor
        .execute(&script, &mut no_lines.into_iter())
        .map_err(|e| anyhow::anyhow!("{}", e))?;

    std::process::exit(status.code());
}

fn run_script(path: &str, use_indent: bool) -> anyhow::Result<()> {
    let script_path = PathBuf::from(path);

    if use_indent && indent::is_waash_script(&script_path) {
        // Route .waash and .ind files through Indent runtime
        let config = config::load_config();
        let indent_bin = indent::find_indent_binary(config.indent_binary.as_deref())
            .ok_or_else(|| anyhow::anyhow!(
                "Indent runtime not found. Install indent or set WAASH_INDENT_BINARY."
            ))?;

        let waash_lib = indent::find_waash_lib()
            .unwrap_or_else(|| PathBuf::from("share/waash"));

        let exit_code = indent::run_indent_script(&indent_bin, &script_path, &waash_lib)?;
        if exit_code != 0 {
            std::process::exit(exit_code);
        }
    } else {
        // Delegate shell scripts (.sh, bash/sh shebang files, or any non-Indent
        // script) to the real shell interpreter. WAASH's own parser is designed
        // for interactive commands and cannot handle full POSIX/bash syntax
        // (if/for/while/case, ${VAR:-default}, functions). Running through
        // bash/sh gives complete compatibility for existing scripts.
        run_shell_script(&script_path)?;
    }

    Ok(())
}

/// Determine the shell interpreter a script asks for via its shebang.
/// Returns (program, extra-args). Defaults to "bash" (broadest POSIX + shell
/// extensions) when there's no usable shebang.
fn shell_interpreter_for(content: &str) -> (String, Vec<String>) {
    let first_line = content.lines().next().unwrap_or("").trim_start();
    if let Some(rest) = first_line.strip_prefix("#!") {
        let mut parts = rest.split_whitespace();
        let interp = parts.next().unwrap_or("");
        if let Some(name) = interp.rsplit('/').next() {
            // Common shells — run them directly by name (resolved via PATH).
            if matches!(name, "bash" | "sh" | "dash" | "zsh" | "ksh") {
                return (name.to_string(), parts.map(|s| s.to_string()).collect());
            }
            // Absolute interpreter path with no name match (e.g. python) —
            // run it directly; WAASH can't parse it, the interpreter can.
            if interp.starts_with('/') {
                return (interp.to_string(), parts.map(|s| s.to_string()).collect());
            }
        }
    }
    ("bash".to_string(), Vec::new())
}

/// Run a shell script file by delegating to its shebang interpreter (bash by
/// default). Inherits stdio so interactive/piped behavior and exit codes are
/// preserved.
fn run_shell_script(path: &std::path::Path) -> anyhow::Result<()> {
    let content = std::fs::read_to_string(path)
        .map_err(|e| anyhow::anyhow!("cannot read {}: {}", path.display(), e))?;
    let (prog, args_prefix) = shell_interpreter_for(&content);
    let status = std::process::Command::new(&prog)
        .args(&args_prefix)
        .arg(path)
        .status()
        .map_err(|e| anyhow::anyhow!("failed to run {} on {}: {}", prog, path.display(), e))?;
    if !status.success() {
        std::process::exit(status.code().unwrap_or(1));
    }
    Ok(())
}

