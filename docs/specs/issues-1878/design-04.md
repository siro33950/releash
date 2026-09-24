# Design 04

## 開始状態

直前の Design は `docs/specs/issues-1878/design-03.md`。

- 差分の基準は base ブランチ `main` からの派生点 `f54e96b6`（branch `feat/issues/1878`）。
- design-01〜03 が挙げた変える部分は実装済みであり、`041779f8` がこの周の開始状態である。
- 購読の受け取りは Tauri の Rust プロセスにある。`StateClientUsecase`（`src-tauri/src/usecase/state_client.rs`）が daemon へ独自に接続し、版の記憶、30 秒の無通信での切断、1 秒後のつなぎ直しを持つ。画面へは Tauri command `subscribe_client_state` / `stop_client_state` と Channel で渡す（`src-tauri/src/adaptor/controller/command/client.rs:92-116`）。
- 画面のその他の呼び出しと push は、`src/lib/client.ts` が daemon へ直接つなぎ、接続の判断とつなぎ直しを持つ。画面から daemon への経路が 2 本あり、接続の判断とつなぎ直しが 2 か所にある。

## 変える部分

- 購読の受け取りの画面側 client への移設: 購読の受け取りを `src/lib/client.ts` に置き、今の接続の上で state stream を開く。対象ごとに最後に受け取った版を覚えてつなぎ直しで渡し、無通信を検出してつなぎ直し、同じ対象の購読を daemon へ 1 件だけ送って画面の受け手へ配る。stream の切断は push の stream と同じ接続の作り直しに乗せる。根拠: 開始状態の 2 本の経路は、マイルストーン（[02] UI と daemon の間の通信の仕組みを一本化する）が原因に挙げる「接続の判断が別々」「再試行のループが別々」を増やす。R-006、R-009、R-011、R-013、B-006、B-009、B-010、B-012、B-014。ルート: 人間が固定。
- Tauri の Rust プロセスにある受け取りの削除: `StateClientUsecase`、その gateway と domain の port（`StateConnection`、`StateClientGateway`、`Cursor`）、Tauri command `subscribe_client_state` / `stop_client_state`、およびそれだけが使う `SubscriptionTimer::sleep` を削除する。根拠: R-021。ルート: 委任。

## 固定するルート

- design-01 で固定した 3 つと、design-02 で固定した版番号の生成方法を今周も維持する。
- 購読の受け取りは、画面側の client（`src/lib/client.ts`）が持つ。範囲は購読の client 側全体。粒度は置き場所まで。理由は 2 点。ロジックの置き場はサーバ（daemon）であり、Tauri の Rust プロセスは client の一部としてロジックを持たない。一般的な作り（Kubernetes の informer、xDS の client）では、stream の受け取り、版の記憶、再開は client が持ち、server との間に中継役を置かない。関係する要求は R-006、R-009、R-011、R-013。
- 版から再開できるかの判定は daemon が持つ。client は最後に受け取った版を渡すだけにし、届いた版の順序を検証しない。

## 変えないもの

- `requirements.md` の R-001〜R-021 と `behavior.md` の B-001〜B-022 を変更しない。理由は、受け取りの置き場所の変更が外部から観測可能な結果を変えないためである。
- daemon 側の配信（`src-tauri/src/domain/state_subscription/mod.rs`、`src-tauri/src/usecase/state_subscription.rs`、`proto/client.proto` の 3 つの rpc）は変えない。
- 単発の呼び出しの失敗 1 回で全ての通信をつなぎ直す扱いは変えない。#1895、#1891 が扱う。
- design-01〜03 の「変えないもの」を維持する。

## 未確定・リスク

なし。
