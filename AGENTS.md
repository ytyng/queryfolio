# queryfolio

JetBrains DataGrip の代替を目指す SQL クライアントデスクトップアプリ。

## 技術スタック

- **Tauri 2** (Rust バックエンド + WKWebView)
- **SvelteKit + Svelte 5 (runes)** / TypeScript / Vite 6
- **Tailwind CSS 4** (@tailwindcss/vite プラグイン方式)
- **CodeMirror 6** + @codemirror/lang-sql (SQL エディタ)
- **Bootstrap Icons** (bootstrap-icons パッケージ、app.css で CSS フォントを import)
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
| db.rs | sqlx プール管理、クエリ実行・キャンセル (CancelRegistry)、型別 JSON 変換、readonly ガード / 危険な文ガード (dangerous_reason) |
| tunnel.rs | SSH ローカルポートフォワード (known_hosts 検証付き)。ssh-agent 認証時は使う agent socket を `ssh_tunnel.identity_agent` → `~/.ssh/config` の IdentityAgent → SSH_AUTH_SOCK の順で解決し libssh2 の `set_identity_path` で指定する (GUI 起動でシェルの SSH_AUTH_SOCK を継承しなくても 1Password 等の agent に届く)。ssh_config パーサは Include の条件付き展開・glob・Host マッチ・エスケープ/コメント除去に対応 (best-effort) |
| query_files.rs | クエリファイル CRUD (パストラバーサル対策) |
| meta_commands.rs | psql 風メタコマンド (\l \dt \dv \dn \du \d) をエンジン別カタログ SQL に変換。識別子バリデーションでインジェクション拒否 |
| history.rs | クエリ実行履歴。接続ごとに JSONL (~/.config/queryfolio/history/<connection>.jsonl) へ追記、上限 10,000 行でローテーション。SQL に機密が含まれ得るためディレクトリ 700 / ファイル 600 |
| schema_info.rs | テーブル・カラムのカタログ照会と SchemaCache (接続+スキーマ単位のキャッシュ。スキーマブラウザと SQL 補完用 get_schema_map で共有) |
| ai.rs | AI 基盤 (AiConfig の解決・OpenAI Chat Completions 呼び出し chat_complete・SQL 生成 / EXPLAIN 解説 / 選択 SQL 解説プロンプト整形・フェンス剥がし)。API キーはフロントに渡さない (get_ai_info は configured / model のみ) |
| error.rs | AppError (フロントには文字列でシリアライズ) |

### フロントエンド (src/)

- `lib/api.ts` — invoke の型付きラッパー (バックエンドとの境界)
- `lib/stores/app.svelte.ts` — Svelte 5 runes ストア (getter + メソッドを default export)
- `lib/components/` — Toolbar (グローバルツールバー。Writable スイッチを含む) / ConnectionsPane / FilesPane / HistoryPane / TablesPane (スキーマブラウザ) / SqlEditor / EditorToolbar / ResultsPane / CellInspector / ConfigInfoModal (読み取り専用の設定表示) / AiAnalysisModal (EXPLAIN / 選択 SQL の AI 解説表示) / PaneDivider (ドラッグ可能なペイン区切り線)
- Writable スイッチ — ツールバーの `data-annotate="toggle-writable"` トグル。OFF (既定、セッションごとに OFF から始め永続化しない) の間は SELECT/SHOW 等の副作用の無い文しか実行できない。app.svelte.ts の `writable` state が run_query に `writable` として渡り、バックエンド (lib.rs) が readonly ガードの由来を `db.rs` の `ReadonlyGuard` (Off / Config / Switch) で決めて強制する。実効 readonly = `config readonly || !writable`。config で `readonly: true` の接続はスイッチより優先 (ロック表示 = `writable-locked`) で解除できない。ブロック時のメッセージは由来で出し分ける (Config / Switch)
- ペインのサイズ変更 — `+page.svelte` が接続一覧幅 / サイドバー幅 / エディタ縦割合を `$state` で管理し、`PaneDivider` (Pointer Events + setPointerCapture) のドラッグで変更する。ドラッグ終了時に localStorage (`queryfolio.layout.*`) へ保存し起動時に復元。各ペインコンポーネントの root は `w-full` で、幅は `+page.svelte` のラッパー div が inline style で与える
- `lib/export.ts` — CSV/TSV/JSON 変換 (formula injection 対策込み)
- `lib/sqlFormat.ts` — SQL 整形器 (自前トークナイザ。SELECT / UNION 系のみ整形し、INSERT / UPDATE / WITH 等やパース不能な文は原文維持。整形結果を再トークナイズして入力とトークン列が一致しなければ原文に戻す安全ネット付き)

## 設定 (config.yml)

