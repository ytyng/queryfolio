---
name: publish-macos-release
description: QueryFolio の macOS 版をビルドして GitHub Release として公開 (サイトで配布) する手順。「mac 版をリリース」「アプリを公開」「新しいバージョンを配布」「release the mac app」「publish a new build」と言及された時に使う。GitHub Actions の build-macos ワークフローを fab で起動 → 署名付き DMG を確認 → Release を公開 → ダウンロードリンクを更新する一連の作業を扱う。
---

# QueryFolio macOS 版のリリース手順

macOS 版アプリを GitHub Actions でビルドし、GitHub Release としてサイト (ダウンロードページ) で
公開するまでの runbook。ビルドは `.github/workflows/build-macos.yml` (workflow_dispatch のみ)、
起動は `fabfile/__init__.py` の `build_mac` タスクで行う。

## 前提

- `gh` が `ytyng` アカウントで認証済みで、リポジトリは `ytyng/queryfolio`。
- 署名用の GitHub Secrets が設定済み (下記「初回セットアップ」)。未設定でもビルドは通るが
  **ad-hoc 署名のみのテストビルド** (`APPLE_SIGNING_IDENTITY=-`) になり、配布には使えない
  (Apple Silicon でも開けるが、身元不明の開発者として扱われる)。
- 公証 (notarization) は未対応。署名のみなので、初回起動時は Gatekeeper の警告が出る
  (右クリック → 開く、または システム設定 › プライバシーとセキュリティ で許可)。

## リリース手順

### 1. バージョンを上げる

タグは `tauri.conf.json` の `version` から `queryfolio-v<version>` として生成される。同じバージョンで
2 回リリースするとタグが衝突するので、**リリースごとに必ず version を上げる**。2 ファイルを一致させる:

- `src-tauri/tauri.conf.json` の `version`
- `package.json` の `version`

変更を `main` にコミット & プッシュする (Release のタグはこのコミットに付く)。

```shell
git add src-tauri/tauri.conf.json package.json
git commit -m "chore: bump version to v<version>"
git push origin main
```

### 2. ビルド & Release 作成を起動する

```shell
fab build_mac              # draft の Release を作り、実行をフォロー
```

内部的には `gh workflow run build-macos.yml -f draft=true` を実行し、`gh run watch` で追う。
直接叩くなら:

```shell
gh workflow run build-macos.yml -f draft=true
gh run watch "$(gh run list --workflow=build-macos.yml --limit 1 --json databaseId --jq '.[0].databaseId')" --exit-status
```

ワークフローは universal (Apple Silicon + Intel) でビルドし、署名付きの `.dmg` を
`queryfolio-v<version>` という **draft の Release** に添付する。所要 10〜20 分程度。

### 3. 成果物を検証する

Release から DMG を落として署名を確認する:

```shell
gh release download queryfolio-v<version> --pattern '*.dmg' --dir /tmp/qf-release
codesign -dv --verbose=2 "/Volumes/.../QueryFolio.app" 2>&1   # マウント後
spctl -a -t open --context context:primary-signature /tmp/qf-release/*.dmg  # 署名の受理確認
```

`Developer ID Application: Cyberneura K.K. (2YN5TLNQ9J)` で署名されていること、DMG がマウントでき
アプリが起動することを確認する。

### 4. サイトで公開する

draft を外して Release を公開すると、その Release ページ (と DMG の asset URL) が
そのままダウンロードサイトになる:

```shell
gh release edit queryfolio-v<version> --draft=false --latest
```

公開後の DMG の直リンクを取得してダウンロードリンクに使う:

```shell
gh release view queryfolio-v<version> --json assets --jq '.assets[] | select(.name|endswith(".dmg")) | .url'
```

必要に応じて配布導線を更新する (このプロジェクトの「サイト」= リリースページが正)。

- `README.md` の見出し付近に最新版のダウンロードリンク / バッジを追加・更新する。
  常に最新を指すリンクは
  `https://github.com/ytyng/queryfolio/releases/latest` を使う。
- 告知が必要なら ytyng-blog (`ytyng-blog-cli` スキル) に BlogPost / Achievement を作る。

### 5. 公開後の確認

- `https://github.com/ytyng/queryfolio/releases/latest` が新バージョンを指していること。
- DMG の公開 URL が匿名 (未ログイン) で落とせること。

## 初回セットアップ (署名 Secrets)

ローカルの署名 ID を CI に持ち込むため、`.p12` を書き出して Secrets に登録する。1 回だけでよい。

```shell
security find-identity -v -p codesigning | grep "Developer ID Application: Cyberneura"
# → キーチェーンアクセスで該当の証明書 + 秘密鍵を .p12 として書き出す (例: queryfolio-signing.p12)

base64 -i queryfolio-signing.p12 | gh secret set APPLE_CERTIFICATE
gh secret set APPLE_CERTIFICATE_PASSWORD   # .p12 のパスワードを入力
printf 'Developer ID Application: Cyberneura K.K. (2YN5TLNQ9J)' | gh secret set APPLE_SIGNING_IDENTITY
printf '%s' "$(openssl rand -base64 24)" | gh secret set KEYCHAIN_PASSWORD   # 一時キーチェーン用の任意文字列
```

登録確認: `gh secret list`

### notarization を有効化する場合 (現状は未対応)

現在のワークフローは署名のみで notarization は行わない。有効化するには **Secrets の登録**と
**ワークフローの改修**の両方が要る (Secrets 登録だけでは効かない)。

```shell
gh secret set APPLE_ID           # Apple Developer のメール
gh secret set APPLE_PASSWORD     # app 用パスワード (appleid.apple.com で発行)
printf '2YN5TLNQ9J' | gh secret set APPLE_TEAM_ID
```

そのうえで `.github/workflows/build-macos.yml` の "Configure code signing" ステップの条件付き
export に `APPLE_ID` / `APPLE_PASSWORD` / `APPLE_TEAM_ID` を追加し、Release 本文の「not notarized」
記述も更新する。

## トラブルシューティング

- **タグ衝突でワークフローが失敗する** → 同じ version で再実行している。手順 1 で version を上げる。
  やり直すだけなら既存の draft Release とタグを消す: `gh release delete queryfolio-v<version> --cleanup-tag --yes`。
- **ad-hoc 署名 (未署名相当) でビルドされる** → 署名 Secrets が未設定。"Configure code signing"
  ステップのログが "building with ad-hoc signing (test build)" になる。配布ビルドには
  「初回セットアップ」を実施する。
  「初回セットアップ」を実施する。
- **`draft=false` でジョブが即失敗する** → 署名 Secrets が未設定のまま公開しようとしている。
  未署名 (ad-hoc) ビルドを公開 Release にしないための安全弁 (ワークフローが `::error::` で停止)。
  「初回セットアップ」で署名 Secrets を設定するか、`draft=true` で試すこと。
- **`gh run watch` がすぐ終わる / run が見つからない** → dispatch 直後で run がまだ登録されていない。
  数秒待って `gh run list --workflow=build-macos.yml` で databaseId を確認して再度 watch する。
