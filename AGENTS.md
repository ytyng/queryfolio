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
| config.rs | config.yml のロード・ソース宣言解決・テンプレート展開・expand_tilde |
| db.rs | sqlx プール管理、クエリ実行・キャンセル (CancelRegistry)、型別 JSON 変換 |
| tunnel.rs | SSH ローカルポートフォワード (known_hosts 検証付き) |
| query_files.rs | クエリファイル CRUD (パストラバーサル対策) |
| schema_info.rs | テーブル・カラムのカタログ照会と SchemaCache (接続+スキーマ単位のキャッシュ。スキーマブラウザと SQL 補完用 get_schema_map で共有) |
| ai.rs | AI 基盤 (AiConfig の解決・OpenAI Chat Completions 呼び出し chat_complete・SQL 生成 / EXPLAIN 解説 / 選択 SQL 解説プロンプト整形・フェンス剥がし)。API キーはフロントに渡さない (get_ai_info は configured / model のみ) |
| error.rs | AppError (フロントには文字列でシリアライズ) |

### フロントエンド (src/)

- `lib/api.ts` — invoke の型付きラッパー (バックエンドとの境界)
- `lib/stores/app.svelte.ts` — Svelte 5 runes ストア (getter + メソッドを default export)
- `lib/components/` — Toolbar / ConnectionsPane / FilesPane / HistoryPane / TablesPane (スキーマブラウザ) / SqlEditor / EditorToolbar / ResultsPane / CellInspector / ConfigInfoModal (読み取り専用の設定表示) / AiAnalysisModal (EXPLAIN / 選択 SQL の AI 解説表示)
- `lib/export.ts` — CSV/TSV/JSON 変換 (formula injection 対策込み)

## 設定 (config.yml)

設定は `~/.config/queryfolio/config.yml` (無ければ `config.yaml`) に一本化されている。settings.json は存在しない。

- `sql_servers` はリスト (sql-agent-mcp-server 互換の直書き) か、ソース宣言マッピングのどちらか。
- ソース宣言は `command:` / `env:` / `file:` の**ちょうど 1 つ** (複数はエラー)。取得した YAML は sql-agent 互換フォーマットとしてパースされ、さらなるソース宣言の再帰は禁止。
- `command` はシェル非経由 (shlex 分解) で実行。GUI 起動の最小 PATH 対策として /opt/homebrew/bin と /usr/local/bin を補完する。60 秒タイムアウト + kill_on_drop。
- `default_limit` (任意、デフォルト 500、0 で無効) — LIMIT 未指定の SELECT に自動で `LIMIT n` を付与する (db.rs の should_auto_limit。サブクエリ LIMIT / FOR UPDATE 等は保守的にスキップ)。
- `readonly: true` (任意、デフォルト false。sql-agent 互換フォーマットへの queryfolio 独自拡張) — その接続で書き込み系の文 (INSERT / UPDATE / DELETE / DDL 等) の実行を拒否する。判定は db.rs の is_readonly_allowed: leading_keyword が select / with / show / describe / desc / explain / pragma / values / table / call 以外なら拒否し、さらに WITH は CTE 付き DML (insert / update / delete / merge)、SELECT は SELECT INTO、EXPLAIN は EXPLAIN ANALYZE + DML (対象文を実際に実行するため) を、リテラル・コメント除去済みの単語境界判定で拒否する。メタコマンドは読み取り系のみなので常に許可。SELECT に副作用のある関数 (nextval 等) までは防げない、あくまで事故防止のガード。
- `sqlfiles_dir` (任意) でクエリファイル保存先を変更できる。デフォルトは `~/.config/queryfolio/sqlfiles/<connection>/<name>.sql`。
- `ai:` (任意) — AI SQL 生成の設定 (`provider: openai` / `api_key` / `model` 任意 / `base_url` 任意)。ローカル config.yml のトップレベルと、ソース宣言で取得する接続 YAML のトップレベルの両方に書ける。**両方ある場合は接続 YAML 側を優先** (API キーを 1Password に置ける)。provider は現状 openai のみで、不明値はエラー。AppState にセッションキャッシュされ reset_connections でクリア。
- `QUERYFOLIO_CONFIG_YAML` 環境変数は設定ファイル全体を上書きする開発・テスト用フック (実機 E2E 検証で使用)。

`config.example.yaml` 参照。sqlite は `schema` を DB ファイルパスとして扱う独自拡張。

## 開発上の注意

- **アプリ名の表記はユーザーに見える箇所では「QueryFolio」に統一する** (ウインドウタイトル / ツールバー / productName / 生成される設定ファイルのコメント / README 見出し等)。リポジトリ名・bundle identifier (com.ytyng.queryfolio)・crate 名は小文字の queryfolio のまま。

- **アプリ内メッセージ (UI ラベル・トースト・placeholder・エラーメッセージ・自動生成される設定ファイルのコメント) はすべて英語で書く**。Rust の AppError 等、フロントに表示される文字列も対象。コードコメントは日本語でよい。
- ユーザーアクションを受ける要素には `data-annotate="<識別子>"` を付ける (E2E テスト用)。
- `window.prompt` / `alert` / `confirm` は使わない (ブラウザ自動化がブロックされる + UX)。
- 64bit 整数は JS の Number.MAX_SAFE_INTEGER を超えると Tauri invoke 境界で丸められるため、db.rs の json_i64 / json_u64 で範囲外は文字列化している。
- sqlx は Postgres の数値型互換が厳密 (INT4 は i32 でしかデコードできない等)。デコード追加時は ~/.cargo/registry の sqlx ソースで `compatible` 実装を確認すること。
- sqlite `:memory:` はプール接続ごとに別 DB になる。テストでは max_connections(1) にする。
- 実機検証はテスト用 SQLite DB を作り `QUERYFOLIO_CONFIG_YAML` 環境変数で注入して `pnpm tauri dev` を起動すると、ユーザーの実設定を汚さない。orca computer-use で操作する場合、文字入力は type-text でなく paste-text を使う (type-text は二重配送することがある)。
