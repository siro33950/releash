# Design 04

## 開始状態

- 差分の基準は `feat/issues/1954` の `ea1e69a4`。design-03 の変える部分が実装され、その周のレビューが済んだ状態である。
- 直前の Design は `design-03.md`。
- この周までに閉じた Thread は次の 8 件。いずれも design-03 の実装で直って閉じた。`77149a6b`（待機対象以外への配信のテスト）、`556a8ca9`（並行の追加・削除のテスト）、`0a08ef03`（購読 worker の対象専用の仕組み）、`a772d4d5`（配信の組み立ての集約）、`82743209`（`usecase/retry.rs` の adaptor への参照）、`9cd286f4`（Repository 一覧の通知の集約）、`1f520953`（`WorkFailure::from_error` のテスト）、`ecc66a0c`（テストの Given / When / Then）。

## 変える部分

- テスト用ヘルパーの重複解消: `repository_state` の 2 ファイルに同じ形で残っている `CapturingNotifier` を `test_support` の 1 つの実装にまとめ、`RecordedWorktrees` の利用側も含めてそれを使う。`service_test.rs` に残る受信ループも `take_changes` に寄せる。根拠: Thread `dacf2a01`。ルート: 委任

## 固定するルート

design-01 の「固定するルート」をすべて維持する。この周で新しく固定する実装上の指定は無い。

## 変えないもの

- design-01・design-02・design-03 の「変えないもの」。

## 未確定・リスク

なし。
