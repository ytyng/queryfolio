# queryfolio

JetBrains DataGrip の代替を目指す SQL クライアントデスクトップアプリ。

## 技術スタック

- **Tauri 2** (Rust バックエンド + WKWebView)
- **SvelteKit + Svelte 5 (runes)** / TypeScript / Vite 6
- **Tailwind CSS 4** (@tailwindcss/vite プラグイン方式)
- **CodeMirror 6** + @codemirror/lang-sql (SQL エディタ)
- **sqlx 0.8** (MySQL / PostgreSQL / SQLite)
- **ssh2** (SSH ローカルポートフォワードトンネル)
- パッケージマネージャ: pnpm

Tauri 2 / Svelte 5 / Tailwind 4 は比較的新しいため、API に迷ったら context7 MCP でドキュメントを参照すること。

## コマンド

```shell
pnpm tauri dev          # 開発起動 (Rust ビルド + vite dev + ネイティブウインドウ)
pnpm check              # svelte-check (型チェック)
cd src-tauri && cargo test   # Rust ユニットテスト
cd src-tauri && cargo check  # Rust 型チェック
pnpm tauri build        # リリースビルド
```

## アーキテクチャ

### Rust (src-tauri/src/)

| ファイル | 役割 |
|---------|------|
| lib.rs | Tauri コマンド定義と AppState (接続設定キャッシュ + DbManager) |
| config.rs | 接続設定 YAML のロード・テンプレート展開・getter command 実行 |
| settings.rs | アプリ設定 (~/.config/queryfolio/settings.json)、expand_tilde |
| db.rs | sqlx プール管理、クエリ実行、型別 JSON 変換 |
| tunnel.rs | SSH ローカルポートフォワード (known_hosts 検証付き) |
| query_files.rs | クエリファイル CRUD (パストラバーサル対策) |
| error.rs | AppError (フロントには文字列でシリアライズ) |

### フロントエンド (src/)

- `lib/api.ts` — invoke の型付きラッパー (バックエンドとの境界)
- `lib/stores/app.svelte.ts` — Svelte 5 runes ストア (getter + メソッドを default export)
- `lib/components/` — Toolbar / ConnectionsPane / FilesPane / SqlEditor / ResultsPane / SettingsModal
- `lib/export.ts` — CSV/TSV/JSON 変換 (formula injection 対策込み)

## 接続設定の解決順

1. `QUERYFOLIO_CONFIG_YAML` 環境変数 (YAML 文字列そのもの。テスト用に便利)
2. `QUERYFOLIO_CONFIG_YAML_GETTER_COMMAND` 環境変数
3. アプリ設定の `config_yaml_getter_command` (例: `op read "op://..."`)
4. アプリ設定の `config_yaml_path`
5. `~/.config/queryfolio/config.yaml`

YAML フォーマットは sql-agent-mcp-server (~/workspace/sql-agent-mcp-server) と互換。`config.example.yaml` 参照。sqlite は `schema` を DB ファイルパスとして扱う独自拡張。

クエリファイルの保存先: `~/.config/queryfolio/sqlfiles/<connection>/<name>.sql` (設定で変更可)。

## 開発上の注意

- ユーザーアクションを受ける要素には `data-annotate="<識別子>"` を付ける (E2E テスト用)。
- `window.prompt` / `alert` / `confirm` は使わない (ブラウザ自動化がブロックされる + UX)。
- 64bit 整数は JS の Number.MAX_SAFE_INTEGER を超えると Tauri invoke 境界で丸められるため、db.rs の json_i64 / json_u64 で範囲外は文字列化している。
- sqlx は Postgres の数値型互換が厳密 (INT4 は i32 でしかデコードできない等)。デコード追加時は ~/.cargo/registry の sqlx ソースで `compatible` 実装を確認すること。
- sqlite `:memory:` はプール接続ごとに別 DB になる。テストでは max_connections(1) にする。
- 実機検証はテスト用 SQLite DB を作り `QUERYFOLIO_CONFIG_YAML` 環境変数で注入して `pnpm tauri dev` を起動すると、ユーザーの実設定を汚さない。orca computer-use で操作する場合、文字入力は type-text でなく paste-text を使う (type-text は二重配送することがある)。
