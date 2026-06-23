# Design

#1258: NewRelic 観測リソース（ダッシュボード / アラート / データ管理）の Terraform IaC 化。

本書は `requirements.md` / `behavior.md` を実装に落とすための設計を定める。アプリ計装（#1209）は変更せず、観測先 SaaS（NewRelic）側の構成を `newrelic/newrelic` Terraform provider でコード化する。

## 概要

`infra/newrelic/` に Terraform 構成を新規作成し、以下を IaC 化する。

1. **ダッシュボード** (`newrelic_one_dashboard`) — #1209 メトリクス集合の可視化（R1）
2. **アラート** (`newrelic_alert_policy` + `newrelic_nrql_alert_condition`) — 性能予算逸脱 / crash 急増 / 接続品質劣化 / payload 異常の検知（R2）
3. **データ管理** (`newrelic_metric_pruning_rule`) — 無料枠 100GB/月 の取り込み量 / カーディナリティ制御（R3）
4. **リモート state backend** — HCP Terraform（organization `releash` / workspace `newrelic`）（R4）
5. **認証注入** — provider 認証情報を変数経由で供給（R5）

Terraform は自動実行環境に組み込まず、検証・plan・apply は作業者が CLI で手動実行する（B4 / A4）。

## 変更対象

新規ディレクトリ `infra/newrelic/` のみを追加する。アプリコード（`src-tauri/`, `src/`）は変更しない。

```text
infra/newrelic/
├── versions.tf              # required_version / required_providers / backend 宣言
├── backend.tf              # HCP Terraform cloud 設定（organization / workspace）
├── providers.tf            # provider "newrelic"（認証は変数経由）
├── variables.tf            # 入力変数（account_id / api_key / region 等）
├── locals.tf               # メトリクス名・属性キー・しきい値・NRQL 断片の単一定義
├── dashboards.tf           # newrelic_one_dashboard（R1）
├── alerts.tf               # newrelic_alert_policy + newrelic_nrql_alert_condition（R2）
├── data_management.tf      # metric pruning（取り込み量 / カーディナリティ制御）（R3）
├── outputs.tf              # dashboard permalink / policy id 等（任意・参照用）
├── terraform.tfvars.example# 変数サンプル（実値はコミットしない）
└── README.md               # 認証・HCP Terraform・plan 手順
```

Terraform 用の GitHub Actions job は追加しない。

## アーキテクチャと責務分割

レイヤーではなくファイル単位で関心を分離する。`locals.tf` を**単一の真実源**とし、メトリクス名・属性キー・しきい値・許可属性 allowlist をここに集約する。ダッシュボード / アラート / metric pruning は `locals` を参照し、定義の重複を排除する。これにより #1209 実装値（A1 / A2）との突き合わせ（behavior「可視化メトリクス名が #1209 実装値と一致する」シナリオ）を `locals.tf` 1 箇所のレビューで担保できる。

| ファイル | 責務 |
|---|---|
| `locals.tf` | #1209 メトリクス集合 / 属性キー / allowlist / 予算しきい値の定義。他ファイルはここを参照 |
| `dashboards.tf` | `locals` のメトリクスを page / widget へ配置。表示基準（目標線）に予算値を埋める |
| `alerts.tf` | `locals` の予算・初期値から NRQL 条件を生成。policy に紐付け、通知連携は持たない |
| `data_management.tf` | `newrelic_metric_pruning_rule` による高カーディナリティ属性の削除 |
| `providers.tf` / `variables.tf` / `backend.tf` / `versions.tf` | provider 認証・state backend・version pin |

### provider / version pin（仮定 D1）

- `required_version >= 1.10`（HCP Terraform cloud backend と `.terraform.lock.hcl` を安定利用するため）。
- `newrelic/newrelic` provider はメジャー固定でピン留めする（例 `~> 3.x`）。具体版は実装時の最新安定版を `versions.tf` に固定し、`.terraform.lock.hcl` をコミットして再現性を担保する。

## データモデルまたは型

Terraform リソースとして固定する構成。

### `locals.tf`（定義の単一源）

