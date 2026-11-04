# Context

- 要求の正本: [siro33950/releash#1764](https://github.com/siro33950/releash/issues/1764)
- 補助資料: [#1204](https://github.com/siro33950/releash/issues/1204)、[#1944](https://github.com/siro33950/releash/issues/1944)、[#1766](https://github.com/siro33950/releash/issues/1766)、[#2054](https://github.com/siro33950/releash/issues/2054)、[#1854](https://github.com/siro33950/releash/issues/1854)、マイルストーン [#78](https://github.com/siro33950/releash/milestone/78) の説明文（「確定した設計」「UI の決まり」「テストと CI の方針」）
- 対象の commit: main `cca7be50`

確定済みの制約は次のとおりである。

- アプリケーションロジックはサーバに置く（`AGENTS.md`「サーバがロジックを所有する」）。
- サーバの発見、互換判定、起動は同梱した CLI を呼んで行い、Swift.app はサーバを監督しない（マイルストーン #78「確定した設計」）。
- ログイン項目は `SMAppService.mainApp` で登録し、KeepAlive は付けない。メニューバーは Swift.app が持つ（マイルストーン #78「確定した設計」）。
- 状態は 3 値で表す。黄は人の番（attention）、青はシステムの番（active）、緑は誰の番でもない（idle）。色はシステムの色だけを使い、例外は状態の 3 色と差分の色に限る（マイルストーン #78「UI の決まり」）。
- 回帰の E2E は、機能ごとに主たる動作だけを確かめる（`AGENTS.md`、マイルストーン #78「テストと CI の方針」）。
- 「アップデートを確認」は #1766 で足す。
- ISSUE は、ログイン項目としての起動を Apple Event（`keyAELaunchedAsLogInItem`）で判定し、`defaultLaunchBehavior(.suppressed)` でウィンドウを出さない方法を挙げている。今の Swift.app のウィンドウは、delegate が AppKit の `NSWindow` として開いている（`clients/macos/Sources/WorkspaceWindows.swift:9-41`）。この文書は実現の仕組みを決めない。
- 人と決めた内容（ISSUE 本文に無い、または本文と異なるもの）:
  - メニューバーの「workflow 状態の表示」は、アイコンが全 worktree を集約した 3 値を示し、メニューの中に実行中の実行木の一覧を出して、選ぶとその worktree を開く。
  - 最小化状態で起動したとき、サーバに接続できなくてもウィンドウを出さない。失敗の画面は「開く」で開いたときに見える。
  - Tauri シェル（`clients/desktop/src-native/`）はこの開発で触らない。Tauri シェル側にあるログイン項目の登録判断の規則は、#1765 の撤去で消える。
  - Settings のログイン項目の節の表示（承認待ちの文と導線、登録できない理由）は #1763 で作ったままにする。#1764 で登録が実際に行われることで、承認待ちが実機で観測できるようになる。
  - Tauri 版と Swift.app の両方を入れた利用者は、#1766 までログイン項目を 2 つ持つ。どちらか一方を利用者が無効にする。Swift.app はこれを扱わない。

# Outcome

対象者は Swift.app を使う開発者である。

Swift.app にはメニューバーのアイコンが無い。
ウィンドウを閉じた後に実行木の状態を見る入口と、ウィンドウを開き直す入口が Dock のアイコンしか無い。
Settings の「ログイン時に起動」と「最小化状態で起動」はサーバに保存されるが、Swift.app はログイン項目を登録しないので、ログイン時に起動しない。
ログイン項目として起動されたかを判定しないので、最小化状態で起動することもできない。
Swift.app を二重に起動したときの扱いも無い。
Tauri 版にはこれらがあり（tray、LaunchAgent のログイン項目、`--hidden` での起動、単一起動）、Swift.app へ移っていない。

変更後、開発者は次のことができる。

- メニューバーのアイコンで、全 worktree を集約した状態を見る。メニューから、実行中の実行木の一覧を見てその worktree を開き、ウィンドウを開き、Settings を開き、About を見て、Swift.app を終了する。
- Settings の「ログイン時に起動」を有効にすると、次のログインで Swift.app が起動する。「最小化状態で起動」も有効なら、ウィンドウを出さずメニューバーだけで起動する。
- ウィンドウを閉じても、Swift.app を終了しても、サーバと実行木は続く。次に Swift.app を起動すると、動いているサーバに接続する。
- Swift.app を二重に起動すると、既存の画面が前面に出る。

# Current Behavior

開発開始時点（main `cca7be50`）で確認した状態。

起動と接続:
- 起動時に delegate がウィンドウを 1 つ開き、同梱した CLI の `status --json` でサーバを探し、居なければ `server start` で起動してから接続する（`clients/macos/Sources/WorkspaceWindows.swift:9-13`、`clients/macos/Sources/AppModel.swift:96-107`、`clients/macos/Sources/CLI.swift:63-103`）。
- 接続に失敗すると、ウィンドウの中に「サーバに接続できません」と理由と「再試行」を出す（`clients/macos/Sources/ReleashApp.swift:51-58`）。
- CLI はサーバを `setsid` で切り離して起動する（`clients/cli/src/daemon.rs:85-110`）。Swift.app の終了でサーバを止めるコードは無い。

ウィンドウと終了:
- ウィンドウを閉じると、delegate がウィンドウの一覧から外すだけで、プロセスは続く。Dock のアイコンをクリックすると、ウィンドウが無ければ開き直す（`clients/macos/Sources/WorkspaceWindows.swift:49-60`）。
- 終了（アプリメニューの Quit、⌘Q、Dock の終了）は、AppKit の既定の動作で画面のプロセスを終える。終了に介入するコードは無い。

メニューバー:
- メニューバーのアイコン（`MenuBarExtra`、`NSStatusItem`）は無い（`clients/macos/Sources/` を検索して 0 件）。
- 全 worktree をまたいだ集約状態は、proto にもサーバにも無い。画面に届く状態は、worktree ごとの実行木の 3 値（`proto/client.proto:1732`、`WorkspaceStatusClassification`）で、サイドバーが行ごとに色で表示している（`clients/macos/Sources/SidebarView.swift:133-167`）。

ログイン項目と最小化状態での起動:
- Settings の System 節は「ログイン時に起動」と「最小化状態で起動」を `UpdateLoginItemPreference` と `UpdateAppSettings` でサーバに保存し、購読 `desktop-settings` の値を表示する（`clients/macos/Sources/Settings/SystemSettingsView.swift:13-32`、`:150-165`）。`SMAppService` の `register` と `unregister` を呼ぶ箇所は無い。
- Settings は `SMAppService.mainApp.status` が `requiresApproval` のとき「ログイン項目は承認待ちです」とシステム設定を開くボタンを出し、`CheckLoginRegistration` が `translocated` か `readOnly` を返したときサーバの理由を出す（`SystemSettingsView.swift:33-43`、`:122-129`、`:191-202`）。
- 登録してよい場所かの規則と理由はサーバの domain にある。AppTranslocation の path か読み取り専用の volume なら登録しない（`server/src/domain/installation/mod.rs:16-34`、`:52-59`）。
- ログイン項目として起動されたかを判定するコードと、ウィンドウを出さずに起動するコードは無い。

二重起動:
- 二重起動を検知するコードは無い。

CLI の設置:
- Settings の「CLIを設置」が RPC `InstallCli` を呼ぶ。設置を自動で行う箇所は無い（`SystemSettingsView.swift:56-61`、`:184-189`）。

Tauri シェル（参照のみ。この開発では触らない）:
- tray の項目は Show Releash、サーバを停止、Quit である（`clients/desktop/src-native/infrastructure/platform/tray.rs:39-48`）。
- 接続時と `desktop-settings` が変わるたびに、`auto_launch` に合わせて LaunchAgent を登録・解除する（`clients/desktop/src-native/usecase/desktop_lifecycle.rs:76-88`、`:110-122`）。登録が要るかの規則は Tauri シェルの domain にある（`clients/desktop/src-native/domain/login_item.rs:26-34`）。
- `--hidden` で起動され `start_minimized` が真ならウィンドウを出さない。接続に失敗したときはウィンドウを出す（`clients/desktop/src-native/domain/desktop_lifecycle.rs:32-42`）。
- 単一起動はデータディレクトリの lock file と socket で行う（`clients/desktop/src-native/infrastructure/platform/single_instance.rs`）。

回帰の E2E:
- `clients/macos/UITests/` にあり、fixture が隔離したデータディレクトリと provider の代替 TUI を用意する（`clients/macos/UITests/Fixture.swift`）。終了後にサーバと実行木が続くことを確かめるテストは無い。

# Scope / Non-goals

変更する対象:

- メニューバーのアイコンとメニュー（集約状態の表示、実行中の実行木の一覧、開く、設定、About、Quit）。
- 全 worktree を集約した状態の配信。
- ログイン項目の登録、解除、起動時の登録し直し。
- ログイン項目としての起動の判定と、最小化状態での起動。
- 終了の各経路（アプリメニューの Quit、⌘Q、Dock の終了、メニューバーの Quit、AppleScript の quit、ログアウト・再起動・システム終了）が画面のプロセスだけを終えること。
- 二重起動の扱い。
- 回帰の E2E（終了後もサーバと実行木が続き、次の起動で動いているサーバに接続する）。

変更しない対象:

- メニューバーの「アップデートを確認」（#1766）。
- メニューバーからサーバを止める項目。サーバを止めるのはフッターの「サーバを停止」（#1944）と CLI だけである。
- Tauri シェルと React の画面。
- Settings の System 節の表示と、ログイン項目のトグルが設定の保存だけを行うこと（#1763）。
- 登録してよい場所かの規則と CLI の設置の規則（サーバの `domain/installation/` のまま）。
- 画面を起動したときのサーバの発見と起動の経路（#1204）。
- Tauri 版と Swift.app を両方入れた利用者のログイン項目の重複。

# Requirements

メニューバー:
- R-001: Swift.app が動いている間、ウィンドウの有無によらず、メニューバーにアイコンが表示される。
- R-002: アイコンは、サーバに接続しているとき、全 worktree の実行木を集約した状態（attention、active、idle の 3 値）を状態の 3 色で示す。接続していないときは状態を示さない。
- R-003: アイコンは、明るいメニューバーと暗いメニューバーのどちらでも見え、メニューバーの色付けに合わせ、メニューを開いている間は選択状態の見た目になる。
- R-004: メニューには、archive されていない実行木が、worktree ごとに一覧される。各項目は worktree 名、Session なら provider の表示名、workflow なら workflow 名、その実行木の 3 値の状態を示す。項目を選ぶと、ウィンドウが前面に出て（無ければ開いて）その worktree が選ばれる。
- R-005: メニューには「開く」「設定」「About」「Quit」がある。「開く」はウィンドウを前面に出し、無ければ開く。「設定」は Settings のウィンドウを開く。「About」は Swift.app の名前と版を表示する。「Quit」は R-010 の終了を行う。

ログイン項目:
- R-006: 「ログイン時に起動」が有効なら、Swift.app は自分をログイン項目として登録する。登録は `SMAppService.mainApp` による 1 つだけで、LaunchAgent は使わず、KeepAlive は付けない。無効にすると登録を解除する。Swift.app は、起動したときと設定が変わったときに、登録の状態を設定に合わせる。登録が失われていれば登録し直す。
- R-007: Swift.app が AppTranslocation または読み取り専用の場所で実行されているときは、「ログイン時に起動」が有効でも登録しない。理由はサーバが判定して返し、Settings に表示される（#1763 の表示のまま）。
- R-008: 登録が承認待ちのとき、Settings に承認待ちである旨とシステム設定のログイン項目を開く導線が表示される（#1763 の表示のまま）。
- R-009: Swift.app の .app を削除した後、macOS はログイン時に Swift.app の起動を試みない。

起動と終了:
- R-010: 「ログイン時に起動」と「最小化状態で起動」がともに有効で、Swift.app がログイン項目として起動されたときは、ウィンドウを出さずメニューバーのアイコンだけを出す。サーバに接続できなくてもウィンドウを出さない。開発者が Swift.app を起動したときは、設定によらずウィンドウを出す。
- R-011: ウィンドウを閉じると、ウィンドウだけが閉じる。Swift.app のプロセス、サーバ、実行木は続く。
- R-012: 終了の各経路（アプリメニューの Quit、⌘Q、Dock アイコンの終了、メニューバーの Quit、AppleScript の quit、ログアウト・再起動・システム終了）は、Swift.app のプロセスだけを終える。サーバと実行木は続く。
- R-013: Swift.app を起動したとき、サーバが居れば接続し、居なければ起動する。終了した後に起動し直すと、動いているサーバに接続し、サーバは起動し直されない。
- R-014: Swift.app が動いているときにもう一度起動すると、既存の画面が前面に出て、2 つ目の画面と 2 つ目のサーバは起動しない。

CLI:
- R-015: CLI の `/usr/local/bin/releash` への設置は、Settings の明示の操作でだけ行われる。起動時や設定の変更時に自動では行わない。

検証:
- R-016: 回帰の E2E で、Swift.app を終了した後もサーバと実行中の workflow が続き、次に Swift.app を起動すると動いているサーバに接続することを確かめる。

サーバとの境界:
- R-017: この開発で追加する購読は、特定の client に依存しない形である。

# Assumptions

- Dock の扱いは変えない。Swift.app は通常のアプリのままで、ウィンドウが無いときも Dock にアイコンを出す。
- メニューの文言は、「開く」「設定…」と、About と Quit は macOS 標準の表記（「Releash Preview について」「Releash Preview を終了」）を使う。

# Invariants

なし。

# Risks

- `SMAppService.mainApp` で登録したログイン項目として起動されたときに、Apple Event で起動元を判定できることを実機で確かめていない。判定できなければ R-010 を満たせない。
