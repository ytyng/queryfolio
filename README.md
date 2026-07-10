# queryfolio

SQL client desktop app built with Tauri 2 + SvelteKit. A lightweight alternative to JetBrains DataGrip.

## Features

- MySQL / PostgreSQL / SQLite support (via sqlx)
- SSH tunnel with known_hosts verification
- Connection config in YAML, compatible with the sql-agent-mcp-server format
  - Secrets can stay in 1Password: the config YAML is fetched lazily via a getter command like `op read "op://..."`
- Query files per connection, auto-saved (`~/.config/queryfolio/sqlfiles/<connection>/*.sql`)
- CodeMirror 6 SQL editor with per-engine dialect, statement highlighting, and Cmd+Enter to run the statement under the cursor
- Results grid with CSV / TSV / JSON copy (formula-injection safe)

## Setup

```shell
pnpm install
pnpm tauri dev
```

## Connection config

Create `~/.config/queryfolio/config.yaml` (see `config.example.yaml`), or open the in-app settings (⚙) and register a getter command such as:

```
op read "op://development/queryfolio/config-yaml"
```

Environment variables `QUERYFOLIO_CONFIG_YAML` and `QUERYFOLIO_CONFIG_YAML_GETTER_COMMAND` are also supported (mainly for development; GUI apps launched from Finder do not inherit shell env vars).

## Development

```shell
pnpm check                   # svelte-check
cd src-tauri && cargo test   # Rust unit tests
pnpm tauri build             # release build
```

See `AGENTS.md` for architecture details.

## License

MIT
