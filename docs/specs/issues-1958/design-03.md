# Design 03

## 開始状態

`feat/issues/1958` の `39b413f0`（design-02 の実装を確定したコミット）。直前の Design は `design-02.md`。この周までに閉じた Thread は b343f48f-9c0e-4055-9f7a-6c0fb6ff6340（invalid）と a68e7bab-b239-46c1-917c-7a8faa15fc2e（invalid）。

## 変える部分

- `value_objects/mod.rs` のテストモジュールの `#[path]` 取り込みをファイル末尾へ移す。根拠: Thread 0d55f010-b471-4fb9-9d22-cbd62270d774。ルート: 委任。
- 中断した紐づき済み Session Node で、Session が Stop した状態、子がないときに緑になることのテストを追加する。根拠: B-011、Thread 4c53db61-6a48-4f85-b6b6-94d2628d6ac3。ルート: 委任。
- 実行中の紐づき済み Session Node で、Working なら青、AwaitingAnswer なら黄になることのテストを、規則ごとに追加する。根拠: B-002、B-003、Thread 27764e59-2d5d-4e3c-987d-ade448029822。ルート: 委任。
- 今回追加・変更した Rust テストを、規則ごとのテスト関数に分け、Given・When・Then を別々のコメントで区切る。表駆動を残す場合は、各 assert にケース名を付ける。根拠: Thread bb612462-b9c7-4a81-b8b9-481697dcc64f。ルート: 対象は `value_objects/mod_test.rs`、`workflow_execution/mod_test.rs`、`fact_replay_test.rs` の今回のテスト。
- 完了済みの Session Node で、記録上の activity が Working のままプロセスが消えたら緑になり、実行中の Session Node なら黄になることのテストを置く。単独 Session だけのテストは作らない。根拠: R-002、B-017、Thread a7c34c3a-a037-4cc8-b90e-d49a63bddad0。ルート: 委任。
- 完了済みの Workflow Session Node と完了済みの単独 Session の Node に同じ Stop と Submit を与え、結果が一致することを比べるテストを追加する。根拠: R-005、Thread 14539619-7106-4b78-a6a5-493aa86ac949。ルート: 委任。
- `complete_standalone_session_node` が拒否するケースで、対象 Node の status と completed_at が変わらないことを確かめる。根拠: B-015、Thread 664f2773-2c2b-4b88-b154-ed4e545c98e1。ルート: 委任。
- Delegate の親の行の色を、事実を LocalEventStore に書き込み、repository の読取経路（`fold_tree_from` による復元を含む）で確かめるテストを追加する。子が Working と AwaitingAnswer の場合を分ける。根拠: B-012、B-013、Thread 180cecff-54e9-4a99-8903-6a51e111fffb。ルート: ExecutionTree や FoldedTree をメモリ上で直接組み立てない。
- Thread 8183357d-ce22-4b29-bfba-eb4f7ee03149 は改名が実装済みで、確認の条件だけを直した。コードの変更はない。根拠: Thread 8183357d-ce22-4b29-bfba-eb4f7ee03149。ルート: 委任。

## 固定するルート

- 事実から表示状態を導出する処理を domain の一つの関数に置くことは、design-01 から維持する。
- 単独 Session は Workflow の Session Node と同じものとして扱い、違いは Node を完了した状態で起動することだけにする。これは design-02 から維持する。

## 変えないもの

- すでに記録されている Workflow の外の Session で、Node が Submit を待ったまま完了していないものは移行しない（design-01 から維持）。
- main にもとからある単独 Session 専用の分岐は変えない（design-02 から維持）。

## 未確定・リスク

なし
