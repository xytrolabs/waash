# WAASH — What An Amazing SHell 🐚

A Linux shell that combines **FISH's interactivity** (autosuggestions, syntax
highlighting, tab completion) with **BASH's power** (heredocs, pipes,
redirections, job control) — and adds a **modern scripting language (Indent)**
for scripts.

**FISH-style** interactive prompt, **BASH-style** power, **Indent-style**
scripts. WAASH is unique.

```
╭─ raf@raf-cachy ~/Desktop/WAASH (HEAD)  🔋85% ⚙1.2 │ 01:44:00
%
```

---

## ✨ Features

| Feature | What it does |
|---|---|
| 🪄 **Autosuggestions** | Ghost-text predictions as you type (like FISH) — from history, `PATH`, common args (`git ` → `status`) and paths. Accept with `→` or `Ctrl+F` |
| 🎨 **Syntax highlighting** | Colors commands, paths, strings, vars, operators live |
| ⏹ **Tab completion** | Commands, files, variables, options |
| 🐚 **Startup banner** | `neofetch`-style welcome screen with the Xytro logo + system info |
| 🧱 **Heredocs** | Full BASH support: `<<EOF`, `<<-EOF`, `<<'EOF'`, `<<<string` |
| 🔀 **Pipelines** | `cmd1 \| cmd2`, `|&`, logical `&&`/`\|\|` |
| 📁 **Redirections** | `>`, `>>`, `<`, `>&`, `<&`, `2>&1`, `&>`, `<>` |
| ⏱ **Command timing** | Shows how long each command took (like FISH) |
| ⚡ **Live prompt** | Time, CPU load & the sudo badge update every second while you type (no flicker, zero drift) |
| 🔐 **Login shell ready** | Use it instead of bash/fish: sources `/etc/profile` + `~/.profile`, plus your own `startup_commands` |
| 🍎 **macOS-style prompt** | Clean, professional, configurable |
| 🦾 **Indent scripting** | Write scripts in Indent — simpler than bash |
| 📦 **Cross-platform** | Pure Rust, static binary, works on any Linux distro |

---

## 🚀 Quick Install

```bash
curl -fsSL https://xytro.site/waash/install.sh | bash
```

Or build from source:

```bash
git clone https://github.com/xytrolabs/waash
cd waash
bash install.sh
```

The installer works on **Debian/Ubuntu, Fedora, Arch, openSUSE, and any distro**
with a C toolchain — it installs to `~/.local/bin` (no root needed).

---

## 🏁 First Steps

```bash
waash                    # start the interactive shell
help                     # built-in help
exit                     # leave (or Ctrl+D)
```

Try the FISH features:

```
% ls <TAB>               # tab-complete files
% git <TAB>              # tab-complete commands
% cd ~/De<space>         # autosuggest path → press →
% echo "hi" | cat        # pipelines just work
```

---

## 📖 Documentation

- [01 — Getting Started](docs/01-getting-started.md)
- [02 — Using WAASH like Bash](docs/02-usage.md)
- [03 — Scripting with Indent](docs/03-scripting.md)
- [04 — Configuration](docs/04-configuration.md)
- [05 — The Prompt](docs/05-prompt.md)
- [06 — FAQ & Troubleshooting](docs/06-faq.md)

---

## ⚙️ Configuration

WAASH reads `~/.config/waash/config.toml`. Generate a default one:

```bash
waash --init
```

Then edit it:

```toml
[prompt]
template = "{separator} {user}@{host} {dir} {git}{newline}{exit_code}{prompt} "
char_ok = "%"

[theme]
command = "green"
string = "yellow"

[shell]
autosuggestions = true
syntax_highlighting = true
live_refresh = true   # time, CPU load & sudo badge update every second
```

See [04 — Configuration](docs/04-configuration.md) for the full reference.

---

## 🧪 Scripting

WAASH scripts use **Indent** — a simple, readable language:

```indent
#! build.waash — just type shell commands directly!
header "Building WAASH"
cargo build --release
echo "Done!"
```

See [03 — Scripting with Indent](docs/03-scripting.md).

---

## 🗺 Roadmap / Architecture

- `src/lexer/` — tokenizer (words, strings, operators, heredocs)
- `src/parser/` — AST + grammar
- `src/executor/` — fork/exec, pipes, redirections, signals, job control
- `src/builtins/` — `cd`, `echo`, `export`, `test`, etc.
- `src/repl/` — interactive shell (prompt, completions, highlighting, hints)
- `src/indent.rs` — Indent runtime integration for scripts

---

## 📄 License

MIT © Xytro Labs.
