# Design 01

## 開始状態

初回。差分の基準は base ブランチ `main`、派生点 `4267f22f release: v0.4.15 (#1849)`。作業ブランチ `feat/issues/1838` の未コミットの変更は `docs/specs/issues-1838/` の追加だけで、コードは派生点のままである。実装の状態は `requirements.md` の Current Behavior を正とする。この周までに解消・見送りとなった Thread は無い。

Current Behavior が挙げる箇所に加えて、開始状態の確認で次を追認した。

- `src-tauri/src/workflow_control_plane_acceptance.rs:58-64` の `AcceptanceWorkflowExecutionStatus` は `WaitingApproval` / `Interrupted` を `#[cfg(test)]` 無しで持ち、`:1165-1174` で domain の状態から変換している
- `proto/client.proto:1773-1780` の `NodeExecutionStatusView.waiting_approval = 3` は NodeExecution の状態であり、本 ISSUE の対象ではない
- `proto/client.proto:3254-3268` の `WorkspaceHistoryStatus` に入る値は、`adaptor/gateway/workspace_tree/query_service.rs:238` で Workflow 実行の `ExecutionStatus::as_str()` から作られる
- `docs/glossary/DOMAIN.md:112` は既に「WorkflowExecution は `Running` / `Completed` / `Aborted` を所有する」と定めており、本 ISSUE で改訂する必要はない

## 変える部分

- Workflow 実行の状態を3値にする: `WaitingApproval` と `Interrupted` を、テストビルドを含めて全層から取り除く。根拠: R-001「Workflow 実行の状態は `Running` / `Completed` / `Aborted` の3つだけである。中断中および承認待ちにあたる Workflow 実行の状態は、テストビルドを含めてどの層にも存在しない」/ B-001。ルート: 委任（proto は「固定するルート」に従う）
- 中断・再開・承認待ちの遷移を削除する: `stop()` / `interrupt()` / `resume()` / `request_approval()` / `approve()` / `reject()` と、それらが前提にする再開可能性の判定を取り除く。根拠: R-002「Workflow 実行の状態を中断中または承認待ちへ変える状態遷移と、そこから再開する状態遷移は存在しない」/ B-002。ルート: 委任
- 中断理由と再開位置を取り除く: `ExecutionInterruptionReason` と `interruption_reason` / `resume_from_node` を domain、usecase DTO、protocol、presenter、CLI 出力から取り除く。根拠: R-003「Workflow 実行の読み取り結果に、中断理由と再開位置の項目が含まれない」/ B-003。ルート: 委任（proto は「固定するルート」に従う）
- proto の削除跡を reserved にする: 取り除いた message field と enum value の番号・名前を再利用できない形にする。根拠: R-004「取り除いた proto の項目と列挙値の番号・名前は、以後別の意味で再利用されない」/ B-004。ルート: 「固定するルート」のとおり
- Workflow 実行状態のファイル永続化一式を削除する: `workflow_executions/*.json` の読み書き・走査・原子的書き込みと `ExecutionMetadataStore` / `metadata_store()` を取り除く。根拠: R-005「Workflow 実行の状態をファイルへ保存し、そこから復元する仕組みは存在しない」/ B-005。ルート: 委任
- `ExecutionStore` の未呼び出しメソッドを削除する: 根拠: R-006「`ExecutionStore` に、本番の実行経路から呼ばれないメソッドは存在しない」/ B-006。ルート: 「固定するルート」のとおり実装時点で呼び出し元を再確認してから削除する
- `recovery_effect_suppression` を削除する: 宣言・生成・参照を取り除く。根拠: R-007「値が書き込まれることのない実行時状態は存在しない」/ B-006。ルート: 委任
- `workflow_execution_logs/` を現在の実装として説明するコメントを削除する: 根拠: R-008「読み書きするコードが存在しない記録先を、コメントが現在の実装として説明していない」/ B-007。ルート: 委任
- CLI ガイドから本番で出力されない状態値と項目を削除する: `docs/guide/cli.md` の `releash workflow status` の節から `waiting_approval` / `interrupted` / `interruptionReason` / `resumeFromNode` を取り除く。根拠: R-009「CLI ガイドに、本番で出力されない Workflow 実行の状態値と項目は載っていない」/ B-008。ルート: 委任

