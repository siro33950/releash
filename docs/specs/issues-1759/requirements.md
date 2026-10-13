# Context

- 入力: https://github.com/siro33950/releash/issues/1759
- 補助資料: #1204（[01] SwiftUI クライアントの器）と `docs/specs/issues-1204/`、#2012（[02] E2E の仕組み）、#1763（[10] Settings）、マイルストーン #78（ネイティブ UI、「UI の決まり」、「テストと CI の方針」）、ISSUE に添付された mock（Terminal タブ）。
- mock は機能の正であり、色と質感の正ではない（マイルストーン #78「UI の決まり」）。
- `AGENTS.md`「サーバがロジックを所有する」: 判定・取得・計算はサーバに置き、client には表示、入力の受付、サーバの呼び出しと購読、表示用の整形だけを許す。
- `AGENTS.md`「通信の原則」: サーバは状態を配信し、client は購読する。client からの単発の呼び出しは、状態を変える操作と、client の入力に対する計算だけに使う。
- proto の互換性は `buf.yaml` の `breaking: WIRE_JSON` で検査する。
- 開発の開始時点は main `8d443b89`（#1204 と #2012 を含む）。
- この開発で人と決めた内容のうち ISSUE 本文に無いもの（タブごとの shell、タブを閉じたときの停止と確認、帯の作業ディレクトリの追従、起動コマンドを変える呼び出しと購読）は、ISSUE 本文を書き換えず、この文書と behavior.md・design.md を正とする。
- Settings の Swift の画面は #1763 で作り、この開発で足す起動コマンドの呼び出しと購読を使う。

# Outcome

対象者: Releash の利用者（開発者）と、Releash を開発する人と agent。

現在の問題:
- Swift.app の Terminal タブは中身の無い仮の表示で、shell を使えない。
- サーバは 1 つの worktree に Terminal を 1 つしか起こせないため、同じ worktree で複数の shell を並べられない。
- Terminal の起動コマンドは React の画面が保存し、terminal を起こす要求に載せてサーバへ渡している。Swift.app からは同じ設定を使えない。

変更後の状態:
- 利用者は、Swift.app の Terminal タブで worktree の shell を使える。同じ worktree に複数の Terminal タブを開くと、それぞれが別の shell になる。
- 利用者は、帯で shell の今の作業ディレクトリと worktree の branch を確かめられる。
- 利用者は、処理が動いている shell を確認なしに閉じて止めてしまうことがない。
- Terminal の起動コマンドはサーバの設定にあり、React と Swift の両方の画面が同じ値を読み書きできる。

# Current Behavior

開発開始時点（main `8d443b89`）で確認した状態。

Swift.app:
- 「+」の「開く」から Terminal を選ぶと Terminal タブが開くが、中身はタブ名だけの仮の表示である（`clients/macos/Sources/PaneWorkspace.swift` の `WorkspacePaneView`）。
- 同じ pane に Terminal タブを何個でも開ける（`PaneWorkspace.open`）。タブの保存項目は id と種類だけである（`proto/client.proto:1921-1924`）。
- SwiftTerm は依存に無い（`clients/macos/project.yml`）。SwiftTerm の macOS 版は画面の文字をアクセシビリティに公開していない（SwiftTerm の `Sources/SwiftTerm/Mac/MacAccessibilityService.swift` は中身の無いクラス）。
- 購読は対象と引数が同じなら 1 本にまとめる（`clients/macos/Sources/AppModel.swift` の `subscribe`）。

