# Requirements

## Type

新規。NewRelic の観測リソース（ダッシュボード / アラート / データ管理）を `newrelic/newrelic` Terraform provider で IaC 化する。アプリ計装ではなく**観測先 SaaS 側の構成**をコード化する Issue。

関連: #1258 / #1209（テレメトリ計装の実装） / `docs/releash-performance-architecture-audit.md` M0

## 背景と目的

#1209 で OpenTelemetry → NewRelic への性能・crash・利用イベントテレメトリ送信を実装し、観測先 SaaS を NewRelic 無料枠に一本化した。一方で、送信先である NewRelic 側のリソース（ダッシュボード・アラート・データ管理ポリシー）は現状すべて**手動運用または未設定**であり、コード化されていない。

このため次の課題がある。

- ダッシュボード / アラートの定義がレビュー対象にならず、変更履歴も残らない。
- 性能予算（audit M0）や #1209 の計測項目とダッシュボードの整合がコード上で担保されない。
- 無料枠 100GB/月の取り込み量制御が明示的なルールとして固定されていない。
- NewRelic Pipeline Control Cloud Rules は Advanced Compute の有料アドオンであり、無料枠前提では使えない。PII 非送信要件は #1209 のアプリ側一次防御（送らない）を正とし、NewRelic 側では次元付き Metric の高カーディナリティ属性削除に限定する。

目的は、上記3種のリソースを `newrelic/newrelic` Terraform provider で IaC 化し、**レビュー可能・再現可能・履歴管理可能**な状態にすること。これにより、#1209 の計測項目および性能予算（audit M0）と整合する可視化・検知・データ管理を、コードとして固定する。

## スコープ

Terraform 管理対象は以下の3点に限定する。

### 1. ダッシュボード

#1209 で計測する hot path メトリクスを可視化する。可視化対象は #1209 実装で実際に送信している以下の OTel メトリクス名に揃える（仮定: メトリクス名は #1209 実装の現行値を正とする。下記「## 仮定」参照）。

- 起動・初期化: `releash.startup.duration_ms`（`releash.operation` = `startup.app` / `startup.first_window_ready` / `startup.first_repo_snapshot_ready`）
- Git / Diff hot path: `releash.hot_path.duration_ms`（`releash.operation` = `git.status_scan` / `git.diff_stats` / `review.file_open`）
- AgentSession session IO: `releash.hot_path.duration_ms`（`releash.operation` = `session.list` / `session.get_meta` / `session.get_page` / `session.load_full` / `session.append` / `session.persist_parts` / `session.save_full`）、`releash.session.save_bytes`
- AgentSession streaming / payload: `releash.agent_stream.payload_bytes`（`releash.channel` = `tauri_event` / `websocket`）、`releash.agent_stream.emit_interval_ms`、`releash.agent_stream.dropped_frames`、`releash.agent_stream.ws_reconnects`
- リソース観測: `releash.process.rss_bytes`、`releash.process.cpu_percent`、`releash.frontend.mounted_xterm_count`、`releash.pty.active_count`
- 操作成否 / 利用イベント: `releash.operation.status`（`releash.status` = `success` / `failure`）、`releash.usage.events`（`releash.usage_event` = `settings_saved` / `worktree_created` / `worktree_removed`）

### 2. アラート / NRQL 条件

性能予算逸脱・crash 急増・接続品質劣化を検知する NRQL アラート条件を定義する。検知観点（しきい値の具体値は behavior / design で確定。下記「## Open Questions」参照）:

- 性能予算逸脱: repo snapshot / diff open / session IO 等の duration が audit M0 の予算（repo snapshot 中規模で 200ms 台、file diff open 小/中ファイル 500ms 未満 等）を逸脱
- crash 急増: `releash.error`（Errors Inbox）相当のエラーログ / exception の急増
- 接続・描画品質劣化: `releash.agent_stream.ws_reconnects` / `releash.agent_stream.dropped_frames` の増加
- streaming payload 異常: `releash.agent_stream.payload_bytes` が通常 frame 予算（64KB 未満）を継続的に超過

### 3. データ管理

- アラート条件 / policy の定義までを本 Issue のスコープとし、通知先（NewRelic workflow / destination: Slack / Discord / メール等）の連携は含めない（Q2 確定）。
- 取り込み量制御: 無料枠 100GB/月に収めるため、次元付き Metric の高カーディナリティ属性を `newrelic_metric_pruning_rule` で削除する。
- 有料アドオンである Pipeline Control Cloud Rules / `newrelic_pipeline_cloud_rule` は定義しない。Log / Span の保存前 drop は本 Issue の Terraform 構成では行わない。

### 配置 / 構成

- Terraform コードはアプリと同リポジトリ内 `infra/newrelic/` に配置する。
- state は **HCP Terraform** で管理する。organization は `releash`、workspace は `newrelic` とする（Q3 更新）。
- NewRelic 認証情報（provider 認証用 User API key 等）は Terraform 変数経由で注入し、リポジトリにコミットしない。

