# Design 01

## 開始状態

初回の Design である。`docs/specs/issues-1883/` に既存の `design-NN.md` は無い。

- 差分の基準は base `main` からの派生点 `42be41e0`（`refactor(error): 失敗の分類をエラーの値に持たせる (#1880) (#1919)`）。branch は `feat/issues/1883`。
- 未コミットの変更は `docs/specs/issues-1883/requirements.md` と `docs/specs/issues-1883/behavior.md` だけで、コードは `42be41e0` のままである。この実装を開始状態とする。
- 開始状態の挙動は `docs/specs/issues-1883/requirements.md` の Current Behavior を参照する。本文で引く行番号は、`42be41e0` の実装で確認したものである。
- この周までに解消・見送りとなった Thread は無い。

## 変える部分

- 既定の期限の設定: `ClientService` の router（`client.rs:227-240`。現在は受信サイズの上限だけを設定している）に `DeadlinePolicy` を設定し、client が期限の header を送らない呼び出しに daemon の既定の期限を当てる。根拠: R-001「client が期限を付けない単発の呼び出しには、daemon の既定の期限 120 秒が適用され、その期限を超えた呼び出しは `DEADLINE_EXCEEDED` で終わる」、B-001。ルート: 既定の期限は 120 秒。client が指定した期限に daemon 側の上限・下限を設けない。stream の item への適用は有効にしない。`DeadlinePolicy` の組み立て方と router への設定の書き方は委任。
- 取り消しの仕組みの導入: 期限切れと client の中断を 1 つの取り消しの仕組みで表し、`execute` から handler の async 経路へ伝える。開始状態では `src-tauri/src` に取り消し用の token を扱うコードが無く、生成ハンドラと手書きハンドラは `connectrpc::RequestContext` を `_ctx` として受け取って捨てている（`build.rs:140`、`client_service.rs:3,18,61,78,103,123,143,154,165`）。根拠: R-005「期限切れと client の中断は、呼び出しの入口から処理の先へ伝わり、取り消しを受け取った処理は実行を止める」、B-005。ルート: 手段は `tokio_util::sync::CancellationToken`。token の生成箇所、handler への渡し方、`execute` 内の構造は委任。
- `tokio_util` の直接依存への追加: `src-tauri/Cargo.toml` に `tokio-util` を直接依存として追記する。開始状態では `src-tauri/Cargo.lock` に 0.7.19 で入っているが、`src-tauri/Cargo.toml` の直接依存ではない。根拠: R-005 の実現手段として `CancellationToken` を使うことがルートで固定されている。ルート: `src-tauri/Cargo.toml` に直接依存として追記する。
- 切り離した処理の終了: `execute` が `tokio::spawn` で切り離す task（`client.rs:162`）と、`dispatch_admitted` がさらに `tokio::spawn` で切り離す task（`dispatch.rs:135-148`）を、呼び出しの終わりと同時に終わらせる。開始状態では、待っている future が drop されても切り離した task は走り続ける。根拠: R-004「期限切れまたは client の中断で終わった単発の呼び出しでは、daemon 側で切り離された処理も同時に終わり」、B-004。ルート: 委任（取り消しの手段は `CancellationToken`）。
- 同時実行の枠の解放: permit の保持を全経路で揃え、期限切れと client の中断で枠が解放されるようにする。開始状態では保持が 4 通りに分かれ（`client.rs:136,162,194`、`client_service.rs:6,81`）、`StopWatching` と `watch` は `spawn_blocking` の task が permit を持つため外から中断できない。根拠: R-004「その呼び出しが確保していた同時実行の枠が解放される」、B-004「その呼び出しが確保していた枠は解放される」。ルート: 委任。permit を blocking task の外で持つか、該当経路の blocking をやめるかを含め、方法は指定されていない。
- 切り離した task の終わり方の分類の集約: `client.rs:139-146, 168-175, 197-204` の 3 か所にある同じ形の処理（いずれも `FailureKind::Internal` に分類する）を 1 か所にまとめ、取り消しで終わった場合を `FailureKind::Cancelled` に分類する。根拠: R-003「client が単発の呼び出しをやめたとき、その呼び出しは `CANCELLED` として終わる」、B-003。ルート: まとめる範囲は `client.rs` の 3 か所のみ。置き場所は `client.rs` 内。ヘルパーの形（関数名）は委任。`repository/mod.rs` の同型（`run_blocking` / `run_repository_state`）は対象外。

R-002（client が付けた期限で `DEADLINE_EXCEEDED` になること）と R-006（購読の stream が開いた後に期限で打ち切られないこと）は、開始状態で満たされている。既定の期限を設定した後もこれらが変わらないことは「変えないもの」に書く。既定の期限の設定により、stream を開く呼び出し自体は既定の期限の対象になる（R-006 の「stream を開く呼び出し自体は、単発の呼び出しと同じ期限の対象になる」）。

## 固定するルート

