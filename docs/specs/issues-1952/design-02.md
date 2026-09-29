# Design 02

## 開始状態

基準は branch `feat/issues/1952` の `984238fc`。直前の Design は `design-01.md`。この周までに閉じた Thread は `7e06fcdb`（POLICY の初期化が panic に倒れるという指摘。`invalid` で閉じた）だけである。

## 変える部分

- 再接続の対象でない失敗で生存の監視をやめる経路を無くす: stream の開始と受信の失敗を Connect のコードで区別せず、いずれも生存の失敗として 1 回数えてつなぎ直す。`WatchExit::Unretryable` と、それを「接続中」と返す分岐、失敗を握りつぶす分岐を削除する。根拠: Thread `4f101c03`。B-004「WHEN stream が切れる THEN シェルは生存の失敗を 1 回数え、stream をつなぎ直す」。ルート: `reconnect_status_code` を当てるのは購読の開始の失敗だけにする
- descriptor と extension の解決の重複を無くす: `client_descriptor.bin` の `DescriptorPool` 化と `releash.client.v1.<name>` の extension 解決を 1 か所にまとめ、`adaptor/presenter/client/json.rs` と `adaptor/gateway/desktop_client.rs` の両方がそれを使う。根拠: Thread `74954dce`。ルート: 委任
- `observe()` の責務を分ける: 購読の開始の扱いと、設定の変換・反映を別の関数へ切り出す。根拠: Thread `64594197`。ルート: 委任
- `stream()` の合図の送出の重複を無くす: `StateSubscriptionEvent::Bookmark` を返す箇所を 1 つにする。根拠: Thread `d1eb33e5`。ルート: 委任
- `DaemonLiveness::succeeded()` の使われていない戻り値を無くす: 根拠: Thread `c23b627d`。ルート: 委任
- `startup_terminated` の期限超過で接続の分類を保つ分岐のテストを足す: 根拠: Thread `22e419a0`。ルート: 委任

## 固定するルート

- `design-01.md` で固定したルートを維持する。proto の service option を正とし同じ値を Rust 側の定数として持たないこと、無音と判断する時間は 20 秒で画面とシェルで分けないこと、連続失敗の回数は 2 とすること、シェルの stream は 1 本にすること、シェルに失敗の記録先を作らないこと、`RetryBackoff::SERVICE` と proto の `connection_backoff` の値の重複をこの開発では直さないこと。
- proto の `reconnect_status_code` を当てるのは購読の開始と停止の失敗だけとし、stream 自体の開始と受信の失敗には当てない。stream の失敗は分類にかかわらず生存の失敗として数え、つなぎ直す。

## 変えないもの

- `requirements.md` と `behavior.md`。今周の指摘はいずれも実装の問題であり、要求と受入条件の追加・変更を伴わないため。

## 未確定・リスク

なし。`design-01.md` に挙げた「Rust から proto の service option を読み出せるか」は、`984238fc` の実装と `test_接続規則_protoの値を読む` で解消した。
