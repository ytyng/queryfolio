# QueryFolio

SQL client desktop app built with Tauri 2 + SvelteKit. A lightweight alternative to JetBrains DataGrip.

## Features

- MySQL / PostgreSQL / SQLite support (via sqlx)
- SSH tunnel with known_hosts verification
- Connection config in YAML, compatible with the sql-agent-mcp-server format
  - Secrets can stay in 1Password: the config YAML is fetched lazily via a getter command like `op read "op://..."`
- Query files per connection, auto-saved (`~/.config/queryfolio/sqlfiles/<connection>/*.sql`)
- CodeMirror 6 SQL editor with per-engine dialect, statement highlighting, and Cmd+Enter to run the statement under the cursor
- Results grid with CSV / TSV / JSON copy (formula-injection safe)
- psql-style meta commands (`\l` `\dt` `\dv` `\dn` `\du` `\d [table]`) translated to catalog queries, with MySQL / SQLite equivalents where possible
- AI SQL generation (OpenAI): describe the query in natural language and the generated SQL is inserted into the editor (never auto-executed). Only the schema (table / column names), engine dialect, and your instruction are sent — never query results

## Setup

```shell
pnpm install
pnpm tauri dev
```

## Configuration

Everything lives in one file: `~/.config/queryfolio/config.yml` (see `config.example.yaml`). The `sql_servers` key accepts either the server list itself, or a source declaration pointing to where the list comes from:

```yaml
# Inline (sql-agent-mcp-server compatible)
sql_servers:
  - name: dev-postgres
    engine: postgres
    host: localhost
    ...

# Or fetch from 1Password (exactly one of command / env / file)
# sql_servers:
#   command: op read "op://development/queryfolio/config-yaml"

# Or from a file / an environment variable
# sql_servers:
#   file: ~/secrets/sql-servers.yaml
# sql_servers:
#   env: QUERYFOLIO_CONNECTIONS_YAML

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

The `ai:` section can live at the top level of the local config file, or at the top level of the connection YAML fetched via a source declaration. When both exist, the fetched YAML wins — so the API key can stay in 1Password together with the connection secrets.

The `QUERYFOLIO_CONFIG_YAML` environment variable overrides the whole config file (for development; GUI apps launched from Finder do not inherit shell env vars).

## Development

```shell
pnpm check                   # svelte-check
cd src-tauri && cargo test   # Rust unit tests
pnpm tauri build             # release build
```

See `AGENTS.md` for architecture details.

## License

MIT