## 固定するルート

- proto から項目を取り除くときは、protobuf の規約に則り、番号と名前をともに `reserved` にする。範囲: `proto/client.proto` の削除対象すべて。message field（`WorkflowExecutionView` の `interruption_reason = 11` / `resume_from_node = 12`、`WorkflowExecutionSummaryDto` の同 2 件）に加え、enum value（`ExecutionStatusView.Value` と `ExecutionStatusDto.Value` の `waiting_approval = 1` / `interrupted = 4`、`WorkspaceHistoryStatus.Value` の `waiting_approval = 7` / `interrupted = 8`）も対象にする。粒度: 既存例（`proto/client.proto:1885-1886` の `WorkspaceNodeCapabilitiesDto`、`:3575-3576` の `ServerInfo`）と同じく `reserved <番号>;` と `reserved "<名前>";` を置く。message そのもの（`ExecutionInterruptionReasonView`、`ExecutionInterruptionReasonDto`）は field 番号を持たないため `reserved` の対象外で、定義ごと削除する。enum value の番号は詰め直さない。理由: 取り除いた番号と名前が別の意味で再利用されるのを防ぐ。
- 削除対象のメソッドは、実装時点で呼び出し元を再確認してから削除する。範囲: Issue 本文および `requirements.md` の Current Behavior が挙げる `ExecutionStore` の未呼び出しメソッド一覧。理由: 一覧は調査時点（2026-09-21、`4267f22f`）のものであり、Issue 自身が実装時の再確認を指示している。`shutdown_all_active_commands` のように、本文の「本番の呼び出し元が無い」という前提が現状と食い違う例が実際にある。
- `shutdown_all_active_commands` と `command_shutdown_intents` には手を入れない。範囲: `workflow_host.rs:124` の宣言、`:453` の生成、`:1775-1786` の読み取り、`:1986` と `:2028-2064` の書き込み・削除、`:173` の `ActiveCommandShutdownIntent`、および `runtime_command_gateway.rs:307` からの呼び出し経路。理由: アプリケーション終了処理から本番経路で呼ばれるため。

## 変えないもの

- `shutdown_all_active_commands`（`src-tauri/src/adaptor/gateway/workflow/workflow_host.rs:2014`）と `command_shutdown_intents`（同 `:124`）。範囲: 宣言、生成、書き込み・削除、`workflow_host.rs:1775-1786` の読み取りを含む一式。理由: アプリケーション終了処理から本番経路で呼ばれており、本 ISSUE の削除基準（本番に到達しない実装）に当たらない。削除すると「終了時に workflow が起動した command を止める」振る舞いが失われ、R-010 と矛盾する
- 本番の挙動。範囲: Workflow の起動・実行・完了・Abort。出力から消えてよいのは、本番で常に `null` だった `interruptionReason` / `resumeFromNode` と、本番で到達しなかった状態値 `waiting_approval` / `interrupted` だけである。理由: 本 ISSUE は到達しない実装の削除であり、振る舞いの変更ではない
- NodeExecution が所有する状態（`WaitingApproval` / `Paused` / `Failed` など）と、利用者の Stop / Resume 操作（`stop_workflow` / `resume_workflow`）。理由: `docs/glossary/DOMAIN.md` の状態所有に従い Workflow 実行の状態だけを対象にする。操作そのものの整理はマイルストーン #90 の #1839 が扱う

## 未確定・リスク

- enum value を `reserved` にした proto を `pnpm generate:protocol`（protoc + protoc-gen-es + `scripts/generate-client-protocol.mjs`）が期待どおり扱うかは未検証である。リポジトリ内の `reserved` の既存例は message field だけで、enum value の前例が無い。生成 TypeScript 型から `waiting_approval` / `interrupted` / `interruptionReason` / `resumeFromNode` が落ちない場合、R-001 と R-003 を満たせない
