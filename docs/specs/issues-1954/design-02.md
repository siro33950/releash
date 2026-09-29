# Design 02

## 開始状態

- 差分の基準は `feat/issues/1954` の `fa6107fe`。design-01 の変える部分が実装され、その周のレビューが済んだ状態である。
- 直前の Design は `design-01.md`。
- この周までに閉じた Thread は次の 2 件。いずれも `invalid` で閉じた。
  - `d4248b45`: `protected_targets` の `RepositoryPaths` の保持にテストが無いという指摘。`adaptor/controller/api/client_test.rs` の `test_状態購読_connectで初期状態と変更と再開を配信する` が、唯一の購読を停止した後の旧 cursor での再開を検証しており、根拠が成り立たない。
  - `d05fd56c`: Output Data が domain の `FailureRecord` を持つことが規約に反するという指摘。規約が禁じているのは Entity の参照であり、`FailureRecord` は Entity ではないため成り立たない。

## 変える部分

- 購読 worker の対象専用の仕組み: worker から `RepositoryPaths` 専用の受信口と分岐を無くし、配信の完了を待つ仕組みを対象を問わない形にする。同じ変更が、1 回の変更に対する読み直しと配信を 1 回にする。根拠: Thread `0a08ef03`、`6a5180d7`。ルート: 委任
- agent session の通知の集約: `AgentSessionLifecycleUsecase` の 6 か所の通知の組み立てを private なヘルパーに集約する。根拠: Thread `b6c9351a`。ルート: 委任
- 失敗の記録のテスト用の仕掛けの削除: domain の trait の `as_any` と、usecase から adaptor の具体型を返すテスト用の API を無くす。根拠: Thread `c3bcd655`。ルート: 委任
- テスト用ヘルパーの重複解消: `StateChangeSource` を集めるテスト用の実装を 1 つにする。根拠: Thread `dacf2a01`。ルート: 委任
- 並行の追加・削除のテスト: `RepoPathsUsecase` の add と remove を並行に開始したときの配信の順序を検証するテストを足す。根拠: Thread `556a8ca9`。ルート: 委任
- domain の失敗のテスト: `domain/failure.rs` へ移したロジックのテストを足す。根拠: Thread `1f520953`。ルート: 委任
- テストの構造: この変更で追加したテストに Given / When / Then の区切りを入れる。根拠: Thread `ecc66a0c`。ルート: 委任
- use の位置: `usecase/failure_test.rs` の use をファイル冒頭にまとめる。根拠: Thread `c68e5d70`。ルート: 委任

## 固定するルート

design-01 の「固定するルート」をすべて維持する。この周で新しく固定する実装上の指定は無い。

## 変えないもの

- `FailureRecord` を domain に置くこと。design-01 で固定したルートであり、Thread `d05fd56c` の指摘が成り立たないため変えない。
- `protected_targets` が `RepositoryPaths` を常に保持対象に含めること。旧 cursor での再開を従来どおり Change で返すために要る。

## 未確定・リスク

- 配信の完了を待つ仕組みを作り直すため、R-008 が求める「購読している画面が受け取る内容と、受け取る時点は、この変更の前と同じ」が崩れうる。特に、追加の直後に削除した場合に追加後の一覧を取りこぼす経路が戻りうる。
