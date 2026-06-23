# Behavior

#1258: NewRelic 観測リソース（ダッシュボード / アラート / データ管理）の Terraform IaC 化。

本書は観測可能な振る舞いのみを Gherkin で定義する。Terraform リソースの内部 HCL 構造や provider API 呼び出しの詳細は扱わず、「構成として何が固定されるか」「どの入力に対しどの結果が観測されるか」を記述する。

## 用語

- **観測者**: `infra/newrelic/` の Terraform 構成をレビュー・実行する人。
- **クリーンな plan**: 作業者が `terraform plan` を実行し、HCP Terraform workspace 上でエラーなく完了すること。state とコードの差分有無自体は問わない（新規 apply 前は差分ありが正常）。
- **許可属性 allowlist**: resource 属性 `service.version` / `os.type` / `releash.build_type` / `service.name` の4種（#1209 実装値、A2）。
- **#1209 メトリクス集合**: requirements スコープ1 に列挙されたメトリクス名・属性キー（A1）。

## 仮定（behavior レベルで確定したもの）

- B1. アラートしきい値は audit M0 の性能予算（A5）を正本とし、以下を採用する。
  - 性能予算逸脱（duration 系）: P95 集計で評価する。repo snapshot 系（`git.status_scan` / `session.list` / `session.get_*` / `session.load_full` 等）は **300ms**（M0「200ms 台」の上限）、diff open 系（`git.diff_stats` / `review.file_open`）は **500ms**（M0「500ms 未満」）を超過したら逸脱とみなす。
  - streaming payload 異常: `releash.agent_stream.payload_bytes` が **65536 bytes（64KB）** を継続的に超過したら異常とみなす。
- B2. 評価集計は P95、評価ウィンドウは一定時間内の継続（瞬間スパイクで誤検知しない）とする。具体的なウィンドウ長・継続条件の数値は design で確定する（OQ3 確定: behavior は「一定ウィンドウの継続超過で発火する」方針のみ固定し、ウィンドウ長・連続回数等の数値は design に委ねる）。なお B5 / B6 のカウント型アラートは「5 分間で N 件」を初期値として持つが、これも design でチューニング可能とする。
- B5. crash 急増アラートは**固定の絶対値**で検知する（OQ1 確定）。初期値は「5 分間で 5 件以上の crash/exception 相当エラー」とする。audit M0 に根拠数値が無いため初期値であり、運用実績に応じた具体値は design でチューニングする。
- B6. 接続・描画品質劣化アラートは**固定の絶対値**で検知する（OQ2 確定、B5 と方式を統一）。初期値は「5 分間で `releash.agent_stream.ws_reconnects` 合計 5 回以上、または `releash.agent_stream.dropped_frames` 合計 10 件以上」とする。根拠数値が無いため初期値であり、具体値は design でチューニングする。
- B3. 無料枠前提のため、Advanced Compute の有料アドオンである Pipeline Control Cloud Rules は使わない。データ管理は次元付き Metric の高カーディナリティ属性を `newrelic_metric_pruning_rule` で削除する範囲に限定する（A2）。
- B4. Terraform は自動実行環境に組み込まない。検証・plan・apply は作業者が CLI で手動実行する（A4 / R6）。

---

