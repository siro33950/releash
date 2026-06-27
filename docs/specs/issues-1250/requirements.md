# Requirements

## Type

Workflow runtime の失敗処理の挙動変更。failure policy の分類・責務境界を確立し、そのポリシーに従った enforcement（自動 retry / timeout 適用 / structured output repair / parallel failure 伝播）まで実装する。observable behavior の変更を伴うため migration / behavior note を残す。

関連: #1250（本 Issue） / #965（手動 Resume、本 Issue を failure-policy 依存として扱う） / #1245（本ポリシーが分類すべき具体 failure case の提供元） / #1209（telemetry 連携先） / #1217（`bridge_common.rs` の責務分割） / #1178（CLOSED、Agent SDK 応答停止の検出・復旧） / #1192（CLOSED）

## 背景と目的

Workflow / Step の起動・実行失敗は複数パターンに分かれるが、現状の対応方針は Codex 起動 timeout、Claude stale timeout、model refusal、parallel child failure propagation、structured output routing failure が混在しており、いずれも同じ「failed」として扱われている。

現状のコード（調査結果）では失敗が以下のように分散して表現されている:

- `WorkflowExecutionState::Failed { reason: String }`（`domain/workflow/value_objects/state.rs`）— workflow 全体の失敗を理由文字列のみで保持。
- `STEP_STATE_FAILED = "failed"` / `STEP_STATE_ABORTED = "aborted"` / `STEP_STATE_INTERRUPTED = "interrupted"`（`domain/workflow/value_objects/step_output.rs`）— step 状態を文字列定数で表現。
- `TurnCompleteDecision::SessionError { node_name, exit_code }`（`domain/workflow/services/transition.rs`）— agent session の非ゼロ exit を exit_code のみで保持。
- `WorkflowEvent::{NodeFailed, RunFailed, RunAborted}`（`adaptor/gateway/workflow/event.rs`）— 失敗 event を理由文字列で記録。
- parallel child の失敗は `ParallelChildState::Failed` と reduce ルール（`AnyNeedsFix` 等、`domain/workflow/services/parallel.rs`）で集約。
- structured output 不整合は `MAX_CONTRACT_REPAIR_ATTEMPTS = 2`（`adaptor/gateway/workflow/runtime_engine_impl.rs`）と `ContractRepairRequested` event で repair 試行回数のみ管理。
- retry / timeout に相当する `RetryPolicy` / `TimeoutPolicy` / `STALE_TIMEOUT` 等の概念は workflow domain に存在しない（runtime 側の codex.rs / bridge_common.rs に個別の sleep/timeout が散在）。

このため「失敗」が exit_code != 0 か否かにほぼ一元化され、retry すべき失敗・部分成功として受容できる失敗・回復不能で terminal な失敗・人間の操作を要する失敗を区別できない。結果として次の改善が必要な failure mode が放置されている:

- Codex app-server の起動遅延が即 `node_failed` になり retry されない。
- 重い判断 step が stale timeout で失敗扱いになる。
- 1 つの review child の model refusal が workflow 全体を巻き込む。
- structured output が少し崩れただけで repair / reroute なしに fail する。
- user abort と runtime failure が同じ failed 系として扱われ、分析しづらい。

本 Issue の目的は、Workflow runtime が失敗を **retryable / partial / terminal / user-action-required** の観点で分類できる共通 failure policy（分類軸と責務境界）を確立し、**そのポリシーに従って実際に retry / timeout 適用 / structured output repair / parallel failure 伝播を行う enforcement までを実装し**、上記 5 つの failure mode を実改善することである。あわせて失敗の性質を telemetry（#1209）から観測可能にする。これにより #965（手動 Resume）が「どの失敗を resume/retry でき、どれを partial として受容し、どれを terminal とするか」を判断できる土台を提供する。

## スコープ

### 失敗分類（failure kind）の定義

`WorkflowStepFailureKind` 相当の分類を domain に定義する。少なくとも以下の失敗源を区別できること:

- startup timeout（Codex app-server 起動遅延等）
- stale runtime timeout（重い判断 step の応答停止）
- model refusal / provider policy rejection
- structured output mismatch（contract 不整合）
- validation failure
- user abort
- infrastructure crash

各 kind は retryable / partial / terminal / user-action-required のいずれの扱いになりうるかを区別できる属性を持つ。

### ポリシーの責務境界の確立

以下 4 つのポリシーの責務境界（何を決定し、何を決定しないか）を明確に定義し、それぞれを Workflow runtime の実行経路に適用する（enforcement）。

- `RetryPolicy` — failure kind ごとの retry 可否・retry 回数の上限を決定し、retryable な失敗を実際に再試行する責務。
- `TimeoutPolicy` — model / node kind / workflow template ごとの timeout 値（startup / stale 等）を決定し、その値を実際の待機判定（Codex 起動・stale 検知）に適用する責務。
- `ParallelFailurePolicy` — parallel node で単一子失敗を全体 failed にするか aggregate へ委譲するかを決定し、その通りに伝播させる責務。
- `StructuredOutputRepairPolicy` — structured output mismatch 時の repair / reroute 試行と試行上限超過時の扱いを決定し、実際に repair / reroute を行う責務。

### 既存 failure case の分類と実改善

「改善する failure mode」に挙げられた具体 4 ケース（Codex 起動遅延・stale timeout・model refusal・structured output 崩れ）を含む既存 failure case を上記分類表に対応付け、各ケースが retryable / partial / terminal / user-action-required のどれに属するかを文書化する。あわせて、各ポリシーの enforcement により次の挙動を実現する:

