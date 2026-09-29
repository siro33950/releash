# Design 04

## 開始状態

`feat/issues/1958` の `53c24e7f`（design-03 の実装を確定したコミット）。直前の Design は `design-03.md`。この周までに閉じた Thread は b343f48f-9c0e-4055-9f7a-6c0fb6ff6340（invalid）、a68e7bab-b239-46c1-917c-7a8faa15fc2e（invalid）と、前の周で直ったと確認された Thread。

## 変える部分

- agent の Stop を Node の状態に関係なく agent の activity に反映する。完了済みの Session Node に provider の Stop が届いたとき、Node の status と completion_signals は変えずに、その Session の activity を AwaitingInstruction にする。根拠: R-002「Stop した状態は緑」、B-016、Thread b6a88e87-748e-479b-a9b8-014b26ecb1af。ルート: 単独 Session と完了済みの Workflow Session Node に同じ規則を使い、単独 Session 専用の分岐は足さない（design-02 の固定するルート）。事実の記録と再生の方法は委任。
- 完了済みの紐づき済み Session Node で AwaitingAnswer かつプロセスが消えているとき、緑になることのテストを追加する。根拠: R-002、B-017、Thread 0704ec1a-0d04-406f-93e5-87b2900e2d1d。ルート: 委任。
- `fact_replay_test.rs` で単独 Session の初期の事実を FactLog へ積む手順を、テスト用の関数1か所にまとめる。根拠: Thread 601d7b90-3126-4add-8c49-4fc3ee06afb9。ルート: 委任。
- 完了済みの Workflow Session Node と単独 Session の Node の比較テストを、どちらも実行中の木にある完了済み Node という前提にそろえる。そのうえで、Stop・Submit の結果、`admit_node_submit` の結果、status と completion_signals が変わらないことを比べる。根拠: R-005、Thread 14539619-7106-4b78-a6a5-493aa86ac949。ルート: 委任。
- `complete_standalone_session_node` の拒否のテストを、拒否の条件ごとの関数に分ける。表駆動のテストは、操作を When に、検証を Then に置く。根拠: Thread bb612462-b9c7-4a81-b8b9-481697dcc64f。ルート: 委任。
- 存在しない Node の拒否のテストで、木の Node 一覧全体が前後で変わらないことを確かめる。根拠: B-015、Thread 664f2773-2c2b-4b88-b154-ed4e545c98e1。ルート: 委任。

## 固定するルート

- 事実から表示状態を導出する処理を domain の一つの関数に置くことは、design-01 から維持する。
- 単独 Session は Workflow の Session Node と同じものとして扱い、違いは Node を完了した状態で起動することだけにする。これは design-02 から維持する。

## 変えないもの

- すでに記録されている Workflow の外の Session で、Node が Submit を待ったまま完了していないものは移行しない（design-01 から維持）。
- main にもとからある単独 Session 専用の分岐は変えない（design-02 から維持）。

## 未確定・リスク

なし
