# Context

要求の正本は ISSUE #1951「[10] 購読の stream がつながった直後に切れると、つなぎ直しの待ちが伸びない」。マイルストーン #99「UI と daemon の間の通信の仕組みを一本化する」に属する。

補助資料

- #1891「client のつなぎ直しを 1 つの実装にする」（closed）
- #1895「接続の状態を client の 1 か所に持たせる」
- #1952「Tauri のシェルの生存確認と設定の受け取りが、#1879 の意図からずれている」
- マイルストーン #78（Swift の client）
- #1883「呼び出しに期限と取り消しを通す」（closed）

対象ファイル

- `src/lib/client.ts`
- `src/lib/connectionBackoff.ts`
- `proto/client_options.proto`
- `proto/client.proto`

参照する標準

- gRPC Connection Backoff（https://github.com/grpc/grpc/blob/master/doc/connection-backoff.md）。つなぎ直しの間隔の値（初回 1 秒・1.6 倍・±20% のずれ・上限 120 秒）の出どころ。#1891 で採用済み
- gRPC Service Config（https://github.com/grpc/grpc/blob/master/doc/service_config.md）。期限・再試行の対象にするステータスコードを、サービスの定義と同じ場所にデータとして置く形
- client-go の reflector（`tools/cache/reflector.go:62-67`、`:336`）と apimachinery（`pkg/util/wait/backoff.go:347`）。watch のつなぎ直しで待ちを初期値に戻す条件と、その時間 2 分

確定済みの制約

- つなぎ直しの間隔の値は #1891 のまま変えない。client-go から採るのは、待ちを初期値に戻す条件と 2 分だけである。
- 待ちを初期値に戻す条件は「前回の待ちが終わってつなぎ直しを始めた時点から 2 分を超えて待ちが要らなかったら戻す」。根拠は `backoff.go:347` の doc comment「If the backoff is not called within resetDuration, the backoff is reset.」と `reflector.go:65` のコメント「If we don't backoff for 2min, assume API server is healthy and we reset the backoff.」である。現在の `DelayWithReset` の実装（`backoff.go:292-304`）は、前回初期値に戻した時点からの経過時間で戻しており、この条件とは違う。採るのは doc comment 側である。
- 2 分を測る起点は、前回の待ちが終わってつなぎ直しを始めた時点とする。上限 120 秒とずれ ±20% により 1 回の待ちが 2 分を超えうるため、待ちを始めた時点を起点にすると、切れ続けている間に待ちが初期値へ戻ってしまう。client-go は上限 30 秒・ずれ +100% で 1 回の待ちが 60 秒未満に収まるため、旧実装が待ちを始めた時点を起点にしていても問題が出ない。
- マイルストーン #99 の方針により、言語が分かれる client と daemon の間では、規則を proto 側に 1 か所で定義する。TypeScript 版と #78 以降の Swift 版は、同じ規則を読む。
- `src/lib/client.ts:51` の単発の呼び出しの既定の期限（120 秒）は、#1883 が「この ISSUE で変えないもの」として #1891・#1895 に回したが、#1891 は扱わずに閉じ、#1895 の範囲は接続の状態である。どの ISSUE の範囲にも無いため、今回の範囲に含める。

# Outcome

対象者は、Releash の画面を使う開発者と、マイルストーン #78 で Swift の client を作る開発者。

問題は 2 つある。

- daemon が購読の stream を開いた直後に閉じる状態が続くと、画面は約 1 秒ごとに接続を作り直し続ける。待ち時間が伸びないため、daemon にも同じ頻度で stream が開かれ、負荷が下がらない。
- つなぎ直しの規則の一部と、単発の呼び出しの既定の期限が TypeScript の中にある。Swift の client を作るときに、同じ値を二重に書くことになる。

変更後は、つながった直後に切れることが続く間はつなぎ直しの待ちが上限まで伸び、つながった状態が 2 分を超えて続いたときだけ初期値に戻る。client の通信の規則は proto 側に 1 か所で定義され、TypeScript はそれを読むだけになる。

# Current Behavior

確認の方法は、コードの読み取りと既存テストの内容による。アプリは実行していない。基準は main `636bb2f0`。

待ちが伸びない

- `src/lib/client.ts:255-265` — 購読の stream が `ready` を受け取るたびに `backoff.reset()` を呼ぶ。
- `src/lib/connectionBackoff.ts:10-18` — `reset()` の直後の `next()` は `initialBackoffMs`（1000）をずれ無しで返す。
- `src/lib/client.ts:293-307` — stream が終わると `backoff.next()` の値だけ待って次を開く。したがって `ready` の直後に stream が終わることが続く間、つなぎ直しは毎回きっかり 1 秒間隔になる。
- `src/lib/client.ts:226-234` — 購読の開始が `Unavailable`・`Aborted`・`ResourceExhausted` で失敗した場合も `RETRY` で同じループに入る。`ready` の後なので待ちは同じく 1 秒である。
- `src/lib/client.ts:283-294` — 終了理由が `RETRY` 以外のときは `refreshClient(client)` で接続全体を作り直す。1 周ごとに `get_client_endpoint`・`GetServerInfo`・`validate_daemon_connection`・新しい stream が発生する。
- `src-tauri/src/infrastructure/state_subscription.rs:176` — daemon 側の `ready` は stream 生成時の先頭要素として無条件に流れる。購読の登録や snapshot の完成より前である。したがって `ready` が示すのは「daemon が stream を受け入れた」ことだけである。
- `src/lib/client.test.ts:360`「状態のstreamのつなぎ直しは1秒から1.6倍ずつ伸び、readyを受け取ると1秒に戻る」が、この挙動を固定している。

