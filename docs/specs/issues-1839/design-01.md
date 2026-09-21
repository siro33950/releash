# Design 01

## 開始状態

差分の基準は base branch `main` の `7948c6ec fix(review): backend再接続後の差分更新停止を修正 (#1856)`、作業 branch は `feat/issues/1839`。未コミットの変更は `docs/specs/issues-1839/` の requirements.md と behavior.md だけで、コードは未変更である。

`docs/specs/issues-1839/` に既存の `design-NN.md` は無く、初回の周である。実装の状態は `requirements.md` の Current Behavior を参照する。この周までに解消・見送りとなった Thread は無い。

## 変える部分

- 実行木の Stop / Resume の入口の削除: 利用者から実行木を止める操作と再開する操作を、画面・local API・Connect のどの入口からも取り除く。根拠: R-001「実行木に対して利用者が行える、実行の状態を変える操作は Abort だけである」、B-001。ルート: 固定（「固定するルート」2）
- 取り除いた proto の番号と名前の reserved 化: 消した RPC、message、oneof の項目を `reserved` にする。`command_request`（`proto/client.proto:132`・`:144`）と `command_result`（`:305`・`:317`）の双方に番号 123 / 137 がある。根拠: R-012、B-014。ルート: 固定（「固定するルート」2）
- Stop / Resume の usecase と gateway の削除: 入口が無くなる Stop / Resume の実行経路一式を取り除く。根拠: R-001。ルート: 固定（「固定するルート」3）
- Abort の worktree フォルダ存在確認の削除: Abort の入口が対象の worktree のフォルダの存在を確かめる処理を外し、フォルダが無くても Abort できるようにする。根拠: R-002、B-002。ルート: 委任
- Session Node の Resume の追加: 終わっていない Session Node に、プロセスが居ないときの Resume を持たせる。根拠: R-003、B-005 / B-018。ルート: 委任（在否の確かめ方は「固定するルート」1 の materialize 先までが固定で、それ以外は委任）
- 会話も作業場所も無い Session の Resume の起動経路: 再開できる会話も実行できる作業場所も無い場合の Resume を、新しい attempt での起動として振る舞わせる。根拠: R-003「その場合の Resume は新しい attempt での起動として振る舞う」、B-003 / B-004。ルート: 委任
- Session Node の手動 Retry の削除: 利用者が Session Node を手動で Retry する操作を取り除く。根拠: R-004、B-006。ルート: 固定（「固定するルート」5）
- Retry の条件からの完了信号の偏りの削除: 「Submit と provider Stop の片方だけ届いている」ことを理由に Retry できる経路を取り除く。根拠: R-011、B-013。ルート: 固定（「固定するルート」5）
- 再開した Session への自動指示の削除: Resume した Session へ Releash が会話の続きを促す指示を送る処理を取り除く。根拠: R-005、B-005。ルート: 委任
- 未注入の child の結果を渡す契機の移動: 親 Session へまだ渡していない child の結果を渡す処理を、Workflow の Resume から Session Node の Resume の契機へ移す。根拠: R-006、B-007。ルート: 委任
- Command Node の Retry の条件の変更: Command Node の Retry を「プロセスが居ないとき」に出す条件へ変える。根拠: R-007、B-009 / B-018。ルート: 委任
- Paused の Command を起動し直す専用経路の削除: Command 専用の再起動経路を取り除く。根拠: R-007 / R-008。ルート: 固定（「固定するルート」4）
- Node の状態からの Paused と Failed の削除: 中断して再開を待つ値と失敗を表す値、およびそれらを作る・戻す遷移と `failure` / `failure_kind` / `origin` の保持を取り除き、終わっていない Node を実行中のままにする。根拠: R-008、B-010 / B-012。ルート: 固定（「固定するルート」7、削除範囲まで）。削除後の型・関数の再構成は委任
- プロセスの在否の判定の追加: Command に「プロセスが居るか」の判定を足し、Session と Command の双方について、状態とは別の 1 項目として外部から読めるようにする。根拠: R-009、B-011。ルート: 固定（「固定するルート」1）
- プロセスが居ない Node の状態分類の変更: プロセスが居ない Node を、実行木の状態分類として利用者の介入を待つものとして読めるようにする。根拠: R-016、B-019。ルート: 委任
- 読み取り側の Stop / Resume 判定の削除: `can_stop` / `can_resume` / `resume_eligible` とその再計算、および各層の型を取り除く。根拠: R-001。ルート: 固定（「固定するルート」6）
- workflow 定義からの `on_failure` の削除: child が失敗したときの扱いを宣言する構文と、その評価・Diagnostic を取り除き、宣言した定義を定義の誤りとして拒否する。根拠: R-013、B-015。ルート: 固定（「固定するルート」7）。取り除く Diagnostic コードの番号のその後の扱いは委任
- 起動失敗時の自動的な起動し直しの追加: Session Node と Command Node の起動が失敗した場合に、規定回数まで自動で起動し直す。起動し直しは 1 回ごとに新しい attempt を作り、間隔は回を追うごとに長くする。使い切った Node は実行中かつプロセスが居ない状態で利用者の操作を待つ。根拠: R-015、B-017 / B-018。ルート: 固定（「固定するルート」8、回数と間隔の性質まで）。待機時間の具体値は委任
- 正本とガイドの記述の更新: 利用者の操作・Node の状態・workflow 定義の構文の記述を、変更後に実際に行える操作、読み取れる状態、書ける定義に合わせる。`docs/glossary/DOMAIN.md:112` の NodeExecution が所有する状態の列挙は、変更後に実在する状態に合わせる。根拠: R-014、B-016。ルート: 固定（「固定するルート」9）
- 対応するテストの更新: 上記の削除・変更に対応するテストを同じ範囲で扱う。根拠: 上記各変更。ルート: 固定（「固定するルート」10）

