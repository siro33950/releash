# Context

- 要求の正本: https://github.com/siro33950/releash/issues/1895
- 補助資料: #1878（購読と bookmark）、#1879（Tauri のシェルの生存の判定）、#1891（つなぎ直しの実装）、#1896（再接続で画面を作り直さない）、#1951（つなぎ直しの規則を proto 側に置く）、gRPC Connectivity Semantics（https://github.com/grpc/grpc/blob/master/doc/connectivity-semantics-and-api.md）、gRPC Connection Backoff（https://github.com/grpc/grpc/blob/master/doc/connection-backoff.md）
- ISSUE の方針「gRPC の接続状態に従う」を基準にする。ISSUE の「接続状態が READY でない間は、terminal への入力を含む操作を受け付けない」は、この方針を言い損ねたものとして読む。求めているのは「表示無く捨てない」ことであり、gRPC の接続状態ごとの呼び出しの扱い（IDLE は接続を始めて待つ、CONNECTING は期限まで待つ、TRANSIENT_FAILURE・SHUTDOWN は即座に失敗させる）に従い、失敗を画面に出すことで満たす。
- つなぎ直しの間隔、待ちを初期値に戻す条件、つなぎ直す対象のステータスコード、無音と判断する時間、単発の呼び出しの既定の期限は、#1891・#1951 で `proto/client.proto` のサービスの option に定義済みであり、client の実装はそれを読む。この ISSUE で加える接続の規則も同じ場所に置く。
- daemon は、購読の対象の有無にかかわらず、購読の stream ごとに bookmark を定期的に流す（#1879）。
- 計測（telemetry）の失敗の扱いは、OpenTelemetry の Error handling の仕様（計測の失敗はアプリの動作を妨げず、ログに残す）に従う。

# Outcome

対象者: Releash のデスクトップ画面の利用者。

現在の問題: client と daemon の接続の状態を、画面の中の複数の箇所（通信部分、terminal、`DaemonBoundary`）が別々に判断している。購読の stream が切れると接続全体が作り直され、その間の terminal への入力は表示無く捨てられる。単発の呼び出しの失敗の多くも、画面に出ないまま捨てられている。接続先を受け取る処理は期限無しで待つ。

変更後の状態: 接続の状態を client の 1 か所が gRPC の 5 つの状態で持ち、全ての呼び出し元はそれを読むだけになる。READY でない間の呼び出しは gRPC どおりに待つか失敗し、失敗は画面に出る。terminal への入力は、つなぎ直しの前後で順番どおりに届くか、失敗として画面に出る。

# Current Behavior

main `c519ef9f` のコードを読んで確認した挙動。実機での再現は行っていない。

- `src/lib/client.ts:40-77` `open()`: Tauri のシェルから `get_client_endpoint` で接続先を受け取り、`GetServerInfo` を呼び、`validate_daemon_connection` で検証する。シェル側の `attach`（`src-tauri/src/usecase/daemon_supervision.rs:174-204`）は、シェルの接続が保留中の間、期限無しで待つ。
- `src/lib/client.ts:92-100` `refreshClient`: 接続全体を破棄し、`connectionListeners` に切断を知らせる。呼び出し元は購読の stream の終了時（`:291`）だけで、無音（`:165,247`、20 秒）を含む、IDLE・RETRY 以外の理由の終了が対象。
- `src/generated/client_commands.ts:1793-1815` `invokeClient`: 失敗しても接続を作り直さない。接続が確立するまで `getClient()` で待つ。
- `src/hooks/useTerminal.ts:636-647`: 切断を知らされると、attachment の世代番号（`attachmentEpoch`、`:301`）を進め、`attachmentId` を消して attachment を外す。つながると attachment を張り直す。
- `src/hooks/useTerminal.ts:651-653` `deliverInput`: `attachmentId` が無い間の入力を、表示無く捨てる。
- `src/hooks/useTerminal.ts:672-678`: 入力の送信が失敗すると、理由を問わず失敗を表示して attachment を張り直す。
- 購読がつなぎ直されると、`startState`（`src/lib/client.ts:212-223,260`）は terminal の購読を同じ attachment ID で送り直す。daemon は購読の開始のたびに入力の受け付けを有効にし（`src-tauri/src/usecase/terminal_surface/application.rs:144-157`）、同じ ID でも入力の番号を 0 に戻す（`src-tauri/src/domain/terminal_surface/entities/terminal_surface_input_ingress.rs:40-49`）。client 側の入力の番号は attachment の張り直しのときだけ 0 に戻る。
- `src/components/DaemonBoundary.tsx:36-54`: シェルの状態を 250 ms ごとに読み直す。client 側の接続が切れていても、シェルが Ready のままなら覆いは出ない。`:73-80` の復元の完了は、`completeClientRestoration`（`src/lib/client.ts:429-438`）が `getClient()` と接続の保持値から launch ID と attachment ID を取る。
- `invokeClient` の呼び出し元（`src/` の 73 か所）のうち、失敗を画面に出さずに捨てている箇所は次のとおり。
  - 利用者の操作: `App.tsx:41`、`components/panels/TerminalPanel.tsx:105`、`components/workspace/WorkspaceList.tsx:885`、`components/panels/ReviewPanel.tsx:487,561`、`components/panels/useDiffOperations.ts:41`、`hooks/useNotionTasks.ts:36`、`hooks/useWorkspaceStateCache.ts:30`、`hooks/useIssues.ts:9`、`hooks/useRepoList.ts:15,21,27`、`hooks/useTerminal.ts:292,387,758`、`hooks/useNotionLabelOptions.ts:12`、`hooks/useSettings.ts:132`、`hooks/useBaseBranch.ts:25`、`hooks/useDiffComments.ts:23,36,47,59`
  - 経路で分かれるもの: `hooks/useGitActions.ts:6,10`（メニューからは表示される。`components/panels/ReviewPanel.tsx:398,409,421,431` からは表示されない）
  - 表示用の計算（失敗を「結果が無い」と同じ値にする）: `components/panels/CodeDiffViewer.tsx:73`、`components/panels/ShikiDiffViewer.tsx:1701`、`hooks/useFileNavigation.ts:32`
  - 計測: `lib/telemetry.ts:14`（ログにも残さず捨てる）、`hooks/useTerminal.ts:313`（catch が無い）
