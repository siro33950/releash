# Design 05

## 開始状態

`feat/issues/1958` の `de770b38`（design-04 の実装を確定したコミット）。直前の Design は `design-04.md`。この周までに閉じた Thread は、b343f48f-9c0e-4055-9f7a-6c0fb6ff6340（invalid）、a68e7bab-b239-46c1-917c-7a8faa15fc2e（invalid）、8c60d229-712c-4446-af55-f815b6a82d67（invalid）と、前の周までに直ったと確認された Thread。

## 変える部分

- 実行中でない Node に対して `record_provider_stop` が AlreadyApplied を返す変更を戻す。「agent の Stop を Node は変えずに activity としてだけ記録する」かどうかは、domain が明示した判定として返す。usecase は AlreadyApplied を読み替えず、この判定で事実を記録する。根拠: R-002「Stop した状態は緑」、Thread 9a0bf3b0-aa08-435d-89f6-bf8988f39242。ルート: 判定の値の形は委任。単独 Session 専用の分岐は足さない（design-02 の固定するルートを維持）。
- 完了済みの Session Node と中断した Session Node に provider の Stop が届いたとき、Node は変わらず、activity が AwaitingInstruction になることのテストを置く。根拠: R-002、B-011、B-016、Thread 6be6ca0a-99b7-4c57-94f5-ea4b3abbbc6c。ルート: 委任。
- provider の Stop から Workspace ツリーの行までを通すテストを置く。完了済みの Workflow Session Node（木は実行中）と完了済みの単独 Session の Node の両方で、プロセスが Live のまま行が idle になることを確かめる。比較テストでは Stop の後に Submit を追記しない。根拠: R-002、R-005、B-016、Thread b6a88e87-748e-479b-a9b8-014b26ecb1af。ルート: Stop は control_plane の `record_provider_stop` で届ける。
- reconciliation のテストに、Stop の後の activity が AwaitingInstruction であることの確認を足す。根拠: Thread 1f7cc952-7610-4e67-acd7-cfda2a0199a4。ルート: 委任。
- 新しく追加したテストの名前を、業務機能が日本語の `test_{業務機能}_{条件と期待結果}` の形にする。根拠: Thread e8969ce5-13b0-4ddb-956c-ce2bbb6a6c62。ルート: 委任。
- `fact_replay_test.rs` のテスト用関数から、bool 引数と事実の個数での切り出しをなくす。前提ごとに名前の付いた関数に分け、旧記録は `StandaloneSessionNodeCompleted` を事実の種類で除いて作る。根拠: Thread 6325a87f-0057-4b76-b144-1c4cd2333d6b。ルート: 委任。
- `value_objects/mod_test.rs` の表駆動のテストを、ケースごとのテスト関数に分ける。Then では assert だけを行う。根拠: Thread bb612462-b9c7-4a81-b8b9-481697dcc64f。ルート: 委任。

## 固定するルート

- 事実から表示状態を導出する処理を domain の一つの関数に置くことは、design-01 から維持する。
- 単独 Session は Workflow の Session Node と同じものとして扱い、違いは Node を完了した状態で起動することだけにする。これは design-02 から維持する。

## 変えないもの

- すでに記録されている Workflow の外の Session で、Node が Submit を待ったまま完了していないものは移行しない（design-01 から維持）。
- main にもとからある単独 Session 専用の分岐は変えない（design-02 から維持）。

## 未確定・リスク

なし
