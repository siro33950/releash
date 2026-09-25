# Design 01

## 開始状態

初回。直前の Design は無い。実装の状態は `docs/specs/issues-1888/requirements.md` の Current Behavior を参照する。

- 差分の基準: base ブランチ `main`、派生点は `9194bd57`（branch `feat/issues/1888`）。
- 作業ツリーの未コミットの変更は `docs/specs/issues-1888/` の Requirements・Behavior だけであり、コードは派生点のままである。
- この周までに解消・見送りとなった Thread は無い（open Thread は 0 件）。

## 変える部分

- terminal を購読の対象にする: terminal を `SubscriptionTarget` に加え、購読の開始で現在の状態（再現する画面、その寸法、終了の有無と終了コード）が届き、以後は出力・寸法の変更・終了・入力の受付不可が届くようにする。根拠: R-001「terminal の状態と変化は、その client の他の購読対象と同じ 1 本の購読 stream で届く」、R-003、B-001、B-003、B-004。ルート: 委任（proto の形、コードの配置）。現在の状態を作る契機は D5 に従う。
- terminal の変化を差分で届ける: terminal を `Delivery::Delta` の対象にし、画面側の購読の受け取りが `StateChange.delta` を読んで差分として画面へ反映するようにする。根拠: R-002「terminal の変化は差分として届き、UI は受け取った差分を画面へ反映する」、B-002。ルート: 委任。現在の状態を作る契機は D5 に従う。
- terminal の版番号を出力の番号にする: 購読の土台が振る番号ではなく、terminal が持つ出力の番号を版番号として使う。根拠: R-004「terminal の版番号は出力の番号である」、B-005。ルート: D6。
- つなぎ直しの再開を版番号で行う: 最後に受け取った版番号より後の変化を届け、その版番号から再開できない場合は現在の状態を最初から届ける。根拠: R-005、B-006、B-007。ルート: D5（再開できないときに現在の状態を作る）、D6（版番号の所有）。
- terminal 専用の購読の stream を削除する: `SubscribeTerminalSurfaces` と、それに伴う daemon・client 両端の経路を削除する。根拠: R-006「terminal 専用の購読の stream は無い」、B-008。ルート: D1。
- 出力のひとかたまりごとの受信確認を削除する: `AckTerminalSurfaceOutput` と、`src/lib/terminalSurfaceStream.ts` / `src/hooks/useTerminal.ts` / `src/lib/client.ts` のかたまりごとの確認の呼び出しを削除する。根拠: R-007「出力のひとかたまりごとの受信確認は無い」、B-009。ルート: D1。
- client が処理済みの量を積んで通知単位ごとに知らせる: xterm.js の `write` の完了 callback で処理済みの量を積み、通知単位に達するたびにその量を daemon へ知らせる。根拠: R-008、B-010。ルート: D4（方式と値）、D8（所有者は client の `src/lib/` 配下）。
- daemon が terminal ごとに未確認の量を数え、出力元を一時停止・再開する: 知らせを受け取っていない量が上限を超えたら terminal の出力元を一時停止し、知らせを受けて下限を下回ったら再開する。同じ terminal を複数の client が購読している場合は最も遅れている購読の量で判定する。根拠: R-009、B-011、B-012、B-026。ルート: D4（方式と値）、D7（規則の所有と判定の対象）。
- 送る量の制御の値を変える: 現在の 256K（UTF-16 code unit）の一つの閾値を、上限 100K・下限 5K・通知単位 5K にする。単位は UTF-16 code unit のまま。根拠: R-010、B-013。ルート: D4。
- 全 Session で共有する `attach_lock` を削除し、最初の状態を対象ごとに作る: `src-tauri/src/usecase/terminal_surface/application.rs:21,255,353-356` の lock を無くし、ある terminal の最初の状態の作成が他の terminal の出力・daemon の他の呼び出し・他の terminal の最初の状態の作成を止めないようにする。根拠: R-011、R-012、R-013、B-014、B-015、B-016。ルート: D3（削除の指定。lock の粒度と非同期化の組み方は委任）。
- 送る量の制御の待ち合わせを除去する: `src-tauri/src/adaptor/gateway/terminal_surface/output_flow_control.rs` の `Condvar` による待ち合わせを無くし、出力の順番を守る lock を握ったまま待たないようにする。これにより一時停止中も他の terminal の出力・daemon の他の呼び出し・その terminal の寸法の変更が止まらない。根拠: R-014、R-015、R-016、B-017、B-018、B-019。ルート: D7（gateway は出力元の pause / resume の I/O だけを行う）。
- terminal の状態を返す読み取りの呼び出しを削除する: `GetTerminalSurface` と `src/hooks/useTerminal.ts:365-369` の呼び出しを削除する。根拠: R-017「terminal の状態を返す単発の読み取りの呼び出しは無い」、B-020。ルート: D2。
- terminal を起動する操作が状態を返さないようにする: `GetOrSpawnTerminalSurface` は操作として残し、その結果として terminal の状態を返さないようにする。根拠: R-018、B-021。ルート: D2（削除と存置の指定のみ。返す内容の具体は委任）。
- attach 専用の呼び出しを削除し、購読の開始と停止に一本化する: `AttachTerminalSurface` / `DetachTerminalSurface` を無くし、terminal の購読の開始と停止を他の購読対象と同じ手段（`StartStateSubscription` / `StopStateSubscription`）で行えるようにする。根拠: R-019、B-022。ルート: D3。
- terminal 固有の件数の上限を削除する: `src-tauri/src/domain/terminal_surface/subscriptions.rs:3-4` の `TERMINAL_SUBSCRIPTION_LIMIT` / `TERMINAL_ATTACHMENT_LIMIT` と、購読 1 本あたりの未処理の attach の上限（`src-tauri/src/usecase/terminal_surface/subscriptions.rs:54,83-86`）を無くす。根拠: R-020、B-023。ルート: 委任。
- terminal への入力の経路を attach から切り離す: 現在 `attach` が呼ぶ `activate_input_attachment` / `deactivate_input_attachment`（`src-tauri/src/usecase/terminal_surface/application.rs:366,377,403`）に依存する入力の経路を、attach の削除後も入力が届く形にする。根拠: R-021「terminal への入力は、購読へ移した後も送れる」、B-024。ルート: 委任。
- 購読の土台に差分で進む対象を載せる規則を持たせる: 現在の状態をいつ作るかの規則と、版番号を対象から受け取る規則を土台に持たせる。現在の `publish`（`src-tauri/src/domain/state_subscription/mod.rs:244-271`）は変化のたびに全体の状態を受け取って前回と比較し、版番号を土台が振る。根拠: Requirements の Scope「購読の土台に、差分で進む対象の現在の状態をいつ作るかの規則と、版番号を対象から受け取る規則を持たせること」、R-003、R-004、R-005、R-011、R-012、R-014。ルート: D5、D6。
- 使われなくなるコードと未使用のコードを削除する: この変更で使われなくなる Rust・TypeScript・proto と、それらだけを対象とするテスト、および今回触れたファイルの中で使われていないコードを削除する。`GetOrSpawnTerminalV1` の `restored_from_checkpoint` / `is_new` は `src` の本番コードから使われていない。根拠: R-022、B-025。ルート: 委任。

