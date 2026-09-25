# Design 02

## 開始状態

- 差分の基準: base ブランチ `main`、派生点は `9194bd57`（branch `feat/issues/1888`）。
- 直前の Design は `docs/specs/issues-1888/design-01.md`。
- 作業ツリーの未コミットの変更は `docs/specs/issues-1888/` の Requirements・Behavior・Design だけであり、コードは派生点のままである。design-01 の「変える部分」は一つも実装されていないため、実装の状態は `docs/specs/issues-1888/requirements.md` の Current Behavior のままである。
- この周までに解消・見送りとなった Thread は無い。open Thread は 2 件で、どちらも修正と決まり `[FIX_POLICY]` が付いた状態で open のまま残る。`d5046d06-163d-45d6-aa4f-ccf41acd6f8a`（送る量の制御の通知単位の所有者）、`92a33786-cca0-4bd5-bb28-2fde6fe92617`（terminal が作り直されたときの版番号の境目）。`[REJECTED]` と `[DEFERRED]` で閉じた Thread は無い。

## 変える部分

- terminal を購読の対象にする: terminal を `SubscriptionTarget` に加え、購読の開始で現在の状態（再現する画面、その寸法、終了の有無と終了コード）が届き、以後は出力・寸法の変更・終了・入力の受付不可が届くようにする。根拠: R-001「terminal の状態と変化は、その client の他の購読対象と同じ 1 本の購読 stream で届く」、R-003、B-001、B-003、B-004。ルート: 委任（proto の形、コードの配置）。現在の状態を作る契機は D5 に従う。
- terminal の変化を差分で届ける: terminal を `Delivery::Delta` の対象にし、画面側の購読の受け取りが `StateChange.delta` を読んで差分として画面へ反映するようにする。根拠: R-002「terminal の変化は差分として届き、UI は受け取った差分を画面へ反映する」、B-002。ルート: 委任。現在の状態を作る契機は D5 に従う。
- terminal の版番号を出力の番号にする: 購読の土台が振る番号ではなく、terminal が持つ出力の番号を版番号として使う。根拠: R-004「terminal の版番号は出力の番号である」、B-005。ルート: D6。
- つなぎ直しの再開を版番号で行う: 最後に受け取った版番号より後の変化を届け、その版番号から再開できない場合は現在の状態を最初から届ける。根拠: R-005、B-006、B-007。ルート: D5（再開できないときに現在の状態を作る）、D6（版番号の所有）。
- terminal が作り直されたときの版番号の境目を再開可能と判定しない: 購読の対象が同じまま terminal が終了して起動し直された後に、作り直しより前に受け取った版番号を指定して購読し直した場合、再開せず現在の状態を最初から届ける。現在は `Target::resume` が epoch の一致と sequence の範囲だけで再開可否を決め（`src-tauri/src/domain/state_subscription/mod.rs:74-93`）、epoch の世代は購読者を失った対象を取り除くときにだけ進む（同 `:144-148,288-301`）。一方 `TerminalSurface::with_checkpoint` は `latest_sequence` を checkpoint の値から初期化する（`src-tauri/src/domain/terminal_surface/entities/terminal_surface.rs:51-68`）。根拠: R-005 の 2 文目、B-007、Thread `92a33786-cca0-4bd5-bb28-2fde6fe92617`。ルート: 委任（D6 と、購読の土台への変更を D5・D6・D11 の 3 点に限る制約の範囲内で行う）。
- terminal 専用の購読の stream を削除する: `SubscribeTerminalSurfaces` と、それに伴う daemon・client 両端の経路を削除する。根拠: R-006「terminal 専用の購読の stream は無い」、B-008。ルート: D1。
- 出力のひとかたまりごとの受信確認を削除する: `AckTerminalSurfaceOutput` と、`src/lib/terminalSurfaceStream.ts` / `src/hooks/useTerminal.ts` / `src/lib/client.ts` のかたまりごとの確認の呼び出しを削除する。根拠: R-007「出力のひとかたまりごとの受信確認は無い」、B-009。ルート: D1。
- 送る量の制御の通知単位を daemon が定め、terminal の購読の最初の状態として client へ渡す: client 側に通知単位の定数と判定の規則を残さない。根拠: R-008「送る量の制御の通知単位は daemon が定め、terminal の購読の最初の状態として client へ届く」、B-027、Thread `d5046d06-163d-45d6-aa4f-ccf41acd6f8a`。ルート: D8（所有者と渡す場所。proto の形、値の保持場所と表現は委任）。
- client が処理済みの量を積んで、受け取った通知単位ごとに知らせる: xterm.js の `write` の完了 callback で処理済みの量を積み、daemon から受け取った通知単位に達するたびにその量を daemon へ知らせる。根拠: R-008 の 2 文目、B-010。ルート: D4（方式と値）、D8（所有者）。
- daemon が terminal ごとに未確認の量を数え、出力元を一時停止・再開する: 知らせを受け取っていない量が上限を超えたら terminal の出力元を一時停止し、知らせを受けて下限を下回ったら再開する。同じ terminal を複数の client が購読している場合は最も遅れている購読の量で判定する。根拠: R-009、B-011、B-012、B-026。ルート: D4（方式と値）、D7（規則の所有と判定の対象）。
- 送る量の制御の値を変える: 現在の 256K（UTF-16 code unit）の一つの閾値（`src-tauri/src/adaptor/gateway/terminal_surface/output_flow_control.rs:5,49-66`）を、上限 100K・下限 5K・通知単位 5K にする。単位は UTF-16 code unit のまま。根拠: R-010、B-013。ルート: D4、D9。
- 全 Session で共有する `attach_lock` を削除し、最初の状態を対象ごとに作る: `src-tauri/src/usecase/terminal_surface/application.rs:21,255,352-356` の lock を無くし、ある terminal の最初の状態の作成が他の terminal の出力・daemon の他の呼び出し・他の terminal の最初の状態の作成を止めないようにする。根拠: R-011、R-012、R-013、B-014、B-015、B-016。ルート: D3（削除の指定。lock の粒度と非同期化の組み方は委任）。
- 送る量の制御の待ち合わせを除去する: `src-tauri/src/adaptor/gateway/terminal_surface/output_flow_control.rs` の `Condvar` による待ち合わせを無くし、出力の順番を守る lock を握ったまま待たないようにする。これにより一時停止中も他の terminal の出力・daemon の他の呼び出し・その terminal の寸法の変更が止まらない。根拠: R-014、R-015、R-016、B-017、B-018、B-019。ルート: D7（gateway は出力元の pause / resume の I/O だけを行う）。
- terminal の状態を返す読み取りの呼び出しを削除する: `GetTerminalSurface` と `src/hooks/useTerminal.ts:365-369` の呼び出しを削除する。根拠: R-017「terminal の状態を返す単発の読み取りの呼び出しは無い」、B-020。ルート: D2。
- terminal を起動する操作が識別子だけを返すようにする: `GetOrSpawnTerminalSurface` は操作として残し、`GetOrSpawnTerminalV1`（`proto/client.proto:1127-1133`）を `session_key` だけにする。根拠: R-018「その操作は、その terminal を指す識別子だけを返し、terminal の状態は返さない」、R-022、B-021。ルート: D2（削除と存置）、D10（残す field と削る field）。
- attach 専用の呼び出しを削除し、購読の開始と停止に一本化する: `AttachTerminalSurface` / `DetachTerminalSurface` を無くし、terminal の購読の開始と停止を他の購読対象と同じ手段（`StartStateSubscription` / `StopStateSubscription`）で行えるようにする。根拠: R-019、B-022。ルート: D3。
- terminal 固有の件数の上限を削除する: `src-tauri/src/domain/terminal_surface/subscriptions.rs:3-4` の `TERMINAL_SUBSCRIPTION_LIMIT` / `TERMINAL_ATTACHMENT_LIMIT` と、購読 1 本あたりの未処理の attach の上限（`src-tauri/src/usecase/terminal_surface/subscriptions.rs:54,83-86`）を無くす。根拠: R-020、B-023。ルート: 委任。
- terminal への入力の経路を attach から切り離す: 現在 `attach` が呼ぶ `activate_input_attachment` / `deactivate_input_attachment`（`src-tauri/src/usecase/terminal_surface/application.rs:366,377,403`）に依存する入力の経路を、attach の削除後も入力が届く形にする。根拠: R-021「terminal への入力は、購読へ移した後も送れる」、B-024。ルート: 委任。
- 購読の土台に差分で進む対象を載せる規則を持たせる: 現在の状態をいつ作るかの規則と、版番号を対象から受け取る規則を土台に持たせる。現在の `publish`（`src-tauri/src/domain/state_subscription/mod.rs:244-271`）は変化のたびに全体の状態を受け取って前回と比較し、版番号を土台が振る。根拠: Requirements の Scope「購読の土台に、差分で進む対象の現在の状態をいつ作るかの規則と、版番号を対象から受け取る規則を持たせること」、R-003、R-004、R-005、R-011、R-012、R-014。ルート: D5、D6。
- 購読ごとの送り待ちの上限を件数から量へ変える: 差分で進む対象の、購読ごとの送り待ちの上限（`src-tauri/src/domain/state_subscription/mod.rs:277-286` が `RETAINED_CHANGES` を件数の上限として使う箇所）を量にし、送る量の制御の上限と同じ尺度にする。根拠: Requirements の Scope「購読の土台の、差分で進む対象の購読ごとの送り待ちの上限を、件数から量へ変えること」、R-005、R-011、R-012、R-014。ルート: D11（尺度と変えない層。量の値、上限の表現、実装の配置は委任）。
- 使われなくなるコードと未使用のコードを削除する: この変更で使われなくなる Rust・TypeScript・proto と、それらだけを対象とするテスト、および今回触れたファイルの中で使われていないコードを削除する。`GetOrSpawnTerminalV1` の `restored_from_checkpoint` / `is_new` は `src` の本番コードから使われておらず、`src/hooks/useTerminal.test.ts` だけが参照する。根拠: R-022、B-025。ルート: 委任。

