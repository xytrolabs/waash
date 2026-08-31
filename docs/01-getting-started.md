# 01 — Getting Started

Welcome to WAASH — What An Amazing SHell. This guide gets you up and running.

---

## What is WAASH?

WAASH is a Linux shell designed to be the best of both worlds:

- **Interactive like FISH** — autosuggestions, syntax highlighting, tab
  completion, a clean prompt, and command timing.
- **Powerful like BASH** — heredocs, pipes, redirections, logical operators,
  and full job control.
- **Scripted in Indent** — your own simple scripting language instead of
  arcane POSIX shell syntax.

---

## Installing WAASH

### Option A: One-line installer (any distro)

```bash
curl -fsSL https://xytro.site/waash/install.sh | bash
```

The installer:
1. Detects your OS and architecture
2. Ensures a Rust toolchain is available (installs via `rustup` if missing)
3. Builds WAASH in release mode
4. Installs the binary to `~/.local/bin/waash`
5. Installs the Indent helper library to `~/.local/share/waash/lib/`
6. Adds `~/.local/bin` to your `PATH` if needed

### Option B: Build from source

```bash
git clone https://github.com/xytrolabs/waash
cd waash
cargo build --release
./target/release/waash --help
```

Then optionally:

```bash
cp target/release/waash ~/.local/bin/waash
bash install.sh --no-build   # just install the already-built binary + lib
```

### Option C: Any distro, no Rust toolchain

The one-liner works everywhere — even if you don't have `cargo` installed.
It detects your OS/arch, installs Rust via `rustup` if needed, builds, and
installs to `~/.local/bin` (no root required):

```bash
curl -fsSL https://xytro.site/waash/install.sh | bash
```

This works on **Debian/Ubuntu, Fedora/RHEL, Arch, openSUSE, Alpine**, and any
other distro with a C toolchain. Native `.deb`/`.rpm` packages are planned
for future releases.

---

## First Launch

```bash
waash
```

You'll see a neofetch-style startup screen with the Xytro logo and your
system info, then the prompt:

```
╭─ raf@raf-cachy ~/Desktop/WAASH (HEAD)  ⚙1.2 │ 01:44:00
%
```

Type a command and press `Enter`. It works just like bash:

```
% echo "hello world"
hello world
% ls -la
% pwd
/home/raf/Desktop/WAASH
```

---

## Exiting

- Type `exit`
- Or press `Ctrl+D` (EOF)
- `Ctrl+C` cancels the current command and shows a fresh prompt

---

## Make WAASH your login shell

WAASH is a real compiled binary, so you can use it as your default shell
instead of bash/fish — including over SSH and console logins.

```bash
# 1. Register the shell (needs root — one-time)
echo "$HOME/.local/bin/waash" | sudo tee -a /etc/shells

# 2. Switch your login shell
chsh -s "$HOME/.local/bin/waash"

# 3. Log out and back in for it to take effect
```

When WAASH runs as a **login shell** it automatically sources
`/etc/profile` then `~/.profile` and imports the resulting environment, so
your `PATH`, `export`s, `umask`, etc. keep working exactly as before
(profiles are evaluated by the system POSIX `sh`, which understands their
full control flow).

Things to know:

- **Aliases & functions** from `~/.bashrc`/`~/.profile` are shell-specific
  and won't transfer — define them in WAASH's config instead (see
  [04 — Configuration](04-configuration.md)).
- **Your own startup commands** (like an rc file) go in the
  `[shell] startup_commands = [...]` config option — they run every
  interactive session.
- To switch back at any time: `chsh -s /bin/fish` (or `/bin/bash`).

---

## Interactive Features to Try

### Tab completion

```
% git <TAB>          # completes git subcommands from history/PATH
% cd <TAB>           # completes directories
% echo $<TAB>        # completes environment variables
```

### Autosuggestions

As you type, WAASH predicts the rest of the command in ghost text
(dimmed). Press **`→` (Right Arrow)** or **`Ctrl+F`** to accept it.

```
% echo hello[→]      # becomes: echo hello world
```

WAASH predicts from:
1. Your command history (most recent first)
2. Completing a partially-typed command from `PATH` (e.g. `sys` → `systemctl`)
3. Common next arguments (e.g. `git ` → `status`, `cargo ` → `build`)
4. File paths (e.g. `cd ~/De` → `cd ~/Desktop`)
5. Common idioms

### Startup screen

When WAASH starts you get a `neofetch`-style banner — the Xytro logo next to
your system info (OS, host, kernel, uptime, packages, DE/WM, CPU, GPU,
memory, swap, disks, local IP and locale):

```
    ████      ████   OS CachyOS
    █████    █████   Host raf-cachy
    ██████  ██████   Kernel 6.9.1-1-cachyos
     ████████████    Uptime 3 days, 1 hour
      ██████████     Packages 1523 (pacman)
       ████████      Shell waash 0.2.0
        ██████       Terminal waash
       ████████      CPU 13th Gen Intel(R) Core(TM) i5-13400F
      ██████████     GPU GeForce RTX 4060
     ████████████    Memory 16.55 GiB / 46.88 GiB (35%)
    ██████  ██████   Disk 370G / 954G (39%)
    █████    █████   Local IP 10.0.0.201/24 (wlan0)
    ████      ████   Locale en_US.UTF-8
   WHAT AN AMAZING
      SHELL
```

Disable it with `waash -q` or set `show_banner = false` in your config.

### Syntax highlighting

Commands, options, strings, variables, and operators are colorized as you
type, giving instant feedback about whether a command exists.

### Command timing

After every command, the prompt shows how long it took (like FISH):

```
% sleep 2
╭─ raf@raf-cachy ~/Desktop/WAASH (HEAD)  ~2.0s │ 01:45:00
%
```

---

## Getting Help

- `help` — built-in help text
- `type <cmd>` — is it a builtin or external?
- `waash --help` — CLI options
- `man` pages still work for external commands

---

## Next Steps

- [02 — Using WAASH like Bash](02-usage.md) — commands, pipes, redirects, heredocs, jobs
- [04 — Configuration](04-configuration.md) — make it yours