## Feature: NewRelic 観測リソースの Terraform IaC 化

  #1258 の受け入れ基準に対応する。`infra/newrelic/` の Terraform 構成として、
  ダッシュボード・アラート・データ管理・state backend・認証注入の各振る舞いを固定する。

  Background:
    Given アプリと同一リポジトリに `infra/newrelic/` の Terraform 構成が存在する
    And provider は `newrelic/newrelic` である
    And 観測対象メトリクスは #1209 メトリクス集合に揃っている
    And 性能予算の正本は audit M0 である

  ### Rule: ダッシュボードは #1209 の計測項目を可視化する（R1）

    Scenario: hot path メトリクスがダッシュボードで可視化される
      Given #1209 メトリクス集合が NewRelic に取り込まれている
      When 観測者がダッシュボード構成を確認する
      Then 起動・初期化（`releash.startup.duration_ms`）が可視化されている
      And Git/Diff hot path（`releash.hot_path.duration_ms` の `git.status_scan` / `git.diff_stats` / `review.file_open`）が可視化されている
      And AgentSession session IO（`releash.hot_path.duration_ms` の session 系操作 / `releash.session.save_bytes`）が可視化されている
      And streaming/payload（`releash.agent_stream.payload_bytes` / `emit_interval_ms` / `dropped_frames` / `ws_reconnects`）が可視化されている
      And リソース観測（`releash.process.rss_bytes` / `cpu_percent` / `releash.frontend.mounted_xterm_count` / `releash.pty.active_count`）が可視化されている
      And 操作成否・利用イベント（`releash.operation.status` / `releash.usage.events`）が可視化されている

    Scenario: 可視化メトリクス名が #1209 実装値と一致する
      Given ダッシュボードが参照するメトリクス名・属性キーが構成に記述されている
      When 観測者がメトリクス名を #1209 実装値（A1）と突き合わせる
      Then 構成に存在しないメトリクス名・属性キーは参照されていない

    Scenario: ダッシュボードのしきい値表現が性能予算と整合する
      Given ダッシュボードが duration 系メトリクスを表示する
      When 観測者が表示基準（目標線・閾値表示等）を確認する
      Then その基準は audit M0 の性能予算（B1 の値）と整合している

  ### Rule: アラート条件は予算逸脱・異常を検知する（R2）

    Scenario Outline: 性能予算逸脱を検知するアラート条件が定義されている
      Given duration メトリクス "<metric_op>" が NewRelic に取り込まれている
      When その P95 が "<budget_ms>" ms を継続的に超過する
      Then 当該観点のアラート条件が発火しうるよう構成に定義されている

      Examples:
        | metric_op                         | budget_ms |
        | git.status_scan (repo snapshot)   | 300       |
        | session.list (session IO)         | 300       |
        | git.diff_stats (diff open)        | 500       |
        | review.file_open (diff open)      | 500       |

    Scenario: crash 急増を検知するアラート条件が定義されている
      Given crash/exception 相当のエラーイベントが取り込まれている
      When エラー発生数が 5 分間で 5 件以上に達する
      Then crash 急増を検知するアラート条件が発火しうるよう構成に定義されている

    Scenario: 接続・描画品質劣化を検知するアラート条件が定義されている
      Given `releash.agent_stream.ws_reconnects` / `releash.agent_stream.dropped_frames` が取り込まれている
      When 5 分間で ws_reconnects 合計が 5 回以上、または dropped_frames 合計が 10 件以上に達する
      Then 接続・描画品質劣化を検知するアラート条件が発火しうるよう構成に定義されている

    Scenario: streaming payload 異常を検知するアラート条件が定義されている
      Given `releash.agent_stream.payload_bytes` が取り込まれている
      When payload が 65536 bytes（64KB）を継続的に超過する
      Then streaming payload 異常を検知するアラート条件が構成に定義されている

    Scenario: アラート条件は policy に紐づくが通知連携は含まない
      Given アラート条件群が定義されている
      When 観測者が構成を確認する
      Then 各条件は policy に紐づいている
      And NewRelic workflow / destination（Slack / Discord / メール等）の通知連携は構成に含まれない

  ### Rule: データ管理は無料枠と有料アドオン不使用を固定する（R3）

    Scenario: 取り込み量制御ルールが定義されている
      Given 無料枠は 100GB/月である
      When 観測者がデータ管理構成を確認する
      Then 次元付き Metric の高カーディナリティ属性を削除する `newrelic_metric_pruning_rule` が構成に定義されている

    Scenario: 有料アドオンの Cloud Rules は使わない
      Given Pipeline Control Cloud Rules は Advanced Compute の有料アドオンである
      When 観測者がデータ管理構成を確認する
      Then `newrelic_pipeline_cloud_rule` は構成に含まれない
      And Log / Span の保存前 drop はこの Terraform 構成に含まれない

    Scenario: Metric pruning はアプリ側 allowlist と衝突しない
      Given #1209 の許可属性 allowlist が存在する
      When 観測者が Metric pruning の削除対象を確認する
      Then `service.version` / `os.type` / `releash.build_type` / `service.name` は削除対象に含まれない

  ### Rule: state は HCP Terraform で管理される（R4）

    Scenario: state backend が HCP Terraform に設定されている
      Given Terraform 構成に cloud 設定が存在する
      When 観測者が backend 種別を確認する
      Then HCP Terraform organization は `releash` である
      And workspace は `newrelic` である
      And state はローカルファイルに固定されていない

  ### Rule: 認証情報はコードに含まれず変数経由で供給される（R5）

    Scenario: provider 認証情報が変数経由で注入される
      Given Terraform 構成が NewRelic provider 認証を要求する
      When 観測者が認証情報の供給経路を確認する
      Then 認証情報（User API key 等）は Terraform 変数経由で注入される
      And 認証情報のリテラル値はリポジトリにコミットされていない

    Scenario: 認証変数が未供給だと plan は認証エラーになる
      Given 認証変数が供給されていない
      When 観測者が `terraform plan` を実行する
      Then 認証情報不足に起因するエラーで失敗する

  ### Rule: plan はクリーンに通り import は不要（R6）

    Scenario: 認証変数を供給した plan がクリーンに通る
      Given 必須変数（認証情報）が供給されている
      And HCP Terraform workspace への認証と権限がある
      And NewRelic アカウントに手動構築済みリソースは存在しない（A6）
      When 観測者が `terraform plan` を実行する
      Then plan は構文・参照エラーなく完了する
      And 全リソースは新規作成（add）として計画される
      And `terraform import` を必要とするリソースは存在しない

    Scenario: 構成は静的検証を通過する
      Given `infra/newrelic/` の Terraform 構成が存在する
      When 観測者が `terraform validate` を実行する
      Then 構文・型エラーなく成功する

---

## Open Questions

なし（OQ1〜OQ3 は確定済み。確定内容は B5 / B6 / B2 へ反映済み）。
