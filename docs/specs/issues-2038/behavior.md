## B-001: 各 manager の更新 PR

GIVEN cargo、npm、github-actions、rust-toolchain、`project.yml`、`Package.swift`、`mise.toml` のいずれかに記載された依存に、新しい版が公開されている
WHEN Renovate がリポジトリを処理する
THEN その依存の版を上げる PR が出る

## B-002: rust-toolchain の版の固定

GIVEN `rust-toolchain.toml` の `channel` が具体的な版で書かれている
AND Rust の新しい版が公開されている
WHEN Renovate が rust-toolchain の更新 PR を出す
THEN PR の `rust-toolchain.toml` の `channel` は、新しい具体的な版になっている

## B-003: swift-protobuf と connect-swift の同時更新

GIVEN swift-protobuf または connect-swift に新しい版が公開されている
WHEN Renovate がその更新 PR を出す
THEN 1 つの PR が、`project.yml` と BuildTools の `Package.swift` の両方で、その依存の版を同じ版に上げている

## B-004: Nx の更新 PR

GIVEN Nx に新しい版が公開されている
WHEN Renovate がリポジトリを処理する
THEN 1 つの PR が、`nx.json` の `installation.version` と `installation.plugins.@nx/js` を新しい版に上げている

## B-005: Nx のメジャーの版ごとの PR

GIVEN 今の Nx の版から見て、2 つ以上先のメジャーの版が公開されている
WHEN Renovate がリポジトリを処理する
THEN Nx の更新 PR が、メジャーの版ごとに別々に出る

## B-006: Swift の解決結果の作り直し

GIVEN Renovate の PR が `project.yml` または BuildTools の `Package.swift` を変えている
WHEN 後処理の workflow がその PR で実行される
THEN 2 つの `Package.resolved` が新しい版で作り直され、同じ PR に commit される

## B-007: Nx の migrate

GIVEN Renovate の PR が `nx.json` の Nx の版を変えている
WHEN 後処理の workflow がその PR で実行される
THEN `nx migrate <新しい版>` と `nx migrate --run-migrations` の結果が、同じ PR に commit される

## B-008: 後処理の commit での CI の起動

GIVEN 後処理の workflow が Renovate の PR に commit を push した
WHEN GitHub がその push を受け取る
THEN その commit に対して CI が起動する

## B-009: 後処理の後の Renovate の更新

GIVEN 後処理の workflow が commit を足した Renovate の PR が開いている
AND その依存に、さらに新しい版が公開されている
WHEN Renovate がリポジトリを処理する
THEN Renovate はその PR を新しい版に更新する

## B-010: ラベル

GIVEN Renovate が更新 PR を出す
WHEN Renovate が PR に付けるラベルを決める
THEN 指定されるラベルは `dependencies` だけである

## B-011: Nx 以外の PR の自動 merge

GIVEN Nx 以外の依存を上げる Renovate の PR が開いている
WHEN その PR の必須チェックがすべて通る
THEN PR は自動で merge される

## B-012: Nx の PR は自動 merge しない

GIVEN Nx を上げる Renovate の PR が開いている
WHEN その PR の必須チェックがすべて通る
THEN PR は自動で merge されず、開いたまま残る

## B-013: 脆弱性を直す PR

GIVEN Dependabot alerts が、リポジトリの依存に脆弱性を報告している
WHEN Renovate がリポジトリを処理する
THEN 更新の予定と PR の上限にかかわらず、脆弱性を直す PR が出る

## B-014: Dependabot の停止

GIVEN この変更が main に入っている
WHEN Dependabot の版の更新の時刻になる
THEN Dependabot は版の更新 PR を出さない

## 要件IDとBehavior IDの対応表
| Requirement ID | Behavior ID |
| --- | --- |
| R-001 | B-001 |
| R-002 | B-002 |
| R-003 | B-003 |
| R-004 | B-004 |
| R-005 | B-005 |
| R-006 | B-006 |
| R-007 | B-007 |
| R-008 | B-008 |
| R-009 | B-009 |
| R-010 | B-010 |
| R-011 | B-011, B-012 |
| R-012 | B-013 |
| R-013 | B-014 |