```hcl
locals {
  # #1209 メトリクス集合（A1）。実装値: src-tauri/src/other/telemetry/
  metrics = {
    startup        = "releash.startup.duration_ms"
    hot_path       = "releash.hot_path.duration_ms"
    session_bytes  = "releash.session.save_bytes"
    payload_bytes  = "releash.agent_stream.payload_bytes"
    emit_interval  = "releash.agent_stream.emit_interval_ms"
    dropped_frames = "releash.agent_stream.dropped_frames"
    ws_reconnects  = "releash.agent_stream.ws_reconnects"
    rss_bytes      = "releash.process.rss_bytes"
    cpu_percent    = "releash.process.cpu_percent"
    xterm_count    = "releash.frontend.mounted_xterm_count"
    pty_count      = "releash.pty.active_count"
    op_status      = "releash.operation.status"
    usage_events   = "releash.usage.events"
  }

  # 属性キー（A1）
  attr = {
    operation   = "releash.operation"
    status      = "releash.status"
    channel     = "releash.channel"
    usage_event = "releash.usage_event"
  }

  # 許可属性 allowlist（A2 / #1209 実装値）
  allowed_resource_attrs = [
    "service.version", "os.type", "releash.build_type", "service.name",
  ]

  # 予算しきい値（B1 / audit M0）
  budget = {
    repo_snapshot_p95_ms = 300    # M0「200ms 台」上限
    diff_open_p95_ms     = 500    # M0「500ms 未満」
    payload_bytes        = 65536  # 64KB
  }

  # カウント型アラート初期値（B5 / B6。design でチューニング可）
  alert_counts = {
    crash_window_min       = 5
    crash_threshold        = 5
    quality_window_min     = 5
    ws_reconnects_threshold = 5
    dropped_frames_threshold = 10
  }
}
```

`session.*` 系の repo-snapshot 相当 operation（`session.list` / `session.get_meta` / `session.get_page` / `session.load_full` / `session.append` / `session.persist_parts` / `session.save_full`）と diff-open 相当（`git.diff_stats` / `review.file_open`）、repo-snapshot 相当（`git.status_scan`）の振り分けは `locals` 内のリストで保持し、アラート生成を `for_each` で展開する。

### ダッシュボード（`newrelic_one_dashboard`）

behavior の可視化要件（6 群）に対応する page / widget を定義する。

| ページ / 群 | widget 種別 | NRQL（例） |
|---|---|---|
| 起動・初期化 | line / billboard | `SELECT percentile(releash.startup.duration_ms, 95) FACET releash.operation` |
| Git/Diff hot path | line | `SELECT percentile(releash.hot_path.duration_ms, 95) WHERE releash.operation IN ('git.status_scan','git.diff_stats','review.file_open') FACET releash.operation` |
| AgentSession session IO | line / billboard | `releash.hot_path.duration_ms`（session 系 operation）+ `releash.session.save_bytes` |
| streaming / payload | line | `releash.agent_stream.payload_bytes` / `emit_interval_ms` / `dropped_frames` / `ws_reconnects` |
| リソース観測 | line | `releash.process.rss_bytes` / `cpu_percent` / `releash.frontend.mounted_xterm_count` / `releash.pty.active_count` |
| 操作成否 / 利用イベント | billboard / bar | `releash.operation.status` FACET `releash.status` / `releash.usage.events` FACET `releash.usage_event` |

duration 系 widget には予算値（`budget.repo_snapshot_p95_ms` / `budget.diff_open_p95_ms`）を threshold（critical/warning ライン）として埋め込み、behavior「ダッシュボードのしきい値表現が性能予算と整合する」を満たす。

### アラート（`newrelic_alert_policy` + `newrelic_nrql_alert_condition`）

policy 1 本に全条件を紐付け、通知連携（workflow / destination）は定義しない（behavior「通知連携は含まない」）。

| 条件 | NRQL（query） | 閾値 / 評価 |
|---|---|---|
| repo snapshot 逸脱 | `SELECT percentile(releash.hot_path.duration_ms, 95) WHERE releash.operation IN (repo_snapshot 群)` | P95 > 300ms が一定ウィンドウ継続（B1/B2） |
| diff open 逸脱 | `SELECT percentile(releash.hot_path.duration_ms, 95) WHERE releash.operation IN ('git.diff_stats','review.file_open')` | P95 > 500ms 継続 |
| crash 急増 | crash/exception 相当エラーの `count` | 5 分で 5 件以上（B5、固定絶対値） |
| 接続品質劣化 | `SELECT sum(releash.agent_stream.ws_reconnects)` / `sum(releash.agent_stream.dropped_frames)` | 5 分で reconnect ≥5 または dropped ≥10（B6） |
| payload 異常 | `SELECT percentile(releash.agent_stream.payload_bytes, 95)`（または max） | 65536 bytes 継続超過（B1） |