規則の所在

- つなぎ直しの間隔は proto 側にある。`proto/client_options.proto:26-32` の `ConnectionBackoff`（`initial_backoff_ms`・`multiplier`・`jitter`・`max_backoff_ms`）と、`proto/client.proto:2697-2702` の `ClientService` の option。`src/lib/connectionBackoff.ts:1-5` が `getOption` で読む。
- つなぎ直す対象にするステータスコードは TypeScript の中にある。`src/lib/client.ts:168-172` の `RECONNECT_CODES`（`Unavailable`・`Aborted`・`ResourceExhausted`）。
- 無音と判断する時間は TypeScript の中にある。`src/lib/client.ts:165` の `STATE_SILENCE_MS`（30 秒）。daemon の bookmark 間隔は `src-tauri/src/infrastructure/state_subscription.rs:14` の 10 秒である。
- 単発の呼び出しの既定の期限は TypeScript の中にある。`src/lib/client.ts:51` の `defaultTimeoutMs: 120_000`。購読の stream は `:257` で `timeoutMs: 0` に上書きするため、対象は単発の呼び出しだけである。`src/generated/client_commands.ts` の `invokeClient` は期限を渡さないので、出どころはこの 1 か所である。

# Scope / Non-goals

変更するもの

- 購読の stream のつなぎ直しの待ち時間の扱い（`src/lib/client.ts`、`src/lib/connectionBackoff.ts`）
- つなぎ直しの規則の置き場所（`proto/client_options.proto`、`proto/client.proto`、それを読む TypeScript の実装）
- 単発の呼び出しの既定の期限の置き場所（同上）

変更しないもの

- つなぎ直しの間隔の値。#1891 で決めた gRPC の値のままとする
- 接続の状態を持つ場所と、`refreshClient`・`connectionListeners`。#1895 が扱う
- 再接続した後に画面を作り直すかどうか。#1896 が扱う
- Tauri のシェルのつなぎ直しと、シェルの単発の呼び出しの既定の期限（`src-tauri/src/adaptor/gateway/desktop_client.rs:31` の 30 秒）。#1952 が扱う
- daemon が bookmark を送る条件（`src-tauri/src/infrastructure/state_subscription.rs`）。#1952 が扱う
- daemon が期限を付けない呼び出しに当てる既定の期限（`src-tauri/src/adaptor/controller/api/client.rs:159` の 120 秒）。#1883 で決めたサーバ側の規則であり、client の規則ではない
- Swift 版の実装。マイルストーン #78 が扱う

# Requirements

- R-001: 購読の stream が `ready` を受け取っただけでは、つなぎ直しの待ちは初期値に戻らない。`ready` の直後に stream が終わることが続く間、待ちは初回の値から倍率ずつ、ずれを付けて上限まで伸びる。
- R-002: 前回のつなぎ直しの待ちが終わってつなぎ直しを始めた時点から、初期値に戻すまでの時間を超えて次の待ちが要らなかった場合に限り、次の待ちは初回の値に戻る。
- R-003: 購読の開始・停止の失敗によるつなぎ直しも、stream が終わったときのつなぎ直しと同じ待ちの状態を使う。
- R-004: つなぎ直しの規則は proto 側に 1 か所で定義され、client はそれを読む。TypeScript の中に同じ値を持たない。対象は、間隔の初期値・倍率・ずれ・上限、待ちを初期値に戻すまでの時間、つなぎ直す対象にするステータスコード、無音と判断する時間である。
- R-005: 単発の呼び出しの既定の期限も proto 側の規則から読む。TypeScript の中に値を持たない。
- R-006: 規則の値は、初回 1 秒、倍率 1.6、ずれ ±20%、上限 120 秒、待ちを初期値に戻すまで 2 分、無音と判断する時間 30 秒、つなぎ直す対象にするステータスコードは `Unavailable`・`Aborted`・`ResourceExhausted`、単発の呼び出しの既定の期限 120 秒とする。2 分以外は現在の値のままとする。

# Assumptions

- 待ちを初期値に戻す条件は、client-go の doc comment 側（「待ちが要らなかったら戻す」）を採る。現在の `DelayWithReset` の実装とは違うことを承知のうえで決めた。
- `src/lib/client.ts:51` の単発の呼び出しの既定の期限を、今回の範囲に含める。
