# Design 02

## 開始状態

`feat/issues/1958` の `cea1e58c`（design-01 の実装を確定したコミット）。直前の Design は `design-01.md`。この周までに閉じた Thread は b343f48f-9c0e-4055-9f7a-6c0fb6ff6340（invalid）と a68e7bab-b239-46c1-917c-7a8faa15fc2e（invalid）。

## 変える部分

- Session が出す色を agent の状態だけで決める: Node の完了と単独 Session root の判定を外す。agent のプロセスが消えていれば、動作中・回答待ちとはみなさず緑にする。根拠: R-002「Session が出す色は、Node の状態に関係なく、agent の状態だけで決まる」、B-017、Thread a7c34c3a-a037-4cc8-b90e-d49a63bddad0。ルート: 委任。
- 今回の変更で足した単独 Session 専用の分岐を、Node を完了状態で起動する処理（`complete_standalone_session_node` とその事実）を除いて削除し、単独 Session の完了済み Node を完了済みの Workflow Session Node と同じ規則で動かす。削除対象は、`record_completion_signal` の `standalone_session` 引数と完了済み Node での Stop・Submit の受付、`record_provider_stop` と `admit_node_submit` での受付、`complete` での AlreadyApplied の許容、handshake の完了済み単独 Session の CompleteAuto 分岐、`newly_terminal_sessions_since` の単独 Session の木の終了の条件。根拠: R-005（単独 Session の Node は完了した状態で起動し、何も待たない）、Thread 14539619-7106-4b78-a6a5-493aa86ac949。ルート: 単独 Session は Workflow の Session Node と同じものとして扱い、違いは Node を完了状態で起動することだけにする。
- `classify_status` の Node が出す色の条件式を整理する: 紐づき済み Session の判定を1か所にし、「プロセスが消えた」と「agent が止まっていて Submit がない」を分ける。根拠: Thread d06d0010-9c15-47ae-a955-4d406c931992。ルート: 委任。
- 今回追加・変更した Rust テストに Given / When / Then の区切りを入れ、表示状態分類のテストを規則ごと、またはケース名付きに分ける。根拠: Thread bb612462-b9c7-4a81-b8b9-481697dcc64f。ルート: 委任。
- 今回追加した domain テストを `<impl>_test.rs` に置く。根拠: Thread d93c34f4-4673-44e3-86c6-59901398fdfb。ルート: workflow_execution は既存の `mod_test.rs`、projection と value_objects は `_test.rs` を新設して `#[path]` で取り込む。既存の inline テストは移さない。
- テスト名とコメントを3分類の期待値と命名規約に合わせる。根拠: Thread 0dc7861a-2e7f-472c-ab0b-48e009043525。ルート: 委任。
- 単独 Session 用の完了の事実が、対象外の木・Node では拒否されることのテストを追加する。根拠: B-015、Thread 664f2773-2c2b-4b88-b154-ed4e545c98e1。ルート: 委任。
- Session Node の詳細のヘッダーに状態アイコンがないことのテストを、detail を設定してヘッダーを描画した状態で検証する形に直す。根拠: B-019、Thread bf2ca0d5-4891-44a7-ab35-fa792b122f74。ルート: 委任。
- Delegate の親の行の色（B-012、B-013）を、repository の読取経路で確認するテストを追加する。根拠: B-012、B-013、Thread 180cecff-54e9-4a99-8903-6a51e111fffb。ルート: 委任。
- `src/components/workspace/WorkflowNodeStatusIcon.tsx` を中身（色の対応表と pulse の判定）に合う名前に改名する。根拠: Thread 8183357d-ce22-4b29-bfba-eb4f7ee03149。ルート: 委任。

## 固定するルート

- 事実から表示状態を導出する処理を domain の一つの関数に置くことは、design-01 から維持する。
- 単独 Session は Workflow の Session Node と同じものとして扱う。違いは Node を完了した状態で起動することだけで、それ以外に単独 Session 専用の分岐を足さない。

## 変えないもの

- すでに記録されている Workflow の外の Session で、Node が Submit を待ったまま完了していないものは移行しない（design-01 から維持）。
- main にもとからある単独 Session 専用の分岐（`workflow.is_none()` や `launched_as == Session` を条件にしたもの、agent の Session 側の archive・削除・GC の扱い）は変えない。この開発では、今回の変更で足した分岐だけを直すと人が決めたため。

## 未確定・リスク

なし