- duration / payload 系は `threshold_occurrences = "all"` + `threshold_duration`（ウィンドウ長）で「継続超過」を表現し、瞬間スパイクを除外（B2）。ウィンドウ長は初期値 5 分相当を `locals` 化してチューニング可能にする。
- crash / 品質劣化はカウント型（`sum`/`count` を 5 分 window で評価）。
- crash 条件の NRQL 対象は #1209 のエラー送出経路（exception / error log）に依存する。**仮定 D2**: crash は OTel ログ / span error として `Log` または `Span` に記録され、`WHERE` 句で error 種別を絞れるものとする。具体 NRQL は実装時に NewRelic 側データ型を確認して確定する（リスク参照）。

### データ管理（metric pruning + 取り込み量制御）

- **有料アドオン不使用**: NewRelic Pipeline Control Cloud Rules は Advanced Compute の有料アドオンであり、無料枠前提では使えない。したがって `newrelic_pipeline_cloud_rule` は定義しない。Log / Span の保存前 drop は本 Terraform 構成では行わない。
- **取り込み量制御**: #1209 の高頻度テレメトリは次元付き `Metric` としてエクスポートされるため、`newrelic_metric_pruning_rule` で高カーディナリティな永続識別子・パス属性を Metric から削除する。**仮定 D3**: 無料枠のデータ保持期間（retention）はアカウント tier で固定され Terraform から変更不可のため、「取り込み量制御」は retention 変更ではなく metric pruning によるカーディナリティ制御で達成する。

### backend（HCP Terraform）

state backend と手動 CLI-driven run は **HCP Terraform** を採用する（OQ-D1 更新）。organization は `releash`、workspace は `newrelic` に固定する。

```hcl
# backend.tf
terraform {
  cloud {
    organization = "releash"

    workspaces {
      name = "newrelic"
    }
  }
}
```

- HCP Terraform 認証は `terraform login`、CLI config、または `TF_TOKEN_app_terraform_io` で供給し、リポジトリにコミットしない。
- NewRelic 認証変数は作業者のローカル環境変数（`TF_VAR_*`）または HCP Terraform workspace variables / variable set で供給する。リポジトリには保存しない。

## 処理フロー

実装・運用フローは Terraform の標準フロー。

1. `terraform init` — HCP Terraform cloud backend を初期化する。
2. `terraform fmt -check` / `terraform validate` — 構文・型検証（認証不要、behavior「静的検証を通過する」）。
3. `terraform plan` — 作業者が認証変数（`TF_VAR_newrelic_api_key` 等）供給下で手動実行する。全リソースが add として計画され、import 不要（R6 / A6）。
4. `terraform apply` — 作業者が plan 確認後に手動実行する（自動 apply は行わない、A4）。

認証変数の供給経路（behavior「認証変数が未供給だと plan は認証エラーになる」）:
- `newrelic` provider の `api_key` / `account_id` を `variable` 化し、環境変数 `TF_VAR_newrelic_api_key` / `TF_VAR_newrelic_account_id` で注入。
- 既定値を持たせず、未供給時は provider 認証エラーで `plan` 失敗（behavior 準拠）。
- 値は 1Password CLI の `--account my.1password.com` を明示し、`op://releash/new-relic/account-id` / `op://releash/new-relic/user-key` 等から作業者のローカル環境変数へ読み込む。リポジトリにリテラルを置かない（R5）。

## エラー処理

Terraform 構成レベルの失敗モードを behavior に揃える。

| 状況 | 期待挙動 | 根拠 |
|---|---|---|
| 認証変数 未供給 | `plan` が認証情報不足エラーで失敗 | behavior「認証変数が未供給だと plan は認証エラー」。変数に default を置かない |
| HCP Terraform 認証不足 / workspace 権限不足 | `init` または `plan` が失敗 | R4 |
| 構文・型エラー | `validate` が失敗 | behavior「静的検証を通過する」 |
| 手動構築リソースとの衝突 | 発生しない（A6 で手動リソース無し前提） | R6。import 不要 |
| provider / リソース参照誤り | `validate` / `plan` で参照エラー | behavior「構文・参照エラーなく完了」 |

