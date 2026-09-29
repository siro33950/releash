# Design 05

## 開始状態

- 差分の基準は `feat/issues/1954` の `60ee249a`。design-04 の変える部分が実装され、その周のレビューが済んだ状態である。
- 直前の Design は `design-04.md`。
- この周までに閉じた Thread は `dacf2a01`（`repository_state` の 2 ファイルに同じ形で残っていたテスト用ヘルパーの重複）。design-04 の実装で直って閉じた。

## 変える部分

- テスト用ヘルパーが持つ購読 Usecase: `CapturingNotifier` から `StateSubscriptionUsecase` の保持を無くし、利用側が自分で作ったものを渡す形にする。`Default` 実装をやめ、Repository 用も `worktrees` と同じ形の構築にする。根拠: Thread `f27a91e2`。ルート: 委任
- ロックの失敗の書き方: `CapturingNotifier` の `lock()` と `take()` で、ロックの失敗に対する書き方を揃える。根拠: Thread `c60db53b`。ルート: 委任

## 固定するルート

design-01 の「固定するルート」をすべて維持する。この周で新しく固定する実装上の指定は無い。

## 変えないもの

- design-01・design-02・design-03・design-04 の「変えないもの」。

## 未確定・リスク

なし。
