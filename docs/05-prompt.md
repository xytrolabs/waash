# 05 — The Prompt

WAASH's prompt is the most customizable part of the shell. It's a
FISH/macOS-inspired, multi-segment prompt that updates live as you work.

---

## Default Look

With the default (macOS-style) config, you get:

```
╭─ raf@raf-cachy ~/Desktop/WAASH (HEAD)  🔋85% ⚙1.2 │ 01:44:00
%
```

- **Line 1** — `user@host`, directory, git status, and right-aligned system
  info (battery, CPU load, command duration, time)
- **Line 2** — the prompt character (`%`, like macOS zsh)

---

## The Segments

### `{separator}` — ╭─ powerline

A subtle top-left corner that ties the prompt together. Its color cycles
through a soft rainbow so each prompt looks slightly different.

### `{user}` `{host}` — identity

Your username in cyan and hostname in green.

### `{dir}` — directory

The current directory, shortened when `shorten_path = true`:

```
~/Desktop/WAASH            # full
~/D/WAASH                  # shortened (parent dirs → first letter)
```

### `{git}` — git status

```
(main)        # clean, on branch main
(main ↑2)     # 2 commits ahead of upstream
(main ↓3)     # 3 commits behind
(main ↑2↓3)   # both
(main ✗)      # dirty working tree
```

### Right-side info

Duration, battery, CPU load, and time render inline at the end of the first
line (dimmed):

```
~1.6s │ 🔋85% │ ⚙1.2 │ 01:44:00
```

- `~1.6s` — command duration (bold if ≥ 1s)
- `🔋85%` — battery percent
- `⚙1.2` — 1-minute CPU load average
- `01:44:00` — current time

---

## Status Feedback

- **`✗127`** — when the last command failed, the error code appears in red
  before the prompt character
- **Prompt char color** — turns red on error, cycles rainbow on success
- **`~3.2s` duration** — always tells you how long the last thing took

---

## Customizing

Set `template` in `[prompt]` in `~/.config/waash/config.toml`. Any
combination of placeholders works. Examples:

### Simple

```toml
[prompt]
template = "{dir} {prompt} "
```

### FISH-like

```toml
[prompt]
template = "{user}@{host} {dir} {git}{prompt} "
char_ok = "❯"
```

### Full two-line

```toml
[prompt]
template = "{separator} {time_icon} {time} {dir} {git}{venv}{duration}{newline}{exit_code}{prompt} "
```

### Change the prompt character

```toml
[prompt]
char_ok = "❯"
char_err = "❯"
```

Or the classic dollar sign:

```toml
[prompt]
char_ok = "$"
char_err = "#"
```

---

## Placeholder Reference

| Placeholder | Example output |
|---|---|
| `{user}` | `raf` |
| `{host}` | `raf-cachy` |
| `{dir}` | `~/D/WAASH` |
| `{full_dir}` | `~/Desktop/WAASH` |
| `{git}` | `(main ↑2)` |
| `{venv}` | `🐍myenv` |
| `{sudo}` | `⚡` (sudo verified) or `#` (root) |
| `{exit_code}` | `✗127` |
| `{prompt}` | `%` |
| `{separator}` | `╭─` |
| `{time}` | `01:44:00` |
| `{date}` | `2026-08-04` |
| `{duration}` | `~3.2s` |
| `{battery}` | `🔋85%` |
| `{load}` | `⚙1.2` |
| `{flair}` | `🚀` |
| `{time_icon}` | `🌅` |
| `{shlvl}` | `+1` |
| `{shell}` | `waash` |
| `{version}` | `0.2.0` |
| `{newline}` | (newline) |

---

## Next Steps

- [04 — Configuration](04-configuration.md)
- [06 — FAQ & Troubleshooting](06-faq.md)