設定は `~/.config/queryfolio/config.yml` (無ければ `config.yaml`) に一本化されている。settings.json は存在しない。

- `sql_servers` はリスト (sql-agent-mcp-server 互換の直書き) か、ソース宣言マッピングのどちらか。
- ソース宣言は `command:` / `env:` / `file:` の**ちょうど 1 つ** (複数はエラー)。取得した YAML は sql-agent 互換フォーマットとしてパースされ、さらなるソース宣言の再帰は禁止。
- `command` はシェル非経由 (shlex 分解) で実行。GUI 起動の最小 PATH 対策として /opt/homebrew/bin と /usr/local/bin を補完する。60 秒タイムアウト + kill_on_drop。
- `default_limit` (任意、デフォルト 500、0 で無効) — LIMIT 未指定の SELECT に自動で `LIMIT n` を付与する (db.rs の should_auto_limit。サブクエリ LIMIT / FOR UPDATE 等は保守的にスキップ)。
- `readonly: true` (任意、デフォルト false。sql-agent 互換フォーマットへの queryfolio 独自拡張) — その接続で書き込み系の文 (INSERT / UPDATE / DELETE / DDL 等) の実行を拒否する。判定は db.rs の is_readonly_allowed: leading_keyword が select / with / show / describe / desc / explain / pragma / values / table / call 以外なら拒否し、さらに WITH は CTE 付き DML (insert / update / delete / merge)、SELECT は SELECT INTO、EXPLAIN は EXPLAIN ANALYZE + DML (対象文を実際に実行するため) を、リテラル・コメント除去済みの単語境界判定で拒否する。メタコマンドは読み取り系のみなので常に許可。SELECT に副作用のある関数 (nextval 等) までは防げない、あくまで事故防止のガード。
- `allow_dangerous_statements: true` (任意、デフォルト false。queryfolio 独自拡張) — 省略時は危険な文 (WHERE 無しの UPDATE / DELETE、DROP、TRUNCATE) を誤操作による全行破壊・テーブル消失防止のため拒否する (db.rs の dangerous_reason: readonly と同じ scan_sql による単語境界判定。UPDATE/DELETE は where 語の有無、DROP/TRUNCATE は常に危険)。先頭キーワードだけでなく、実際に書き込みが走るラップ形 — `WITH ... DELETE/UPDATE` (Postgres の CTE 付き DML) と `EXPLAIN ANALYZE ...` (対象文を実行する) — の中の危険な DML も対象にする。true にすると実行できるが、フロントは実行前に確認ダイアログ (DangerousConfirmModal) を出す。確認要否の判定は check_dangerous_statement コマンド (db.rs の dangerous_statement_reason)。readonly が先に評価されるため readonly 接続ではこのガードには到達しない。弱点: WITH で無関係な CTE / 外側の SELECT に where があると WHERE 無し DML を見逃す (where を一切含まない典型形は捕捉)。サブクエリ内だけの where も同様に安全側 (許可側) に倒れる。
- `sqlfiles_dir` (任意) でクエリファイル保存先を変更できる。デフォルトは `~/.config/queryfolio/sqlfiles/<folder>/<name>.sql`。`<folder>` は接続ごとに `folder_name` 設定があればそれを使い、無ければ `<host>_<engine>_<schema>_<user>` を組み立てる (接続 name は使わない。config.rs の `ServerConfig::sqlfiles_folder_name`。パス区切り等はサニタイズ)。既存接続でフォルダ名が変わるとそれまでのクエリファイルは旧フォルダに残る点に注意。
- `folder_name` (任意、queryfolio 独自拡張) — クエリファイルの保存フォルダ名を明示する。省略時のフォルダ名ルールは上記 `sqlfiles_dir` を参照。
- `ssh_tunnel.identity_agent` (任意、queryfolio 独自拡張) — ssh-agent 認証で使う agent socket を明示する (OpenSSH の IdentityAgent 相当)。`none` で agent を無効化。省略時は `~/.ssh/config` の IdentityAgent → SSH_AUTH_SOCK の順で解決 (tunnel.rs)。鍵を 1Password SSH agent に置き GUI 起動する環境向けの解決策。
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
- macOS の署名は package.json の `tauri` スクリプトで `APPLE_SIGNING_IDENTITY` (Developer ID Application: Cyberneura K.K.) を設定済み。`pnpm tauri build` で署名される。公証 (notarization) は未対応。
- 実機検証はテスト用 SQLite DB を作り `QUERYFOLIO_CONFIG_YAML` 環境変数で注入して `pnpm tauri dev` を起動すると、ユーザーの実設定を汚さない。orca computer-use で操作する場合、文字入力は type-text でなく paste-text を使う (type-text は二重配送することがある)。