## 固定するルート

1. 「プロセスが居るか」の判定は、Session は既存の `ManagedPtyPresence`（`src-tauri/src/domain/agent_session/aggregates/agent_session.rs:248-252`）、Command は実行中の command を載せるメモリ上の表 `active_commands`（`src-tauri/src/adaptor/gateway/workflow/workflow_host.rs:117`）への在籍で行う。粒度は判定の materialize 先の指定までとする。理由は、Session 側に既にある判定を Command へ広げるため。関係: R-009 / B-011
2. Stop / Resume の入口は、画面のメニュー項目（`src/components/workspace/WorkspaceList.tsx:385-402`、`src/lib/workflowExecutionActions.ts`）、画面からのコマンド（`src-tauri/src/adaptor/controller/client/workflow/shared.rs:669-692`・`:784-808`、`runtime.rs:57-85`）、local API の 2 ルート（`src-tauri/src/adaptor/controller/api/workflow.rs:79-86`）、proto の `StopWorkflow` / `ResumeWorkflow` まで含めて消す。proto の番号と名前は `reserved` にする。関係: R-001 / R-012、B-001 / B-014
3. usecase と gateway は、`stop_execution.rs` / `resume_execution.rs`、`ports.rs:150-158` の 2 つの port、`runtime_command_gateway.rs:260-277`、`lifecycle_commands.rs` の Stop・Resume・失敗時に元へ戻す処理一式を消す。`stop_execution.rs` / `resume_execution.rs` の実際の配置は `src-tauri/src/usecase/workflow/command/` 配下であり、ISSUE の記述位置（`usecase/workflow/` 直下）とは異なる。関係: R-001
4. Paused の Command を起動し直す専用経路（`restart_paused_command_node`、`NodeRestartMode::CommandResume`、`can_restart_paused_command`）を消す。関係: R-007 / R-008
5. Session の手動 Retry と、Retry の条件のうち「Submit / Stop の片方だけ届いている」を消す。関係: R-004 / R-011、B-006 / B-013
6. 読み取り側の `can_stop` / `can_resume` / `resume_eligible` と再計算（`src-tauri/src/domain/workspace_tree/entities/mod.rs:632-688`）、および DTO・proto・生成 TypeScript・手書き TypeScript の各層を消す。関係: R-001
7. Node の状態から失敗を表す値を、workflow 定義から `on_failure` を廃止する。削除範囲は `NodeExecutionStatus::Failed` と `failure` / `failure_kind` / `origin`、`OnFailure` と `apply_on_failure_treatment` / `auto_retry_budget_left` / fanout の `ignore` 集約、Diagnostic の `WFC009` / `WFC010` と `WFS008` の `on_failure` 部分。関係: R-008 / R-013、B-010 / B-012 / B-015
8. 自動再起動の回数は 4 回とし、待機時間は試行ごとに長くする。理由は、既存の `CONTROL_PLANE_MAX_ATTEMPTS = 4`（`src-tauri/src/usecase/workflow/command/mod.rs:25`）と同じ回数にしたうえで、Node の起動し直しは worktree 生成や provider 起動を伴うため即時再試行にしないこと。関係: R-015 / B-017
9. 更新する正本とガイドは、`docs/glossary/DOMAIN.md:112`・`:126`、`docs/glossary/WORKFLOW.md:11`・`:168-171`・`:197`・`:325`、`docs/guide/workflow/concepts.md:189-197`、および `docs/guide/workflow/yaml.md`・`common.md`・`lua.md` の `on_failure` の構文説明と Diagnostic の表。関係: R-014 / B-016
10. 対応するテストを削除範囲に含める。ISSUE の調査時点の件数は Rust 約 50 本、フロントエンド約 5 本、E2E 1 本だが、現時点の実数は未確認である

## 変えないもの

- Command が 0 以外の終了コードで終わったときに `ok: false` の Artifact を出して完了する振る舞いと、`ok` を宣言なしの routing field として使う設計。理由は、非ゼロ exit をやり直すかどうかを定義の辺が決める設計を崩さないため（R-017 / B-020）
- 新しい attempt を作る仕組み（`restart_node_attempt_at` の採番と記録形式）。手動 Retry も自動再起動もこの仕組みを使う
- AgentSession の `open` / `paused` / `archived` の lifecycle と、その遷移・記録。別の集約が所有するため
- 事実ログの記録先と形式。起動・runtime の失敗は事実として記録され続け、状態を導出しなくなるだけである
- Retry が worktree のフォルダを必要とする性質。Retry はそのフォルダでプロセスを起動し直すため。Abort だけがフォルダの存在確認を外す
- Archive を 1 つの操作にし内側で Abort を使う変更と、消えた worktree の片付け（#1826 が扱う）、および起動時に「プロセスを失った」を記録する処理の置き換え（#1836 が扱う）
- Approve と Submit の操作

## 未確定・リスク

なし。
