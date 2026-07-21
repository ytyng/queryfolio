# QueryFolio

SQL client desktop app built with Tauri 2 + SvelteKit. A lightweight alternative to JetBrains DataGrip.

https://github.com/user-attachments/assets/90439816-49c8-4ebd-a068-b102cfe9c7aa

![QueryFolio screenshot](docs/screenshot.png)

## Features

- MySQL / PostgreSQL / SQLite support (via sqlx)
- SSH tunnel with known_hosts verification
- Connection config in YAML, compatible with the sql-agent-mcp-server format
  - Secrets can stay in 1Password: the config YAML is fetched lazily via a getter command like `op read "op://..."`
- Query files per connection, auto-saved (`~/.config/queryfolio/sqlfiles/<folder>/*.sql`, where `<folder>` is `folder_name` or `<host>_<engine>_<schema>_<user>`)
- CodeMirror 6 SQL editor with per-engine dialect, statement highlighting, Cmd+Enter to run the statement under the cursor, and schema-based autocompletion (table / column names)
- SQL formatting for SELECT statements (conservative: unsupported or unparsable statements are left untouched)
- Schema browser (TABLES pane) with lazy-loaded columns
- Results in tabs with pinning, a cell inspector, and CSV / TSV / JSON copy (formula-injection safe)
- Query cancellation while running
- Per-connection query history (searchable, stored locally with restrictive file permissions)
- psql-style meta commands (`\l` `\dt` `\dv` `\dn` `\du` `\d [table]`) translated to catalog queries, with MySQL / SQLite equivalents where possible
- `\c <database>` switches the active database of the connection (MySQL / PostgreSQL). The pool is rebuilt, and the database selector, schema browser, and SQL completion follow
- `readonly: true` per connection rejects write statements (INSERT / UPDATE / DELETE / DDL, including CTE-wrapped DML) as a safety guard
- Auto `LIMIT` for SELECTs without one (`default_limit`, default 500)
- AI features (OpenAI): SQL generation from natural language, Fix with AI on query errors, EXPLAIN plan analysis with index suggestions, and explanation of selected SQL. Generated SQL is inserted into the editor, never auto-executed. Only the schema (table / column names), engine dialect, statements, and plans are sent — never query results
- Resizable panes: drag the dividers between the sidebars, editor, and results; sizes are persisted across restarts
- Window size / position restored across restarts

## Setup

```shell
pnpm install
pnpm tauri dev
```

## Configuration

Everything lives in one file: `~/.config/queryfolio/config.yml` (see `config.example.yaml`). Connections are written under `sql_servers`, and any key can be overridden from an external source with `config_override_command`:

```yaml
# Inline (sql-agent-mcp-server compatible)
sql_servers:
  - name: dev-postgres
    engine: postgres
    host: localhost
    readonly: true   # optional: reject write statements on this connection
    ...

# Servers can be grouped for the connections list (queryfolio extension).
# Group entries and plain servers can be mixed; order is preserved.
# sql_servers:
#   - group_name: production
#     sql_servers:
#       - name: prod-main
#         ...
#   - name: ungrouped-db
#     ...

# Optional: run a command whose stdout is YAML, and merge it over this file.
# Mappings are merged recursively; scalars and lists (including sql_servers)
# are replaced wholesale. Any key can be overridden this way.
# config_override_command: op read "op://development/queryfolio/config-yaml"

# Optional
sqlfiles_dir: ~/queries
default_limit: 500   # auto-appended to SELECTs without LIMIT (0 = disabled)

# Optional: AI SQL generation (OpenAI)
ai:
  provider: openai   # only openai is supported for now
  api_key: sk-your-api-key
  model: gpt-5.1     # optional (default: gpt-5.1)
  base_url: https://api.openai.com/v1  # optional (for OpenAI-compatible APIs)
```

The `ai:` section can live at the top level of the local config file, or at the top level of the YAML fetched by `config_override_command`. The fetched YAML wins (that is just the merge result) — so the API key can stay in 1Password together with the connection secrets.

The `QUERYFOLIO_CONFIG_YAML` environment variable overrides the whole config file (for development; GUI apps launched from Finder do not inherit shell env vars).

## Development

```shell
pnpm check                   # svelte-check
cd src-tauri && cargo test   # Rust unit tests
pnpm tauri build             # release build (macOS: signed with Developer ID via the tauri script; not notarized)
```

See `AGENTS.md` for architecture details.

## Release (macOS)

The signed macOS build is produced on GitHub Actions (`.github/workflows/build-macos.yml`,
manual trigger only) and published as a GitHub Release. Kick it off with Fabric:

```shell
fab build_mac                # build a universal (Apple Silicon + Intel) app, draft Release
fab build_mac:draft=false    # publish the Release directly
fab -l                       # list all tasks (dev / check / unittest / build_local / build_mac / releases)
```

The workflow signs with the Developer ID certificate (Cyberneura K.K.) when the signing
secrets are configured; it is not notarized. See the `publish-macos-release` skill
(`.claude/skills/publish-macos-release/`) for the full build → verify → publish runbook,
including the one-time signing-secrets setup.

## License

MIT
