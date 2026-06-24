# requirements — issues-1246

Releash 上の Claude turn の応答開始・turn 完了レイテンシを安全に計測できる telemetry を追加し、CLI/公式アプリより遅いケースで遅延がどの区間に存在するかを判別可能にする。

## Type

改善 / 観測性整備（latency telemetry の追加。observable behavior は不変、ユーザー操作 UI の追加なし）

## Goal

Releash 上の Claude が、Claude Code CLI や公式アプリに比べて「送信してから応答が返り始めるまで／turn が完了するまで」の実応答レイテンシが遅く感じられる問題について、**まずどの区間で時間を使っているかを New Relic 上で確定できる latency telemetry を追加する**。これにより、遅延が bridge spawn / SDK `query()` init / Claude subprocess init / resume / permission 待ち / モデル応答待ちのどこにあるかを判別できる状態を作る。

完了時に成功と判断する状態:

- Releash 上の Claude turn について、first SDK event / first assistant event / turn complete までの時間を、ユーザーデータ（本文・tool 入出力・worktree path 等）を含めずに安全に計測できる。
- 上記レイテンシを、resume 有無 / sessionId 有無 / permission mode / model / workflow step 種別 / channel といった次元で分解して比較できる。
- New Relic 上で、遅延が bridge spawn・SDK query init・Claude subprocess init・resume・permission 待ち・モデル応答待ちのどの区間にあるか判別できる。
- 描画遅延（#1214）・WebView メモリ（#1195）・stale/停止（#1178）と混同しない形でメトリクスが分離されている。

## Background

Issue #1246 の記述（および Issue 作者によるコメントの実装方針）と、本リポジトリ調査による事実に基づく。

ここで扱う「遅い」は描画遅延ではなく、送信してから Claude 側の応答が返り始めるまで、または turn が完了するまでの**実応答レイテンシ**を指す。

### 現状の Claude bridge の構造（リポジトリ調査による事実）

- `src-tauri/resources/claude-sdk-bridge.mjs` は Node bridge プロセス自体は常駐させる意図だが、turn ループ（`while` ループ内）で各 turn ごとに `query({ prompt: generator, options })` を新規作成している（行 294 付近）。
- `currentSessionId` がある場合は `options.resume = currentSessionId` を設定して resume している（行 286–287、278）。
- そのため、Node bridge は残していても、Claude Code subprocess / SDK query / session resume の初期化コストを turn ごと、または初回 prompt ごとに払っている可能性がある。
- `@anthropic-ai/claude-agent-sdk` には `startup()` / `WarmQuery` があり、型定義上「CLI subprocess を pre-warm し、query 時の startup latency をなくす」ための API として用意されている（Issue 記載。本変更では計測のみで利用検証は Phase 2）。

### 既存の telemetry 基盤（リポジトリ調査による事実）

- `src-tauri/src/other/telemetry/` に telemetry 基盤が存在する（`attributes.rs` / `mod.rs` / `resource.rs`）。
- `attributes.rs` に `HotPathMetric`（hot path 計測の metric enum）、`PayloadChannel`（`tauri_event` / `websocket`）が定義済み。
- `mod.rs` に `record_hot_path_duration`（histogram + counter）、`set_startup_origin` 等の one-shot origin パターンが存在する。
- 送信エントリは `src-tauri/src/adaptor/controller/command/agent_session/session.rs` の `send_agent_message`（行 1227 付近）。
- turn 実行と SDK event 分岐は `src-tauri/src/infrastructure/agent_session/runtime/bridge_common.rs`。
- 直近で New Relic 連携（Terraform リソース）が追加されている（commit 5d31b455）。本 telemetry は New Relic 上で評価する前提。

### 関連 Issue との差分（混同しないための整理）

- **#1214**: `streaming_parts` の cumulative snapshot 配信による transport/表示負荷が中心。長文応答中の UI/transport 負荷には効くが、first token / model response start latency の主因とは限らない。
- **#1195**: WebView 側で全セッション・全メッセージを保持するメモリ問題。長時間利用後の UI 重さに関係するが、Claude 実応答開始の遅さとは別問題。
- **#1178**: Claude SDK 経路の stale / 応答停止 / 180 秒 timeout に近い「止まる」問題。通常時の応答開始レイテンシ計測・改善とは分けて扱う。

## Users / Actors

- **Releash 本体の開発者**: 遅延の支配項を New Relic 上で確定でき、改善（prewarm / resume 最適化）の対象と効果測定の基準を得られる（直接の受益者）。
- **エンドユーザー**: 間接的受益者。本変更（Phase 1）では計測のみで体験は変わらない。レイテンシ改善は後続の Phase 2 で行う。

## Scope

本 Issue の範囲は、Issue 作者コメントの段階分けに従い **Phase 1（latency telemetry の追加のみ）** とする（人間と合意済み）。受け入れ条件③のレイテンシ改善（prewarm / resume 最適化）は別 Issue（Phase 2）として切り出す。

