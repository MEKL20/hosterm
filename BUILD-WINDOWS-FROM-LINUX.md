# Cross-building the Windows .exe from Linux (reproducible recipe)

This produced `dist-win/hosterm.exe` (PE32+ GUI, x86-64) on an Ubuntu 22.04 box.
It builds the **binary only** — the installer (.msi / NSIS .exe) still needs a Windows
runner (see `.github/workflows/build-windows.yml`).

## One-time toolchain setup

```bash
# Rust Windows MSVC target
rustup target add x86_64-pc-windows-msvc

# cross-build helper (downloads Windows SDK/CRT automatically)
cargo install cargo-xwin

# MSVC-compatible LLVM tools (Debian/Ubuntu)
sudo apt-get install -y llvm clang lld
# cargo-xwin expects these names on PATH:
sudo ln -sf /usr/lib/llvm-14/bin/clang /usr/bin/clang-cl   # clang in MSVC driver mode
# llvm-rc, llvm-lib, lld-link come from the packages above
```

Tools needed and why:
- `clang-cl`  — C compiler in MSVC mode (cc-rs uses it for C deps)
- `llvm-rc`   — Windows resource compiler (embeds icon/version into the .exe)
- `llvm-lib`  — MSVC static-lib archiver
- `lld-link`  — MSVC-compatible linker

## Build

```bash
cd src-tauri
cargo xwin build --release --target x86_64-pc-windows-msvc
# -> target/x86_64-pc-windows-msvc/release/hosterm.exe
```

The `LNK4099` warnings about missing `.pdb` files are harmless (xwin's CRT ships
without debug symbols); the resulting binary is valid.

## Runtime requirement on the target Windows machine

Tauri uses the system **WebView2** runtime, which is preinstalled on current
Windows 10/11. On older/stripped installs, install "Microsoft Edge WebView2 Runtime".

## Not yet verified

The .exe is a valid PE binary but has **not been run on a real Windows machine**
from here (no Windows host / wine available). Smoke-test on Windows, or use the
GitHub Actions workflow which both builds and could run a basic launch check.