- `hooks/useWorkflowConfig.ts:13,22` の呼び出しは、呼び出す側が無い。

# Scope / Non-goals

変更するもの

- 画面の接続状態の持ち方と、それを読む側（`src/lib/client.ts`、`src/hooks/useTerminal.ts`、`src/components/DaemonBoundary.tsx`、`scripts/generate-client-protocol.mjs` と生成物 `src/generated/client_commands.ts`）
- 接続の確立の期限の、proto 側の規則への追加
- READY でない間の呼び出しの扱い
- Current Behavior に挙げた、失敗を画面に出さずに捨てている呼び出し元（利用者の操作、経路で分かれるもの、表示用の計算）の失敗の表示
- 計測の失敗をログに残すこと
- terminal の入力の宛先（attachment ID）と入力の番号の扱い
- `DaemonBoundary` がシェルの状態を読む方法（定期の読み直しから、変化の通知へ）
- 呼び出す側の無い `hooks/useWorkflowConfig.ts:13,22` の削除

変更しないもの

- つなぎ直しの間隔と回数、待ちを初期値に戻す条件（#1891・#1951）
- 全体の「再接続中」の表示と、再接続中に画面を残したまま操作を止めること。#1896 が扱う
- 再接続で画面を作り直すこと（`DaemonBoundary` の key）と、復元の手続き（`begin_restoration`・`finish_restoration`・`connection_generation`）。#1896 が扱う
- Tauri のシェルの生存の判定。#1879 が扱った
- terminal の、接続と無関係な attachment の張り直し（出力のあふれ、stream の項目の適用の失敗、処理済みの報告の失敗）
- 計測の失敗を画面に出すこと

# Requirements

- R-001: client と daemon の接続の状態は、client の通信部分の 1 か所が、CONNECTING・READY・TRANSIENT_FAILURE・IDLE・SHUTDOWN の 5 つの状態として持ち、gRPC Connectivity Semantics の遷移に従う。IDLE は最初に接続する前の状態である。
- R-002: 単発の呼び出し（`invokeClient`）、terminal、`DaemonBoundary` は、接続の状態を読むだけにする。接続の状態を判断する処理、切断の通知、接続全体の作り直し、terminal の接続を追う世代番号を持たない。
- R-003: 単発の呼び出しが失敗しても、接続の状態は変わらない。
- R-004: 購読の stream が切れたとき、bookmark を含めて何も届かない時間が規則の時間を超えたとき、接続の確立が失敗したとき、接続の状態は TRANSIENT_FAILURE になる。#1891 の待ちの後に CONNECTING へ戻り、接続先を Tauri のシェルから受け取り直す。
- R-005: 接続の確立（接続先の受け取り、`GetServerInfo`、接続先のインスタンスとリリースの検証）は、gRPC Connection Backoff の MIN_CONNECT_TIMEOUT（20 秒）の期限を持つ。期限内に終わらなければ、確立の失敗として扱う。期限の値は proto 側の規則に置く。
- R-007: `GetServerInfo` は、接続の確立にだけ使う。
- R-008: 単発の呼び出しは、発行したときの接続の状態によって次のように扱う。terminal への入力も同じに扱う。
  - IDLE: 接続を始め、READY になってから送る
  - CONNECTING: READY になるまで、呼び出しの期限まで待つ
  - READY: そのまま送る
  - TRANSIENT_FAILURE・SHUTDOWN: 即座に失敗させる
- R-009: Current Behavior に挙げた、失敗を画面に出さずに捨てている呼び出し元のうち、利用者の操作、経路で分かれるもの、表示用の計算は、呼び出しの失敗を画面に出す。表示用の計算は、失敗を「結果が無い」と区別して表示する。
- R-010: 計測の呼び出しの失敗は、画面に出さず、アプリの動作を妨げず、ログに残す。
- R-011: terminal の入力は、つなぎ直しの前後と、送った後に届いたか分からない失敗（UNAVAILABLE・DEADLINE_EXCEEDED など）の後で、送った順番どおりに terminal に届くか、失敗として画面に出る。表示の無いまま保留され続けたり捨てられたりしない。
- R-012: 接続が READY でない間は、terminal の入力の失敗によって attachment を張り直さない。
- R-013: `DaemonBoundary` は、Tauri のシェルの状態を、定期の読み直しではなく、変化の通知で受け取る。シェルの状態と client の接続の状態は、別の値として読む。
- R-014: terminal の入力が、送った後に届いたか分からない失敗になったときは、その入力が実行されたかもしれないことが分かる文言で画面に出す。

# Assumptions

なし
