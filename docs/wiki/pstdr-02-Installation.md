# Installation

## One-Line Installer (Linux / macOS)

Run the POSIX `sh` installer:

```sh
curl -fsSL https://raw.githubusercontent.com/kaito65452/pstdr-dsl/main/install.sh | sh
```

The installer checks for `git` and `cargo` (offering to install Rust via `rustup` if missing), prompts for optional components (`pstdr-lsp`, `PackStudio`), builds release binaries, and installs them to `$HOME/.local/bin`.

### Interactive Menu

- `1) [x] pstdr-c` — Required compiler and CLI.
- `2) [x] pstdr-lsp` — Language server (enabled by default; press `2` to toggle).
- `3) [ ] PackStudio` — Optional companion app (disabled by default; press `3` to toggle).

---

## Non-Interactive Mode

Set `PSTDR_YES=1` to skip all prompts:

```sh
PSTDR_YES=1 PSTDR_LSP=1 PSTDR_STUDIO=0 curl -fsSL https://raw.githubusercontent.com/kaito65452/pstdr-dsl/main/install.sh | sh
```

| Environment Variable | Default | Description |
| :--- | :--- | :--- |
| `PSTDR_YES` | `0` | Set to `1` to accept defaults without prompting. |
| `PSTDR_LSP` | `1` | Build and install `pstdr-lsp` (`1` or `0`). |
| `PSTDR_STUDIO` | `0` | Build and install `packstudio` (`1` or `0`). |
| `PSTDR_INSTALL_DIR` | `$HOME/.local/bin` | Target directory for installed binaries. |
| `PSTDR_REPO_URL` | `https://github.com/kaito65452/pstdr-dsl.git` | Source repository URL. |
| `PSTDR_BRANCH` | `main` | Git branch to clone. |

---

## Manual Build via Cargo

```sh
git clone https://github.com/kaito65452/pstdr-dsl.git
cd pstdr-dsl
cargo build --release --workspace
```

Binaries are produced at `target/release/pstdr-c` and `target/release/pstdr-lsp`.

---

## `PATH` Configuration

If `$HOME/.local/bin` is not in your `PATH`, add it to your shell configuration:

**Bash (`~/.bashrc`) / Zsh (`~/.zshrc`):**
```sh
export PATH="$HOME/.local/bin:$PATH"
```

**Fish (`~/.config/fish/config.fish`):**
```fish
fish_add_path $HOME/.local/bin
```

---

## Verification

```sh
pstdr-c --help
pstdr-c doctor
```

---

> [← Previous: Overview](./pstdr-01-Overview.md) | [Next: Quick Start →](./pstdr-03-Quick-Start.md)
