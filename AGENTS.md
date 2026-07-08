# AGENTS.md — cnowledje

Read-only Confluence CLI (Server / Data Center) written in Rust.

## Commands

```bash
cargo build                  # debug build
cargo build --release        # release build
cargo test                   # unit tests (in-module) + integration tests
cargo clippy                 # lint
cargo fmt                    # format
cargo install --path .       # install binary as `cnowledje`
nix build                    # Nix package build
nix develop                  # enter dev shell (provides bashInteractive)
```

No CI exists yet. Run `cargo fmt && cargo clippy && cargo test` before committing.

## Project structure

| Path | Role |
|---|---|
| `src/lib.rs` | Library crate — re-exports all public modules |
| `src/main.rs` | Binary entrypoint; adds `mod cli` (not part of lib) |
| `src/cli.rs` | clap arg structs — **binary-only, not in lib** |
| `src/client.rs` | `ConfluenceClient` — HTTP (GET-only) |
| `src/config.rs` | Config loading: env vars → system keyring (token only) → TOML file → defaults |
| `src/cql.rs` | CQL generation + page ID extraction |
| `src/markdown.rs` | Confluence storage HTML → Markdown converter |
| `src/models.rs` | API response types + CLI output types + `NOTICE` constant |
| `src/skill.rs` | Bundled SKILL.md (`include_str!`) + `install_skill` (writes to `~/.agents/skills`) |
| `src/types.rs` | `SearchIn`, `PageFormat` enums |
| `src/error.rs` | `ConfluenceError` enum |
| `src/format.rs` | Output formatting helpers |
| `tests/integration_tests.rs` | Integration tests — use lib crate, no live HTTP |

> `src/cli.rs` is NOT exported from `src/lib.rs`. Integration tests import from the lib crate directly.

## Configuration

Priority: **env vars > system keyring (token only) > TOML file > hard-coded defaults**

| Env var | Required | Default |
|---|---|---|
| `CONFLUENCE_BASE_URL` | env **or** TOML `base_url` | — |
| `CONFLUENCE_TOKEN` | env, keyring, or both (never in TOML) | — |
| `CONFLUENCE_API_PATH` | No | `/rest/api` |
| `CONFLUENCE_ALLOWED_SPACES` | No | (all spaces allowed) |
| `CONFLUENCE_DEFAULT_SPACE` | No | — |

Config file: `~/.config/cnowledje/config.toml` (profiles: `[default]`, `[staging]`, …)

TOML-only settings (no env var override): `default_limit` (default: 10), `max_limit` (default: 50), `max_page_chars` (default: 50000). Note: `default_limit` is loaded but not currently applied to the CLI's `--limit` default (which is hardcoded to 10 in clap).

**Token resolution order: `CONFLUENCE_TOKEN` env var → system keyring (service `cnowledje`, account = profile name) → error. Never write the token in the config file.**

Token keyring commands: `cnowledje config token set [--profile <name>]` / `cnowledje config token delete [--profile <name>]`

`cnowledje config check` validates config and makes a live API connectivity check — requires a real Confluence instance to succeed.

`cnowledje skill install [--force]` writes the embedded `confluence-lookup` SKILL.md to `~/.agents/skills/confluence-lookup/SKILL.md`. Use `--force` when upgrading the binary to overwrite an older installed version.

## Key implementation details

- **`both` search** runs two CQL queries concurrently via `tokio::try_join!` (title + text), deduplicates by page ID, title matches sorted first. Internal fetch limit per query: `min(limit * 2, max_limit)`.
- **CQL is generated internally** — raw CQL input from users/agents is intentionally not supported.
- **Token redaction** — `CONFLUENCE_TOKEN` is never logged. Tracing output goes to stderr only.
- **Confluence macros** (`ac:structured-macro`) — supported macros are converted as follows:
  - `expand` → `**▸ title**` + body inline
  - `code` / `noformat` → fenced code block (with language for `code`)
  - `info` / `note` / `warning` / `tip` → `> **Label:**` blockquote
  - `panel` → blockquote with optional title header
  - `status` → inline `[title]` badge (rendered as `<span>` to avoid breaking paragraphs)
  - `toc` → `[TOC]`
  - `anchor` → silent (no output)
  - `excerpt-include` / `excerpt-includeplus` → `> [excerpt from: Page Name]` placeholder (cross-page fetch is out of scope). `run_page` resolves the referenced page's ID via a CQL exact-title search (scoped to `ri:space-key` if present, else the current page's space) and appends it as `(id: 123456)`; falls back silently to title-only on a search miss/error. `markdown::extract_excerpt_refs` + `html_to_markdown_with_excerpt_ids` do the extraction/injection — `html_to_markdown` itself stays a pure, network-free function.
  - `sv-translation` → language-selected expansion (see `--language` flag)
  - All other macros → `[unsupported confluence macro: NAME]`
- **Content truncation** appends `[content truncated]` when `max_chars` is exceeded; effective limit is `min(--max-chars, config.max_page_chars)`, counted in Unicode chars, not bytes.
- **JSON error output** — in `--json` mode, errors are emitted as `{"error":{"kind":"…","message":"…"}}`.
- **Space allowlist** — if `allowed_spaces` is set, passing an unlisted space to `search` is a hard error. `page <id>` does **not** check `allowed_spaces`; access is controlled solely by the token's Confluence permissions.
- **Page URL formats** — `page` accepts a numeric ID or a URL containing `?pageId=<id>` or `/pages/<id>`. Pretty URLs like `/display/SPACE/Title` are **not** supported and return an error.

## Testing

Unit tests live inside each module (`#[cfg(test)]`). Integration tests are in `tests/integration_tests.rs` and cover:
- CQL generation (single/multi-space, escaping, `both`/`title`/`text` modes)
- Page ID extraction from numeric strings and URLs (`?pageId=` and `/pages/<id>` patterns)
- Markdown conversion (headings, lists, tables, code blocks, macros, Japanese UTF-8, truncation, sv-translation language selection)
- Format helpers (`make_page_url`)

No live HTTP tests exist. Mock server tests are a planned future addition.

To run a single test: `cargo test <test_fn_name>`

## Intentionally out of scope

Do not add: raw CQL input, write operations (POST/PUT/PATCH/DELETE), CQL `OR` for `both` mode (uses two separate queries by design), MCP server, RAG/embeddings, OAuth/SSO, attachment upload/delete.