## 非スコープ

- NewRelic アカウント自体の作成（Terraform で作成不可・手動）。
- ライセンスキー / Ingest・API キーの発行・配布。現状の 1Password（`op://releash/new-relic/`）手動運用 → ローカル環境変数注入を継続する。Terraform でのキー発行はキー値が state に平文で残るため対象外。
- アプリ側の計装実装（#1209 の範囲）。メトリクス名・属性・PII 一次防御の変更は本 Issue では行わない。
- Terraform の自動実行環境への組み込み。検証・plan・apply は作業者が CLI で手動実行する。

## 要求事項

### R1. ダッシュボードの IaC 化

- `infra/newrelic/` 配下に、上記スコープ1のメトリクスを可視化する `newrelic_one_dashboard`（または相当リソース）を Terraform で定義する。
- ダッシュボードのメトリクス定義が #1209 の計測項目および性能予算（audit M0）と整合していること。

### R2. アラート / NRQL 条件の IaC 化

- 上記スコープ2の検知観点を `newrelic_nrql_alert_condition`（および policy）等で Terraform 定義する。
- しきい値は audit M0 の性能予算および #1209 の payload 予算と整合させること。

### R3. データ管理の IaC 化

- 無料枠 100GB/月を超過しないため、次元付き Metric の高カーディナリティ属性を削除する `newrelic_metric_pruning_rule` を Terraform 定義する。
- Advanced Compute の有料アドオンを必要とする Pipeline Control Cloud Rules / `newrelic_pipeline_cloud_rule` は Terraform 定義に含めない。

### R4. HCP Terraform state

- state が HCP Terraform organization `releash` / workspace `newrelic` で管理されていること。

### R5. 認証情報の非コミット

- provider 認証情報がコードに含まれず、Terraform 変数経由で供給されること。

### R6. plan のクリーン通過と import 方針

- `terraform plan` がクリーンに通ること。
- NewRelic アカウントに手動構築済みリソースは存在しない（Q1 確定）ため、全リソースを Terraform で新規定義・apply する。`terraform import` は不要。

## 受け入れ基準の概要

Issue #1258 の完了条件に対応する。

- [ ] `infra/newrelic/` に上記3スコープ（ダッシュボード / アラート / データ管理）を定義した Terraform 構成が存在する（R1 / R2 / R3）。
- [ ] HCP Terraform organization `releash` / workspace `newrelic` が設定されている（R4）。
- [ ] `terraform plan` がクリーンに通る（手動構築済みリソースは無いため新規 apply、import 不要）（R6）。
- [ ] 有料アドオンを必要とする Pipeline Control Cloud Rules が Terraform 定義に含まれていない（R3）。
- [ ] 認証情報がコードに含まれず、変数経由で供給される（R5）。
- [ ] ダッシュボードのメトリクス定義が #1209 の計測項目および性能予算（audit M0）と整合している（R1）。

## 仮定

- A1. 可視化・アラート・metric pruning が参照するメトリクス名・属性キーは、#1209 実装の現行値（`releash.startup.duration_ms` / `releash.hot_path.duration_ms` / `releash.agent_stream.*` / `releash.session.save_bytes` / `releash.process.*` / `releash.frontend.mounted_xterm_count` / `releash.pty.active_count` / `releash.operation.status` / `releash.usage.events`、属性 `releash.operation` / `releash.status` / `releash.channel` / `releash.usage_event`）を正とする。これらは `src-tauri/src/other/telemetry/` から確認した実装値。
- A2. 許可される resource 属性は `service.version` / `os.type` / `releash.build_type` / `service.name` の4種に限定される（#1209 の実装値）。NewRelic 側では Cloud Rules を使わず、Metric pruning の削除対象がこの allowlist と衝突しないことだけを担保する。
- A3. 本 Issue のゴールは観測先（NewRelic 側）構成のコード化であり、アプリ計装は変更しない。
- A4. Terraform は自動実行環境に組み込まない。`init -backend=false` / `fmt` / `validate` / `test` / `plan` / `apply` は作業者が CLI で手動実行する。
- A5. 性能予算の正本は `docs/releash-performance-architecture-audit.md` M0 であり、アラートしきい値はこれに従う。
- A6. NewRelic アカウントに手動構築済みリソースは存在しない（Q1 確定）。全リソースを Terraform で新規作成し、import は行わない。
- A7. アラートの通知連携（NewRelic workflow / destination）は本 Issue のスコープ外（Q2 確定）。アラート条件 / policy の定義までを対象とする。
- A8. リモート state backend は HCP Terraform（organization `releash` / workspace `newrelic`）を採用する（Q3 更新）。

## Open Questions

なし（Q1〜Q3 は確定済み。確定内容は「## 仮定」A6〜A8 および各要求事項へ反映済み）。
