# Editor Setup (`pstdr-lsp`)

`pstdr-lsp` provides real-time diagnostics, contextual completions (effects, sounds, `snake_case` helpers, predicates, and event snippets), hover transpilation previews, quick-fix code actions (`PSTDR-E001` / `PSTDR-E002`), and document formatting over stdio.

---

## Verify `pstdr-lsp` in `PATH`

```sh
command -v pstdr-lsp
```

If missing, build and install `pstdr-lsp` and ensure `$HOME/.local/bin` is in `PATH` (see [Installation](./pstdr-02-Installation.md)).

---

## PackStudio IDE (`builtin.pstdr`)

PackStudio includes a built-in `.pstdr` language server (`builtin.pstdr`) out of the box — no external `pstdr-lsp` configuration is required when editing inside PackStudio.

---

## Helix (`~/.config/helix/languages.toml`)

```toml
[language-server.pstdr-lsp]
command = "pstdr-lsp"

[[language]]
name = "pstdr"
scope = "source.pstdr"
file-types = ["pstdr"]
language-servers = ["pstdr-lsp"]
```

---

## Neovim (`editors/nvim/`)

This repository ships a ready-to-use Neovim plugin in `editors/nvim/` containing:
- `ftdetect/pstdr.lua` — registers `.pstdr` files as filetype `pstdr`
- `syntax/pstdr.vim` — syntax highlighting for all `.pstdr` keywords, units, string interpolation, and embedded JavaScript inside `unsafe` blocks
- `ftplugin/pstdr.lua` — starts `pstdr-lsp` automatically via `vim.lsp.start` with root detection (`pstdr.toml`, `BP`, `.git`)

### Option 1: Using `lazy.nvim`

Add a local plugin entry pointing to `editors/nvim`:

```lua
{
  dir = "/path/to/pstdr-dsl/editors/nvim",
  ft = "pstdr",
}
```

### Option 2: Manual `runtimepath` Copy

Copy `editors/nvim/` into your Neovim config directory:

```sh
cp -r editors/nvim/ftdetect editors/nvim/ftplugin editors/nvim/syntax ~/.config/nvim/
```

Format the active buffer with `:lua vim.lsp.buf.format()`, trigger quick-fixes with `:lua vim.lsp.buf.code_action()`, and inspect transpilation previews with `K`.

---

## VS Code / Code - OSS (`editors/vscode/`)

This repository ships a full VS Code extension in `editors/vscode/` with TextMate grammar (including embedded TypeScript/JavaScript inside `unsafe` blocks), language configuration, and a `vscode-languageclient` launcher for `pstdr-lsp`.

### Build and Install the `.vsix`

```sh
cd editors/vscode
npm install
npm run compile
npm run package
code --install-extension pstdr-vscode-0.1.0-pre-beta.1.vsix
```

### Configuration

If `pstdr-lsp` is not on your `PATH`, set its absolute path in VS Code `settings.json`:

```json
{
  "pstdr.serverPath": "/home/user/.local/bin/pstdr-lsp"
}
```

---

> [← Previous: CLI and Diagnostics](./pstdr-06-CLI-and-Diagnostics.md) | [Next: Overview →](./pstdr-01-Overview.md)
