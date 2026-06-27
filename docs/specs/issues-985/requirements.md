# requirements — issues-985

`git_host`（Git hosting integration）を clean architecture 配置へ移行する。

## Type

改善 / リファクタリング（内部アーキテクチャ整備、external observable behavior は不変）。マイルストーン [12]「クリーンアーキテクチャ移行」の Implementation ISSUE。親 ISSUE ではない。

## 背景と目的

### 現状（本リポジトリ調査による事実）

Git hosting integration は `src-tauri/src/git_host/` に flat module として実装されており、clean architecture のレイヤー分離がなされていない。1 module 内に domain 概念・application 手順・外部プロセス実行・cache 実装・Tauri command が混在している。

- `git_host/types.rs` — value object / DTO（`ProviderStatus` / `PrInfo` / `PrStatus` / `IssueInfo` / `IssueLabel` / `Milestone` / `PrAuthor`）と provider trait（`GitHostProvider`）、および review comment 型（`PrReviewComment` / `PrReviewCommentAuthor`）。
- `git_host/mod.rs` — provider discovery（`create_provider` / `get_origin_url`、git2 remote lookup）、provider status 判定（`check_provider_status` / `check_github_status`、`gh` 実行）、PR status / issue の fetch 手順（`fetch_pr_status_inner` / `fetch_issues_inner`）、cache 読み書き手順（`fetch_pr_status_with_cache` / `fetch_issues_with_cache`、TTL 30 秒）、in-memory cache 実装（`PrCache` / `IssueCache`、`Mutex<HashMap>`）、Tauri command 5 本。
- `git_host/github.rs` — GitHub provider 実装（`GitHubProvider`）、`gh` CLI プロセス実行（`run_gh_with_timeout`、10 秒 timeout・別スレッド stdout 読み取り）、`gh` 出力の JSON parse。
- `lib.rs` — `mod git_host;` と `PrCache` / `IssueCache` を Tauri `manage()` で直接保持。
- `adaptor/controller/command/mod.rs` — `crate::git_host::*` の Tauri command 5 本を直接 `generate_handler!` に登録。

このため domain decision（provider 種別判定・cache TTL・status の語彙）と infrastructure 詳細（`gh` 実行・git2・Mutex cache）が同居し、`docs/architecture/` が定めるレイヤー責務（`infrastructure → adaptor/gateway → domain ← usecase ← adaptor/controller`）に従っていない。

### 目的

`git_host` の責務を clean architecture の 4 レイヤー（`domain/git_host/`・`usecase/git_host/`・`adaptor/gateway/git_host/`・`adaptor/controller/command/git_host/`）へ分割移行し、既存の外部から観測可能な振る舞い（5 本の Tauri command の入出力・cache の TTL 挙動・provider 判定結果）を変えずに、レイヤー境界・依存方向・所有者を明確化する。これにより #878 final sweep の前提（`crate::git_host::*` 直接依存の解消）を満たす。

## スコープ

### 移行対象コード

- `src-tauri/src/git_host/mod.rs`
- `src-tauri/src/git_host/github.rs`
- `src-tauri/src/git_host/types.rs`
- `lib.rs` が manage している `PrCache` / `IssueCache`
- Tauri command 5 本: `check_pr_provider_status` / `fetch_pr_status` / `get_cached_pr_status` / `fetch_issues` / `get_cached_issues`

### レイヤー別の責務配置（ISSUE 責務範囲に準拠）

- **`domain/git_host/`**
  - provider status・PR status・issue summary・label / milestone の value object。
  - cache policy が domain decision なら value object 化する（例: TTL を表す概念、stale 判定ルール）。
  - usecase が外部 host を必要とする場合の provider port（trait）。
  - `gh` 実行・git2 remote lookup・Tauri・mutex cache 実装詳細には依存しない。
- **`usecase/git_host/`**
  - provider detection / status 取得。
  - fetch PR status / cached PR status。
  - fetch issues / cached issues。
  - cache read / write orchestration が application behavior ならここで扱う。
- **`adaptor/gateway/git_host/`**
  - GitHub provider 実装（`gh` 出力 parse を含む）。
  - `gh` CLI プロセス実行。
  - provider discovery のための git remote URL lookup。
  - in-memory cache 実装を残す場合の具象実装。
- **`adaptor/controller/command/git_host/`**
  - Tauri command wrapper と request / response mapping。

## 非スコープ

- review thread / comment storage と handoff（#1132）。本 ISSUE では review comment 機能の振る舞いを追加・変更しない（既存 dead-code の削除は R9 で扱うが、新たな review comment 機能は実装しない）。
- repository branch / worktree read model の変更。
- WebSocket protocol / server migration（#1130 / #1131）。
- dead code sweep（#878）。
- 5 本の Tauri command の external observable behavior（入出力 schema・cache TTL の値・provider 判定結果・`gh` 引数）の変更。
- frontend（React / TypeScript）側の変更。invoke 先 command 名と DTO 形状は不変に保つ。
- 新規 provider（GitLab 等）の追加。現状の GitHub のみ対応を維持する。

## 要求事項

### R1. レイヤー分割と配置

- `git_host` の責務を、上記スコープのレイヤー別責務配置に従って `domain/git_host/`・`usecase/git_host/`・`adaptor/gateway/git_host/`・`adaptor/controller/command/git_host/` へ分割移行すること。
- 各レイヤーは `docs/architecture/` の依存方向（`infrastructure → adaptor/gateway → domain ← usecase ← adaptor/controller`）を守ること。`domain/` は infrastructure dependency（`gh` 実行・git2・Tauri・Mutex）を持たないこと。

