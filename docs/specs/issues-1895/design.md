# Design

## 変える部分
- 接続の状態の保持: `src/lib/client.ts` に 5 つの状態の接続を持たせ、読み取りと変化の通知を公開する。`refreshClient`・`connectionListeners`・`onClientConnection` を削除する。根拠: R-001「client の通信部分の 1 か所が…5 つの状態として持ち」、R-002。ルート: 委任
- 接続の確立とつなぎ直し: 購読の stream の切断・無音・確立の失敗で TRANSIENT_FAILURE にし、#1891 の待ちの後に接続先を受け取り直す。根拠: R-004、B-003。ルート: 委任
- 接続の規則の追加: 接続の確立の期限（20 秒）を `proto/client_options.proto` の option に加え、`proto/client.proto` のサービスに値を置き、client はそれを読む。根拠: R-005「期限の値は proto 側の規則に置く」。ルート: 既存のつなぎ直しの規則と同じ場所（`ClientService` のサービスの option）
- 単発の呼び出しの扱い: `scripts/generate-client-protocol.mjs` が生成する `invokeClient` を、接続の状態を読んで IDLE・CONNECTING では待ち、TRANSIENT_FAILURE・SHUTDOWN では即座に失敗させる形にする。根拠: R-008、B-006〜B-008。ルート: 委任
- terminal の入力の宛先: `src/lib/client.ts` の `startState` が、terminal の購読を開始するたび（つなぎ直しを含む）に新しい attachment ID を作る。`src/hooks/useTerminal.ts` は今の購読の ID を読んで入力に付けるだけにし、ID が変わったら入力の番号を 0 に戻す。古い ID の入力は daemon が attachment の不一致で拒否し、その失敗を画面に出す。根拠: R-011、B-012。ルート: 左記のとおり固定
- terminal の接続を追う処理の削除: `useTerminal.ts:636-647` の切断を受けた世代番号の更新と attachment の取り外しを削除する。根拠: R-002「terminal の接続を追う世代番号を持たない」。ルート: 切断の追跡の用途だけを削除する
- terminal の入力の送信の失敗: 接続の状態による失敗（READY でない、UNAVAILABLE）では失敗を画面に出すだけにし、attachment の張り直しは attachment 自体が受け付けられなかったときだけにする。根拠: R-012、B-013。ルート: 左記のとおり固定
- 復元の完了: `completeClientRestoration` を、接続の状態から READY の接続（launch ID・attachment ID）を読む形にする。根拠: R-002「`DaemonBoundary` は、接続の状態を読むだけにする」。ルート: 左記のとおり固定
- シェルの状態の受け取り: `DaemonBoundary.tsx:36-54` の 250 ms ごとの読み直しを、`DaemonSupervisionUsecase::subscribe` によるシェルの状態の変化の通知に置き換える。根拠: R-013、B-014。ルート: `DaemonSupervisionUsecase::subscribe` を使う
- 呼び出し元の失敗の表示: requirements の Current Behavior に挙げた、利用者の操作・経路で分かれるもの・表示用の計算の呼び出し元で、失敗を画面に出す。表示用の計算は失敗を「結果が無い」と別の値にする。根拠: R-009、B-009、B-010。ルート: 委任
- 計測の失敗: `src/lib/telemetry.ts:14` の `.catch(() => {})` と `src/hooks/useTerminal.ts:313` の catch の無い呼び出しを、失敗をログに残す形にする。根拠: R-010、B-011。ルート: 委任
- デッドコードの削除: `src/hooks/useWorkflowConfig.ts:13,22` の、呼び出す側の無い処理を削除する。根拠: requirements の Scope「呼び出す側の無い `hooks/useWorkflowConfig.ts:13,22` の削除」。ルート: 委任

## 固定するルート
- terminal の attachment ID は、入力の番号の世代として扱う。作るのは `client.ts` の `startState` で、terminal の購読を開始するたびに作る。terminal は接続の状態を見張らず、今の購読の ID を読むだけにする。理由: 同じ ID のまま client 側で番号を 0 に戻すと、READY の直後の購読のやり直しと、CONNECTING の間に待たせた入力（古い番号）が競合して、入力の順番が入れ替わる。daemon 側で番号を保つと、TRANSIENT_FAILURE の間に即座に失敗した入力が番号を消費して番号に抜けができ、後の入力が保留され続ける。
- 接続の確立の期限は、`ClientService` のサービスの option に置く。理由: #1891・#1951 で、つなぎ直しの規則を proto 側の 1 か所に置くと決めているため。
- シェルの状態は `DaemonSupervisionUsecase::subscribe` の変化の通知で受け取り、client の接続の状態とは別の値として読む。

## 変えないもの
- terminal の、接続と無関係な attachment の張り直し（出力のあふれ、stream の項目の適用の失敗、処理済みの報告の失敗）と、それに使う世代番号。接続と無関係な snapshot の取り直しで、購読の共通の API を使っており、この ISSUE の範囲外であるため。

## 未確定・リスク
- 固定するルートに書いた、他の方式での入力の順番の入れ替わりと番号の抜けは、コードの流れから導いたもので、実行して確かめていない。再現のテストを置いて確かめる。前提が外れても B-012 の判定は変わらないが、ルートを選んだ根拠が変わる。
- requirements の Current Behavior は、コードを読んで確認したものであり、実機での再現は行っていない。
