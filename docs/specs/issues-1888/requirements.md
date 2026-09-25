# Context

- 正本: [#1888 `[03] terminal の出力を購読に移す`](https://github.com/siro33950/releash/issues/1888)
- 最初の周の調査基準は branch `feat/issues/1888` の `9194bd57`。
- アーキテクチャの正本は `AGENTS.md`。アプリケーションロジックはサーバ（daemon）が所有し、client は表示とレイアウト制御、入力受付、サーバの呼び出しと購読、表示用フォーマットだけを担う。
- 本 ISSUE は #1878 に依存する。#1878 が作った購読の土台（`docs/specs/issues-1878/requirements.md`、`docs/specs/issues-1878/behavior.md`）の要求は有効な既存条件であり、本 ISSUE はそれを前提に terminal を購読の対象へ加える。特に次が有効である。
    - 一つの client の全ての購読対象は、その client の 1 本の stream で届く（#1878 R-006）。
    - 購読を開始すると現在の状態が届き、続いて区切りの印が届く（#1878 R-004）。
    - 変更の届け方は、対象ごとに丸ごと送るか差分を送るかを選べる（#1878 R-005）。
    - 対象ごとに daemon が単調に増える版番号を付け、つなぎ直しは最後に受け取った版から再開する。再開できない場合は現在の状態が最初から届く（#1878 R-008、R-009）。
    - 送り待ちは購読ごとに持ち、溢れた購読だけが版からの再開になる（#1878 R-010）。
    - 購読の開始と停止は単発の呼び出しで行う（#1878 R-007）。一つの client が持てる購読の数に上限を設けない（#1878 R-012）。
- #1882（`terminal の attach が全 Session で直列化され、snapshot 作成中に全 terminal の出力処理を止める`）は本 ISSUE にまとめられている。#1882 が挙げた現象は本 ISSUE の Current Behavior に取り込む。
- 送る量の制御は xterm.js の公式の方法（ https://xtermjs.org/docs/guides/flowcontrol/ ）に従う。ガイドの内容は次のとおり。
    - 未処理の量（watermark）を数え、上限（HIGH）と下限（LOW）と比べる。`write` の完了 callback を処理済みの合図として使う。
    - 送り手は watermark が HIGH を超えたら一時停止し、callback により LOW を下回ったら再開する。
    - ガイドの例は HIGH=100K、LOW=10K であり、「HIGH は 500K を超えないこと」と述べている。最適値は状況によって変わるとも述べている。
    - 量の単位はガイドの `chunk.length`、すなわち UTF-16 code unit である。調査基準の実装も同じ単位で数える（`src-tauri/src/adaptor/gateway/terminal_surface/event_hub.rs:210` の `data.encode_utf16().count()`）。
    - ガイドの例は送り手と xterm.js の間に RPC が無いプロセス内の構成であり、処理済みをまとめて知らせる方法を定めていない。LOW は再開の閾値としてだけ使う。
- 送る量の制御の方式と値は、Releash と同じ構成（サーバが出力元を所有し、xterm.js の client と RPC で繋ぐ）の標準実装である VS Code に合わせる。VS Code の内容は次のとおり。
    - `FlowControlConstants` は `HighWatermarkChars = 100000`、`LowWatermarkChars = 5000`、`CharCountAckSize = 5000` である（`src/vs/platform/terminal/common/terminal.ts`）。`CharCountAckSize` には「This must be less than or equal to LowWatermarkChars or the terminal max never unpause.」という制約が付く。
    - サーバは知らせを受け取っていない量を数え、上限を超えたら出力元を一時停止し、知らせを受けて下限を下回ったら再開する（`src/vs/platform/terminal/node/terminalProcess.ts`）。
    - client は xterm.js の `write` の完了 callback で処理済みの量を積み、通知単位に達するたびにその単位でサーバへ知らせる（`src/vs/workbench/contrib/terminal/browser/terminalProcessManager.ts` の `AckDataBufferer`、`src/vs/workbench/contrib/terminal/browser/terminalInstance.ts` の `_writeProcessData`）。通知の契機は処理済みの量の蓄積であり、未処理の量が下限を下回ったことではない。
    - 量の単位は `data.length`、すなわち UTF-16 code unit である。
    - 上の 3 つの値は `src/vs/platform/terminal/common/terminal.ts` の共有の定数であり、サーバと client が同じ定義を読む。Releash はサーバが Rust、client が TypeScript に分かれており、同じ定義を直接共有できない。
- マイルストーン [02] UI と daemon の間の通信の仕組みを一本化する（ https://github.com/siro33950/releash/milestone/99 ）の中で、本 ISSUE が扱わない事項は次が担う。#1891（client のつなぎ直し）、#1894（同時実行の枠を優先度で分ける）、#1895（接続の状態と terminal の世代番号）、#1898（設定とアプリ全体の表示の購読への移行）。
- マイルストーン [05] Session と Terminal の実行を tmux に任せる（ https://github.com/siro33950/releash/milestone/98 ）は、自前の PTY と画面の扱いを削除し、出力を tmux から受け取って配信の経路へ流すと決めている。このマイルストーンより後に行うため、変わるのは出力の出どころだけであり、配信の経路は本 ISSUE で作る購読をそのまま使う。
- 現行実装の確認先: `proto/client.proto`、`src-tauri/src/adaptor/controller/api/client_service.rs`、`src-tauri/src/adaptor/controller/api/state_subscription.rs`、`src-tauri/src/domain/state_subscription/mod.rs`、`src-tauri/src/domain/state_subscription/subscription_target.rs`、`src-tauri/src/usecase/state_subscription.rs`、`src-tauri/src/domain/terminal_surface/subscriptions.rs`、`src-tauri/src/usecase/terminal_surface/subscriptions.rs`、`src-tauri/src/usecase/terminal_surface/application.rs`、`src-tauri/src/adaptor/gateway/terminal_surface/runtime_gateway_impl.rs`、`src-tauri/src/adaptor/gateway/terminal_surface/event_hub.rs`、`src-tauri/src/adaptor/gateway/terminal_surface/output_flow_control.rs`、`src/lib/client.ts`、`src/lib/terminalSurfaceStream.ts`、`src/hooks/useTerminal.ts`
- 正本が挙げた行番号は main `42be41e0` 時点のものであり、調査基準では一部がずれている。調査基準での位置は次のとおりで、指している内容は一致する。`src/lib/client.ts:279-291`（画面の購読の受け取り）は `src/lib/client.ts:313-338`、`src/lib/client.ts:608-616`（受信確認の呼び出し）は `src/lib/client.ts:645-654`、`src-tauri/src/adaptor/controller/api/client_service.rs:76-99`（attach の handler）は同ファイル `:78-105`、`src/hooks/useTerminal.ts:650`（接続が切れている間の入力の扱い）は同ファイル `:644-646`。`src/hooks/useTerminal.ts:365-369`（`get_terminal_surface` の呼び出し）は調査基準でも同じ位置である。

# Outcome

対象者は、Releash の terminal を使う利用者と、UI と daemon の間の通信を実装・保守する開発者である。

現在、terminal だけが購読とは別の stream で配信されている。terminal の最初の状態を作る処理は全 Session で 1 本の lock により直列化され、その間、全 terminal の出力処理が止まり、daemon の非同期処理用のスレッドも塞がる。送る量の制御は出力のひとかたまりごとの受信確認で行われ、未確認の量が上限を超えると出力の配信が期限なく待ち、その間はその terminal の寸法の変更も受け付けられない。terminal の状態は購読ではなく単発の読み取りの呼び出しでも取れる。

変更後は、terminal も他の対象と同じ 1 本の購読 stream で届く。terminal は差分（出力）で変化を受け取る唯一の対象になり、つなぎ直しは最後に受け取った出力の番号から再開する。terminal の最初の状態は対象ごとに作られ、他の terminal の出力も daemon の他の呼び出しも止めない。送る量の制御は xterm.js の公式の方法に従い、UI が処理済みの量をまとめて知らせ、daemon が出力元を一時停止・再開する。terminal の状態を返す単発の読み取りの呼び出しは無くなり、terminal を起動する操作だけが残る。

# Current Behavior

調査基準 `9194bd57` のコードで確認した挙動である。

## terminal は購読とは別の stream で配信される

- 購読の土台は `OpenStateStream` / `StartStateSubscription` / `StopStateSubscription`（`proto/client.proto:2945-2947`）である。terminal はこれとは別に `SubscribeTerminalSurfaces`（`proto/client.proto:2958`）で snapshot と番号付きの出力を流す。
- 購読の対象は `SubscriptionTarget`（`src-tauri/src/domain/state_subscription/subscription_target.rs:4-21`）に列挙されており、terminal に対応する対象は無い。
- terminal の購読は 16 件まで、attach は daemon 全体で 16 件までである（`src-tauri/src/domain/terminal_surface/subscriptions.rs:3-4`、同 `:54-56,69-71`）。購読 1 本あたりの未処理の attach も 16 件までである（`src-tauri/src/usecase/terminal_surface/subscriptions.rs:54,83-86`）。

## 購読の土台は差分を送れるが、差分を使う対象が無い

- 土台は対象ごとに `Delivery::Full` と `Delivery::Delta` を選べ、`publish` は差分を受け取れる（`src-tauri/src/domain/state_subscription/mod.rs:22-26,240-271`）。
- 実際に登録される対象は全て `Delivery::Full` である。起動時に登録する Repository のパス一覧（`src-tauri/src/usecase/state_subscription.rs:56-61`）も、購読の開始時に登録する対象（`src-tauri/src/domain/state_subscription/mod.rs:195`）も同じである。`publish` の呼び出しは全て差分を `None` で渡す（`src-tauri/src/usecase/state_subscription.rs:184`）。
- 画面の購読の受け取りは、`change` の `delta` を読まず、`payload` をそのまま値として扱う（`src/lib/client.ts:327-338`）。

## terminal の最初の状態を作る処理は全 Session で直列化される

- attach は `attach_lock`（`std::sync::Mutex`）を握って全 Session を通して直列化される（`src-tauri/src/usecase/terminal_surface/application.rs:21,255,352-356`）。
- lock を握ったまま最初の状態を作る。最初の状態の作成は、その terminal の emulator の lock と、gateway 全体で共有する `registry` の lock を握って画面全体を組み立てる（`src-tauri/src/adaptor/gateway/terminal_surface/runtime_gateway_impl.rs:113-135`）。
- 同じ `registry` の lock は、全 terminal の出力処理も取る（同 `:245-260`）。
- attach の RPC は async の handler から同期で呼ばれ、`spawn_blocking` を使わない（`src-tauri/src/adaptor/controller/api/client_service.rs:78-105`）。

## 送る量の制御は出力のひとかたまりごとの受信確認で行われる

- UI は出力のひとかたまりを xterm.js が処理し終えるたびに受信確認を送る（`src/lib/terminalSurfaceStream.ts:114-115`、`src/hooks/useTerminal.ts` の `acknowledgeOutput`、`src/lib/client.ts:645-654`）。受信確認は `AckTerminalSurfaceOutput`（`proto/client.proto:2951`）であり、他のコマンドと同じ同時実行の枠を使う。
- 未確認の出力が 256K（UTF-16 code unit）を超えると、確認が届くまで期限なく待つ（`src-tauri/src/adaptor/gateway/terminal_surface/output_flow_control.rs:5,49-66`）。
- 待つ間、その terminal の出力の順番を守る lock を握っている（`src-tauri/src/adaptor/gateway/terminal_surface/runtime_gateway_impl.rs:192-207,245-247`、`src-tauri/src/adaptor/gateway/terminal_surface/event_hub.rs:209-210`）。同じ lock を寸法の変更も取る（`runtime_gateway_impl.rs:993`）。

## terminal の状態を返す単発の読み取りの呼び出しがある

- `GetTerminalSurface`（`proto/client.proto:2997`）は `TerminalSurfaceSummaryV1`（`session_key`、`is_exited`、`exit_code`。同 `:1410-1414`）を返す。UI は既存の terminal へつなぐときにこれを呼ぶ（`src/hooks/useTerminal.ts:365-369`）。
- terminal を起動する `GetOrSpawnTerminalSurface`（`proto/client.proto:2989`）は `GetOrSpawnTerminalV1`（`session_key`、`restored_from_checkpoint`、`is_new`、`is_exited`、`exit_code`。同 `:1127-1133`）を返す。UI は `session_key` と `is_exited` を使う（`src/hooks/useTerminal.ts:388,398`）。`restored_from_checkpoint` と `is_new` は `src` の本番コードから使われておらず、`src/hooks/useTerminal.test.ts` だけが参照する。

## terminal の購読で届く内容

- 届くのは最初の状態と 4 種類の変化である。最初の状態は再現する画面、その寸法、出力の番号、終了の有無と終了コードを持つ（`proto/client.proto:2759-2767`）。変化は出力、寸法の変更、終了、入力の受付不可である（同 `:2750-2757`、`src/lib/terminalSurfaceStream.ts:19-46`）。
- 出力の番号は `registry` が振る番号であり、出力、寸法の変更、終了のいずれでも進む（`src-tauri/src/adaptor/gateway/terminal_surface/runtime_gateway_impl.rs:251-253,296,1002`）。

# Scope / Non-goals

## 変更する対象

- terminal の出力の配信。購読の対象としての terminal と、画面側の差分の受け取り。
- terminal の最初の状態の作り方と、それに関わる lock の範囲。
- 送る量の制御。出力のひとかたまりごとの受信確認をやめ、処理済みの量の通知と、出力元の一時停止・再開にする。
- terminal 専用の購読の stream（`SubscribeTerminalSurfaces`）の削除。
- 出力のひとかたまりごとの受信確認（`AckTerminalSurfaceOutput`）の削除。
- terminal の状態を返す読み取りの呼び出し（`GetTerminalSurface`）の削除。
- attach 専用の手続きと、全 Session で共有する `attach_lock` の削除。
- terminal の購読と attach の件数の上限の削除。
- 入力が受け付けられなかったことを購読で配信する経路（購読の差分、proto のメッセージ、画面側の分岐）の削除。
- 購読の土台に、差分で進む対象の現在の状態をいつ作るかの規則と、版番号を対象から受け取る規則を持たせること。
- 購読の土台の、差分で進む対象の購読ごとの送り待ちの上限を、件数から量へ変えること。
- この変更で使われなくなるコードの削除と、今回変更したファイルの中で使われていないコードの削除。

## 変更しない対象

- terminal の接続が切れている間の入力の扱い（`src/hooks/useTerminal.ts:644-646` で表示無く捨てる）と、terminal の独自の世代番号。#1895 が扱う。
- 購読の stream のつなぎ直し（`src/lib/client.ts` の `ensureStateStream` の 1 秒固定の再接続）。#1891 が扱う。本 ISSUE は購読の受け取りに terminal と差分を足すだけで、つなぎ直しの処理は変えない。
- 入力が受け付けられなかったときの画面側の扱い（エラーの通知と terminal のつなぎ直し）。入力の呼び出しの失敗の応答に対する既存の扱いをそのまま使う。
- 処理済みの量の通知を同時実行の枠から外すこと。#1894 が扱う。
- terminal の性能計測の設定（`GetTerminalPerformanceSwitches`、`src/lib/terminalPerformanceSwitches.ts`）の購読への移行。#1898 が扱う。
- terminal を起動する操作そのもの（`GetOrSpawnTerminalSurface`）の廃止。操作として残す。
- 出力の出どころ（PTY から tmux への置き換え）。マイルストーン [05] Session と Terminal の実行を tmux に任せる が扱う。
- 対象ごとに保持する変化の履歴の窓の尺度。件数のまま残す。
- 購読の土台の規則のうち、差分で進む対象を載せるのに必要な範囲を超える変更。その範囲を除き、#1878 が定めたものを変えない。

# Requirements

- R-001: terminal の状態と変化は、その client の他の購読対象と同じ 1 本の購読 stream で届く。
- R-002: terminal の変化は差分として届き、UI は受け取った差分を画面へ反映する。
- R-003: terminal の購読では、最初に現在の状態（再現する画面、その寸法、終了の有無と終了コード）が届き、以後は出力、寸法の変更、終了が届く。
- R-004: terminal の版番号は出力の番号である。
- R-005: terminal の購読をつなぎ直すときは、最後に受け取った版番号から再開する。その版番号から再開できない場合は、現在の状態が最初から届く。
- R-006: terminal 専用の購読の stream は無い。
- R-007: 出力のひとかたまりごとの受信確認は無い。
- R-008: 送る量の制御の通知単位は daemon が定め、terminal の購読の最初の状態として client へ届く。UI は、terminal の出力を処理し終えた量を積み、受け取った通知単位に達するたびに、その量を daemon へ知らせる。
- R-009: daemon は terminal ごとに、知らせを受け取っていない出力の量を数える。同じ terminal を複数の client が購読している場合は、最も遅れている購読の量を用いる。その量が上限を超えたら terminal の出力元を一時停止し、知らせを受けて下限を下回ったら再開する。
- R-010: 送る量の制御の量の単位は UTF-16 code unit であり、上限は 100K、下限は 5K、通知単位は 5K である。通知単位は下限以下である。
- R-011: ある terminal の最初の状態を作っている間も、他の terminal の出力は届き続ける。
- R-012: ある terminal の最初の状態を作っている間も、daemon は他の呼び出しに応答する。
- R-013: 複数の terminal の最初の状態を同時に作れる。
- R-014: ある terminal の出力元が送る量の制御で一時停止している間も、他の terminal の出力は届き続ける。
- R-015: ある terminal の出力元が送る量の制御で一時停止している間も、daemon は他の呼び出しに応答する。
- R-016: ある terminal の出力元が送る量の制御で一時停止している間も、その terminal の寸法の変更は受け付けられる。
- R-017: terminal の状態を返す単発の読み取りの呼び出しは無い。
- R-018: terminal を起動する操作は使える。その操作は、その terminal を指す識別子だけを返し、terminal の状態は返さない。
- R-019: terminal の購読の開始と停止は、他の購読対象と同じ手段で行える。terminal 専用の attach の呼び出しは無い。
- R-020: terminal の購読の数と、同時に表示できる terminal の数に、terminal 固有の上限は無い。
- R-021: terminal への入力は、購読へ移した後も送れる。
- R-022: この変更で使われなくなるコードと、この変更で触れたファイルの中で使われていないコードは無い。対象は Rust、TypeScript、proto、およびそれらだけを対象とするテストである。
- R-023: terminal の購読では、入力が受け付けられなかったことは届かない。入力が受け付けられなかったことは、入力の呼び出しの応答で伝わる。

# Assumptions / Open Questions

なし。