## 固定するルート

D1〜D7 は design-01 で固定したものを今周も維持する。範囲・粒度・理由は design-01 の記載のまま変えない。D8 は design-01 の D8 を置き換える。D9〜D11 は今周に固定した。

- D1: terminal 専用の購読の stream（`SubscribeTerminalSurfaces`）と、出力のひとかたまりごとの受信確認（`AckTerminalSurfaceOutput`）を削除する。範囲: proto の定義と daemon・client の両端。粒度: 削除対象の指定のみで、置き換え後の proto の形は委任。理由: 購読を 1 本に一本化する（Request の指定）。関係: R-006、R-007、B-008、B-009。design-01 の D1 を維持する。
- D2: terminal の状態を返す読み取りの呼び出し（`GetTerminalSurface`、`src/hooks/useTerminal.ts:365-369`）を削除し、terminal を起動する `GetOrSpawnTerminalSurface` は操作として残す。状態は購読で受け取る。範囲: proto と client の呼び出し。粒度: 削除と存置の指定のみ。理由: 状態の入口を購読へ一本化する（Request の指定）。関係: R-017、R-018、B-020、B-021。design-01 の D2 を維持する。
- D3: attach 専用の手続きと、全 Session で共有する `attach_lock`（`src-tauri/src/usecase/terminal_surface/application.rs:21,255,352-356`）を削除する。terminal の最初の状態は、他の購読対象と同じく対象ごとに作る。範囲: attach の RPC・usecase・lock。粒度: 削除の指定のみで、lock の粒度と非同期化の組み方は委任。理由: 最初の状態の作成が全 Session を直列化し、daemon の非同期処理用スレッドと他の terminal の出力を止めるため（Request の指定。#1882 を本 ISSUE に統合）。関係: R-011、R-012、R-013、R-019、B-014、B-015、B-016、B-022。design-01 の D3 を維持する。
- D4: 送る量の制御は VS Code の方式と値に合わせる。daemon は terminal ごとに知らせを受け取っていない量を数え、上限 100K を超えたら出力元を一時停止し、下限 5K を下回ったら再開する。client は処理済みの量を積み、通知単位 5K に達するたびにその量を知らせる。単位は UTF-16 code unit、通知単位は下限以下。範囲: 方式と 3 つの値。粒度: 値を固定し、値を保持する場所と表現は委任。理由: Releash と同じ構成の標準実装であり、原案の「下限を下回ったら知らせる」は再開しないか、かたまりごとの通知に退化するため。関係: R-008、R-009、R-010、B-010、B-011、B-012、B-013。design-01 の D4 を維持する。
- D5: 差分で進む対象の「現在の状態をいつ作るか」の規則は `domain/state_subscription` の Target が所有する。購読開始時と、版番号から再開できないときにだけ現在の状態を作り、publish のたびには作らない。範囲: 購読の土台の規則の変更のうちこの 1 点。粒度: 所有者と規則の指定のみで、API の形は委任。理由: 出力ごとに画面全体を組み立てて比較する経路を作らないため。関係: R-003、R-005、R-011、R-012、R-014、B-003、B-007、B-014。design-01 の D5 を維持する。
- D6: terminal の版番号は `domain/terminal_surface` の `TerminalSurface.latest_sequence` が所有し、購読の土台は差分で進む対象の版番号を対象から受け取る。範囲: 購読の土台の規則の変更のうちこの 1 点。粒度: 所有者の指定のみ。理由: 番号を 2 つ持つと、最初の状態を作るたびに番号空間の対応付けが必要になるため。関係: R-004、R-005、B-005、B-006、B-007。design-01 の D6 を維持する。
- D7: 送る量の制御の未確認量・閾値・停止／再開の判定は `domain/terminal_surface` に新設する値オブジェクトが所有する。terminal ごとに 1 つ持ち、同じ terminal を複数の client が購読している場合は最も遅れている購読の量で判定する。`adaptor/gateway/terminal_surface/output_flow_control.rs` は出力元の pause / resume の I/O だけを行い、`Condvar` による待ち合わせを無くす。範囲: 規則の domain への移設と待ち合わせの除去。粒度: 所有者と判定の対象の指定のみで、値オブジェクトの名前と配置は委任。理由: 規則が gateway にあり、待ち合わせが他の terminal の出力・他の呼び出し・寸法の変更を止めるため。関係: R-009、R-014、R-015、R-016、B-011、B-012、B-017、B-018、B-019、B-026。design-01 の D7 を維持する。
- D8: 送る量の制御の通知単位は daemon（サーバ）が所有し、terminal の購読の最初の状態として client へ渡す。client は受け取った値と、xterm.js の `write` 完了 callback で積んだ処理済みの量を比べて daemon へ知らせるだけで、通知単位の定数と判定の規則を持たない。範囲: 所有者と渡す場所の 2 点。粒度: 所有者（サーバ）と渡す場所（terminal の購読の最初の状態）だけを固定し、proto の形、値の保持場所と表現は委任。通知単位を `GetTerminalPerformanceSwitches` 経由で渡すことは #1898 の範囲であり採らない。理由: `AGENTS.md`「全てのアプリケーションロジックはサーバ（daemon）に置く。例外なし。」に例外を作らないため。処理済みの量は xterm.js の `write` 完了 callback でしか得られないため観測と送信は client に残るが、閾値の source of truth はサーバに置く。VS Code は `FlowControlConstants` を `src/vs/platform/terminal/common/terminal.ts` の共有の定数に置いてサーバと client が同じ定義を読むが、Rust と TypeScript に分かれた Releash では共有はサーバから渡す形になる。関係: R-008、B-010、B-027。design-01 の D8（所有者は client、`AGENTS.md` への例外として記録）を置き換える。
- D9: 送る量の制御の 3 つの値は、上限 100K、下限 5K、通知単位 5K、単位は UTF-16 code unit、通知単位は下限以下とする。範囲: 3 つの値と単位。粒度: 値を固定し、値を保持する場所と表現は委任。理由: upstream の `src/vs/platform/terminal/common/terminal.ts` で `HighWatermarkChars = 100000` / `LowWatermarkChars = 5000` / `CharCountAckSize = 5000` と、`CharCountAckSize` は `LowWatermarkChars` 以下でなければ terminal が再開しない旨の注記を確認した。VS Code はサーバが出力元を所有して xterm.js の client と RPC で繋ぐ構成で Releash と同じであり、通知単位を定める標準は VS Code だけである。xterm.js のガイドの例（HIGH=100K / LOW=10K）は通知単位を定めておらず、下限に合わせた通知単位は標準からは出てこない自前の判断になるため採らない。関係: R-010、B-013。値は D4 が持つものと同じであり、D4 の範囲・粒度・理由は変えない。
- D10: `GetOrSpawnTerminalV1` は `session_key` だけを返し、`is_exited`、`exit_code`、`restored_from_checkpoint`、`is_new` を削る。範囲: この proto メッセージの field 構成。粒度: 残す field と削る field の指定。理由: `session_key` は terminal を指す識別子であって状態ではない。標準（AIP-133）は作成の呼び出しが作ったリソースを返すと定めるが、R-018 が状態を返さないことを固定済みであり、その条件下で標準から残るのは呼び出し側が作ったものを指せる識別子だけである。購読の最初の状態 `TerminalSnapshot` は既に `session_key` を持ち（`proto/client.proto:2760`）、画面側も snapshot の値で上書きするが（`src/hooks/useTerminal.ts:455-456`）、購読を張る前に component が外れる経路（同 `:398` の `onTerminalReady`）では起動の結果の識別子が要る。`restored_from_checkpoint` と `is_new` は本番コードから使われておらず（`src/hooks/useTerminal.test.ts` だけが参照）、R-022 の削除対象に当たる。関係: R-018、R-022、B-021。
- D11: 購読の土台の、差分で進む対象の購読ごとの送り待ちの上限を、件数から量へ変え、送る量の制御の上限と同じ尺度にする。範囲: 購読の土台の規則の変更のうちこの 1 点（土台への変更は D5・D6 と合わせて 3 点になる）。粒度: 尺度（件数から量へ）と、変えない層（対象ごとの履歴の窓は件数のまま）だけを固定し、量の値、上限の表現、実装の配置は委任。差分の結合（coalescing）は導入しない。理由: `RETAINED_CHANGES = 64`（`src-tauri/src/domain/state_subscription/mod.rs:7`）が対象ごとの履歴の窓と購読ごとの送り待ちの上限を兼ねており、送る量の制御が量（100K code unit）で効く一方で送り待ちが件数で効くため、平均 1.6K code unit より小さいかたまりが続くと量より先に件数の上限に当たり、その購読が `overflowed` になって版番号から再開できず現在の状態の作り直しになる（同 `:272-289`、`:76-92`）。在庫の境界を量で持つのが標準である（RFC 9113 §5.2 の flow control window は octet 単位で frame 数の窓を持たず、xterm.js のガイドの watermark は `chunk.length`、VS Code は文字数だけで境界を持ち購読ごとの件数上限を持たない）。VS Code の `src/vs/platform/terminal/common/terminalDataBuffering.ts` の結合は throttle で flush して RPC 回数を減らす仕組みであり在庫上限の代替ではないため、結合は使わない。関係: R-005、R-011、R-012、R-014。