- Claude turn の各区間を計測する latency telemetry を追加する。少なくとも以下の区間を計測できること:
  - UI 送信 → Rust `send_agent_message`（`start_agent_turn`）到達（`ui_to_start`）
  - bridge spawn 開始 → 完了（`bridge_spawn`）
  - 当該 turn の prompt yield → Claude Code subprocess initialize 完了（`query_init`、turn 開始・初回/turn 間の user prompt 待ち idle を除外し、bridge 側 clock で計測して Rust へ転送）
  - turn 開始 → first SDK message 受信（`first_sdk_event`）
  - turn 開始 → first assistant text/thinking/tool event 受信（`first_assistant_event`）
  - permission request 受信 → permission 応答適用（`permission_wait`）
  - turn 開始 → turn complete 受信（`complete`）
- 上記レイテンシを次元で分解できること。次元は少なくとも: resume 有無、sessionId 有無、permission mode、model（既知ファミリへ正規化、未知は `other`）、context（`chat` | `workflow_step`）、channel（`tauri_event` | `websocket`、既存 `PayloadChannel` 再利用）。
- 計測値は本文・tool 入出力・worktree path 等のユーザーデータを含めない（有界集合へ正規化した次元のみ）。
- bridge（Node, mjs）側で計測すべき区間（`query_init` 等）は bridge 側 clock で計測し、本文を含めない telemetry イベントとして Rust に転送して記録する。
- New Relic 上で operation × turn_kind × context で p50/p95 を比較できる形にする。
- **New Relic インフラ（Terraform, `infra/newrelic/`）を本変更で同時に更新し、追加した turn latency メトリクスを可視化できる状態にする**:
  - `locals.tf` に新規 turn-phase metric 名（または既存 `hot_path` への turn 用 operation 追加）、turn 次元属性（`releash.agent.*` = resume / sessionId / permission mode / model / context）、turn 用 operation リストと `operation_filter` を追加する。
  - `dashboards.tf` に turn latency（`ui_to_start` / `bridge_spawn` / `query_init` / `first_sdk_event` / `first_assistant_event` / `permission_wait` / `complete`）を operation × context で p50/p95 表示するウィジェットを追加する。
  - `data_management.tftest.hcl` 等の既存 Terraform テストが通る状態を維持する。
- #1214 / #1195 / #1178 と混同しないよう、メトリクス名で明確に分離する。

## Non-goals

- **Phase 2 の改善実装**（`startup()` / `WarmQuery` による prewarm、同一 session 内の不要な再 resume 回避、query lifecycle 改善）そのもの。本 Issue では計測基盤のみを用意し、改善は別 Issue（Phase 2）とする（人間と合意済み）。
  - ただし Phase 2 で before/after を同一メトリクスで評価できるよう、計測軸は prewarm 経路と query 直呼び経路を区別可能な設計にしておく。
- CLI/公式アプリとの first-event latency 比較を自動化すること。比較は `first_sdk_event` / `first_assistant_event` の絶対値を基準に手動突き合わせで行う。
- #1214（cumulative snapshot 配信負荷）・#1195（WebView メモリ）・#1178（stale/停止）の問題自体の解決。
- 描画・レンダリング側のパフォーマンス計測。
- **New Relic の latency budget アラート（`alerts.tf` への閾値条件追加）**。本 Issue は計測優先フェーズでベースラインが未確定のため、turn latency のアラート閾値は設定しない。ダッシュボードでの可視化までを範囲とし、アラートはベースライン確定後（Phase 2 以降）に回す（人間と合意済み）。
- 全ロジックを Rust に置く原則の例外を作ること。フロントは送信時刻（epoch ms）を渡すのみで、集計・正規化・記録は Rust 側で行う。

## Requirements

### R1. turn 区間レイテンシの計測
- Claude turn について、`ui_to_start` / `bridge_spawn` / `query_init` / `first_sdk_event` / `first_assistant_event` / `permission_wait` / `complete` の各区間を計測する metric を追加する。
- first SDK event / first assistant event / turn complete は turn ごとに one-shot で記録する（同一 turn 内で重複記録しない）。
- permission_wait は permission request から permission 応答までの待ち時間として記録し、同一 turn 内に複数回発生する場合は request/response ごとに分離して記録する。

### R2. 区間判別が可能な計測粒度
- 遅延が bridge spawn / SDK query init / Claude subprocess init / resume / permission 待ち / モデル応答待ちのどこにあるか判別できる粒度で区間を分離する。
- permission 待ち時間と、stale watchdog、workflow system prompt 増加に起因する時間を、通常のモデル応答待ちと区別してログ上で分離できること。

### R3. 次元による分解
- 各レイテンシを、resume 有無 / sessionId 有無 / permission mode / model / context（chat | workflow_step）/ channel（tauri_event | websocket）で分解できる。
- model は既知モデルファミリへ正規化し、未知は `other` とする。各次元は有界集合に正規化する。

### R4. ユーザーデータ非混入
- telemetry には本文、tool 入出力、worktree path 等のユーザーデータを一切含めない。記録するのは時間（duration / 絶対 ms）と有界集合へ正規化した次元のみ。

