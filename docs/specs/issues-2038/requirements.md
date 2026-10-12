# Context

- 入力: https://github.com/siro33950/releash/issues/2038
- 補助資料:
  - https://github.com/siro33950/releash/issues/2037（`mise.toml` と Nx の導入）
  - https://docs.github.com/en/actions/writing-workflows/choosing-when-your-workflow-runs/triggering-a-workflow
- #2037 は PR #2047 で main に入っている（merge commit `b1c16576`）。
  #2038 本文の「現状」は main `3e2b5e3e` を基準にしているが、この開発は `b1c16576` を基点にする。
- `GITHUB_TOKEN` で push した commit では、新しい workflow が起動しない。
  GitHub App の installation token か PAT で push すれば起動する（GitHub の文書「Triggering a workflow from a workflow」）。
- Renovate の manager には `cargo`、`npm`、`github-actions`、`rust-toolchain`、`xcodegen`、`swift`、`mise` がある。
  Nx 用の manager は無い。
- Renovate の `swift` manager の文書は、`Package.resolved` を更新するかどうかを書いていない。
- Renovate の `mise` manager は aqua と github の backend に対応する。
  `mise.toml` の `aqua:EmbarkStudios/cargo-deny` と `aqua:taiki-e/cargo-llvm-cov` は aqua registry 上で `github_release` 型であり、対応の範囲に入る。
- Nx は、版を上げるときに `nx migrate <版>` で版を書き換えて `migrations.json` を作り、`nx migrate --run-migrations` で設定とコードを新しい版に合わせる手順を標準としている。
  複数のメジャーの版をまたぐときは、1 つずつ上げるよう勧めている。
- Renovate は、Renovate 以外の Git author が branch に commit を足した PR を modified として扱い、以後その PR に commit しない。
  他の bot が足す commit を無視させるには、その author を `gitIgnoredAuthors` に入れる（Renovate の文書の `gitIgnoredAuthors` の項）。
- Renovate は、脆弱性を直す PR（`vulnerabilityAlerts`）を、既定で `schedule` と同時の上限を無視して出す。
  脆弱性の検知は、GitHub の dependency graph と Dependabot alerts から読む。
- Renovate が推奨の設定とする `config:recommended` は、`schedule`、`prConcurrentLimit`、`prHourlyLimit` を設定しない。
- リポジトリの追跡ファイルには、上の 7 つの manager 以外の manager が反応するファイルが無い。
- 後処理用の GitHub App の名前（slug）は `releash-dependency-updates` とし、1Password の項目名と同じにする。
  この App の bot が commit の author になるときのメールアドレスは `<bot の ID>+releash-dependency-updates[bot]@users.noreply.github.com` の形になり、bot の ID は App を作るまで決まらない。
- 秘密の値は、既存の workflow と同じく `.github/scripts/load-secrets.mjs` で 1Password から読む。
- merge の前に利用者が行う操作は #2038 本文の「merge の前に利用者が行う操作」に従う。
  Renovate の GitHub App のインストールと Vulnerability alerts の権限、後処理用の GitHub App の作成（名前は `releash-dependency-updates`）とインストールと秘密鍵の 1Password への保存、Dependabot security updates の停止（dependency graph と Dependabot alerts は残す）がこれに当たる。
- #2038 本文と異なる決定:
  - ラベル: 本文は「今の Dependabot と同じラベル（`dependencies` と対象ごとの `rust`、`javascript`、`ci`）」とする。
    この開発では、すべての PR に `dependencies` だけを付ける。
  - 間隔と上限: 本文は指定していない。
    今の Dependabot は間隔が weekly、同時に開く PR の上限が cargo 10、npm 10、github-actions 5 である。
    この開発では Renovate の既定に任せ、指定しない。
  - 自動 merge: 本文は触れていない。
    この開発では、Nx 以外の PR を自動で merge し、Nx の PR は人が merge する。

# Outcome

対象者: Releash の開発者。

現在の問題: 依存の更新は Dependabot が cargo、npm、github-actions、rust-toolchain についてだけ PR を出している。
XcodeGen の `project.yml`、SwiftPM の `Package.swift` と `Package.resolved`、`mise.toml`、Nx の版（`nx.json`）は Dependabot の対象外で、開発者が手で版を上げている。
swift-protobuf と connect-swift は 2 か所に版があり、ずれると生成コードの版の検査でビルドが失敗する。
Nx は版の数字の書き換えだけでは足りず、`nx migrate` の実行が要る。

変更後の状態: Renovate が、今の 4 つの対象に加えて `project.yml`、`Package.swift`、`mise.toml`、Nx の版の更新 PR を出す。
Renovate の PR は、解決結果の作り直しと `nx migrate` の結果を同じ PR に含み、CI で検証できる状態になる。
Dependabot は版の更新 PR を出さない。

