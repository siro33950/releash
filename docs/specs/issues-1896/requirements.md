# Context

- 要求の正本: https://github.com/siro33950/releash/issues/1896
- 補助資料: #1895（画面の接続状態）、#1878（購読と版からの再開）、#1885〜#1888・#1898（表示の購読への移行）、#1938（通知と監視の削除）、#1952・#1879（シェルの生存の判定）、#1904・マイルストーン #100（画面をサーバの利用者にする）、Kubernetes probes（https://kubernetes.io/docs/tasks/configure-pod-container/configure-liveness-readiness-startup-probes/）
- ISSUE 本文は main `42be41e0` 時点の記述である。行番号は main `2a2fbb6a` の位置に読み替える。消す対象（画面の作り直しと復元の手続き）は、`2a2fbb6a` でも全て残っている。
- 方針は Kubernetes の readiness に従う。受付を止めるだけで、作り直さない。
- 画面の接続状態は、#1895 で `src/lib/client.ts` の 1 か所が gRPC の 5 つの状態（CONNECTING・READY・TRANSIENT_FAILURE・IDLE・SHUTDOWN）として持つ形になっている。READY でない間の呼び出しの扱いも #1895 で決まっている（IDLE は接続を始めて待つ、CONNECTING は期限まで待つ、TRANSIENT_FAILURE・SHUTDOWN は即座に失敗させて画面に出す）。#1896 はこれを読むだけにする。
- シェルの状態と画面の接続状態は、別の値として読む（#1895）。`DaemonBoundary` は、シェルの状態を変化の通知（`subscribe_daemon_status`）で受け取る。この受け取り方は変えない。
- 購読の版からの再開は #1878 で実装済みで、全ての表示は購読に移っている（#1885〜#1888・#1898）。
- シェルは、phase が Ready でない間、シェルの Tauri コマンドを ApplicationUnavailable で拒否する（`src-tauri/src/adaptor/controller/command/mod.rs:34-50,82-88`、`src-tauri/src/domain/daemon_supervision.rs:511-516`）。

# Outcome

対象者: Releash のデスクトップ画面の利用者。

現在の問題: シェルが daemon との切断を判定すると、画面全体が捨てられる。つながると画面は作り直され、Workspaces の一覧、開いていたタブ、入力中の内容などの画面の状態が失われる。作り直した画面の復元を確かめる手続きは、画面の購読の stream が切れていない再接続では必ず失敗する。Retry しても同じ失敗を繰り返し、抜けるには Quit するしかない。画面の接続が切れても、画面全体としては「再接続中」であることが分からない。

変更後の状態: 一度出した画面は、再接続をまたいで残る。画面の接続が切れている間は、画面を残したまま「再接続中」と表示し、操作は #1895 のとおり待つか失敗する。つながると、購読は版から再開する。復元の手続きは無くなる。

# Current Behavior

main `2a2fbb6a` のコードを読んで確認した挙動である。実機での再現は行っていない。

- `src/components/DaemonBoundary.tsx:124-135`: 画面は、シェルの phase が `ready` か `restoring` のときだけ描かれ、`key={status?.connectionGeneration}`（`:127`）が付いている。シェルが切断を判定すると phase は Starting に戻り（`src-tauri/src/domain/daemon_supervision.rs:173-178`）、画面全体が unmount される。つながると、`connected()` が `connection_generation` を 1 つ進めて Restoring にし（`:179-196`）、画面は新しい key で作り直される。
- `src/components/DaemonBoundary.tsx:136-178`: シェルの phase が `ready` 以外の間は、画面全体を覆う表示（見出し、理由、Retry、Quit）が出る。
- 復元の手続き:
  - 画面側: `src/App.tsx:105-112` が、設定と Repository 一覧の読み込みを待ってから `restoration.complete()` を呼ぶ。`DaemonBoundary.tsx:94-101` → `completeClientRestoration`（`src/lib/client.ts:499-509`）が、Tauri の `complete_desktop_restoration` に launch ID・attachment ID・世代を渡す。
  - シェル側: `src-tauri/src/adaptor/controller/command/client.rs:50-76`（`complete_desktop_restoration`・`fail_desktop_restoration`）、`src-tauri/src/usecase/daemon_supervision.rs:194-247`（`attach`・`finish_restoration`・`fail_restoration`）、`src-tauri/src/domain/daemon_supervision.rs:111-275`（`connection_generation`・`restoration_deadline`・`attachment_id`・`begin_restoration`・`finish_restoration`・`fail_restoration`・`expire_restoration`・`retry_restoration`）。
