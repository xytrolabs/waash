# 06 — FAQ & Troubleshooting

Common questions and issues when using WAASH.

---

## General

### What makes WAASH different from bash/fish/zsh?

WAASH combines the **interactive experience of FISH** (autosuggestions, syntax
highlighting, tab completion, timing) with the **power of BASH** (heredocs,
pipes, redirections, job control). Scripts are written in **Indent**, a simple
language, instead of POSIX shell syntax.

### Is WAASH a POSIX-compliant shell?

It aims for compatibility with everyday bash workflows. It is not intended as
a strict POSIX-compliant implementation (no `[[ ]]`, no `case`/`for` shell
loops — those are handled by Indent in scripts). For interactive use and
straightforward scripting it behaves like bash.

### Where does WAASH store its data?

| What | Location |
|---|---|
| Config | `~/.config/waash/config.toml` |
| History | `~/.config/waash/history` |
| Helper library (scripts) | `~/.local/share/waash/lib/` |
| Binary | `~/.local/bin/waash` |

---

## Installation

### "waash: command not found"

`~/.local/bin` isn't on your `PATH`. Add it:

```bash
# bash / zsh
echo 'export PATH="$HOME/.local/bin:$PATH"' >> ~/.bashrc && source ~/.bashrc

# fish
fish_add_path ~/.local/bin
```

### "cargo: command not found" during install

WAASH is written in Rust. Install a Rust toolchain first:

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

### I'm on Debian/Fedora/Arch — does it matter?

No. WAASH is a static Rust binary. The installer detects your distro and
handles package managers, but the build process is identical everywhere.

---

## Interactive Issues

### The prompt has a weird gap before where I type

This was a known bug caused by right-side prompt cursor movement. It's fixed —
right-side info now renders inline on the first line. If you still see a gap,
update WAASH to the latest version.

### Autosuggestions aren't showing

Make sure they're enabled:

```toml
[shell]
autosuggestions = true
```

Autosuggestions need history. Run a few commands first. The suggestion is
accepted with **`→`** or **`Ctrl+F`**.

### Tab completion isn't working

Completion scans your `PATH`. If a directory on your PATH is slow or
unreadable (e.g. a network mount), completion can be slow. Ensure
`auto_completion = true` in `[shell]`.

### My prompt shows the wrong time

WAASH uses your **local timezone** (via `date +%z`). If it's wrong, check your
system timezone:

```bash
timedatectl
```

---

## Command Issues

### A command's environment variable leaked into my shell

This shouldn't happen — `FOO=bar cmd` is scoped to that command only. If you
see a leak, update WAASH (this was fixed). To set a persistent variable, use
`export FOO=bar`.

### Ctrl+C kills the whole shell

Fixed in recent versions. WAASH ignores `SIGINT` itself and resets signal
handlers in child processes, so Ctrl+C kills the running command but keeps
the shell alive. Update WAASH.

### Background jobs become zombies

WAASH reaps finished background jobs before showing each prompt. If you see
zombies, update WAASH.

### Heredocs don't work

Heredocs (`<<EOF`, `<<-EOF`, `<<'EOF'`, `<<<`) are fully supported. If they
aren't working, you're on an old version — update.

---

## Scripting Issues

### "Undefined function 'header'"

WAASH auto-imports its helper library into every `.waash` script. If you see
this, either:

- Your script isn't using `.waash`/`.ind` extension
- The helper library isn't installed (`waash --install-lib`)
- You're running an old version

### "Indent runtime not found"

WAASH scripts run through the **Indent** runtime. Install it or point WAASH
to it:

```toml
indent_binary = "/path/to/indent"
```

Or set the environment variable:

```bash
export WAASH_INDENT_BINARY=/path/to/indent
```

See [03 — Scripting](03-scripting.md).

### A bare shell command line didn't run as a command

WAASH routes lines that look like **Indent code** to Indent, and lines that
look like **shell commands** to the shell. If a line starts with an Indent
keyword (`var`, `if`, `say`, `fun`, ...) it's treated as Indent. Otherwise
it's run as a shell command.

---

## Prompt / Theme Issues

### Where do I change the prompt?

`~/.config/waash/config.toml`, under `[prompt]`. See
[05 — The Prompt](05-prompt.md) and [04 — Configuration](04-configuration.md).

### A color in the config isn't recognized

Colors are matched by name (case-insensitive). Valid names: `black`, `red`,
`green`, `yellow`, `blue`, `magenta`, `cyan`, `white`, plus `bright *`
variants. Unknown names fall back to white.

### How do I disable the startup banner?

```toml
[shell]
show_banner = false
```

Or pass `-q` / `--no-banner` on the command line.

---

## Getting Help

- `waash --help` — CLI usage
- `help` inside WAASH — built-in help
- Open an issue on the repository

---

## Feature Status

| Feature | Status |
|---|---|
| Autosuggestions | ✅ |
| Syntax highlighting | ✅ |
| Tab completion | ✅ |
| Heredocs (all forms) | ✅ |
| Pipelines & redirections | ✅ |
| Job control (bg/fg/jobs) | ✅ |
| Command timing | ✅ |
| Indent scripting | ✅ |
| Custom prompt | ✅ |
| Custom keybindings | ✅ |
| Brace expansion | ✅ |
| `[[ ]]` tests | ✅ |
