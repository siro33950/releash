# Design 06

## 開始状態

`feat/issues/1958` の `3cb1198d`（design-05 の実装を確定したコミット）。直前の Design は `design-05.md`。この周までに閉じた Thread は、b343f48f-9c0e-4055-9f7a-6c0fb6ff6340（invalid）、a68e7bab-b239-46c1-917c-7a8faa15fc2e（invalid）、8c60d229-712c-4446-af55-f815b6a82d67（invalid）と、前の周までに直ったと確認された Thread。

## 変える部分

- `ProviderStopDecision` から、`TransitionOutcome` と同じ意味の変種をなくす。`record_provider_stop` は、Session の所有を確かめたうえで agent の Stop の受理を Ok で明示し、Node の完了シグナルとしての結果は `TransitionOutcome` のまま持たせる。実行中でない Node では、その結果を NotApplicable にする。usecase は、Ok なら NodeStopReceived を記録し、完了の handshake は Node の完了シグナルが Applied のときだけ行い、commit にはその結果をそのまま渡す。domain の判定を別の `TransitionOutcome` に読み替えない。根拠: R-002、Thread c59bc7c0-ccbe-4d06-9b20-29fe3da40cf0、Thread 2f1f3bbc-85fd-490b-9bc8-44e29accc7fa。ルート: 型の形は委任。単独 Session 専用の分岐は足さない（design-02 の固定するルートを維持）。
- 完了済みの Session Node への Stop が、Ok で受理され、Node の完了シグナルとしては NotApplicable になり、Node が変わらないことの domain のテストを置く。根拠: Thread 9b4cea6c-b708-4a3f-b704-cb609d115a16。ルート: 委任。
- 中断済み Session への Stop のテストで、Stop の前に activity が Working であることを確かめてから、Stop の後に AwaitingInstruction になることを確かめる。根拠: B-011、Thread 26ca7441-b9e0-4bd5-a3eb-e134de885652。ルート: 委任。
- `workflow_host.rs` の inline モジュールに置いた provider Stop から行までのテストを、`workflow_host_test.rs` へ移す。根拠: Thread 51cc2806-fe22-490b-982f-df7e0fd8e7db。ルート: 委任。
- そのテストで、Stop の前後に永続化された木を fold し、両 Node の status と completion_signals が変わらないことを確かめる。根拠: R-002、R-005、B-016、Thread b6a88e87-748e-479b-a9b8-014b26ecb1af。ルート: Stop は control_plane の `record_provider_stop` で届ける（design-05 から維持）。

## 固定するルート

- 事実から表示状態を導出する処理を domain の一つの関数に置くことは、design-01 から維持する。
- 単独 Session は Workflow の Session Node と同じものとして扱い、違いは Node を完了した状態で起動することだけにする。これは design-02 から維持する。

## 変えないもの

- すでに記録されている Workflow の外の Session で、Node が Submit を待ったまま完了していないものは移行しない（design-01 から維持）。
- main にもとからある単独 Session 専用の分岐は変えない（design-02 から維持）。

## 未確定・リスク

なし