機微情報の漏洩防止: `api_key` 等は `variable` に `sensitive = true` を付与し、plan 出力・state での露出を抑える（key 値自体は #1258 非スコープのため Terraform で発行しない）。

## テスト方針

Terraform は自動実行環境から実行しない（B4）。検証・plan・apply は作業者が CLI で手動実行する。

- **ローカル / レビュー**:
  - `terraform fmt -check`（整形）
  - `terraform validate`（構文・型・参照。認証不要）
  - `terraform plan`（認証変数供給下。全 add・import 不要を確認）
- **自動実行環境**:
  - Terraform 用の GitHub Actions job は追加しない。
- **手動 plan/apply**:
  - 作業者が HCP Terraform token と NewRelic 認証変数を環境変数で供給する。
  - `infra/newrelic` で `terraform init` / `terraform plan` / `terraform apply` を実行する。
- **構成整合の確認**（behavior 準拠、人手レビュー + 可能なら軽量 grep）:
  - ダッシュボード / アラート / metric pruning が参照するメトリクス名・属性キーが `locals.tf` 経由で #1209 実装値（A1/A2）に一致し、未定義名を参照しない。
  - アラートしきい値が `locals.budget`（B1）と一致。
  - metric pruning の削除対象が allowlist（A2）と衝突しない。

## リスクと代替案

- **R-1: Log / Span の保存前 drop は無料枠前提では Terraform 管理できない**。Pipeline Control Cloud Rules は Advanced Compute の有料アドオンであり、無料枠前提では使わない。
  - 対策: PII / 絶対パスを送らない責務は #1209 のアプリ側 allowlist と path scrub に置く。NewRelic 側 Terraform は Metric pruning に限定し、高カーディナリティな Metric 属性削除だけを管理する。
- **R-2: Metric pruning は Log / Span の PII 二次防御ではない**。Metric の属性削除には有効だが、Log / Span の保存前破棄は行わない。
  - 対策: README と requirements にこの制約を明記し、`newrelic_pipeline_cloud_rule` を再導入しない。
- **R-3: crash / exception の NRQL 対象データ型が #1209 実装に依存**。crash 急増条件の query は NewRelic 側で error がどの event 型（Log / Span / ErrorTrace）に入るかに依存する。
  - 対策: #1209 のエラー送出経路を実装時に確認し NRQL を確定。確認できるまでは仮定 D2 を置く。
- **R-4: plan/apply の実行には HCP Terraform 認証と NewRelic 認証が必要**。自動実行環境で secrets を扱うと運用経路が増える。
  - 対策: Terraform は自動実行環境に組み込まず、plan/apply は作業者がローカル環境で手動実行する（仮定 D4）。
- **代替案（backend ロック）**: S3 互換 backend + lock file を使う案は、状態管理と実行履歴を HCP Terraform に集約できないため不採用。HCP Terraform の workspace state / run 履歴を正とする。

## 仮定

requirements の A1〜A8、behavior の B1〜B6 を前提とする。本書で追加する設計仮定:

- **D1**: `required_version >= 1.10`。`newrelic` provider はメジャー固定でピン留めし `.terraform.lock.hcl` をコミット。
- **D2**: crash/exception は OTel ログまたは span error として NewRelic に記録され、NRQL の `WHERE` で error 種別を絞れる。具体 query は実装時に #1209 実装で確認して確定（R-3）。
- **D3**: 無料枠の retention はアカウント tier 固定で Terraform 変更不可。取り込み量制御は Metric pruning によるカーディナリティ制御で達成する。
- **D4**: Terraform は自動実行環境に組み込まない。`fmt -check` / `validate` / `terraform test` / `terraform plan` / `terraform apply` は作業者が CLI で手動実行する。
- **D5**: `locals.tf` を定義の単一源とし、ダッシュボード / アラート / metric pruning は `locals` を参照して #1209 実装値との整合をレビューしやすくする。

## Open Questions

なし（OQ-D1 は更新済み。state backend は HCP Terraform organization `releash` / workspace `newrelic` を採用し、backend 設計へ反映済み）。
