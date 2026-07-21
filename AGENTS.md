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
fab build_mac           # macOS 版を GitHub Actions でビルド → draft Release 作成 (手動トリガー。公開は publish-macos-release スキル参照)
fab -l                  # fab タスク一覧 (dev / check / unittest / build_local / build_mac / releases)
```

macOS 版のリリースは `.github/workflows/build-macos.yml` (workflow_dispatch のみ) で universal ビルドし GitHub Release に署名付き DMG を添付する。起動は `fabfile/__init__.py` の `build_mac` タスク (`gh workflow run`)。公開までの runbook は `publish-macos-release` スキル (`.claude/skills/publish-macos-release/`。署名 Secrets の初回設定手順を含む)。

## アーキテクチャ

### Rust (src-tauri/src/)

| ファイル | 役割 |
|---------|------|
| lib.rs | Tauri コマンド定義と AppState (接続設定キャッシュ + DbManager)、メニューバーの組み立て (build_menu / rebuild_menu) |
| config.rs | config.yml のロード・config_override_command による設定の再帰マージ (load_merged / merge_mapping)・テンプレート展開・expand_tilde・設定エディタの読み書き (read_config_file / write_config_file) |
| db.rs | sqlx プール管理、クエリ実行・キャンセル (CancelRegistry)、型別 JSON 変換、readonly ガード / 危険な文ガード (dangerous_reason) |
| tunnel.rs | SSH ローカルポートフォワード (known_hosts 検証付き)。ssh-agent 認証時は使う agent socket を `ssh_tunnel.identity_agent` → `~/.ssh/config` の IdentityAgent → SSH_AUTH_SOCK の順で解決し libssh2 の `set_identity_path` で指定する (GUI 起動でシェルの SSH_AUTH_SOCK を継承しなくても 1Password 等の agent に届く)。ssh_config パーサは Include の条件付き展開・glob・Host マッチ・エスケープ/コメント除去に対応 (best-effort) |
| query_files.rs | クエリファイル CRUD (パストラバーサル対策) |
| folder_meta.rs | クエリファイル保存フォルダに接続を説明するメタファイル (`_queryfolio.md`) を生成する (エージェント/人間がフォルダを見て「どの DB 用のクエリか」を理解できるように)。非機密のみ (パスワード・SSH 鍵は含めない)。`create_query_file` / `write_query_file` / `list_query_files` の後に lib.rs (refresh_folder_meta) が書き出す。フォルダ未作成なら何もしない・内容が同じなら書かない (mtime churn 回避)。`.sql` でないため一覧・検索には出ない |
| meta_commands.rs | psql 風メタコマンド (\l \dt \dv \dn \du \d) をエンジン別カタログ SQL に変換 (MetaCommand::Sql)。識別子バリデーションでインジェクション拒否。`\c <database>` だけは SQL にならずアクティブスキーマ切替 (MetaCommand::Connect) として lib.rs が処理する |
| history.rs | クエリ実行履歴。接続ごとに JSONL (~/.config/queryfolio/history/<connection>.jsonl) へ追記、上限 10,000 行でローテーション。SQL に機密が含まれ得るためディレクトリ 700 / ファイル 600 |
| schema_info.rs | テーブル・カラムのカタログ照会と SchemaCache (接続+スキーマ単位のキャッシュ。スキーマブラウザと SQL 補完用 get_schema_map で共有) |
| ai.rs | AI 基盤 (AiConfig の解決・OpenAI Chat Completions 呼び出し chat_complete・SQL 生成 / EXPLAIN 解説 / 選択 SQL 解説プロンプト整形・フェンス剥がし)。API キーはフロントに渡さない (get_ai_info は configured / model のみ) |
| error.rs | AppError (フロントには文字列でシリアライズ) |

### フロントエンド (src/)

- `lib/api.ts` — invoke の型付きラッパー (バックエンドとの境界)
- `lib/stores/app.svelte.ts` — Svelte 5 runes ストア (getter + メソッドを default export)
- `lib/components/` — Toolbar (グローバルツールバー。Writable スイッチ・検索ボタンを含む) / ConnectionsPane / FilesPane / HistoryPane / TablesPane (スキーマブラウザ) / SqlEditor / EditorToolbar / ResultsPane / CellInspector / ConfigInfoModal (読み取り専用の設定表示) / ConfigEditorModal (config.yml のアプリ内エディタ) / AiAnalysisModal (EXPLAIN / 選択 SQL の AI 解説表示) / SearchModal (接続・クエリファイル横断検索) / PaneDivider (ドラッグ可能なペイン区切り線)
- 検索モーダル (SearchModal) — ツールバーの検索ボタン (`data-annotate="button-open-search"`) または Cmd+K / Ctrl+K (`+page.svelte` の `handleGlobalKeydown` + `<svelte:window>`) で開くコマンドパレット風モーダル。接続は `app.svelte.ts` の一覧を名前・説明で絞り込み (フロント)、クエリファイルは選択中接続のものを `search_query_files` コマンド (query_files.rs) でファイル名 + 中身検索 (大小無視の部分一致、中身は最初の一致行をプレビュー)。検索は純 Rust (rg/grep 等の外部プロセスは使わない。クエリファイルは少数のため堅牢・インジェクション面なし)。↑↓ で候補移動・Enter で開く (接続はその接続へ切替、ファイルは選択中接続で開く)・Esc で閉じる。デバウンス 150ms + 世代番号で古い応答の上書きを防ぐ
- 設定エディタ (ConfigEditorModal) — メニューバー QueryFolio の `Edit config.yml` で開く CodeMirror (YAML) のモーダル。`read_config_file` / `write_config_file` コマンド (config.rs) で ~/.config/queryfolio/config.yml を読み書きする。保存時は YAML マッピングとしてパースできることを確認してから一時ファイル + rename で書き、常に 600 で書く (config はパスワード等を平文で含み得るため、既存が 644/640 でも 600 へ絞る。ensure_config_file_in の新規生成・AppConfig::load の読込時是正と同方針)。保存後に reloadConnections まで行う。未保存で閉じようとすると破棄確認を出す。`QUERYFOLIO_CONFIG_YAML` で上書き中は編集対象のファイルが無いためエラーを返す。`config_override_command` が設定されている時だけ `View override config yaml (Copy only)` が同メニューに出て、`read_override_config_yaml` で取得した YAML を表示する。こちらは取得元が外部コマンドで書き戻せないため Save は無いが、**エディタ上では編集できる** (メモリ上だけの変更。整形してから 1Password 等の保管場所へコピーする用途)。両モードとも YAML のシンタックスハイライトに加え、`yaml` パッケージの `parseDocument` を使った lint (`@codemirror/lint`) でパースエラー・警告を行内とガター (lintGutter) に表示する
- `\c <database>` — アクティブスキーマ (database) の切替メタコマンド。SQL に変換できないので `lib.rs` の `run_query` がプール取得前に処理する (`switch_active_schema`)。`set_schema_override` でプールを捨てて張り直し、切替後の接続で確認用の `SELECT current_database()` / `SELECT DATABASE()` を実行してその結果を返す (空の結果だと成功が分かりにくいため)。接続できなければ `replace_schema_override` で元のスキーマへ巻き戻す (巻き戻さないと以降の全クエリが繋がらない状態で残る)。切替先は `QueryResult.switched_schema` でフロントへ返し、`app.svelte.ts` の `applySwitchedSchema` が `activeSchema` を更新する (スキーマブラウザは activeSchema の変化を購読しているため自動追従、補完用スキーママップは取り直す)。sqlite は schema が DB ファイルパスのため非対応
- メニューバー — macOS のアプリメニュー (QueryFolio) は NSApplication がメインメニュー設置時の内容で確定するため、tauri のデフォルトメニューに後から insert しても反映されない。そのため `Menu::default` を使わず `build_menu` でアプリメニューを含めて丸ごと組み、`Builder::menu` で最初の設置時から渡す。設定リロード時 (reset_connections) は `rebuild_menu` で組み直し、コピー用ビュー (保存不可) の項目を出し入れする
- Writable スイッチ — ツールバーの `data-annotate="toggle-writable"` トグル。OFF (既定、セッションごとに OFF から始め永続化しない) の間は SELECT/SHOW 等の副作用の無い文しか実行できない。app.svelte.ts の `writable` state が run_query に `writable` として渡り、バックエンド (lib.rs) が readonly ガードの由来を `db.rs` の `ReadonlyGuard` (Off / Config / Switch) で決めて強制する。実効 readonly = `config readonly || !writable`。config で `readonly: true` の接続はスイッチより優先 (ロック表示 = `writable-locked`) で解除できない。ブロック時のメッセージは由来で出し分ける (Config / Switch)
- ペインのサイズ変更 — `+page.svelte` が接続一覧幅 / サイドバー幅 / エディタ縦割合を `$state` で管理し、`PaneDivider` (Pointer Events + setPointerCapture) のドラッグで変更する。ドラッグ終了時に localStorage (`queryfolio.layout.*`) へ保存し起動時に復元。各ペインコンポーネントの root は `w-full` で、幅は `+page.svelte` のラッパー div が inline style で与える
- `lib/export.ts` — CSV/TSV/JSON 変換 (formula injection 対策込み)。テーブル全体用 (`toCsv` / `toTsv` / `toJson`) と選択範囲用 (`toCsvRange` / `toTsvRange` / `toJsonRange`、Cmd+C コピー用) の両系統がある
- 結果ツールバーの出力 UI (ResultsPane) — フォーマット選択プルダウン (TSV / CSV / JSON、既定 TSV、localStorage `queryfolio.results.copyFormat` に永続化) + `Copy` ボタン (テーブル全体をクリップボードへ) + `Export` ボタン (ネイティブ保存ダイアログ `@tauri-apps/plugin-dialog` の `save` で選んだパスへ Rust の `write_export_file` コマンドで書き出す)。Cmd+C の選択範囲コピーも同じ選択フォーマットに従う。`Copy with headers` チェックボックスは Cmd+C 選択コピーのヘッダ有無 (CSV/TSV のみ) に効く
- `lib/sqlFormat.ts` — SQL 整形器 (自前トークナイザ。SELECT / UNION 系のみ整形し、INSERT / UPDATE / WITH 等やパース不能な文は原文維持。整形結果を再トークナイズして入力とトークン列が一致しなければ原文に戻す安全ネット付き)

## 設定 (config.yml)

設定は `~/.config/queryfolio/config.yml` (無ければ `config.yaml`) に一本化されている。settings.json は存在しない。

- `sql_servers` はサーバー定義のリスト (sql-agent-mcp-server 互換の直書き)。マッピングを書くとエラー。
- グループ機能 (queryfolio 独自拡張) — `sql_servers` のリスト項目に `group_name:` + ネストした `sql_servers:` リストを書くと、その中のサーバーが接続一覧 (ConnectionsPane) でグループ見出し付きで表示される。パース時にフラット化され各 `ServerConfig.group_name` に記録 → `ConnectionInfo.group_name` でフロントへ (config.rs の parse_server_entries)。直書きサーバーとの混在可・設定順のまま表示。グループのネスト (深さ 2 以上) と、グループエントリの group_name / sql_servers 以外のキーはエラー。グループ内でも `template:` 継承は有効。
- `config_override_command` (任意、queryfolio 独自拡張) — 書いたコマンドを実行し、その stdout (YAML) を設定全体へ**再帰的にマージ**する (取得 YAML 側が優先)。`sql_servers` に限らずどのキーでも上書きできるため、API キーや接続情報を 1Password 等に置いたまま `default_limit` や `sqlfiles_dir` も差し替えられる。マージ規則は config.rs の `merge_mapping`: **マッピング同士は再帰的に混ぜ、スカラーとシーケンス (sql_servers を含む) は丸ごと置き換える** (リストは要素の同一性を決められないため要素単位マージはしない)。取得 YAML 側に `config_override_command` があっても再帰取得はせず、マージ後にキーを落とす。解決は `AppConfig::load_merged` (async)。`AppConfig::load` はローカルファイルのみ読む同期版で、メニュー出し分け (has_config_override_command) 等で使う。**マージ済み設定は AppState に 1 つだけセッションキャッシュされる** (取得コマンドは 1Password 等で数秒 + Touch ID を要するため、クエリ実行のたびに走らせない)。default_limit / sqlfiles_dir / 接続一覧 / ai はすべてこのキャッシュから導出するので個別キャッシュは持たない (クリア漏れ防止)。reset_connections でクリア。キーが存在するのに文字列でない・空文字ならエラー (黙って「未設定」に倒すと、オーバーライドが効かないままローカル設定で動いていることに気付けないため)。旧方式 (`sql_servers` にソース宣言) の設定はエラーになり、メッセージで `config_override_command` へ移行するよう案内する。なお `View override config yaml (Copy only)` はキャッシュを経由せず毎回コマンドを実行する (保管場所の現在値を確認・コピーする用途のため意図的)。
- `config_override_command` はシェル非経由 (shlex 分解) で実行。GUI 起動の最小 PATH 対策として /opt/homebrew/bin と /usr/local/bin を補完する。60 秒タイムアウト + kill_on_drop。
- `default_limit` (任意、デフォルト 500、0 で無効) — LIMIT 未指定の SELECT に自動で `LIMIT n` を付与する (db.rs の should_auto_limit。サブクエリ LIMIT / FOR UPDATE 等は保守的にスキップ)。
- `readonly: true` (任意、デフォルト false。sql-agent 互換フォーマットへの queryfolio 独自拡張) — その接続で書き込み系の文 (INSERT / UPDATE / DELETE / DDL 等) の実行を拒否する。判定は db.rs の is_readonly_allowed: leading_keyword が select / with / show / describe / desc / explain / pragma / values / table / call 以外なら拒否し、さらに WITH は CTE 付き DML (insert / update / delete / merge)、SELECT は SELECT INTO、EXPLAIN は EXPLAIN ANALYZE + DML (対象文を実際に実行するため)、PRAGMA は代入形 (`=` を含む `PRAGMA journal_mode = WAL` 等の SQLite の DB 変更) を、リテラル・コメント除去済みの単語境界判定で拒否する。メタコマンドは読み取り系のみなので常に許可。SELECT に副作用のある関数 (nextval 等) や括弧形の設定 PRAGMA までは防げない、あくまで事故防止のガード。
- `allow_dangerous_statements: true` (任意、デフォルト false。queryfolio 独自拡張) — 省略時は危険な文 (WHERE 無しの UPDATE / DELETE、DROP、TRUNCATE) を誤操作による全行破壊・テーブル消失防止のため拒否する (db.rs の dangerous_reason: readonly と同じ scan_sql による単語境界判定。UPDATE/DELETE は where 語の有無、DROP/TRUNCATE は常に危険)。先頭キーワードだけでなく、実際に書き込みが走るラップ形 — `WITH ... DELETE/UPDATE` (Postgres の CTE 付き DML) と `EXPLAIN ANALYZE ...` (対象文を実行する) — の中の危険な DML も対象にする。true にすると実行できるが、フロントは実行前に確認ダイアログ (DangerousConfirmModal) を出す。確認要否の判定は check_dangerous_statement コマンド (db.rs の dangerous_statement_reason)。readonly が先に評価されるため readonly 接続ではこのガードには到達しない。弱点: WITH で無関係な CTE / 外側の SELECT に where があると WHERE 無し DML を見逃す (where を一切含まない典型形は捕捉)。サブクエリ内だけの where も同様に安全側 (許可側) に倒れる。
- `sqlfiles_dir` (任意) でクエリファイル保存先を変更できる。デフォルトは `~/.config/queryfolio/sqlfiles/<folder>/<name>.sql`。`<folder>` は接続ごとに `folder_name` 設定があればそれを使い、無ければ `<host>_<engine>_<schema>_<user>` を組み立てる (接続 name は使わない。config.rs の `ServerConfig::sqlfiles_folder_name`。パス区切り等はサニタイズ)。既存接続でフォルダ名が変わるとそれまでのクエリファイルは旧フォルダに残る点に注意。各フォルダには接続を説明するメタファイル `_queryfolio.md` が自動生成される (folder_meta.rs。非機密のみ・`.sql` でないため UI の一覧には出ない)。
- `folder_name` (任意、queryfolio 独自拡張) — クエリファイルの保存フォルダ名を明示する。省略時のフォルダ名ルールは上記 `sqlfiles_dir` を参照。
- `ssh_tunnel.identity_agent` (任意、queryfolio 独自拡張) — ssh-agent 認証で使う agent socket を明示する (OpenSSH の IdentityAgent 相当)。`none` で agent を無効化。省略時は `~/.ssh/config` の IdentityAgent → SSH_AUTH_SOCK の順で解決 (tunnel.rs)。鍵を 1Password SSH agent に置き GUI 起動する環境向けの解決策。
- `ai:` (任意) — AI SQL 生成の設定 (`provider: openai` / `api_key` / `model` 任意 / `base_url` 任意)。ローカル config.yml のトップレベルと、`config_override_command` で取得する YAML のトップレベルの両方に書ける。**両方ある場合は取得 YAML 側を優先** (マージの結果。API キーを 1Password に置ける)。`ai` はマッピングなので再帰マージされ、取得側に `api_key` だけ書けばローカルの `model` 等は残る。provider は現状 openai のみで、不明値はエラー。AppState にセッションキャッシュされ reset_connections でクリア。
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
