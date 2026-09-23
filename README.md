# bloat

Interactive disk-usage treemap for the terminal — a baobab you can ssh into.

```console
$ bloat ~/devel
```

The treemap is drawn as **pixels**: every character cell is a `▀` whose
foreground and background are independent 24-bit colours, so regions get twice
the vertical resolution of a character, with cushion shading instead of flat
bars. Everything degrades gracefully on a plain terminal (it works on a stock
Ubuntu terminal, Windows Terminal/WSL, tmux, over ssh) and quietly picks up
extra terminal features where they exist.

## Install

One-liner (installs Rust first if it is missing, then builds from source):

```sh
curl -fsSL https://raw.githubusercontent.com/BananaBites/bloat/main/install.sh | sh
```

Directly from GitHub with cargo (needs a Rust toolchain):

```sh
cargo install --git https://github.com/BananaBites/bloat
```

From a local checkout:

```sh
cargo install --path .
```

All three install the same way: one static-ish binary, no runtime dependencies.
Keep it up to date with [`bloat update`](#keeping-it-up-to-date), and ask
`bloat doctor` what you are running.

## Shell completions

Completions are generated *by the installed binary* on every shell start, so
they always match the version you have — a `bloat update` that adds new flags
does not require regenerating anything. Add one line:

**bash** (`~/.bashrc`)

```sh
echo 'source <(COMPLETE=bash bloat)' >> ~/.bashrc
```

**zsh** (`~/.zshrc`)

```sh
echo 'source <(COMPLETE=zsh bloat)' >> ~/.zshrc
```

**fish**

```sh
echo 'COMPLETE=fish bloat | source' >> ~/.config/fish/completions/bloat.fish
```

Open a new shell (or `source` the file) and `bloat <TAB>` completes
subcommands, flags, enum values and paths.

If you prefer a static file (package builds, no shell-startup calls), generate
one — just remember to regenerate it after updating:

```sh
bloat completions bash > ~/.local/share/bash-completion/completions/bloat
bloat completions zsh  > ~/.zsh/completions/_bloat
bloat completions fish > ~/.config/fish/completions/bloat.fish
```

`bloat doctor` tells you which of the two it found for each shell.

## Keeping it up to date

```console
$ bloat doctor
bloat doctor  ·  https://github.com/BananaBites/bloat

  installed   v0.1.0 @ 6eb6784
              /home/hannes/.cargo/bin/bloat  (main)
              built 2026-09-22 · release · x86_64-unknown-linux-gnu
  remote      @ 9f3a1c2 (branch main)
              https://github.com/BananaBites/bloat · 2026-09-23
              "labels: keep sizes readable at small region sizes"
  status      update available: 6eb6784 → 9f3a1c2   (run `bloat update`)
  tools       cargo 1.98.1 · git 2.43.0 · curl 8.5.0
  completions bash dynamic · snippet in ~/.bashrc
  terminal    TERM=xterm-256color · truecolor · hyperlinks on · locale en_GB.UTF-8 ✓
```

```console
$ bloat update                 # rebuild the newest commit from main
$ bloat update --check         # only report whether a newer commit exists
$ bloat update --force         # reinstall even if the commit already matches
$ bloat update --no-locked     # ignore Cargo.lock
```

The commit hash is the identity: versions in `Cargo.toml` can lag behind
`main`, so `doctor` and `update` compare the **git commit you built from**
against the **git commit at the remote HEAD** (`git ls-remote`, plus the commit
date and subject from the GitHub API when `curl` is available and you are
online; `bloat doctor --offline` skips the network entirely).

`bloat update` runs `cargo install --git https://github.com/BananaBites/bloat
--force --locked`, so it needs cargo — that is what put the binary there in the
first place. It prints the command, streams cargo's output, and finishes by
running `--version` on the newly installed binary. Set `BLOAT_REPO` to update
from a fork or mirror instead (`BLOAT_REPO=https://github.com/you/bloat bloat
update`).

## Why another du viewer

`dust`, `dua`, `gdu`, `ncdu` and friends all render with plain block
characters. `bloat` renders the treemap as pixels (see above) and offers:

- **Parallel scan** (jwalk/rayon) into a compact arena tree — ~120k entries in
  ~0.6 s on a small VPS — with a live treemap while it is still scanning.
- **Disk usage by default** (`st_blocks`), `-a/--apparent-size` for file length.
- **`du -x` semantics**: stays on one filesystem, skips `/proc`, `/sys`, `/dev`,
  `/run`, counts hardlinked inodes once, `-e GLOB` to exclude more.
- **Cushion treemap**: nested regions, per-directory aggregation of the long
  tail (`+N more`), name and size labels that never overlap, colours that are
  stable for a given path and darken with depth.
- **Spatial navigation** with arrows/hjkl, enter to descend, mouse hover, click
  to select, click again to descend, scroll to step.
- **Terminal niceties**: DEC 2026 synchronized updates (flicker-free redraws),
  OSC 8 clickable paths (only where the terminal supports them), OSC 52
  clipboard for `y`, animated scan readout with a live byte counter, terminal
  title while scanning.
- **`--report`** prints a static, aligned report with Unicode bar charts — used
  automatically when stdout is not a terminal, so `bloat / | head` and cron jobs
  work.
- **Self-maintenance**: `bloat doctor`, `bloat update`, `bloat completions`.

## Keys

| key | action |
| --- | --- |
| `↑ ↓ ← →` / `h j k l` | move selection spatially through the treemap |
| `enter` | descend into the selected directory |
| `backspace` / `u` / `esc` | go up one level |
| `[` `]` | less / more nesting depth |
| `space` | freeze live updates while scanning |
| `y` | copy the selected path (OSC 52, no clipboard binary needed) |
| `r` | rescan from scratch |
| `?` | help overlay |
| `q` / `ctrl-c` | quit |

## Options

```
Usage: bloat [OPTIONS] [PATH]
       bloat <COMMAND>

Commands:
  doctor       Show the installed commit, the remote commit and environment details
  update       Install the newest commit from the default branch
  completions  Print a shell completion script to stdout
  help         Print this message or the help of the given subcommand(s)

Options:
  -r, --report              print a static report instead of the TUI
  -t, --top <N>             entries listed in report mode            [default: 20]
  -a, --apparent-size       use file length instead of disk usage
  -x, --one-file-system     stay on one filesystem (default)
      --cross-filesystem    cross mount points while scanning
  -e, --exclude <GLOB>      skip entries (repeatable); a pattern with `/`
                            matches a path prefix, otherwise it globs the name
      --depth <N>           treemap nesting depth                    [default: 5]
  -j, --jobs <N>            scan threads                             [default: cores]
      --links <auto|on|off> OSC 8 hyperlinks on paths                [default: auto]
```

## Development

```sh
make                 # build everything (debug: binary + tests)
make run             # interactive treemap of this directory
make report          # static report of this directory
make test            # the test suite
make release         # optimised binary -> target/release/bloat
make run DIR=/var ARGS='-a -e *.iso'   # other directory, extra flags
```

`make help` lists every target. The Makefile is only a convenience for working
in a checkout — installing never uses it.

## Layout of the code

| file | what |
| --- | --- |
| `src/scan.rs` | parallel walk (jwalk), progress, hardlink dedup, excludes, top-N files |
| `src/arena.rs` | `Vec<Node>` tree, incremental size/file propagation, child ranking + aggregation |
| `src/treemap.rs` | squarified layout in half-block pixel space |
| `src/render.rs` | cushion shading, region outlines, labels, pixel→cell conversion, hit testing |
| `src/palette.rs` | hash → stable HSL colours, shading, contrast text |
| `src/ui.rs` | header/breadcrumb, detail line, hints, help overlay, OSC 8/52 helpers |
| `src/app.rs` | event loop, spatial selection, navigation, live rebuilds |
| `src/report.rs` | headless report with Unicode bar charts |
| `src/cli.rs` | clap definition, shared with the completion engine |
| `src/version.rs` | build stamp (git hash via `build.rs`), remote commit probe |
| `src/maintenance.rs` | `doctor`, `update`, `completions` |
| `install.sh` | the one-liner installer |
| `Makefile` | local development targets (build, run, report, test) |

Tests (`cargo test`) cover the layout invariants (exact tiling, minimum slices),
arena propagation, globbing, formatting, CLI parsing, completion generation and
a full rendered frame; `cargo test renders_a_full_frame -- --nocapture` prints a
frame as text.

## Roadmap

- **Pixel pass**: when the terminal speaks kitty graphics or sixel, upload the
  same raster as a real image instead of half-blocks (it is already a
  `Vec<[u8;3]>`).
- **Snapshot diffing**: save a JSON snapshot and colour regions by *growth*
  ("what ate my disk since last week").
- **Cold-file overlay**: tint by `mtime`/`atime` so "freeable" stands out.
- **Trash integration**: move/delete selected regions from the TUI.

## License

MIT — see [LICENSE](LICENSE).