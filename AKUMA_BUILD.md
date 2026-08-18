# Building nca for Akuma

This fork (`v0.4.0-akuma` branch) carries the patches Akuma needs directly as
commits — no separate patch file to apply. Two ways to get a binary onto an
Akuma guest: cross-compile on the host (works, this is the normal path) or
build natively inside the guest (currently blocked by a kernel bug — see
below).

## Cross-compiling on the host (working path)

From the main `akuma` repo, normally via the wrapper:

```bash
userspace/build.sh --nca-only
```

That wrapper is `userspace/nca/build.rs`, which shells out to the equivalent
of the command below and copies the result to `bootstrap/bin/nca`. **Gotcha:**
`build.rs` declares `cargo:rerun-if-changed` for only 4 files (itself,
`Cargo.toml`, `crates/cli/src/main.rs`, `crates/core/src/lib.rs`) — any
`rerun-if-changed` declaration disables cargo's default "rerun if anything in
the package changed" behaviour, so after the first build, the wrapper
silently no-ops for changes anywhere else (including all of `crates/tui/`,
which is most of them) — "Finished in 0.0Xs" with a stale binary, no error.
Until that's fixed, build the crate directly:

```bash
cd userspace/nca/native-cli-ai
CARGO_TARGET_AARCH64_UNKNOWN_LINUX_MUSL_LINKER=aarch64-linux-musl-gcc \
CC_aarch64_unknown_linux_musl=aarch64-linux-musl-gcc \
CXX_aarch64_unknown_linux_musl=aarch64-linux-musl-g++ \
AR_aarch64_unknown_linux_musl=aarch64-linux-musl-ar \
RUSTFLAGS="-C opt-level=3 -C lto=fat -C codegen-units=1 -C panic=abort -C overflow-checks=off -C target-feature=+neon,+fp16,+dotprod -C link-arg=-static" \
cargo build --release --no-default-features --target aarch64-unknown-linux-musl -p nca-cli

cp target/aarch64-unknown-linux-musl/release/nca ../../../bootstrap/bin/nca
aarch64-linux-musl-strip ../../../bootstrap/bin/nca
```

`--no-default-features` drops `arboard` (system clipboard) — no display
server over SSH, so it can't do anything on Akuma anyway; `/image <path>`
(file import) still works without it, only `/image paste` is unavailable.

Requires `aarch64-linux-musl-gcc` (`brew install FiloSottile/musl-cross/musl-cross`
on macOS) and the Rust target: `rustup target add aarch64-unknown-linux-musl`.

### Getting the binary onto a live VM

Don't touch `disk.img`/`devbox.img` while a VM has it open — populate it that
way and you corrupt it. Instead:

- `scp`/sftp doesn't work against this project's sshd (no SFTP subsystem —
  the client just hangs).
- Piping the binary through an SSH exec channel's stdin
  (`ssh ... 'cat > file' < localfile`) is unreliable past ~1 MiB — it
  reproducibly stalled at exactly 1,048,576 bytes and dropped the connection,
  twice in a row. Not investigated further; smells like a fixed buffer
  somewhere in the exec-channel path.
- **HTTP + curl works.** QEMU's SLIRP networking always makes the host
  reachable from the guest at `10.0.2.2`:

  ```bash
  mkdir -p /tmp/nca_serve && cp bootstrap/bin/nca /tmp/nca_serve/
  (cd /tmp/nca_serve && python3 -m http.server 8765 --bind 0.0.0.0) &
  ssh -p 2222 root@localhost \
    'curl -s -o /usr/local/bin/nca.new http://10.0.2.2:8765/nca && chmod +x /usr/local/bin/nca.new'
  # verify shasum -a 256 matches on both ends, then swap the file in.
  # the already-running nca process keeps using the OLD binary until you
  # exit it (Ctrl+X Q) and relaunch — the file swap alone doesn't hot-reload.
  ```

## Building inside Akuma (self-hosted) — currently blocked

Tried 2026-08-18: guest disk built with
`scripts/populate_disk.sh --with-rust-toolchain` (installs a full nightly
Rust toolchain + C toolchain under `/usr/local`, no network needed inside the
VM), project source staged at `/tmp/native-cli-ai` on the guest, then:

```bash
cd /tmp/native-cli-ai
cargo build --release -p nca-cli -j1
```

**Caveat about what was actually staged:** that source tree is a snapshot of
upstream `madebyaris/native-cli-ai` from before the TUI rewrite — its
workspace `members` list is `common, core, runtime, cli, autoresearch`, no
`crates/tui` at all (the `ratatui`/`crossterm`/`reedline` dependencies are
still inline in `crates/cli` directly). It is **not** this fork
(`v0.4.0-akuma`) and doesn't have any of the Akuma-specific patches. That
didn't end up mattering — the build fails before touching any nca code — but
it means this attempt only establishes whether *the guest toolchain* can
build *a* nca-shaped dependency tree, not whether this specific fork's source
builds self-hosted.

**Result — fails immediately**, on the first proc-macro crate's build script,
before any nca code is even reached:

```
   Compiling proc-macro2 v1.0.106
error: could not compile `proc-macro2` (build script)

Caused by:
  could not execute process `rustc --crate-name build_script_build --edition=2021 \
    /.cargo/registry/src/index.crates.io-.../proc-macro2-1.0.106/build.rs \
    --error-format=json ... --out-dir /tmp/native-cli-ai/target/release/build/proc-macro2/.../out \
    -C strip=symbols --cap-lints allow` (never executed)

Caused by:
  Bad address (os error 14)
```

This is a **known, already-investigated, still-open Akuma kernel bug** —
not an nca or cargo problem. See
`docs/archive/NCA_MISSING_SYSCALLS.md` §1 in the main `akuma` repo for the
existing writeup. Summary of what's already been ruled out there: argv/envp
size, `env_clear`, `current_dir`, threaded spawn (all reproduced clean when
mimicked directly), and a kernel-returned `EFAULT` (with
`SYSCALL_ERRNO_DIAG_ENABLED` narrowed to `EFAULT`, the kernel logs zero of
them while cargo reports three, and no `[syscall] execve` line appears for
the failing spawns at all) — so the errno is born either pre-`execve` in the
child-side `chdir`/`dup2`/`CLOEXEC`-pipe setup, or in userspace, not inside a
syscall the kernel's own logging would catch. Whoever picks this up next:
that doc is the starting point, not this file — this file only records that
nca hits the exact same wall as everything else that's tried a real
multi-crate guest build.

Until that's root-caused, self-hosting nca inside Akuma isn't possible;
cross-compiling from the host (above) is the only working path.

**Update, same day:** verified this isn't just "spawning a heavy rustc
process" — `ncaprobe bigspawn 50` spawns the exact same rustc invocation via
plain `std::process::Command` (piped stdio, matching how cargo captures JSON
diagnostics) in a loop, and all 50 succeeded, well past the ~8 spawns where
cargo itself starts failing. So it's specifically something *cargo* does
concurrently with the spawn — its jobserver fds, or its own background
threads — not the weight of the rustc invocation or the basic piped-spawn
shape. See `docs/archive/NCA_MISSING_SYSCALLS.md` §1 in the main repo for the
full detail; that's the canonical writeup, kept up to date, and this file
just points at it rather than duplicating it.