# Current Behavior

開発開始時点（`b1c16576`）で確認した状態。

- `.github/dependabot.yml` が、cargo（`/`）、npm（`/clients/desktop`）、github-actions（`/`）、rust-toolchain（`/`）を、どれも `interval: weekly` で更新する。
  ラベルは cargo と rust-toolchain が `dependencies`・`rust`、npm が `dependencies`・`javascript`、github-actions が `dependencies`・`ci`。
  `open-pull-requests-limit` は cargo 10、npm 10、github-actions 5、rust-toolchain は指定なし。
- `.github/workflows/dependabot-auto-merge.yml:11-15` が、author が `dependabot[bot]` の PR に `gh pr merge --auto --squash` を掛ける。
- リポジトリのラベルは `bug`、`documentation`、`enhancement`、`quality` だけで、`dependencies`、`rust`、`javascript`、`ci` は無い。
  直近の Dependabot の PR（#2030〜#2035）には、ラベルが付いていない。
- Renovate の設定ファイルは無い。
- Dependabot の対象外で、版を手で上げているファイル:
  - `clients/macos/project.yml`: `Connect`（connect-swift）は `exactVersion: 1.0.0`、`SwiftProtobuf`（swift-protobuf）は `exactVersion: 1.38.1`。`Bonsplit` は `revision` で固定する。
  - `clients/macos/BuildTools/Package.swift`: XcodeGen `2.44.1`、swift-protobuf `1.38.1`、connect-swift `1.0.0` を `exact` で固定する。
  - `clients/macos/BuildTools/Package.resolved` と `clients/macos/Releash.xcodeproj/project.xcworkspace/xcshareddata/swiftpm/Package.resolved`。
  - `mise.toml`: node、pnpm、buf、ast-grep、`aqua:EmbarkStudios/cargo-deny`、`github:qltysh/qlty`、`aqua:taiki-e/cargo-llvm-cov`、1password-cli の版。
  - `nx.json`: `installation.version` と `installation.plugins.@nx/js` がどちらも `23.3.0`。
- `clients/macos/project.json` の macos の target は、`build` と `test` が `xcodebuild -resolvePackageDependencies ... -onlyUsePackageVersionsFromResolvedFile` で、既存の解決結果だけを使う。
  解決結果を作り直す target は無い。
- `rust-toolchain.toml` の `channel` は `1.99.0`。

# Scope / Non-goals

変更するもの:
- Renovate の設定ファイルを置く。
- 後処理の workflow を置く。
- 解決結果を作り直す macos の Nx target を足す。
- `.github/dependabot.yml` と `.github/workflows/dependabot-auto-merge.yml` を消す。

変更しないもの:
- ラベルをリポジトリに作ること。
- merge の前に利用者が行う GitHub と 1Password の操作。
- Renovate の更新の間隔、同時に開く PR の上限、1 時間に作る PR の上限の指定。
- `project.yml` の `Bonsplit`（`revision` で固定）の更新。

# Requirements

- R-001: Renovate が、cargo、npm、github-actions、rust-toolchain、xcodegen（`project.yml`）、swift（`Package.swift`）、mise（`mise.toml`）の依存の更新 PR を出す。
- R-002: rust-toolchain の更新 PR は、`rust-toolchain.toml` の版を範囲でなく具体的な版のまま上げる。
- R-003: swift-protobuf と connect-swift の更新 PR は、`project.yml` と BuildTools の `Package.swift` の両方の版を 1 つの PR で上げる。
- R-004: Renovate が Nx の更新 PR を出す。その PR は `nx.json` の `installation.version` と `installation.plugins.@nx/js` を 1 つの PR で上げる。
- R-005: Nx の版が複数のメジャーの版をまたいで上がるとき、Renovate はメジャーの版ごとに別の PR を出す。
- R-006: Renovate の PR が `project.yml` または BuildTools の `Package.swift` を変えたとき、2 つの `Package.resolved` を新しい版で作り直した結果が、同じ PR に commit される。
- R-007: Renovate の PR が `nx.json` を変えたとき、`nx migrate <新しい版>` と `nx migrate --run-migrations` の結果が、同じ PR に commit される。
- R-008: 後処理が PR に push した commit で、CI が起動する。
- R-009: 後処理が commit を足した PR についても、Renovate は新しい版への更新と conflict の解消を続ける。
- R-010: Renovate の PR には、`dependencies` のラベルが 1 つだけ指定される。
- R-011: Nx 以外の Renovate の PR は、必須チェックが通ると自動で merge される。Nx の PR は自動で merge されない。
- R-012: 脆弱性を直す PR を、Renovate が予定を待たずに出す。
- R-013: Dependabot が版の更新 PR を出さない。

# Assumptions

なし