### R2. provider port の導入

- usecase が外部 host にアクセスする経路を domain で定義した provider port（trait）越しにし、GitHub の具象実装（`gh` 実行・出力 parse）を `adaptor/gateway/git_host/` に置くこと。
- provider discovery（git remote URL から GitHub か否かを判定する処理）を gateway 側の具象実装として配置すること。

### R3. cache の所有者明確化

- PR / issue の cache（現 `PrCache` / `IssueCache`、TTL 30 秒、`Mutex<HashMap>`）の所有者と読み書き手順の配置を明確にすること。cache hit / miss / stale の判定ルールが domain decision なら value object 化し、in-memory 実装は gateway の具象実装として残すこと。
- cache の TTL（30 秒）・hit / miss / stale の外部から観測可能な挙動は現状を維持すること。

### R4. Tauri command の wrapper 化

- 5 本の Tauri command を `adaptor/controller/command/git_host/` の wrapper として再配置し、business behavior を controller に持たせず usecase 呼び出しと request / response mapping に限定すること。
- command 名・引数・戻り値の DTO 形状は現状を維持し、frontend の invoke 先を変えないこと。

### R5. `lib.rs` / command 登録の更新

- `lib.rs` から `mod git_host;` を除去すること。
- `lib.rs` が `git_host::PrCache` / `git_host::IssueCache` を直接 manage しない構成にすること（cache を引き続き Tauri state として保持する場合は、新レイヤー配置の型を manage する）。
- `adaptor/controller/command/mod.rs` が `crate::git_host::*` を直接登録しない構成にすること（新レイヤー配置の command を登録する）。

### R6. 旧 module の削除

- `src-tauri/src/git_host/` ディレクトリを削除すること。
- 削除後、`crate::git_host::*` への参照がコードベースに残っていないこと。

### R7. テスト

- usecase test が、provider 不在・GitHub available / unavailable・cache hit / miss / stale をカバーすること。
- gateway test が、`gh` 出力 parse と error / invalid JSON をカバーすること（現 `github.rs` の parse 系テスト・現 `mod.rs` の provider discovery / status テストに相当する観点を移行・維持する）。

### R9. dead-code の削除

- production 経路から呼ばれていない以下の dead-code を本移行で削除すること。新レイヤーへは移送しない。
  - review comment 関連: `PrReviewComment` / `PrReviewCommentAuthor` 型、`GitHostProvider::get_pr_review_comments`（provider port からも外す）、`github.rs` の `parse_pr_review_comments` とその `#[cfg(test)]` テスト。
  - 時刻変換ヘルパ: `git_host/mod.rs` の `parse_rfc3339_to_millis` / `days_from_civil` とその `#[cfg(test)]` テスト。
- 削除対象は現状 production から呼ばれていないため、external observable behavior（5 本の Tauri command の結果）には影響しないこと。

### R8. external observable behavior の不変性

- 移行前後で、5 本の Tauri command の入出力、provider 判定結果、cache の TTL 挙動が変わらないこと。

## 受け入れ基準の概要

ISSUE「完了条件」に準拠する。

- `src-tauri/src/git_host/` が削除されている。
- `lib.rs` に `mod git_host` が残っていない。
- `lib.rs` が `git_host::PrCache` / `git_host::IssueCache` を直接 manage していない。
- `adaptor/controller/command/mod.rs` が `crate::git_host::*` を直接登録していない。
- usecase test が provider 不在、GitHub available / unavailable、cache hit / miss / stale をカバーしている。
- gateway test が `gh` output parse と error / invalid JSON をカバーしている。
- `cargo fmt --check` / `cargo clippy -- -D warnings` / `cargo test` が通る。
- 5 本の Tauri command の external observable behavior（command 名・入出力 DTO・cache TTL）が移行前後で不変。

## 仮定

- A1. 本 ISSUE はレイヤー移行（責務の再配置）であり、provider 判定ロジック・`gh` 引数・cache TTL（30 秒）・status の語彙といった振る舞いそのものは変更しない。型・関数の配置先のみが変わる。
- A2. cache（`PrCache` / `IssueCache`）は引き続き Tauri `manage()` の state として保持する。所有者（gateway 具象実装）と読み書き手順（usecase orchestration）の配置をレイヤーに沿って整理するのみで、in-memory + Mutex という保持方式自体は維持する（full-retention 設計の新規導入はしない）。
- A3. 移行先のレイヤー内 module 構成・命名は、既存の移行済みドメイン（`domain/repository/`・`usecase/repository_*`・`adaptor/gateway/repository/`・`adaptor/controller/command/`）の慣習に合わせる。具体的なファイル分割・命名は design.md で確定する。
- A4. review comment 関連コードと時刻変換ヘルパ（`PrReviewComment` / `PrReviewCommentAuthor` 型、`GitHostProvider::get_pr_review_comments`、`github.rs` の `parse_pr_review_comments`、`git_host/mod.rs` 内 `parse_rfc3339_to_millis` / `days_from_civil`）は現状 production 経路から呼ばれていない dead-code であり、本移行で削除する（R9、ユーザー確認済み）。`get_pr_review_comments` は provider port からも外す。review thread / comment 機能そのものは #1132 が将来あらためて担う。

## Open Questions

なし。