- Codex app-server の起動遅延を startup timeout として分類し、`TimeoutPolicy` / `RetryPolicy` に従って即 `node_failed` とせず retry する。
- 重い判断 step の stale timeout 値を `TimeoutPolicy`（node kind / template 別）に従って適用し、正常な長時間処理を失敗扱いにしない。
- review child の model refusal を `ParallelFailurePolicy` に従って分類・伝播し、単一子の refusal で workflow 全体を巻き込まない。
- structured output の軽微な崩れを `StructuredOutputRepairPolicy` に従って repair / reroute し、即 fail させない。

### telemetry 連携

失敗発生時に telemetry（#1209 の OTel 計装基盤）へ **failure kind / retry count / timeout kind** を含められるようにする。失敗を span status / counter / attribute のいずれで送るかは design.md で確定する。

### migration / behavior note

既存 workflow の observable behavior を変える場合は migration / behavior note を残す。

## 非スコープ

- #965 が所有する手動 Resume 操作・既存 run の継続セマンティクスそのものの実装。本 Issue は失敗分類・ポリシー定義とその enforcement（自動 retry / timeout 適用 / repair / 伝播）までを担い、人間による手動 Resume 操作は対象外とする。
- failure policy を利用者が設定値で可変にする設定 UI / 設定項目の追加。
- #1217 が所有する `bridge_common.rs` の責務別 module 分割。
- #1209 が所有する telemetry 計装基盤（OTel / New Relic）そのものの構築。本 Issue は失敗時に送る属性の追加に限定する。
- workflow エンジンの実行モデル・スケジューリング・DAG 構造の変更。
- model refusal の検出精度向上や provider 固有 API の判定ロジック新規実装（既存の判定結果を分類へ落とすのが対象）。

## 要求事項

1. Workflow step の失敗を `WorkflowStepFailureKind` 相当の分類で表現できること。スコープに列挙した失敗源を区別できる。
2. `RetryPolicy` / `TimeoutPolicy` / `ParallelFailurePolicy` / `StructuredOutputRepairPolicy` の責務境界が定義され、互いの責務が重複しないこと。各ポリシーが Workflow runtime の実行経路に適用（enforcement）されること。
3. 「改善する failure mode」の具体 4 ケースが分類表に落ち、各々が retryable / partial / terminal / user-action-required のどれに属するかが文書化されること。かつ各ケースがポリシー適用により実改善されること（即 fail せず retry / repair / 適切な伝播が行われる）。
4. 失敗時に telemetry へ failure kind / retry count / timeout kind を含められること。
5. user abort と runtime failure が分類上区別され、同一の failed 系として混同されないこと。
6. parallel node の単一子失敗について、全体 failed にするか aggregate へ委譲するかが `ParallelFailurePolicy` で決定され、その通りに伝播すること。
7. 既存 workflow の observable behavior を変える変更には migration / behavior note が残ること。
8. 上記分類・ポリシーの責務境界と enforcement 挙動を担保するテストが存在すること。

## 受け入れ基準の概要

- `WorkflowStepFailureKind` 相当の分類がコードに存在し、列挙した失敗源を区別できることがテストで確認できる。
- `RetryPolicy` / `TimeoutPolicy` / `ParallelFailurePolicy` / `StructuredOutputRepairPolicy` の責務境界が文書（design.md）とコードで一致し、各ポリシーが実行経路に適用されていることがテストで確認できる。
- 具体 4 ケースが分類表（requirements.md / design.md）に明示され、各ケースの実改善挙動（retry / timeout 適用 / repair / 伝播）がテストで確認できる。
- telemetry に failure kind / retry count / timeout kind を含む経路が存在することがテストないしコードで確認できる。
- observable behavior を変えた箇所に migration / behavior note がある。
- `cargo fmt --check` / `cargo clippy -- -D warnings` / `cargo test` が通る。

## 仮定

- **A1（スコープの中心）**: 本 Issue は失敗の分類軸（`WorkflowStepFailureKind`）と 4 ポリシーの責務境界の確立に加え、各ポリシーの enforcement（retry 実行・timeout 値の実適用・structured output の実 repair/reroute・parallel failure propagation の実挙動変更）まで含む（ユーザー確認により確定）。これにより 5 つの failure mode を実改善する。手動 Resume 操作（#965）は本 Issue の対象外。
- **A2（分類の置き場）**: 失敗分類とポリシー型は domain 層（`src-tauri/src/domain/workflow/`）に置き、既存の `WorkflowExecutionState::Failed { reason }` / `TurnCompleteDecision::SessionError` 等の失敗表現を分類へマッピングする方針とする。具体的な型名・配置・公開境界は design.md で確定する。
- **A3（telemetry の送出形式）**: 失敗属性は #1209 の OTel 計装に span status / counter attribute として乗せる。具体形式は #1209 の実装状況を踏まえ design.md で確定する。#1209 が未実装の場合でも、本 Issue は「失敗分類が telemetry へ渡せる構造」を用意することを要求の下限とする。
- **A4（既存挙動の変更）**: enforcement により既存 workflow の observable behavior が変わる（即 fail していた失敗が retry / repair される等）。変更箇所には migration / behavior note を残す。ポリシーの既定値（retry 回数上限・各 timeout 値・parallel 伝播の既定）は現状挙動からの後退を避けつつ design.md で具体値を確定する。
- **A5（#1245 の扱い）**: #1245 は分類すべき具体 failure case の提供元として参照する。本 Issue 着手時点で #1245 の case 一覧が未確定の場合は、Issue 本文の「改善する failure mode」4 ケースを最小の分類対象とする。

## Open Questions

なし。