## 固定するルート

`materials.design.directions` にある D1〜D8 を、指定された粒度のまま固定する。

- D1: terminal 専用の購読の stream（`SubscribeTerminalSurfaces`）と、出力のひとかたまりごとの受信確認（`AckTerminalSurfaceOutput`）を削除する。範囲: proto の定義と daemon・client の両端。粒度: 削除対象の指定のみで、置き換え後の proto の形は委任。理由: 購読を 1 本に一本化する（Request の指定）。関係: R-006、R-007、B-008、B-009。
- D2: terminal の状態を返す読み取りの呼び出し（`GetTerminalSurface`、`src/hooks/useTerminal.ts:365-369`）を削除し、terminal を起動する `GetOrSpawnTerminalSurface` は操作として残す。状態は購読で受け取る。範囲: proto と client の呼び出し。粒度: 削除と存置の指定のみ。理由: 状態の入口を購読へ一本化する（Request の指定）。関係: R-017、R-018、B-020、B-021。
- D3: attach 専用の手続きと、全 Session で共有する `attach_lock`（`src-tauri/src/usecase/terminal_surface/application.rs:21,255,352-356`）を削除する。terminal の最初の状態は、他の購読対象と同じく対象ごとに作る。範囲: attach の RPC・usecase・lock。粒度: 削除の指定のみで、lock の粒度と非同期化の組み方は委任。理由: 最初の状態の作成が全 Session を直列化し、daemon の非同期処理用スレッドと他の terminal の出力を止めるため（Request の指定。#1882 を本 ISSUE に統合）。関係: R-011、R-012、R-013、R-019、B-014、B-015、B-016、B-022。
- D4: 送る量の制御は VS Code の方式と値に合わせる。daemon は terminal ごとに知らせを受け取っていない量を数え、上限 100K を超えたら出力元を一時停止し、下限 5K を下回ったら再開する。client は処理済みの量を積み、通知単位 5K に達するたびにその量を知らせる。単位は UTF-16 code unit、通知単位は下限以下。範囲: 方式と 3 つの値。粒度: 値を固定し、値を保持する場所と表現は委任。理由: Releash と同じ構成の標準実装であり、原案の「下限を下回ったら知らせる」は再開しないか、かたまりごとの通知に退化するため。関係: R-008、R-009、R-010、B-010、B-011、B-012、B-013。
- D5: 差分で進む対象の「現在の状態をいつ作るか」の規則は `domain/state_subscription` の Target が所有する。購読開始時と、版番号から再開できないときにだけ現在の状態を作り、publish のたびには作らない。範囲: 購読の土台の規則の変更のうちこの 1 点。粒度: 所有者と規則の指定のみで、API の形は委任。理由: 出力ごとに画面全体を組み立てて比較する経路を作らないため。関係: R-003、R-005、R-011、R-012、R-014、B-003、B-007、B-014。
- D6: terminal の版番号は `domain/terminal_surface` の `TerminalSurface.latest_sequence` が所有し、購読の土台は差分で進む対象の版番号を対象から受け取る。範囲: 購読の土台の規則の変更のうちこの 1 点。粒度: 所有者の指定のみ。理由: 番号を 2 つ持つと、最初の状態を作るたびに番号空間の対応付けが必要になるため。関係: R-004、R-005、B-005、B-006、B-007。
- D7: 送る量の制御の未確認量・閾値・停止／再開の判定は `domain/terminal_surface` に新設する値オブジェクトが所有する。terminal ごとに 1 つ持ち、同じ terminal を複数の client が購読している場合は最も遅れている購読の量で判定する。`adaptor/gateway/terminal_surface/output_flow_control.rs` は出力元の pause / resume の I/O だけを行い、`Condvar` による待ち合わせを無くす。範囲: 規則の domain への移設と待ち合わせの除去。粒度: 所有者と判定の対象の指定のみで、値オブジェクトの名前と配置は委任。理由: 規則が gateway にあり、待ち合わせが他の terminal の出力・他の呼び出し・寸法の変更を止めるため。関係: R-009、R-014、R-015、R-016、B-011、B-012、B-017、B-018、B-019、B-026。
- D8: 処理済みの量の蓄積と、通知単位に達したかの判定は client（`src/lib/` 配下）が所有する。範囲: client 側のこの 1 点。粒度: 所有者の指定のみ。理由: 処理済みの量は xterm.js の `write` 完了 callback でしか得られず、daemon 側に置くと呼び出しがかたまりごとに残るため。`AGENTS.md`「全てのアプリケーションロジックはサーバに置く。例外なし」に対する、この ISSUE の判断として記録する。関係: R-007、R-008、B-009、B-010。

