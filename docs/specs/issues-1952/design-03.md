# Design 03

## 開始状態

基準は branch `feat/issues/1952` の `3d547155`。直前の Design は `design-02.md`。この周までに、`design-02.md` が扱った 6 件のうち 4 件（`4f101c03`・`d1eb33e5`・`c23b627d`・`22e419a0`）が閉じた。加えてこの周で立った 3 件（`98869d1d`・`3c43dc9c`・`75793224`）を `invalid` で閉じた。

## 変える部分

- descriptor の読み込みをテストも含めて 1 か所にする: `adaptor/presenter/client/client_test.rs` の 3 か所で `client_descriptor.bin` を個別に `DescriptorPool::decode` している箇所を、`adaptor/presenter/client/descriptor.rs` の `pool()` に置き換える。根拠: Thread `74954dce`。ルート: 委任
- 購読開始の進行状態を `observe()` から出す: 購読を要求したかどうかと進行中の要求を持つ型を作り、`observe()` は「まだ要求していなければ要求する」「進行中の要求の決着を待つ」の 2 つを呼ぶだけにする。根拠: Thread `64594197`。ルート: 委任

## 固定するルート

- `design-01.md` と `design-02.md` で固定したルートを維持する。

## 変えないもの

- `requirements.md` と `behavior.md`。今周の指摘はいずれも実装の問題であり、要求と受入条件の追加・変更を伴わないため。
- `adaptor/gateway/daemon_supervision.rs` の `finish_pending` と `connection` の失敗の返し方。3 か所とも同じ `supervised_connection_failure` を通っており、片付ける対象が違うため 1 つに畳まない（Thread `98869d1d`）。
- プロセスの spawn / exited に伴う `pending` のクリアと、`first_settings` の失敗で分類が無い場合の経路。前者は実プロセスを伴うため統合テストの領域、後者は watch の task が panic した場合にしか到達しないため、単体テストを足さない（Thread `3c43dc9c`・`75793224`）。

## 未確定・リスク

なし。
