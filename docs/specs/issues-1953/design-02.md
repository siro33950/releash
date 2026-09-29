# Design 02

## 開始状態

design-01 を実装した `feat/issues/1953` の `9b6eaad9`。直前の Design は `docs/specs/issues-1953/design-01.md`。この周までに閉じた Thread は、Main の組み立てを acceptance host が呼ぶ経路の指摘（`d57da6df-e90f-443b-b0c9-aee28c5676e0`）と、拒否と受理が並行したときに記録が解けないという指摘（`c93aacf7-c094-4460-8a1c-506c83f9eb26`）の 2 件で、どちらも `invalid` である。

## 変える部分

- 枠の対象外の呼び出しが保留中の拒否の記録を解かないことを確かめるテストを足す。根拠: Thread `e6c05327-e476-4ca6-81f9-481f5bbcb413`（B-001 と B-005 の組み合わせが未検証）。ルート: Thread の `[PLAN]` に固定
- `common/priority.rs` に `priority_test.rs` を足し、分類バイパス・拒否・受理の 3 分岐を直接確かめる。根拠: Thread `f6d6ae84-ff8b-45a8-a25b-d138d2cf92cd`（`docs/architecture/TEST.md` の配置規約と common 配下の慣行）。ルート: Thread の `[PLAN]` に固定
- 拒否のキーの生成を 1 箇所にまとめる。根拠: Thread `0d44d821-204d-46fe-ac22-af6e874a3635`（`rejected` と `admitted` に同じリテラルが 2 箇所）。ルート: Thread の `[PLAN]` に固定
- 枠に触るためのテスト用アクセサを 1 段にする。根拠: Thread `fdc5663f-6200-4b05-8e8d-40f31156c290`（テストが `deps.priority.gate.limits()` の 3 段を辿る）。ルート: Thread の `[PLAN]` に固定
- `PriorityFailureReporter` の `pending` を `AtomicBool` にし、受理の経路でロックを取らない。根拠: Thread `ffe438c1-311d-46a5-a531-e6bbc0331a80`（既存の同種のフラグは `AtomicBool`）。ルート: Thread の `[PLAN]` に固定
- 呼び出しの名前を `Spec::procedure` の `&'static str` から取り、受理と枠の対象外の経路で複製しない。根拠: Thread `67f55e5e-af30-42c2-991d-f111c84e2b78`（この周で入った、呼び出しごとの複製）。ルート: Thread の `[PLAN]` に固定

## 固定するルート

design-01 の「固定するルート」3 点（common の包みは転送の型を名乗らない／拒否と受理を伝える口は common に定義し実装は adaptor に置き Main が注入する／組み立ての場所は Main）をそのまま維持する。今周に新しく固定したルートは無い。

## 変えないもの

design-01 の「変えないもの」2 点（#1894 で決まった優先度の分け方・枠の大きさ・待ち行列の長さ・枠の対象外の呼び出し、および失敗の記録の仕組みと画面の見え方）をそのまま維持する。

## 未確定・リスク

なし