- attachment ID はページごとに固定の値である（`src/lib/client.ts:44,72`）。`connected()` は ID を消し（`domain/daemon_supervision.rs:191`）、ID を置き直すのは `get_client_endpoint` → `attach` の中の `begin_restoration`（`usecase/daemon_supervision.rs:194-209`）だけである。画面の購読の stream が切れていなければ、画面は `open()` を呼ばず、接続先を受け取り直さない。そのため `finish_restoration` は「Desktop attachment changed during restoration」で失敗し、phase は Failed になる（`domain/daemon_supervision.rs:245-255`）。Retry は `retry_restoration`（`:266-275`）を通るが、ID は消えたままなので同じ失敗を繰り返す。
- `restoration_deadline` は `begin_restoration` でしか設定されない（`domain/daemon_supervision.rs:205-210`）。
- `App.tsx:113-115`: 設定（`desktop-settings`）の読み込みの失敗を `restoration.fail` に渡し、シェルを Failed にして覆いに出す。`useSettings` の `loadError` を画面に出しているのは、ここだけである。
- `restoration.ready` は、自動の更新の確認（`App.tsx:94-96` → `src/hooks/useUpdateChecker.ts:28-56`）と、起動時の処理（`App.tsx:148-171`。cwd の Repository の登録と、worktree が 1 つならそのタブを開く）の条件になっている。今は、再接続のたびに画面ごと作り直されるので、どちらも再接続のたびに動く。
- `App.tsx:252-256`: `set_menu_items_enabled` は、選択中の worktree の有無が変わったときだけ呼ばれ、失敗は `.catch(() => {})` で捨てられる。シェルの phase が Ready でない間は拒否される。
- 設定画面の「Send anonymous performance metrics」（`src/components/panels/SettingsModal.tsx:784-799`）は、`desktop-settings` を読めていない間、`loadSettings()` の値（`src/hooks/useSettings.ts:68,77-84`）を表示する。Save すると、その値を元にした変更を daemon に書き込む（`SettingsModal.tsx:933-935,954-956`）。
- 画面の接続状態が READY でなくなっても、画面全体としてそれを示す表示は無い。接続状態を示すのは、シェルの phase が `ready` でない間の覆いの中の `data-client-connection` 属性（`DaemonBoundary.tsx:151`）だけである。

# Scope / Non-goals

変更するもの

- `DaemonBoundary` の画面の作り直し（key）と、シェルの phase による画面の unmount の削除
- 画面全体の「再接続中」の表示
- シェルの phase ごとの覆いの出し分け
- 復元の手続きの削除（画面側の `completeClientRestoration`・`useDesktopRestoration`・`App.tsx` の復元の呼び出し、シェル側の `complete_desktop_restoration`・`fail_desktop_restoration`・`connection_generation`・`restoration_deadline`・attachment ID・Restoring の phase と、それだけが使うもの）
- 起動時の処理と自動の更新の確認を動かす条件
- `set_menu_items_enabled` を呼ぶ条件
- 設定画面の performance metrics の、読み込みの失敗の表示と保存

変更しないもの

- 画面の接続状態の持ち方と、READY でない間の呼び出しの扱い（#1895）
- `DaemonBoundary` がシェルの状態を受け取る方法（変化の通知）
- 購読の版からの再開の仕組み（#1878）
- シェルの生存の判定と、切断後に daemon を起動し直すかどうか（#1879・#1952・#1904）
- launch ID と、接続先のインスタンスとリリースの検証（#1904 が扱う）
- 利用者の操作によるシェルの Tauri コマンドの呼び出し（`src/hooks/useAppSettings.ts:40,75,108,115`、`src/hooks/useUpdateChecker.ts:69`）。既に失敗を画面に出している
- 設定画面の Background の欄の、読み込みの失敗の表示（`useAppSettings.ts:65,131` → `SettingsModal.tsx:747-750`）
- 最初の起動で、シェルの phase が Ready になるまで画面を出さないこと

# Requirements

- R-001: 一度出した画面は、シェルの phase が変わっても、画面の接続状態が変わっても、作り直されず、unmount されない。画面の状態（開いているタブ、選択、入力中の内容など）は保たれる。
- R-002: 一度 READY になった後に画面の接続状態が READY でなくなったとき、画面を残したまま「再接続中」と表示する。READY に戻ると、この表示は消える。最初に READY になる前の CONNECTING では表示しない。シェルの phase は、この表示の条件にしない。
- R-003: 一度画面を出した後、シェルの phase が Starting・Backoff の間は、画面を覆う表示を出さない。
- R-004: 一度画面を出した後、シェルの phase が Failed の間は、画面を覆う表示を出し、Retry と Quit を選べるようにする。Retry は、シェルが Retry できると示しているときだけ出す。
- R-005: 一度画面を出した後、シェルの phase が Stopping・Installing・Stopped の間は、画面を覆う表示を出す。
- R-006: 画面の接続状態が READY に戻ったとき、購読は最後に受け取った版から再開する。
- R-007: 復元の手続きを無くす。シェルは、つながったら復元の完了を待たずに Ready になる。画面は復元の完了も失敗もシェルに送らない。
- R-008: 起動時の処理（cwd の Repository の登録と、worktree が 1 つならそのタブを開くこと）と自動の更新の確認は、画面を最初に出したときに 1 回だけ動く。つなぎ直しや、シェルの phase の変化では動き直さない。自動更新を設定で ON にしたときは、更新を確認する。一度確認が済んだ後は、設定の切り替えを繰り返しても確認を繰り返さない。確認が失敗したときは、確認が済んでいないものとして、次に ON にしたときに確認し直す。
- R-009: メニューの有効・無効は、シェルの phase が Ready に戻ったときにも、そのときの worktree の選択に合わせ直す。
- R-010: 設定画面の performance metrics の設定は、`desktop-settings` を読めていない間、その値を保存しない。読み込みが失敗したときは、その失敗をその設定の場所に出す。

# Assumptions

なし