サーバの Terminal:
- Terminal の持ち主は worktree の path だけで決まる（`server/src/domain/terminal_surface/value_objects/terminal_surface_owner.rs:4-7`、`:65-68`）。同じ worktree への 2 回目の `GetOrSpawnTerminalSurface` は既存の Terminal を返す（`server/src/usecase/terminal_surface/spawn_usecase.rs:188-193`）。
- 購読の対象は `terminal`（引数は worktree の path、Session の場合は Session の ID も。`server/src/adaptor/presenter/state_subscription_target.rs:35-49`）。`TerminalEvent` は snapshot・output・resize・exit を持つ（`proto/client.proto:2361-2397`）。
- snapshot の `replay` は、サーバの端末エミュレータが作った VT 列で、記録時の cols・rows と一緒に届く。`replay` が空なら新しい terminal である。保存した画面からの復元はサーバの内部で行われ、client には中身のある snapshot として届く（`spawn_usecase.rs:36-38`）。
- 処理した量は `ReportTerminalProcessed` で返す。単位は output の `data` の UTF-16 code unit 数で（`server/src/adaptor/presenter/terminal_subscription.rs:155`）、snapshot の `processed_report_units`（5000）ちょうどの単位でしか受理されない（`server/src/adaptor/controller/api/client_service.rs:78-84`）。未報告が 100,000 を超えると、サーバは端末の出力を止める（`server/src/infrastructure/terminal/output_flow_control.rs:8`、`:59-61`）。
- `WriteTerminalSurface` は `attachment_id`（開始済みの terminal の購読の ID）と `sequence`（購読ごとに 0 から）が必須である（`proto/client.proto:2263-2270`）。古い購読の ID で書くと `STALE_TERMINAL_ATTACHMENT` で拒否される（`server/src/adaptor/presenter/terminal_error.rs:47-62`）。
- `KillTerminalSurface` は持ち主を指定して shell を止め、保存した画面も消す（`server/src/usecase/terminal_surface/lifecycle_usecase.rs:5-23`）。
- サーバは shell の今の作業ディレクトリを追っておらず、shell の前面で shell 以外の処理が動いているかも判定していない（`server/src/infrastructure/terminal`、`server/src/adaptor/gateway/terminal_surface` に OSC 7 の解釈、子プロセスの cwd の取得、前面の process group の取得が無い）。

React の画面:
- Terminal は worktree ごとに 1 つで、右下の固定パネルにある（`clients/desktop/src/components/panels/RightSidebarBottom.tsx:65-69`）。表示を外しても shell は止めない（`clients/desktop/src/hooks/useTerminal.ts:754-759`）。
- 購読の開始より前に打たれた入力は、1024 文字まで貯めて開始後に送る（`clients/desktop/src/lib/terminalStartupInputBuffer.ts`）。
- リンクを開く（`clients/desktop/src/lib/terminalLinkActivation.ts`）、Clear（`clients/desktop/src/components/panels/TerminalPanel.tsx:216`）、終了時の「Process exited」の表示（`clients/desktop/src/lib/terminalSurfaceStream.ts:62`）がある。
- 閉じる前の確認は無い。

起動コマンド:
- 設定画面で編集した値を localStorage に保存する（`clients/desktop/src/components/panels/SettingsModal.tsx:638-642`、`clients/desktop/src/hooks/useSettings.ts:20`、`:116-117`）。
  `useTerminal.ts:339` には、その値を `GetOrSpawnTerminalSurface` の `startup_command`（`proto/client.proto:995`）に載せる経路がある。
  しかし、`TerminalPanel` を使う `RightSidebarBottom.tsx:65` と `AgentSessionPanel.tsx:144` は `terminalStartupCommand` を渡していないため、起動コマンドはサーバへ送られていない。
- サーバは、保存した画面から復元した terminal では起動コマンドを実行しない（`server/src/domain/terminal_surface/value_objects/terminal_surface_startup_command.rs`）。
- サーバの設定ファイルに起動コマンドの項目は無い（`server/src/adaptor/gateway/app_config/config_models.rs:12-23`）。

# Scope / Non-goals

