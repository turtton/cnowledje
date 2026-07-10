# AGENTS.md — cnowledje

Read-only Confluence & Jira CLI (Server / Data Center) written in Rust.

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
| `src/jira_client.rs` | `JiraClient` — HTTP (GET-only), shares `build_http_client`/`handle_response` with `client.rs` |
| `src/config.rs` | Config loading: env vars → system keyring (token only) → TOML file → defaults. Also loads `JiraConfig` (`load_jira_config`) |
| `src/cql.rs` | CQL generation + page ID extraction |
| `src/jql.rs` | JQL generation + issue key extraction (Jira analogue of `cql.rs`) |
| `src/markdown.rs` | Confluence storage HTML → Markdown converter; also renders Jira issue description/comments (`render_issue_content`) |
| `src/models.rs` | Confluence + Jira API response types, `UnifiedSearchOutput` and other CLI output types, `NOTICE`/`JIRA_NOTICE` constants |
| `src/skill.rs` | Bundled `SKILL.md` files (`BUNDLED_SKILLS`, `include_str!`) + `install_skill` (writes to `~/.agents/skills`) |
| `src/types.rs` | `SearchIn`, `SearchSource`, `PageFormat`, `IssueFormat` enums |
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
| `JIRA_BASE_URL` | env **or** TOML `jira_base_url` | — |
| `JIRA_TOKEN` | env, keyring, or both (never in TOML) | — |
| `JIRA_API_PATH` | No | `/rest/api/2` |
| `JIRA_ALLOWED_PROJECTS` | No | (all projects allowed) |
| `JIRA_DEFAULT_PROJECT` | No | — |

Config file (path resolved via `dirs::config_dir()`, profiles: `[default]`, `[staging]`, …):
- Linux: `~/.config/cnowledje/config.toml`
- macOS: `~/Library/Application Support/cnowledje/config.toml`
- Windows: `%APPDATA%\cnowledje\config.toml`

TOML-only settings (no env var override): `default_limit` (default: 10), `max_limit` (default: 50), `max_page_chars` (default: 50000, also the shared Jira issue char budget — see below). Note: `default_limit` is loaded but not currently applied to the CLI's `--limit` default (which is hardcoded to 10 in clap).

**Token resolution order:**
- Confluence: `CONFLUENCE_TOKEN` env var → system keyring (service `cnowledje`, account = profile name) → error.
- Jira: `JIRA_TOKEN` env var → system keyring (service `cnowledje-jira`, account = profile name) → error.

Never write either token in the config file.

Token keyring commands: `cnowledje config token set [--profile <name>] [--jira]` / `cnowledje config token delete [--profile <name>] [--jira]` (`--jira` targets the `cnowledje-jira` keyring service instead of `cnowledje`).

`cnowledje config check` validates Confluence and Jira **independently** — a backend with no `base_url` configured prints `(not configured)` and is skipped; a backend that resolves `base_url` **and** a token gets its fields printed plus a live connectivity check (Confluence: `content/search`; Jira: `/myself`); a backend with `base_url` set but no resolvable token (or any other config error) prints `configuration error: ...` and is recorded as a failure without a connectivity check. If *both* backends are entirely unconfigured, it errors with `MissingBaseUrl` (legacy single-backend behavior preserved). Otherwise, the first failure encountered across either backend is returned as the command's error.
`cnowledje config init [--profile <name>] [--confluence] [--jira]` updates configuration interactively without replacing an entire profile. `--confluence` and `--jira` select only those sections and skip the section-selection prompts; without either flag, each section is offered interactively (configured sections default to No). Selected fields are prefilled from the profile and merged back, while unselected backend fields remain unchanged. Existing profiles also keep shared limits unless their separate confirmation is accepted. `config init` has no `--force` option.


`cnowledje skill install [--force]` writes every entry in `skill::BUNDLED_SKILLS` (currently `confluence-lookup` and `jira-lookup`) to `~/.agents/skills/<name>/SKILL.md`. Use `--force` when upgrading the binary to overwrite an older installed version; the command aborts on the first skill whose destination differs and `--force` wasn't passed (already-installed skills before it are not rolled back).

## Key implementation details