### R5. bridge 側計測の転送
- bridge（mjs）側でしか取れない区間（当該 turn の prompt yield → subprocess initialize 完了 = `query_init`）は、初回/turn 間の user prompt 待ち idle を除外して bridge 側 clock で計測し、本文を含めない telemetry イベントとして Rust に転送し、Rust 側で記録する。

### R6. 関連 Issue とのメトリクス分離
- 本 telemetry のメトリクス名は #1214 / #1195 / #1178 と区別でき、それらの描画・メモリ・停止問題と混同せずに評価できる。

### R7. Phase 2 比較可能性の確保
- 計測軸は、後続 Phase 2 で `query()` 直呼び経路と `startup()` / `WarmQuery` 経路の before/after を同一メトリクスで比較できる設計にしておく（経路差を次元または metric で識別可能にする）。Phase 2 の実装自体は本 Issue の範囲外（Non-goal）。

### R8. New Relic インフラ（Terraform）の同時更新
- `infra/newrelic/` の Terraform を本変更で同時に更新し、追加した turn latency メトリクスと次元（`releash.agent.*`）を New Relic 上で可視化できる状態にする。
- `locals.tf` に metric 名・turn 次元属性・turn operation リスト・`operation_filter` を追加し、`dashboards.tf` に turn latency を operation × context で p50/p95 表示するウィジェットを追加する。
- Rust 側で記録する metric 名・次元キー（`releash.agent.*` 等）と Terraform 側の定義（NRQL のメトリクス名・FACET 属性）が一致していること。
- 既存の Terraform テスト（`*.tftest.hcl`）が通る状態を維持する。
- latency budget アラート（`alerts.tf` への閾値追加）は本 Issue の範囲外（Non-goal。ベースライン確定後に回す）。

## 受け入れ基準の概要

Issue の受け入れ条件に対応する:

1. Releash 上の Claude turn について、first SDK event / first assistant event / turn complete までの時間を安全に（ユーザーデータを含めず）計測できる。（R1・R4）
2. CLI/公式アプリより遅いケースで、遅延が bridge spawn・SDK query init・Claude subprocess init・resume・permission 待ち・モデル応答待ちのどこにあるか判別できる。（R2・R3・R5）
3. prewarm または query lifecycle 改善による応答開始レイテンシ改善を、同一メトリクスで before/after 評価できる基盤がある。改善実装そのものは Phase 2（別 Issue）とする。（R7）
4. #1214 / #1195（描画・WebView メモリ問題）と混同しない形で評価できる。（R6）
5. 追加した turn latency メトリクスが New Relic ダッシュボード（`infra/newrelic/dashboards.tf`）上で operation × context の p50/p95 として可視化され、Rust 側 metric/次元定義と一致している。（R8）

詳細な Gherkin 形式の受け入れ基準は `behavior.md` で定義する。

## Constraints

- 全ロジックは Rust（Tauri バックエンド）側に実装する。フロントは送信時刻（epoch ms）を渡すのみ（`.claude/rules/rust-first-logic.md`）。
- 既存 telemetry 基盤（`src-tauri/src/other/telemetry/` の `HotPathMetric` / `record_hot_path_duration` / origin パターン / `PayloadChannel`）を再利用し、新たな並行基盤を作らない。
- New Relic 上で評価する（既存 New Relic 連携を前提）。Rust 側の metric 名・次元キーと `infra/newrelic/` の Terraform 定義（`locals.tf` のメトリクス名・FACET 属性）を一致させ、Terraform テスト（`*.tftest.hcl`）を通す。
- telemetry がユーザーデータを漏らさないこと（R4）はセキュリティ要件として必須。
- CI と同一のチェック（`cargo fmt --check`・`cargo clippy -- -D warnings`・`cargo test`）を通す。
- 計測追加が通常の turn 実行パスに観測可能な遅延・副作用を持ち込まないこと（observable behavior 不変）。

## Success Criteria

- 受け入れ基準 1・2・4・5 を満たす（計測基盤が機能し、New Relic 上で可視化できる）。
- 受け入れ基準 3 は、改善の before/after を測れる計測軸が用意されていること（改善実装は Phase 2）で満たす。
- 新規ロジック（区間計測・次元正規化・bridge 転送）に単体テストがある。
- telemetry にユーザーデータが含まれないことがテストまたはレビューで確認できる。

## 仮定

- **A2（基盤再利用）**: 計測は既存 telemetry 基盤（`HotPathMetric` への variant 追加・`record_*` の同型関数・`startup_origin` 相当の one-shot origin パターン）を拡張して実装する。Issue 作者コメントの実装タッチポイントに沿う。
- **A3（評価先）**: メトリクスは New Relic 上で operation × 次元の p50/p95 として評価する。直近の New Relic 連携追加を前提とする。
- **A4（CLI 比較）**: CLI/公式アプリとの比較は自動化せず、絶対 ms を手動突き合わせで行う。

## Open Questions

なし（本 Issue のスコープを Phase 1（latency telemetry の追加のみ）に限定することで人間と合意済み）。
