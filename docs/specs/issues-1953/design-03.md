# Design 03

## 開始状態

design-02 を実装した `feat/issues/1953` の `9e6a9750`。直前の Design は `docs/specs/issues-1953/design-02.md`。この周までに閉じた Thread は次のとおり。

- 直ったものとして閉じた: `67f55e5e`（呼び出しの名前の複製）、`ffe438c1`（拒否の保留の持ち方）、`0d44d821`（拒否のキーの重複）
- `duplicate` で `da5c21d4-7be6-4862-a0e7-67db1b2ab553` に寄せた: `4067ee40`（パニック安全）、`71737116`（解く後処理の重複）、`f666405b`（拒否同士の競合の未検証）、`c2e2915e`（2 巡目の未検証）、`4f627f32`（保留中に拒否が重なる遷移の未検証）。いずれも自作状態機械に付いた指摘であり、その状態機械を削除すると対象が残らない

## 変える部分

- `PriorityFailureReporter` を `parking_lot::Mutex<bool>` 1 つへ戻し、`AtomicU8` の 5 状態・`AtomicBool`・`rejected` 側のスピンループを削除する。根拠: Thread `da5c21d4-7be6-4862-a0e7-67db1b2ab553`（分類 `scope`。人と決めて直すことにした）。ルート: Thread の `[PLAN]` に固定
- 呼び出しの名前を `Spec::procedure` から `RequestContext::path()` へ戻し、受理の経路で複製しない。根拠: design-02 で固定した `Spec::procedure` のルートの解除。ルート: 下記「固定するルート」
- 枠の対象外の呼び出しのテストに、席を空けたあと同じ記録が解けることを確かめる節を足す。根拠: Thread `e6c05327-e476-4ca6-81f9-481f5bbcb413`。ルート: Thread の `[PLAN]` に固定
- `priority_test.rs` の受理のケースで、`admitted` と `next` を 1 本の並びに記録して順序を確かめる。根拠: Thread `f6d6ae84-ff8b-45a8-a25b-d138d2cf92cd`。ルート: Thread の `[PLAN]` に固定
- `priority_test.rs` を 1 テスト 1 シナリオへ分ける。根拠: Thread `6252df82-0e3d-4b24-a91d-946b5f08d81d`。ルート: Thread の `[PLAN]` に固定
- 枠に触るテスト用アクセサ: コードの変更は無く、直ったと言える条件だけを差し替えた。根拠: Thread `fdc5663f-6200-4b05-8e8d-40f31156c290`。ルート: Thread の `[PLAN]` に固定

## 固定するルート

- 呼び出しの名前は `RequestContext::path()` から取る。`spec()` は使わない。connectrpc 0.9.1 の `path()` のドキュメント（`src/response.rs` の `pub fn path` の直前）が「auth interceptors, span builders, rate limiters は `path()` を読み、`spec()` ではない。`None` は誤設定・合成された context として扱う」と定めており、この包みは rate limiter である。design-02 で固定した `Spec::procedure` を使うルートは解除する。
- 受理の経路で名前を複製しない。`path()` の `&str` からその場で同期に優先度の段（`&'static str`）へ振り分け、名前を複製するのは拒否のメッセージを作るときだけにする。
- design-01 の「固定するルート」3 点（common の包みは転送の型を名乗らない／拒否と受理を伝える口は common に定義し実装は adaptor に置き Main が注入する／組み立ての場所は Main）をそのまま維持する。

## 変えないもの

design-01 の「変えないもの」2 点（#1894 で決まった優先度の分け方・枠の大きさ・待ち行列の長さ・枠の対象外の呼び出し、および失敗の記録の仕組みと画面の見え方）をそのまま維持する。

## 未確定・リスク

- 受理の経路で名前を複製せず、かつ枠の包みが `next` を包む形（`docs/architecture/CONTROLLER.md` の「受け手の側の横断的関心事」、#1897 の確認の対象）を保てるかは、借用の寿命の扱いに依存する。`next` へ渡す値を包みに預け、名前の取り出しを包みの中で行う形なら両立する見込みだが、確かめていない。両立しない場合は、包みの形と複製のどちらを取るかを人と決める。
