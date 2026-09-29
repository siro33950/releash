# Design 03

## 開始状態

- 差分の基準は `feat/issues/1954` の `add7d56e`。design-02 の変える部分が実装され、その周のレビューが済んだ状態である。
- 直前の Design は `design-02.md`。
- この周までに閉じた Thread は次のとおり。
  - design-02 の実装で直って閉じたもの: `6a5180d7`（1 回の変更で読み直しと配信が 2 回走る）、`c3bcd655`（domain の trait の `as_any` と usecase から adaptor の型を返すテスト用 API）、`b6c9351a`（agent session の通知の組み立ての重複）、`c68e5d70`（use の位置）。
  - `8d0a75bb`: `RecordedWorktrees::lock` に受信ループが残るという指摘。同じ問題の `dacf2a01` を残し、`duplicate` で閉じた。

## 変える部分

- 購読 worker の対象専用の仕組み: `RepositoryPaths` 専用のフィールドと select! の分岐が無い状態を保つ。前の周で `waiting_workers` に置き換わっており、判定の条件を Thread の範囲に合わせ直した。根拠: Thread `0a08ef03`。ルート: 委任
- 配信の組み立ての集約: `notify` と `notify_and_wait` が別々に持つ `StateChange` の組み立てと送信を 1 か所にする。根拠: Thread `a772d4d5`。ルート: 委任
- Repository 一覧の通知の集約: `RepoPathsUsecase` の `add` と `remove` が持つ同じ通知の組み立てを private なヘルパーにまとめる。根拠: Thread `9cd286f4`。ルート: 委任
- 失敗の記録のテスト用の観測: `usecase/retry.rs` から adaptor の具体型への参照を無くす。根拠: Thread `82743209`。ルート: 委任
- テスト用ヘルパーの重複解消: broadcast から `StateChangeSource` を取り出す受信ループを `test_support` の 1 つにし、`repository_state` の 2 ファイルと `RecordedWorktrees` がそれを使う。根拠: Thread `dacf2a01`。ルート: 委任
- 待機対象以外への配信のテスト: 1 つの発生源が複数の対象に影響するとき、`skip` が待機対象以外への配信を止めないことを検証する。根拠: Thread `77149a6b`。ルート: 委任
- 並行の追加・削除のテスト: 並行に開始した `add` / `remove` で、2 つの更新後の一覧が 1 件ずつ順に配信されることを検証する。根拠: Thread `556a8ca9`。ルート: 委任
- domain の失敗のテスト: `WorkFailure::from_error` のテストを足す。根拠: Thread `1f520953`。ルート: 委任
- テストの構造: 区切りが無い 5 つのテストに Given / When / Then のコメントを入れる。根拠: Thread `ecc66a0c`。ルート: 委任

## 固定するルート

design-01 の「固定するルート」をすべて維持する。この周で新しく固定する実装上の指定は無い。

## 変えないもの

- `usecase/state_subscription.rs` の worker に残る `Failures` と `Workspaces` の分岐。基準 commit `9524a6fa` にもあり、この変更の対象ではない。
- design-01 と design-02 の「変えないもの」。

## 未確定・リスク

- `usecase/retry.rs` のテスト用の観測を usecase の抽象に寄せると、`records` / `page` の呼び出し元（約 25 か所）の署名が変わりうる。この変更は #1954 の要求を満たすためのものではなく、レビューの指摘に応じるものなので、呼び出し元の振る舞いを変えないことが要る。
