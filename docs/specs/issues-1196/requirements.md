# Requirements

## Type

メモリ削減（構造的問題の是正）。親 ISSUE #1191 の広域調査で特定した無制限増大経路 C10 を分離して対応する。

## Goal

terminal 化した workflow run の `WorkflowExecution` 本体を runtime の `executions` map に常駐させ続けることで生じる、workflow run の進行・蓄積に比例したメモリの無制限増大を解消する。

完了時には、workflow run を複数回完了させても、terminal 化した run の `WorkflowExecution`（`step_history` / `step_outputs` / `workflow_variables` / `workflow_definition` を含む）が常駐し続けず、run 数・step output 量に比例して常駐メモリが積み上がらない状態を目指す。同時に、完了 run の履歴表示・復元、および active run の進行という外部から観測可能な振る舞いは不変に保つ。

## Background

- 親 ISSUE #1191 の広域調査で特定した無制限増大経路の一つ（候補 C10）。#1191 本体（A群: 振る舞い不変・低リスクな純粋削減）から分離し、解放/復元の設計判断を要するため別 ISSUE（本 #1196）とした。詳細は `docs/specs/issues-1191/design.md`（候補 C10）を参照。
- workflow runtime の `executions`（`run_id -> WorkflowExecution` の `Mutex<HashMap<String, WorkflowExecution>>`）は、terminal 化（Completed / Failed / Aborted）後も `WorkflowExecution` 本体を通常経路では削除しない。
  - 定義: `src-tauri/src/adaptor/gateway/workflow/runtime_engine_impl.rs` L161-167 付近。
  - terminal 経路（L1948-1966 / L2730-2741 / L3316-3329 付近）は step session 解放・`session_workflow_refs` の cleanup・状態 broadcast のみを行い、`executions` からの削除は行わない。
  - `execs.remove` は起動失敗系（RunStarted ログ書き込み失敗時のロールバック、L652 付近）でのみ呼ばれる。
- `WorkflowExecution` は `step_history` / `step_outputs` / `workflow_variables` / `workflow_definition` を保持し、`to_snapshot()` / `to_workflow_state()` でも全フィールドを clone するため、run が大きいほど常駐量と状態 emit 時のピークが増える。
- 旧バージョン（v0.3.53）の `workflow/engine.rs` にも同型の `executions` map と cleanup-only な terminal 経路が存在するため、これは v0.3.55 だけの新規混入ではなく、workflow 利用時の構造的問題である。
- 調査により、terminal run の履歴は既に永続化されており、`executions` から消えても再構築可能な素地があることを確認済み（[仮定] 下記参照）。
  - Run Store（`workflow_runs/{run_id}.json` の metadata）と Event Log（`workflow_logs/{run_id}.ndjson` の append-only event 列）に run の状態・履歴が永続化されている。
  - `restore_execution_by_run_id()`（`runtime_engine_impl.rs` L1040-1097 付近）が、`executions` に run が存在しない場合に Run Store + Event Log から `WorkflowExecution` を on-demand 再構築する経路として既に実装されている。
  - 完了 run の履歴表示は `usecase/workflow/query_service.rs` の `get_run_log` 経由で Event Log から供給される。

## Users / Actors

- Releash で workflow run を複数回実行・完了させるエンドユーザー（terminal run の履歴を後から参照する利用者を含む）。
- workflow run を進行・terminal 化し、状態を永続化・broadcast するバックエンド（workflow runtime）。
- メモリ挙動の切り分け・検証を行う開発者。

## Scope

- terminal 化（Completed / Failed / Aborted）した workflow run の `WorkflowExecution` を、terminal 化と同時に `executions` map から解放する（即時解放方針）。
- terminal run の履歴表示・状態問い合わせが必要になった時点で、Run Store + Event Log からの再構築（`restore_execution_by_run_id()` 経路）で `WorkflowExecution` を供給する。
- active run（Running / WaitingApproval）の進行、および terminal run の履歴表示・復元という外部から観測可能な振る舞いの不変を保つこと。
- terminal run の常駐メモリが解放されること（run 数・step output 量に比例して積み上がらないこと）を確認するための検証手段の整備。

## Non-goals

- 親 #1191 の A群純粋削減（C2/C3/C4/C7/C8/C9）の実装。本 ISSUE では扱わない。
- 他の分離 ISSUE が扱う解放設計（C1 → #1194 / C6 → #1195）の対応。
- terminal run 解放の「即時 / 遅延 / 直近 N 件保持」のうち、即時解放以外の保持戦略の導入。本 ISSUE は即時解放を採用する（[仮定]）。
- workflow run の履歴永続化・event log・Run Store の機能仕様そのものの変更・再設計。
- 完了 run のメモリ削減と無関係なリファクタ（clean architecture 構造の不要な変更を含む）の混在。
- メモリ使用量の新規モニタリング機能・常設プロファイラの製品化。

## Requirements

- terminal 化した run の `WorkflowExecution` が、terminal 化と同時に `executions` map から削除され、本体（`step_history` / `step_outputs` / `workflow_variables` / `workflow_definition`）が常駐し続けないこと。
- terminal run の履歴表示・状態問い合わせが、解放後も Run Store + Event Log からの再構築によって従来どおり供給され、表示内容が変化しないこと。
- active run（Running / WaitingApproval）の進行・状態遷移・broadcast、および worktree 起点の active run 検索（`find_by_worktree` 等）の振る舞いが、本修正によって変化しないこと。
- 即時解放と再構築の併用によって、解放と再構築が競合する状況（terminal 化直後の状態問い合わせ・再開・並列 run など）でも、状態の不整合や run の取りこぼしが発生しないこと。
- 既存のテスト（`cargo test` / `pnpm test`）および lint（`cargo clippy -D warnings` / `pnpm lint`）が通ること。

## Acceptance Criteria（概要）

- workflow run を複数回完了させた後、terminal 化した run の `WorkflowExecution` 相当のメモリが `executions` に常駐し続けず、run 数・step output 量に比例して常駐メモリが積み上がらないことを実測で確認できる。
- terminal run の履歴表示・状態問い合わせが、解放前後で同一の表示内容を返す（再構築経由でも振る舞いが変わらない）。
- active run の進行・terminal 化・broadcast、worktree 起点の active run 検索に退行がない。
- 既存の `cargo test` / `pnpm test` / `cargo clippy -D warnings` / `pnpm lint` が green。

## Assumptions（仮定）

- [仮定] terminal run の解放方針は「即時解放（terminal 化と同時に `executions` から削除し、必要時に再構築）」とする（ユーザー合意済み）。直近 run の表示応答性確保のための「直近 N 件保持」「遅延解放」等の保持戦略は本 ISSUE では採用しない。
- [仮定] terminal run の履歴表示・復元は、既存の Run Store（`workflow_runs/{run_id}.json`）+ Event Log（`workflow_logs/{run_id}.ndjson`）と `restore_execution_by_run_id()` 経路で再構築可能であり、即時解放してもこれらから従来どおり供給できる。再構築経路の網羅性（全 terminal status・全表示経路で再構築が成立するか）は design / 実装フェーズで裏取りする。
- [仮定] terminal 化直後にフロントが run snapshot を要求した場合の再構築コストは許容範囲であり、ユーザーの体感的な表示応答性に劣化を生じさせない範囲に収まる。劣化が観測された場合は design フェーズで保持戦略の再検討を行う。

## Open Questions

なし（解放方針はユーザー合意により「即時解放」で確定）。