委任された範囲は次のとおりで、この Design では方法を補わない。

- lock の粒度と、最初の状態を作る処理の非同期化の組み方。
- proto の具体的な形（メッセージ名、field 構成）。
- 非同期処理の組み方。
- コードの配置と、テストの置き方。

## 変えないもの

- terminal の接続が切れている間の入力の扱い（`src/hooks/useTerminal.ts:644-646` で表示無く捨てる）と、terminal の独自の世代番号。理由: #1895 が扱う。
- 購読の stream のつなぎ直し（`src/lib/client.ts` の `ensureStateStream` の 1 秒固定の再接続）。理由: #1891 が扱う。本 ISSUE は購読の受け取りに terminal と差分を足すだけで、つなぎ直しの処理は変えない。
- 処理済みの量の通知を同時実行の枠から外すこと。理由: #1894 が扱う。
- terminal の性能計測の設定（`GetTerminalPerformanceSwitches`、`src/lib/terminalPerformanceSwitches.ts`）の購読への移行。理由: #1898 が扱う。
- 出力の出どころ（PTY から tmux への置き換え）。理由: マイルストーン #98 が扱う。#98 が変えるのは出力の出どころだけで、配信の経路は本 ISSUE で作る購読をそのまま使う。
- terminal を起動する操作（`GetOrSpawnTerminalSurface`）そのもの。操作として残し、廃止しない。理由: 起動は購読では代替できない操作であるため。
- 購読の土台の規則のうち、D5・D6 の 2 点を除く部分。理由: 土台の規則は #1878 が所有する。