委任された範囲は次のとおりで、この Design では方法を補わない。

- lock の粒度と、最初の状態を作る処理の非同期化の組み方。
- proto の具体的な形（メッセージ名、field 構成）。サーバが通知単位を client へ渡す field の形を含む。ただし `GetOrSpawnTerminalV1` の返す field は D10 で固定済み。
- 送る量の制御の値を保持する場所と表現、および `domain/terminal_surface` に新設する値オブジェクトの名前と配置。
- 購読ごとの送り待ちの上限を量で持つときの、量の値、上限の表現、実装の配置。
- terminal が購読中に作り直されたときの版番号の境目を、再開可能と判定しないようにする方法。
- 非同期処理の組み方、コードの配置、テストの置き方。

## 変えないもの

- `RETAINED_CHANGES` が決めている対象ごとの履歴の窓（件数）。理由: 履歴の窓を件数で持ち、窓外なら現在の状態からやり直す形は標準（Kubernetes の watch が窓外の版に 410 Gone を返して現在の状態からやり直させる形）と同じであり、購読の土台の規則は #1878 が所有する。土台への変更は D5・D6・D11 の 3 点に限る。
- 送る量の制御の量の単位（UTF-16 code unit）と、通知単位は下限以下という制約。理由: 単位を変えると VS Code の値をそのまま使えず、通知単位が下限を超えると出力元が再開しないため。
- 差分の結合（coalescing）。本 ISSUE では導入しない。理由: 結合は RPC 回数を減らす仕組みであり、送り待ちの在庫上限の代替ではないため（D11）。
- terminal を起動する操作（`GetOrSpawnTerminalSurface`）そのもの。操作として残し、廃止しない。理由: 起動は購読では代替できない操作であるため。
- terminal の接続が切れている間の入力の扱い（`src/hooks/useTerminal.ts:644-646` で表示無く捨てる）と、terminal の独自の世代番号。理由: #1895 が扱う。
- 購読の stream のつなぎ直し（`src/lib/client.ts` の `ensureStateStream` の 1 秒固定の再接続）。理由: #1891 が扱う。本 ISSUE は購読の受け取りに terminal と差分を足すだけで、つなぎ直しの処理は変えない。
- 処理済みの量の通知を同時実行の枠から外すこと。理由: #1894 が扱う。
- terminal の性能計測の設定（`GetTerminalPerformanceSwitches`、`src/lib/terminalPerformanceSwitches.ts`）の購読への移行。理由: #1898 が扱う。
- 出力の出どころ（PTY から tmux への置き換え）。理由: マイルストーン #98 が扱う。#98 が変えるのは出力の出どころだけで、配信の経路は本 ISSUE で作る購読をそのまま使う。
- Request の正本が挙げた行番号の読み替え。`requirements.md` の Context に記した調査基準 `9194bd57` への読み替えのまま扱い、再確認しない。

## 未確定・リスク

なし。design-01 の 3 件（購読の土台の保持件数と差分の流量の関係、terminal が作り直されたときの版番号の境目、`GetOrSpawnTerminalSurface` が返してよい内容の範囲）は、それぞれ D11、今周の「変える部分」と委任されたルート、D10 で解消した。今周に自動判断した箇所は無く、Requirements の Assumptions は「なし」、未決のまま残した要求と `[DEFERRED]` で人間へ渡した件も無い。