Scope:
- Swift.app の Terminal タブ（SwiftTerm による表示、入力、resize、帯、閉じる操作と確認、リンク、Clear、終了の表示、アクセシビリティ）
- Terminal の購読、処理した量の報告、入力の送信、接続断からの再同期
- worktree の中で Terminal を区別する持ち主と、タブとの結び付けの保存（サーバ、proto、保存形式）
- shell の今の作業ディレクトリの取得と配信、shell 以外の処理が動いているかの判定と配信（サーバ）
- 起動コマンドのサーバの設定への移動、変更の呼び出しと読む購読、React の設定画面の切り替え
- Terminal タブの回帰の E2E

Non-goals:
- xterm.js と WebGL に固有の機能、`ReportMountedXtermCount` の引き継ぎ
- Session の TUI を映すタブ（[04] #1205）
- Swift の Settings 画面（[10] #1763）
- ファイルのドラッグ＆ドロップによる path の書き込み（[12] #1833）
- localStorage にある起動コマンドの値の引き継ぎ
- React の Terminal を複数にすること。React の画面は worktree ごとに 1 つの Terminal のまま使う
- ISSUE 本文の書き換え

# Requirements

Terminal タブ:
- R-001: 「+」の「開く」から Terminal を選ぶと、選択中の worktree の path を作業ディレクトリとする shell が起動し、その shell の画面を表示する Terminal タブが開く。端末の表示には SwiftTerm を使う。
- R-002: 同じ worktree で Terminal タブを開くたびに、別の shell が起動する。あるタブへの入力と、そのタブの shell の出力は、ほかのタブに表示されない。同じ worktree の Terminal タブは、タブ名で互いに区別できる。
- R-003: Terminal タブの帯に、shell の今の作業ディレクトリと worktree の branch を表示する。shell が作業ディレクトリを移すと、帯の作業ディレクトリも変わる。作業ディレクトリの取得と配信はサーバが行う。
- R-004: 利用者が打った入力は shell に送られ、shell の出力はタブに表示される。
- R-005: タブの表示領域の大きさが変わると、shell の端末の大きさもそれに合わせて変わる。
- R-006: 大量の出力が続いても、Swift.app は処理した量をサーバへ報告し続け、出力は止まらずに表示され続ける。
- R-007: Terminal タブを開いてから購読が始まるまでに打った入力は、購読の開始後に、打った順で shell に送られる。
- R-008: サーバとの接続が切れて回復すると、Terminal タブは購読し直し、切断中の出力を含む shell の今の画面を表示する。回復後の入力は shell に送られる。
- R-009: Terminal タブのある worktree を選び直したとき、および Swift.app を起動し直したとき、そのタブは同じ shell に繋がり、それまでの画面を表示する。サーバを起動し直していた場合は、サーバが保存した画面を表示する。
- R-010: Terminal タブを閉じると、そのタブの shell は止まる。shell の前面で shell 以外の処理が動いているときは、閉じる前に確認を出し、取りやめると shell は止まらずタブも残る。動いているかの判定はサーバが行う。
- R-011: Terminal タブで、表示されたリンクを開ける。表示を Clear できる。shell が終了すると、終了したことと終了コードを表示する。
- R-012: Terminal タブの画面の文字は、アクセシビリティで読める。

起動コマンド:
- R-013: Terminal の起動コマンドはサーバの設定に保存される。サーバは、新しく shell を起こすときに自分の設定から起動コマンドを読んで実行する。保存した画面から復元した shell では実行しない。terminal を起こす要求は起動コマンドを持たない。
- R-014: 起動コマンドを変える呼び出しと、今の値を読む購読がある。React の設定画面の起動コマンドの欄は、この呼び出しと購読で値を読み書きする。

サーバとの境界:
- R-015: この開発で追加・変更するサーバの呼び出しと購読は、特定の client に依存しない形である。状態は購読で配信し、単発の呼び出しは状態を変える操作と client の入力に対する計算だけに使う。
- R-016: proto の変更は `buf.yaml` の breaking 検査（WIRE_JSON）を通る。

検証:
- R-017: Terminal タブを開き、打ったコマンドの出力が表示されることを確かめる回帰の E2E がある。

# Assumptions

なし