- **`both` search** runs two CQL queries concurrently via `tokio::try_join!` (title + text), deduplicates by page ID, title matches sorted first. Internal fetch limit per query: `min(limit * 2, max_limit)`.
- **CQL is generated internally** — raw CQL input from users/agents is intentionally not supported.
- **Token redaction** — `CONFLUENCE_TOKEN` and `JIRA_TOKEN` are never logged; both `ConfluenceClient`/`JiraClient` mark their Bearer header `sensitive` via the shared `client::build_http_client`. Tracing output goes to stderr only.
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
- **JQL is generated internally** (`src/jql.rs`) — raw JQL input from users/agents is intentionally not supported. `build_search_jql` AND-joins clauses in a fixed order (project → text → status → assignee → reporter → issuetype → labels) with a trailing ` ORDER BY updated DESC`; repeatable filters (`--status`/`--type`/`--label`) render as `field in (...)` (OR semantics) when more than one value is given.
- **Unified search routing** — `search` targets both backends when `--source` is omitted or is `all`; `--source confluence` and `--source jira` target only the named backend. `--space`/`--in` are Confluence-only; `--project`/`--status`/`--assignee`/`--reporter`/`--type` are Jira-only; `--label` applies to both backends. Using a backend's flags while `--source` excludes it is an argument error. A query-less search requires a Jira filter or `--label`; `--label` permits label-only Confluence search, while `--in` requires a query.
- **Confluence label search and metadata** — `--label` is a shared filter, AND-joined as a CQL `label` clause; repeated labels use `label in (...)` with OR semantics. A query-less label-only search uses one `build_label_cql` query and reports `matched_by: ["label"]`; `--label` alone does not pin Jira. `search` and `get_page` expand `metadata.labels` and include `labels` in their output.
- **Search configuration and execution** — a backend selected explicitly by `--source` (including `--source all`) or one with its own flags is pinned, so its configuration errors fail the command. An unpinned backend may be skipped with a stderr warning only for a missing base URL or missing default space/project; other errors fail, and if both are skipped the first skip error is returned. When both legs run, they use `tokio::try_join!`, so either failure fails the command. Human output always labels each executed backend; JSON serializes `UnifiedSearchOutput` as `{ "query": string | null, "confluence": SearchOutput | null, "jira": JiraSearchOutput | null }`, retaining both backend keys even when one is `null`.
- **`search --source jira` runs a single JQL query** — no dedup/multi-query merge like Confluence's `both` mode.
- **`issue` rendering** — `expand=renderedFields` HTML is converted via the existing `html_to_markdown` (which handles Confluence `ac:` macros too, but Jira's rendered HTML simply doesn't contain them; `language` is always `None` since `sv-translation` is Confluence-only); when rendered HTML is absent/empty, raw Jira wiki markup is truncated as plain text instead. Description and comments share one character budget (`min(--max-chars, JiraConfig::max_issue_chars)`, where `max_issue_chars` is sourced from the shared TOML `max_page_chars` setting); comments dropped once the budget is exhausted are reported via `omitted_comments` in the output rather than silently disappearing.
- **Jira issue key extraction** (`jql::extract_issue_key`) accepts a bare key (`PROJ-123`, case-normalized to uppercase) or a URL containing `/browse/<KEY>`; other URL shapes (e.g. Cloud's `?selectedIssue=`) are unsupported and return `InvalidIssueKey`.
- **Jira keyring** uses a separate service name `cnowledje-jira` (vs. Confluence's `cnowledje`) so both tokens can coexist per profile; `config::keyring_entry` is the shared private helper behind both.
- **Jira project allowlist** — `search --source jira` enforces `jira_allowed_projects` via `validate_projects` (same shape as Confluence's `allowed_spaces`). `issue <key>` does **not** check it — same policy as `page <id>`; access is controlled solely by the token's Jira permissions.

## Testing

Unit tests live inside each module (`#[cfg(test)]`). Integration tests are in `tests/integration_tests.rs` and cover:
- CQL generation (single/multi-space, escaping, label clauses, `both`/`title`/`text` modes)
- Page ID extraction from numeric strings and URLs (`?pageId=` and `/pages/<id>` patterns)
- Markdown conversion (headings, lists, tables, code blocks, macros, Japanese UTF-8, truncation, sv-translation language selection)
- Jira issue rendering (`render_issue_content`: HTML/raw fallback, char-budget exhaustion + `omitted_comments`, empty description/comments)
- JQL generation (project/status/issuetype/label single vs. `in (...)`, clause order + trailing sort, filters-only queries) and issue key extraction (bare key, `/browse/<KEY>` URL, invalid input)
- Format helpers (`make_page_url`, `make_issue_url`) and `UnifiedSearchOutput` JSON serialization, including stable `null` backend keys
- Config round-trips including `jira_*` `ProfileConfig` fields and `load_profile_config_at_path` merge preservation
- Bundled skill set (`skill::BUNDLED_SKILLS` contains exactly `confluence-lookup` and `jira-lookup`)
- Unified search source planning (routing, query-less filter searches, and conflicting backend flags) and clap parsing for flattened `issue`, `search --source`, and `config init` section flags

No live HTTP tests exist. Mock server tests are a planned future addition.

To run a single test: `cargo test <test_fn_name>`

## Intentionally out of scope

Do not add: raw CQL input, raw JQL input, write operations (POST/PUT/PATCH/DELETE), CQL `OR` for `both` mode (uses two separate queries by design), MCP server, RAG/embeddings, OAuth/SSO, attachment upload/delete.
