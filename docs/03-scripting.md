# 03 — Scripting with Indent

WAASH scripts use **Indent** — your simple, readable programming language —
instead of traditional POSIX shell scripting. This means scripts are easier
to write, read, and maintain, while still being able to run any CLI app.

---

## The Big Idea

In a `.waash` script you can write **plain shell commands** directly
(cargo, git, echo — anything), AND mix in **Indent code** (variables, loops,
conditionals, functions) when you need real logic.

WAASH preprocesses each script: lines that look like shell commands are run
as commands; lines that use Indent syntax are evaluated as Indent.

---

## Your First Script

`hello.waash`:

```indent
#! My first WAASH script

cargo --version
git status
echo "done!"
```

Run it:

```bash
waash hello.waash
```

Output:

```
cargo 1.97.0 (c980f4866 2026-06-30)
On branch main
done!
```

No imports, no wrappers — just commands.

---

## Mixing Shell Commands and Indent

Because WAASH auto-imports a helper library, you can use both freely:

```indent
#! build.waash — a build script that's actually readable

header "Building WAASH"

var has_cargo = has_command "cargo"
if has_cargo == false
  error "cargo is required"
  process_exit 1

success "Building release..."
cargo build --release

var size = sh_capture "ls -lh target/release/waash | awk '{print $5}'"
info "Binary size: " + size
```

---

## WAASH Helper Functions

These are auto-imported into every script — no `get` needed:

| Function | What it does | Example |
|---|---|---|
| `sh "cmd"` | Run a command, return exit code | `var code = sh "cargo test"` |
| `sh_capture "cmd"` | Run a command, return stdout | `var out = sh_capture "git branch"` |
| `has_command "name"` | Is a command on PATH? | `if has_command "cargo"` |
| `header "text"` | Print a section header | `header "Build"` |
| `success "text"` | Print ✅ message | `success "Done!"` |
| `error "text"` | Print ❌ message | `error "Failed"` |
| `info "text"` | Print ℹ️ message | `info "Building..."` |
| `env "NAME" default` | Get env var with fallback | `var home = env "HOME" "/"` |
| `read_file "path"` | Read a file | `var c = read_file "data.txt"` |
| `write_file "p" "content"` | Write a file | `write_file "o.txt" "hi"` |
| `exists "path"` | Path exists? | `if exists "config.toml"` |

---

## Indent Basics (the parts you'll use)

> WAASH runs whatever Indent runtime you have installed (1.3.0+ recommended —
> 1.4.0 adds the `Set` type and `set <var> <type>` type conversion). Check
> with `indent --version`.

### Variables

Types are **inferred** (no annotation needed):

```indent
var name = "Ada"          # string
var count = 42            # int
var ok = true             # boolean
var items = ["a", "b"]    # list
name is "Grace"           # reassign with `is`
```

Convert between types with `set` (Indent 1.4.0):

```indent
set count string          # 42 -> "42"
set name int              # "42" -> 42 (when parseable)
```

### Sets (Indent 1.4.0)

Unique, ordered collections:

```indent
var tags = set ["a", "b", "a"]   # -> {a, b}
contains tags "a"                # -> true
repeat tag in tags
  say tag
```

### Comprehensions (Indent 1.4.0)

Build collections with `for ... in` expressions:

```indent
var nums = [1, 2, 3, 6]
say [x * 2 for x in nums]              # -> [2, 4, 6, 12]    (map)
say [x for x in nums if x > 5]         # -> [6]               (filter)
say {x: x * 2 for x in nums}           # -> {"1": 2, ...}     (dict)
```

WAASH recognizes lines starting with `[`/`{` that contain `for ... in` as
Indent comprehensions (not shell commands), so they work directly in
`.waash` scripts.

### Conditionals

Indent uses `if` / `or` / `otherwise` (no colons):

```indent
if count > 10
  say "big"
or count > 5
  say "medium"
otherwise
  say "small"
```

### Loops

```indent
repeat 5
  say "hello"

var items list = ["a", "b", "c"]
repeat item in items
  say item
```

### Functions

```indent
fun greet name
  say "Hello " + name

greet "World"
```

### Output

```indent
say "Hello"
say "The result is " + 42
```

---

## Example: A Real Build Script

`build.waash`:

```indent
#! Build script — far cleaner than Makefile or build.sh

header "WAASH Build Script"

# Check prerequisites
var has_cargo = has_command "cargo"
if has_cargo == false
  error "cargo is required"
  process_exit 1

success "cargo found"
say sh_capture "cargo --version"

# Test
info "Running tests..."
var test_result = os_system "cargo test --quiet"
if test_result != 0
  error "Tests failed!"
  process_exit test_result
success "All tests pass"

# Build
info "Building release..."
var build_result = os_system "cargo build --release"
if build_result != 0
  error "Build failed!"
  process_exit build_result
success "Build complete"

# Install
var answer = ask "Install to ~/.local/bin? [y/N] "
if lower answer == "y"
  os_system "cp target/release/waash ~/.local/bin/waash"
  success "Installed!"
```

Run it: `waash build.waash`

---

## Running Scripts

```bash
waash script.waash          # run a script file
waash -c 'cargo --version'  # run a one-liner (bare command)
waash -c 'say "hello"'      # run Indent code
echo 'echo hi' | waash      # pipe commands via stdin
```

### Shebang-style execution

You can also make a WAASH script executable:

```bash
#!/usr/bin/env waash

cargo build --release
```

```bash
chmod +x script.waash
./script.waash
```

---

## Passing Arguments

Inside a script, `$0`, `$1`, `$2` … hold the script name and arguments:

```indent
#! args.waash
say "Script: " + $0
say "First arg: " + $1
```

```bash
waash args.waash hello world
```

---

## Next Steps

- [04 — Configuration](04-configuration.md)
- [05 — The Prompt](05-prompt.md)
