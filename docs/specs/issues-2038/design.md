# Design

## 変える部分
- Renovate の設定: リポジトリに Renovate の設定ファイルを置き、`config:recommended` を基にする。根拠: R-001「cargo、npm、github-actions、rust-toolchain、xcodegen、swift、mise の依存の更新 PR を出す」。ルート: `config:recommended` を使い、`enabledManagers` を書かない
- rust-toolchain の版の固定: rust-toolchain の更新に `rangeStrategy: pin` を指定する。根拠: R-002「版を範囲でなく具体的な版のまま上げる」。ルート: `rangeStrategy: pin`
- swift-protobuf と connect-swift の同時更新: `project.yml` と BuildTools の `Package.swift` にある swift-protobuf と connect-swift を、1 つの PR にまとめる。根拠: R-003「両方の版を 1 つの PR で上げる」。ルート: 委任
- Nx の版の検出: custom manager で `nx.json` の `installation.version` と `installation.plugins.@nx/js` を対象にし、1 つの PR にまとめる。根拠: R-004「`installation.version` と `installation.plugins.@nx/js` を 1 つの PR で上げる」。ルート: custom manager（`jsonata` か `regex`）
- Nx のメジャーの版ごとの PR: Nx に `separateMultipleMajor` を指定する。根拠: R-005「メジャーの版ごとに別の PR を出す」。ルート: `separateMultipleMajor`
- ラベル: `labels: ["dependencies"]` を指定する。根拠: R-010「`dependencies` のラベルが 1 つだけ指定される」。ルート: `labels: ["dependencies"]`
- 自動 merge: Renovate の `automerge` を有効にし、Nx の更新と、`xcodegen` と `swift` の manager の更新を対象から外す。根拠: R-011「Nx と Swift 以外の Renovate の PR は、必須チェックが通ると自動で merge される。Nx と Swift の PR は自動で merge されない」。ルート: Renovate の `automerge`
- 後処理の commit の扱い: 後処理の workflow が push する commit の author（後処理用の GitHub App `releash-dependency-updates` の bot のメールアドレス）を `gitIgnoredAuthors` に入れる。根拠: R-009「後処理が commit を足した PR についても、Renovate は新しい版への更新と conflict の解消を続ける」。ルート: `gitIgnoredAuthors` に bot の ID を含まない regex `/^\d+\+releash-dependency-updates\[bot\]@users\.noreply\.github\.com$/` を入れる
- 脆弱性を直す PR: Renovate の既定の `vulnerabilityAlerts` の挙動を使う。根拠: R-012「脆弱性を直す PR を、Renovate が予定を待たずに出す」。ルート: `vulnerabilityAlerts`
- 解決結果を作り直す target: `clients/macos/project.json` に、2 つの `Package.resolved` を作り直す target を足す。根拠: R-006「2 つの `Package.resolved` を新しい版で作り直した結果が、同じ PR に commit される」。ルート: macos の Nx target
- 後処理の workflow: Renovate の PR が `project.yml`、BuildTools の `Package.swift`、`nx.json` のいずれかを変えたときに、その branch で後処理を行い、同じ PR に push する workflow を置く。根拠: R-006、R-007「`nx migrate <新しい版>` と `nx migrate --run-migrations` の結果が、同じ PR に commit される」、R-008「後処理が PR に push した commit で、CI が起動する」。ルート: ツールは `jdx/mise-action` で `mise.toml` から入れる。Swift の解決結果の作り直しは Nx の target から呼ぶ。Nx は `./nx migrate <新しい版>` と `./nx migrate --run-migrations` を実行する。push には `actions/create-github-app-token` で実行のたびに発行する GitHub App の installation token を使い、App の秘密鍵は `.github/scripts/load-secrets.mjs` で 1Password から読む
- Dependabot の停止: `.github/dependabot.yml` と `.github/workflows/dependabot-auto-merge.yml` を消す。根拠: R-013「Dependabot が版の更新 PR を出さない」。ルート: 2 つのファイルを消す

## 固定するルート
- Renovate の設定は `config:recommended` を基にする。`enabledManagers`、`schedule`、`prConcurrentLimit`、`prHourlyLimit` は書かない。Renovate が推奨の設定とする `config:recommended` が `schedule`、`prConcurrentLimit`、`prHourlyLimit` を設定せず、追跡ファイルに対象外の manager が反応するファイルが無いため。
- Nx の版は custom manager（`jsonata` か `regex`）で検出し、版の更新は `nx migrate` で行う。Nx には版を検出する Renovate の manager が無く、Nx は版の数字の書き換えでなく `nx migrate` を標準の手順としているため。
- 後処理の workflow のツールは `jdx/mise-action` で `mise.toml` から入れ、Swift の解決結果の作り直しは macos の Nx target から呼ぶ。開発と CI の入口を mise と Nx に揃えた #2037 に合わせるため。
- 後処理の push には、`actions/create-github-app-token` で実行のたびに発行する GitHub App の installation token を使う。App の秘密鍵は 1Password に置き、既存の workflow と同じく `.github/scripts/load-secrets.mjs` で読む。`GITHUB_TOKEN` で push した commit では CI が起動しないため。
- 後処理の commit の author は `gitIgnoredAuthors` に入れる。Renovate の文書が、他の bot が commit を足す場合にこの形を示しているため。値は bot の ID を含まない regex `/^\d+\+releash-dependency-updates\[bot\]@users\.noreply\.github\.com$/` にする。bot の ID は利用者が App を作るまで決まらず、`gitIgnoredAuthors` は regex を受け付けるため。
- 自動 merge は Renovate の `automerge` で行い、Nx の PR と Swift（`xcodegen` と `swift` の manager）の PR は対象から外す。後処理が migrate の結果や `Package.resolved` の作り直しを push する前に必須チェックが通ると、後処理の結果を含まずに merge されるおそれがあるため。Nx の文書も、`--run-migrations` の変更を人が確かめて commit する手順にしている。

## 変えないもの
なし

## 未確定・リスク
- Renovate の `xcodegen` manager と `swift` manager が swift-protobuf と connect-swift に付ける依存の名前が同じかどうかは確かめていない。名前が異なると、R-003 の 1 つの PR へのまとめ方が変わる。