## 未確定・リスク

- 購読の土台の保持件数と、terminal の差分の流量の関係。`src-tauri/src/domain/state_subscription/mod.rs:7` の `RETAINED_CHANGES = 64` が、対象ごとの履歴の長さと、購読ごとの送り待ちの上限の両方を決めている（同 `:272-289`）。送る量の制御の上限 100K（D4）の下でも、出力のひとかたまりが小さければ在庫のかたまり数が 64 を容易に超える。超えた購読は `overflowed` になり、版番号からの再開ができず現在の状態の作り直しになる（同 `:76-92`）。この保持件数を terminal の流量に対してどう扱うかは未確定であり、想定が外れると R-005・R-011・R-012・R-014 を満たせない。
- terminal が購読中に作り直されたときの版番号の境目。版番号の epoch は購読の土台が持ち、対象が購読者を失って登録し直されたときにだけ切り替わる（同 `:144-148,296-302`）。購読が続いたまま terminal が終了して起動し直されると、`TerminalSurface.latest_sequence`（D6）は 0 に戻る一方で epoch は変わらない。再開の判定は `version.sequence <= self.version.sequence` を見るため（同 `:76-80`）、client が持つ番号が新しい terminal の到達番号より小さい場合、境目をまたいで再開できてしまう。この境目の扱いは未確定であり、想定が外れると R-005・B-006・B-007 を満たせない。
- `GetOrSpawnTerminalSurface` が返してよい内容の範囲。R-018 は「その操作は terminal の状態を返さない」と定める。現在の `GetOrSpawnTerminalV1` は `session_key`、`restored_from_checkpoint`、`is_new`、`is_exited`、`exit_code` を返し、Requirements の Current Behavior は `session_key` を含む組を `GetTerminalSurface` の「terminal の状態」と記している。`is_exited` と `exit_code` が状態であることは定まっているが、terminal を指す識別子である `session_key` を残せるかは未確定であり、想定が外れると R-018・B-021 を満たせない。

自動判断した箇所は無い。Requirements の Assumptions は「なし」であり、`[DEFERRED]` で人間へ渡した件も無い。
