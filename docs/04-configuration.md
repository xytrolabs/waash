# 04 — Configuration

WAASH reads its configuration from `~/.config/waash/config.toml`. If the file
doesn't exist, WAASH uses sensible defaults.

Generate a fully-commented default config:

```bash
waash --init
```

The generated file documents every option. This page is the reference.

---

## Top-Level Keys

```toml
indent_binary = "/path/to/indent"   # optional: Indent runtime location
indent_scripting = true             # use Indent for .waash scripts

aliases = [
  { name = "ll", value = "ls -la" },
  { name = "gs", value = "git status" },
]

keybindings = []                    # custom key bindings (see below)
```

### Custom key bindings

Each entry binds a key sequence to an editing action:

```toml
[[keybindings]]
key = "Ctrl+R"
action = "history_search_backward"

[[keybindings]]
key = "Alt+Left"
action = "backward_word"

[[keybindings]]
key = "Ctrl+N"
action = "history_search_forward"
```

Key syntax: `Ctrl+X`, `Alt+X`, `Shift+X`, or a named key (`Up`, `Down`,
`Left`, `Right`, `Home`, `End`, `Tab`, `Enter`, `Backspace`, `Delete`, `Esc`,
`PageUp`, `PageDown`). Actions include `history_search_backward`,
`history_search_forward`, `beginning_of_line`, `end_of_line`, `forward_char`,
`backward_char`, `forward_word`, `backward_word`, `previous_history`,
`next_history`, `clear_screen`, `undo`, `complete`, and more. Unknown
bindings are ignored with a warning.

---

## `[prompt]` — Prompt Appearance

```toml
[prompt]
# Template with placeholders (see below)
template = "{separator} {user}@{host} {dir} {git}{newline}{exit_code}{prompt} "

# The prompt character on success and on error
char_ok = "%"
char_err = "%"

show_exit_code = true    # show "✗127" when a command fails
show_git = true          # show git branch/status
shorten_path = true      # shorten parent dirs to first letter
show_hostname = true     # show user@hostname
```

### Template placeholders

| Placeholder | What it shows |
|---|---|
| `{user}` | Username |
| `{host}` | Hostname |
| `{dir}` | Current directory (shortened if enabled) |
| `{full_dir}` | Full path to current directory |
| `{git}` | Git branch + ahead/behind/dirty, e.g. `(main ↑2 ✗)` |
| `{venv}` | Python virtualenv name |
| `{exit_code}` | Error code (only when non-zero) |
| `{prompt}` | The prompt character (colored) |
| `{separator}` | `╭─` powerline decoration |
| `{time}` | Current time (HH:MM:SS) |
| `{date}` | Current date |
| `{duration}` | How long the last command took |
| `{battery}` | Battery percent (laptops) |
| `{load}` | CPU load average |
| `{flair}` | A random emoji (changes each second) |
| `{time_icon}` | 🌅☀️🌤🌙🌃 based on time of day |
| `{shlvl}` | Shell nesting level |
| `{shell}` | "waash" |
| `{version}` | WAASH version |
| `{jobs}` | Active background-job count, e.g. `⏳2` (empty when none) |
| `{newline}` | Newline (for multi-line prompts) |

### Example prompts

```toml
# Simple FISH-like
template = "{user}@{host} {dir} {git}{prompt} "

# Two-line macOS style
template = "{separator} {user}@{host} {dir} {git}{newline}{exit_code}{prompt} "

# Info-rich
template = "{time_icon} {time} {dir} {git}{venv}{duration}{newline}{exit_code}{prompt} "
```

---

## `[theme]` — Syntax Highlighting Colors

```toml
[theme]
command = "bright green"      # valid external commands
builtin = "green"             # cd, echo, export...
error_command = "bright red"  # unknown commands
string = "bright yellow"      # "quoted strings"
variable = "bright cyan"      # $VARIABLES
operator = "bright magenta"   # | ; && || < >
flag = "bright blue"          # -f --flag
path = "cyan"                 # /file/paths
comment = "bright black"      # # comments
hint = "bright black"         # autosuggestion ghost text
```

Available colors: `black`, `red`, `green`, `yellow`, `blue`, `magenta`,
`cyan`, `white`, and the `bright *` variants.

---

## `[shell]` — Behavior

```toml
[shell]
show_banner = true           # neofetch-style startup screen
history_size = 100000        # max history entries
history_file = "history"     # filename under ~/.config/waash/
autosuggestions = true       # FISH-style ghost predictions
syntax_highlighting = true   # colorize as you type
case_insensitive_completion = false
auto_completion = true
tab_width = 4
edit_mode = "emacs"          # or "vi"
live_refresh = true          # keep the prompt live: time, CPU load & sudo badge update every second
job_control = true           # Ctrl+Z to suspend a foreground command & move it to the background
startup_commands = [         # WAASH commands to run once at interactive startup (~/.waashrc)
  "alias ll = 'ls -la'",
  "export EDITOR = 'nvim'",
]
```

> **`live_refresh`** — while you're sitting at the prompt, WAASH re-renders it
> every second so the inline **time**, **CPU load** (`⚙N.N`) and the **sudo
> badge** stay current. Set it to `false` to only rebuild the prompt after each
> command (slightly more efficient on very slow terminals). The prompt's width
> never changes while it refreshes, so nothing on screen jumps around.
>
> **Note:** live refresh is **automatically disabled inside VS Code's
> integrated terminal** (detected via `TERM_PROGRAM`). Its terminal emulator
> chokes on the injected refresh sequences — the terminal can appear to hang
> and tool/agent output parsing breaks. WAASH falls back to the normal
> per-command prompt there automatically; other terminals (kitty, GNOME
> Terminal, tmux, etc.) get the live prompt.

> **`startup_commands`** — a list of WAASH commands run once when an
> *interactive* session starts (like a `~/.waashrc`). They run after aliases
> are loaded, so they can define aliases, `export` variables, `source` files,
> or run anything else. If WAASH is your login shell, login profiles
> (`/etc/profile`, `~/.profile`) are sourced *before* this, so environment set
> there is already present.

> **`job_control`** — when enabled (default), each foreground command runs in
> its own process group with the terminal handed to it, so pressing **Ctrl+Z**
> suspends that command instead of the whole shell. You can then resume it in
> the background with `bg` (or bring it back with `fg`) — great for parking a
> long download or build without blocking the shell. Set it to `false` to
> restore the old always-foreground behavior.

---

## Aliases

```toml
aliases = [
  { name = "ll", value = "ls -la" },
  { name = "g",  value = "git" },
  { name = "..", value = "cd .." },
]
```

Aliases are resolved when you run a command, and are shown in the startup
banner ("Loaded N aliases from config").

---

## Indent Runtime

WAASH scripts run through the **Indent** runtime. WAASH finds it
automatically by searching, in order:

1. `WAASH_INDENT_BINARY` environment variable
2. `indent_binary` in the config
3. `indent` on your `PATH`
4. `~/.local/bin/indent`
5. Common development locations

Set it explicitly if needed:

```toml
indent_binary = "/home/you/.local/bin/indent"
```

---

## Reloading Configuration

Config is read at startup. To apply changes, exit and restart WAASH
(`exit` then `waash`).

---

## Next Steps

- [05 — The Prompt](05-prompt.md)
- [06 — FAQ & Troubleshooting](06-faq.md)