- 方針は gRPC の deadline に従う（https://grpc.io/docs/guides/deadlines/ ）。範囲: 期限の当て方とエラーコードの全体。粒度: 方針の指定。理由: milestone #99 の方針が、確立した標準に合わせることを定めている。関係: R-001〜R-006。
- daemon の既定の期限は 120 秒。範囲: `ClientService` の router に設定する `DeadlinePolicy` の既定の期限。粒度: 値の指定。理由: 画面の React が付ける既定と同じ値にして経路ごとに天井を揃え、正常に時間のかかる処理を切らない。関係: R-001、B-001。
- client が指定した期限に daemon 側の上限・下限を設けない。範囲: `DeadlinePolicy` の min / max。粒度: 設定しないことの指定。理由: 呼び出し元が loopback + token 認証の自分たちの client に限られる。関係: R-002、B-002。
- 期限切れと client の中断は 1 つの取り消しの仕組みで伝え、受け取り方を揃える。手段は `tokio_util::sync::CancellationToken` とし、`src-tauri/Cargo.toml` に直接依存として追記する。範囲: `execute` から handler の async 経路まで。粒度: 型の指定。理由: 「全ての処理が同じ受け取り方で止まる」を確立した型で表し、#1893 が同じ token を store・外部プロセスへ引き継げるようにする。`JoinHandle::abort` だけでは `dispatch_admitted` が切り離す task（`dispatch.rs:135-148`）と #1893 の引き継ぎ先に届かない。関係: R-003、R-004、R-005、B-003、B-004、B-005。
- 期限切れは `DEADLINE_EXCEEDED`、中断は `CANCELLED`。既存の `FailureKind` → Connect エラーコードの変換（`adaptor/protocol/connect.rs:26-47`。`FailureKind::Expired` → `DeadlineExceeded`、`FailureKind::Cancelled` → `Canceled`）を使い、新しい分類を作らない。範囲: エラーコードの決定。粒度: 既存経路を使うことの指定。理由: #1880 で 1 か所に集約済み。関係: R-001、R-002、R-003、B-001、B-002、B-003。
- 規則の所有: 期限の適用は connectrpc の `DeadlinePolicy`、期限切れ・中断の分類は `domain::failure::FailureKind`、取り消しの伝搬と枠の解放は `adaptor/controller/api` が持つ。ドメインモデルを新設しない。範囲: 今回書く規則の所有者。粒度: 所有者の指定。理由: 今回書く規則に domain の判断・計算が無く、分類は既に domain が所有している。関係: R-001〜R-005。
- 切り離した task の終わり方を失敗の分類へ写す処理を `client.rs` の 1 か所にまとめる。範囲: `client.rs:139-146, 168-175, 197-204` の 3 か所のみ。`repository/mod.rs` の同型（`run_blocking` / `run_repository_state`）は対象外。粒度: まとめる範囲の指定（置き場所は `client.rs` 内、形は委任）。理由: 3 か所すべて今回触り、取り消しの分類（`JoinError::is_cancelled` による `Cancelled` と `Internal` の分岐）を複製しない。関係: R-003、R-004。
- 購読の stream を開いた後は期限の対象にしない（`DeadlinePolicy` の stream の item への適用を有効にしない）。範囲: `SubscribePush`、`OpenStateStream`、`SubscribeTerminalSurfaces`。粒度: 有効にしないことの指定。理由: 生存の判断は #1878 が定めた bookmark で行う。stream を開く呼び出し自体は、`DeadlinePolicy` が service 単位であるため既定の期限の対象になる（3 つの handler は await せず即座に stream を返すため観測差は生じない）。関係: R-006、B-006。

## 変えないもの

- 画面の React が付ける既定の期限 120 秒（`src/lib/client.ts:70`）を変更しない。理由: client 側のタイマーが、daemon が応答を返さないときに client 自身で打ち切る役目を持ち、その役目を代わりに担うものが今回の範囲に無い。
- Tauri のシェルが付ける既定の期限 30 秒と生存監視の 5 秒（`src-tauri/src/adaptor/gateway/desktop_client.rs:19,53`）を変更しない。理由: 経路ごとの妥当な長さの見直しは今回の要求に含まれない。
- client が指定した期限を daemon 側で変更しない（上限・下限を設けない）。理由: 呼び出し元が loopback + token 認証の自分たちの client に限られ、極端な値を送る呼び出し元が現状存在しない。
- 購読の stream を開いた後は期限の対象にしない（`DeadlinePolicy` の stream の item への適用を有効にしない）。理由: stream の生存の判断は #1878 が定めた bookmark で行う。
- 既に同期処理として動き始めた処理の内側には取り消しを届けない。理由: `spawn_blocking` 上の同期処理と、その先の store・外部プロセスへの引き継ぎは #1893 が担い、同じ箇所を二度触らない。
- 期限切れ・中断の分類に新しい仕組みを作らない。既存の `FailureKind` → Connect エラーコードの変換（`adaptor/protocol/connect.rs:26-47`）を使う。理由: #1880 で 1 か所に集約済み。

## 未確定・リスク

- connectrpc 0.9.1 は、client の中断（接続の切断）を handler へ渡す口を持たない。handler が受け取る `RequestContext` の公開 API は `deadline()` / `time_remaining()` ほかで、取り消しを受け取る口は無い（`connectrpc-0.9.1/src/response.rs` の `impl RequestContext`）。中断を daemon 側で知る手がかりは handler の future が drop されることだけであり、これを取り消しの仕組み（`CancellationToken`）へ結び付けられない場合、R-003 と R-005（B-003、B-005）を満たせない。
- 自動判断した箇所は無い。Requirements と Behavior の対応に誤り・不足・矛盾は見つからなかった。未決のまま残した要求も無い（Requirements の Assumptions / Open Questions は「なし」）。`[DEFERRED]` で人間へ渡した件も無い。
