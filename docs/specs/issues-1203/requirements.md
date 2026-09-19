# Context

- 要求の正本: Issue #1203「A-launchd: launchd 常駐＋1 .app packaging」、milestone 77「01. ローカル server-client 化（基盤）」。
- 背景資料: Issue #1199（A1）、Issue #1200（A2）、Issue #1201（A-flip）、Issue #1202（A-daemon）、`docs/specs/issues-1201/requirements.md`、`docs/specs/issues-1201/design-01.md`、`docs/specs/issues-1202/requirements.md`、`docs/specs/issues-1202/design-01.md`、`docs/specs/issues-1202/design-02.md`、`docs/specs/issues-1202/design-03.md`、`AGENTS.md`。
- milestone 77 で確定済みの設計判断のうち、本変更が従うもの。
  - 判断①: req/resp は Protocol Buffers で定義したエンベロープ（`request_id`＋`oneof command`）を経由して usecase 共有 dispatch へ渡す。push も proto message とする。`.proto` が protocol の正であり、Rust / client の型はそこから生成する。
  - 判断③: クライアント通信は WebSocket 単一トランスポートとする。
  - 判断④: ローカルは loopback＋token ファイル認証とする。
  - 判断⑤: headless デーモン抽出＋launchd LaunchAgent 常駐＋1 .app 同梱。
  - 判断⑥: strangler 移行（A0 掃除 → A1 walking skeleton → ドメイン被覆 → 既定切替 → デーモン抽出 → 常駐）。本変更は常駐にあたり、デーモン抽出（#1202、`00b57d77` で merge 済み）の後に行う milestone 77 の最後の変更である。
  - 判断⑦: クライアント token は discovery file の master token と分けて発行する。権限が同等でも識別子を分け、master token を renderer へ露出させない。
  - 判断⑧: terminal stream は汎用エンベロープに統合し、クライアント接続は 1 本にする。stream 単位のフロー制御（`attachment_id`＋`sequence` / `ack`）はエンベロープ内に保持する。
- milestone 77 の土台。
  - クライアント向け ws は、MS82 で新設した local API（axum、127.0.0.1 bind、discovery file）の上にある。
  - HTTP local API（workflow / provider-lifecycle）は CLI と provider hook のローカル専用入口として残す。クライアントの経路ではない。
  - A0 で温存した旧 ws shell は #1338 で削除済みであり、再利用しない。
- milestone 77 の成果は、ローカルで desktop が daemon＋ws で完全動作すること（desktop 1 クライアント。CLI / hook の HTTP 入口は対象外）であり、Track B（ネイティブ UI）と Track C（リモート）の土台となる。
- Issue #1203 の「現状」節は #1201（PR #1824）と #1202（`00b57d77`）の merge 前の記述である。本文書の Current Behavior は、`feat/issues/1203`（`00b57d77`）のコードで確認した状態を正とする。
- `docs/specs/issues-1202/requirements.md` の Non-goals は、次を本 Issue の範囲と定めている。desktop 起動時の daemon の spawn と接続完了までの待機、daemon の監視、異常終了時の再 spawn、親の終了検知による daemon の自己終了、daemon の起動失敗と再起動制御、起動失敗時の desktop での失敗理由の表示と終了操作、メニュー・トレイの Quit での daemon への停止要求と一括停止完了後の desktop の終了、application quit の再起動（Restart）の意図での daemon と desktop の再起動、launchd LaunchAgent による常駐、ログイン時に起動・最小化状態で起動の設定、1 .app への同梱、更新・再起動時の UI と daemon の切替。
- `docs/specs/issues-1202/design-01.md` から `design-03.md` は、「#1202 は #1203 と同時にリリースする前提であり、#1202 の merge から #1203 の完了までの中間状態は利用者に届かない」を人間の明示として記録し、desktop を daemon なしで起動したときの対処を #1202 では加えていない。
- `docs/specs/issues-1201/requirements.md` は、接続先の backend の再起動で変更要求の結果を確定できない場合（#1201 R-011 / B-013）について、backend のプロセスを実際に再起動した状態での確認を #1202 と #1203 で扱うと定めている。
- `docs/specs/issues-1202/requirements.md` の R-009（daemon の起動時に `/usr/local/bin/releash` を実行中の実行ファイルへの symlink として設置する）は、本 Issue で置き換える。設置先 `/usr/local/bin/releash` と、Tauri を使わない実行ファイルを指すというリンク先は変えず、設置の契機だけを変える。
- 本変更は macOS の標準の仕組みに沿う。利用者の終了操作と OS 起因の終了の扱い、ログイン項目の登録状態の扱い、メニューバーのアイコンの表示、利用者が操作していない時点で管理者の認証を求めないことを含む。
- 対応プラットフォームは macOS である（`AGENTS.md`「リリース」）。
- Issue #1203 は、実装上の次の指定を含む。これらは Design で扱う。
  - LaunchAgent の登録は 1 つに統合し、二重登録にしない。登録は `SMAppService` の agent 方式で行い、plist は `.app` の `Contents/Library/LaunchAgents/` に同梱する。
  - launchd の `KeepAlive` は付けない。
  - 単一インスタンスは、UI 側は既に動作している UI のアクティブ化で、daemon 側は local event store の writer lock で担う。loopback ポートの bind は使わない。
  - 更新に伴う停止・再起動を既存の `ShutdownCoordinator` と統合する。
  - updater は UI shell に残し、Rust の切替制御と連携させる。
  - 起動監督・期限・再試行の判断、切替状態、停止完了の判定、起動対象・接続先の検証は Rust が所有する。
  - 起動期限、試行上限、待機間隔、試行回数のリセット条件は設計で具体化する。

# Outcome

- 対象者は、desktop 利用者と、ネイティブ UI（Track B）・リモート（Track C）を実装する開発者である。
- 現在、#1202 で backend を headless の daemon へ抽出した結果、desktop は起動済みの daemon へ接続するだけで daemon を起動しない。daemon を起動する利用者向けの入口もないため、Releash.app を起動しても daemon が無ければ desktop は接続先を得られずに起動に失敗し、ログイン時に起動しても同じ結果になる。メニューとトレイの Quit は desktop プロセスだけを終了させ、daemon と実行中の workflow は動き続ける。daemon の実行ファイルは `.app` に同梱されていない。このため「daemon が動いているのにメニューバーにアイコンがない」状態と、その逆の「アイコンはあるが daemon がない」状態の両方が起こる。
- 変更後は、`.app` に UI と daemon が同梱され、UI が daemon を子プロセスとして起動・監視・停止する。UI が生きている間だけ daemon が動き、UI が終了すれば daemon も終了するため、両者のライフサイクルが一致する。ログイン時に起動と最小化状態で起動の設定は UI の LaunchAgent 1 つに対応し、二重登録されず、`.app` を削除すればログイン時の起動も試みられなくなる。daemon の起動失敗は段階ごとに区別され、有限の期限と試行上限の範囲で自動再起動が行われ、回復できない場合も理由の確認と終了ができる。更新と再起動では、旧 daemon の停止完了を確認してから新 daemon を起動し、接続先とリリースの一致を確認してから操作を受け付ける。

# Current Behavior

最初の周の開始時点（`feat/issues/1203`、`00b57d77`）で、コードを読んで確認した挙動。アプリケーションの起動・テストの実行による確認は行っていない。

## Issue 本文の現状記述との差

- Issue #1203 は「quit は `ShutdownCoordinator` が workflow runtime / terminal surface / provider exit observer / local API を一括停止する」としているが、#1202 以後 `ShutdownCoordinator` は daemon が所有する。desktop のメニュー・トレイの Quit は desktop プロセスだけを終了させ、daemon へ停止要求を送らない。
- Issue #1203 は「設定を 2 つ追加する」としているが、「Launch at login」（`auto_launch`）と「Start minimized」（`start_minimized`）は `src/components/panels/SettingsModal.tsx` に既にあり、前者は `@tauri-apps/plugin-autostart` の `enable()` / `disable()`、後者は `releash.toml` の `start_minimized` に対応している。
- Issue #1203 が現状とする `tauri-plugin-autostart`（`MacosLauncher::LaunchAgent`、`--hidden`）の登録は `src-tauri/src/desktop.rs` に現存する。

## プロセスと起動

- 実行ファイルは 2 つである。`releash`（Tauri。`main.rs` が `desktop::run` を呼ぶ GUI 専用）と、`releash-backend`（`src/bin/backend.rs`。`--internal-daemon [data_dir]` で daemon、それ以外の引数で CLI の `workflow` / `review` / `hook`）。
- desktop は daemon を起動しない。`desktop.rs` の Tauri `setup` は data_dir を解決し、`ClientConnectionFileQuery` が読む client 用 discovery file を手がかりに起動済み daemon のクライアント ws へ接続して、`ClientHello` から desktop の設定を取得する。daemon が起動していなければこの取得が失敗し、`setup` がエラーを返して desktop は main window を作らずに終了する。
- daemon の backend の初期化の失敗は、local event store の open error から 7 種（`StoreInUse` / `StorageUnavailable` / `UnsupportedRuntime` / `UnsupportedStoreVersion` / `InitializationStateInvalid` / `StoreValidationFailed` / `SchemaEvolutionFailed`）へ分類され、種別ごとに `retry_on_next_launch` を持つ（`src-tauri/src/usecase/application_startup.rs`、`src-tauri/src/adaptor/controller/daemon.rs`）。daemon のプロセスの終了コードは、いずれの種別でも 1 である。
- daemon は `<data_dir>` の local event store を開き、writer lock を保持する。lock を取得できない場合は起動を拒否し、`Local data is currently in use ...` と correlation id を stderr と local log に出して終了コード 1 で終了する（`src-tauri/tests/daemon_smoke.rs`）。同じ data_dir の daemon の重複起動は、この writer lock で拒否される。
- daemon の local API とクライアント ws は `127.0.0.1` のエフェメラルポート（`std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))`）に bind し、master token を含む `<data_dir>/local-api.json` と、client / terminal 用 token を含む別の discovery file を書き出す。固定ポートの bind による単一インスタンス制御はない。UI プロセスの重複起動を防ぐ仕組みもない。
- `/usr/local/bin/releash` の symlink は daemon の起動時に設置され、`releash-backend` を指す（`src-tauri/src/infrastructure/platform/cli_install.rs`）。直接 symlink を作れない場合は `osascript` の `do shell script ... with administrator privileges` を実行し、管理者の認証を求める。

## .app への同梱

- `src-tauri/tauri.conf.json` の `bundle.resources` は空で、`externalBin` の指定はない。`releash-backend` は `.app` に同梱されない。CI は `cargo build --locked --no-default-features --bin releash-backend` で別に build する。

## LaunchAgent

- `.app` に `Contents/Library/LaunchAgents/` は無く、`SMAppService` は使っていない。
- `tauri-plugin-autostart` は `auto-launch` crate の LaunchAgent 方式で `~/Library/LaunchAgents/<app name>.plist` を書く。plist は `Label`、`ProgramArguments`（desktop の実行ファイルのパスと `--hidden`）、`RunAtLoad` を持ち、`KeepAlive` は書かない。登録は desktop のもの 1 つだけで、daemon の LaunchAgent は存在しない。
- 有効・無効の切替は renderer の `src/hooks/useAppSettings.ts` が `enable()` / `disable()` を呼んで行う。設定画面の表示値は OS の `isEnabled()` から読み、`releash.toml` の `auto_launch` は保存されるだけで表示には使われない。
- plist を除去する経路は、設定で「Launch at login」を無効にする操作だけである。`.app` を削除しても plist は残る。
- LaunchAgent から起動された desktop も daemon を起動しないため、接続先が無く起動に失敗する。

## ウィンドウと起動時の表示

- 起動引数に `--hidden` があり、かつ `start_minimized` が真のときだけ、起動時に main window を hide（`close_to_tray` が真）または minimize（偽）する。main window は常に生成される。
- ウィンドウを閉じる操作は `close_to_tray` に従って hide または minimize し、プロセスを終了しない。tray のアイコンは残る。
- 起動失敗ウィンドウ（`startup-failure`）と `get_application_startup_outcome` / `quit_after_startup_failure` の経路は残っているが、現行の `desktop.rs` は起動 authority を常に `ready()` で作り、このウィンドウを生成する本番経路はない。
- メニューバーのアイコンはカラーの `icons/32x32.png` をそのまま使い、template 画像の指定（`icon_as_template`）は無い。

## Quit

- app menu の Quit と Cmd+Q は `PredefinedMenuItem::quit`（`src-tauri/src/infrastructure/platform/menu.rs` の `.quit()`）であり、`NSApplication` の `terminate:` を直接呼ぶ。`applicationShouldTerminate` を実装していないため Tauri の `ExitRequested` は起きず、`ApplicationQuitIngress` を経由しない。Dock アイコンの終了と AppleScript の quit も同じく `terminate:` を呼ぶ。
- tray の Quit は `mark_quit_requested()` の後に `app.exit(0)` を呼ぶ。`ExitRequested` は起きるが、`should_prevent_exit()` が偽になるため `ApplicationQuitIngress` を経由せずそのまま終了する。
- いずれの終了操作も desktop プロセスだけを終了させ、クライアント ws の `request_application_quit` を送らない。daemon と実行中の workflow は動き続ける。
- daemon は `request_application_quit` を受けると `ShutdownCoordinator` の一括停止（workflow runtime / terminal surface / provider exit observer / local API）を行い、完了後に daemon のプロセスを終了する（#1202 R-015 / B-016）。一括停止は preparation と decision の 2 つの期限（13 秒 / 15 秒）を持ち、超えると `OutcomeUnknown` になる。段階には利用者の判断（`RecoveryAction`）を必要とする `ReconciliationRequired` がある。daemon が起動した terminal の PTY と provider のプロセスを片付ける経路は、この一括停止の中にしかない。この command を通常の Quit から呼ぶ本番コードはない。renderer は `src/hooks/useApplicationShutdownSupervision.ts` の未確認 attempt の再送と `retryQuit` からだけ呼ぶ。
- daemon が親プロセスの終了を検知する経路はない。

## 更新

- 更新の確認・ダウンロード・適用は renderer の `src/hooks/useUpdateChecker.ts` が `@tauri-apps/plugin-updater` と `@tauri-apps/plugin-process` の `relaunch()` で行う。適用の前に daemon を停止する処理、停止完了を確認する処理、新 daemon を起動する処理はない。`relaunch()` は desktop プロセスだけを再起動する。

## 接続先の検証

- desktop は discovery file の `pid` と `process_started_at` に一致する生存プロセスがあることと、`instance_id` の一致を確かめてから接続先を受理する。
- desktop と daemon のリリース（version）を照合する情報は、discovery file にも `ClientHello`（`instance_id`、各種期限、`DesktopSettings`）にもない。

# Scope / Non-goals

## 変更するもの

- `.app` への daemon の実行ファイルの同梱と、UI からの起動。
- UI による daemon の起動・監視・停止。起動時の spawn と起動完了までの待機、異常終了時の再 spawn、親の終了検知による daemon の自己終了、Quit での停止要求と一括停止完了の待機を含む。
- daemon の起動失敗の段階の区別、起動待ちの期限、自動再起動の待機間隔と試行上限、回復不能または上限到達時の理由の表示・終了操作・利用者による再試行。
- LaunchAgent の登録方式と登録の一本化、および `.app` の削除後に起動が試みられないこと。
- ログイン時に起動・最小化状態で起動の設定と、LaunchAgent および起動時の表示との対応。
- 更新・再起動時の UI と daemon の切替。停止完了の確認、新 daemon の起動、接続先とリリースの一致の確認、切替中の操作受付の停止と再開を含む。
- 接続先が UI の起動した daemon インスタンスであることと、UI と daemon のリリースが一致することの検証。
- 同じ data_dir に対する daemon の単一インスタンスの扱い。
- 利用者の終了操作（アプリケーションメニューの Quit、Cmd+Q、Dock アイコンの終了、メニューバーのアイコンの Quit、AppleScript の quit）と、macOS のログアウト・再起動・システム終了による終了の扱い。
- daemon の起動完了を待つ間のウィンドウの表示。
- ログイン項目の登録が承認を必要とする状態、登録が失われた状態、登録できない状態の扱い。
- `/usr/local/bin/releash` を設置する契機。
- メニューバーのアイコンの表示。

## 変更しないもの

- CLI に daemon を単体で起動する利用者向けの入口を作ること（#1202 R-013）。
- launchd の `KeepAlive` による UI または daemon の自動復活。
- macOS 以外のプラットフォームへの対応。
- `.app` の削除後に、macOS の Background Task Management の登録レコードと、System Settings のログイン項目一覧の表示が残ること。
- UI shell に残す物（dialog（フォルダ選択）、opener、updater＋process relaunch、autostart、`native-file-drop`、`menu-event`、menu / tray / window lifecycle）をクライアント ws へ載せること。
- CLI と provider hook が使う HTTP local API（workflow / provider-lifecycle）の入口。
- #1202 で確定した実行ファイルの構成（desktop shell と、daemon モードを含む Tauri 非依存の実行ファイルの 2 つ）。
- #1201 が定めた、変更要求の未送信・結果不明・結果確定の区別、期限、生存確認、再接続の規則そのもの。
- `.proto` を protocol の正とするエンベロープの形式（判断①）。
- 旧 `tauri-plugin-autostart` の登録から新しい登録方式への移行。
- `/usr/local/bin/releash` の設置先と、Tauri を使わない実行ファイルを指すというリンク先。
- リモートアクセス経路と、複数クライアント。

# Requirements

- R-001: `Releash.app` は daemon の実行ファイルを含み、UI は `.app` の外に別途配置した実行ファイルを必要とせずに daemon を起動できる。
- R-002: UI を起動すると、UI が daemon を子プロセスとして起動する。UI が通常の操作を受け付けるのは、daemon の backend が Ready に達し、認証済みのクライアント ws 接続が成立した後である。
- R-003: ウィンドウを閉じる操作は UI のウィンドウにだけ作用する。メニューバーのアイコンは残り、UI プロセス、daemon、実行中の workflow は継続し、閉じた後に再びウィンドウを開ける。
- R-004: daemon が異常終了した場合、UI は待機間隔と試行上限の範囲で daemon を再び起動し、再接続した後は現在の状態が画面へ反映される。
- R-005: UI プロセスが終了した場合、daemon はそれを検知して自ら終了する。このとき daemon は一括停止（workflow runtime / terminal surface / provider exit observer / local API）を行わない。daemon が起動した terminal と provider のプロセスも終了し、UI のない daemon とその子プロセスが残らない。
- R-006: 利用者の終了操作（アプリケーションメニューの Quit、Cmd+Q、Dock アイコンの終了、メニューバーのアイコンの Quit、AppleScript の quit）を行うと、UI が daemon へ停止要求を送り、daemon の一括停止（workflow runtime / terminal surface / provider exit observer / local API）のうち利用者の判断を必要としない段階が完了してから、UI と daemon の両方が終了する。一括停止が有限の期限内に完了しない場合も、UI と daemon は終了する。
- R-007: Quit の後に `Releash.app` を起動すると、UI と daemon の両方が起動し、UI は R-002 の接続完了まで待ってから利用可能になる。
- R-008: ログイン時に起動の設定を有効にしてログインすると、メニューバーのアイコンと daemon が起動する。無効にしてログインした場合は、どちらも起動しない。
- R-009: 最小化状態で起動の設定が有効なとき、ログイン時の起動ではウィンドウが表示されず、メニューバーのアイコンだけが出る。
- R-010: ログイン時に起動が有効なとき、Releash の LaunchAgent の登録は 1 つだけであり、UI と daemon で二重に登録されない。
- R-011: `Releash.app` を削除した後は、ログインしても Releash の起動が試みられない。
- R-012: UI プロセスがクラッシュした場合、daemon も終了する。`Releash.app` を起動するまで、UI も daemon も自動で起動し直されない。
- R-013: 同じ data_dir に対して daemon は同時に 1 つだけ動作する。
- R-014: daemon が起動していない状態での CLI（`releash workflow` / `releash review`）と provider hook（`releash hook`）の挙動は変更前と同じである。Releash の CLI のヘルプと利用者向けガイド（`docs/guide/`）に、daemon を単独起動する手段は現れない。
- R-015: daemon の起動が失敗した場合、UI は失敗した段階（spawn、backend の初期化、起動期限超過、起動後の異常終了）と理由を表示する。クライアント ws が確立しない場合も表示できる。
- R-016: daemon の起動待ちには有限の期限がある。期限を超えた場合、UI は起動した子プロセスを停止し、その終了を確認してから次の起動を行う。UI は無限に待機せず、daemon が重複して起動することもない。
- R-017: 自動再起動には待機間隔と試行上限がある。Ready に達した直後のクラッシュを繰り返しても試行上限は回避されず、UI は無限に再起動しない。
- R-018: 自動再起動の対象は、daemon プロセスの異常終了（終了コードが 0 以外、またはシグナルによる終了）と、起動期限の超過である。daemon の正常終了と spawn の失敗では自動再起動を行わない。失敗の種類によって自動再起動の可否を分けない。自動再起動の対象外の失敗、または試行上限に達した場合、UI は自動再起動を止め、理由と終了操作を表示する。再試行できる場合は、利用者が明示的に再試行できる。
- R-019: Quit を開始した後は再起動の予約が解除され、daemon の終了を契機に UI が daemon を再び起動することはない。自動再起動の待機中に Quit した場合も同じである。
- R-020: UI が通常の操作を受け付けるのは、接続先が UI の起動した daemon インスタンスであり、UI と daemon のリリースが一致することを確認した後である。古い接続情報による別インスタンスへの接続や、リリースの不一致では、通常の操作を受け付けず、失敗した段階と理由を表示する。
- R-021: 更新の適用では、必要な利用者の判断を経て daemon の一括停止を完了し、旧 daemon の終了を確認してから新 daemon を起動する。実行中の workflow がある場合も同じである。
- R-022: 更新・再起動のための意図的な停止では、daemon の終了を契機とする自動の再起動は行われない。旧 daemon と新 daemon が同じデータ領域を同時に使用することはない。
- R-023: 切替の間、UI は通常の操作を受け付けない。新 daemon が Ready に達し、接続先とリリースの一致と認証済みのクライアント ws 接続を確認し、必要な状態を再取得した後に受付を再開する。
- R-024: daemon の停止結果が不明な場合、停止完了とみなして切替を進めない。
- R-025: 更新の適用失敗、新 daemon の起動失敗、接続先またはリリースの不一致では、失敗した段階と理由が表示され、利用者は原因を確認して終了できる。新 daemon の起動失敗とその再試行は R-015 から R-019 に従う。
- R-026: 更新を伴わない再起動（application quit の Restart の意図）でも、旧 daemon の一括停止の完了と終了を確認してから、新しい daemon と UI が起動し、R-020 と R-023 の確認の後に利用可能になる。
- R-027: daemon の再起動、更新に伴う切替、更新を伴わない再起動をまたいで結果を確定できない変更要求は、結果不明として表示され、処理失敗としては表示されず、安全を確認できない再実行は自動でも利用者の再試行でも行われない。結果不明の記録は、利用者がその結果不明を確認したときにだけ破棄され、破棄した後は同じ対象への変更要求が通常どおり受け付けられる。利用者が確認するまで、この記録は自動で破棄されない。
- R-028: Releash が既に動作している状態で `Releash.app` を起動すると、既に動作している UI がアクティブになり、2 つ目の UI プロセスは終了する。2 つ目の daemon は起動されない。
- R-029: macOS のログアウト、再起動、システム終了による終了でも、R-006 と同じ停止が行われ、UI と daemon の両方が終了する。この停止はログアウト、再起動、システム終了を妨げない。
- R-030: ログイン時に起動の登録が macOS の承認を必要とする状態のとき、ログイン時に起動の設定は無効として示され、承認が必要であることと承認を行う場所への導線が示される。この状態でも UI の起動は失敗しない。
- R-031: `Releash.app` を起動すると、daemon の起動完了を待つ間もウィンドウが表示され、起動中であることが示される。最小化状態で起動の設定が有効なログイン時の起動では、このウィンドウも表示されない。
- R-032: ログイン時に起動を有効にしていた状態で OS のログイン項目の登録が失われた場合、UI の起動時に現在の場所で登録し直され、ログイン時に起動の設定は有効のままになる。
- R-033: `Releash.app` が読み取り専用の一時的な場所で実行されている場合、ログイン時に起動の登録は行われず、登録できない理由が示される。
- R-034: `/usr/local/bin/releash` の設置は、利用者が設定で明示的に操作したときにだけ行われる。UI または daemon の起動時には設置されず、起動時に管理者の認証を求められない。設置に管理者の認証が必要な場合は、その明示操作の中で求められる。
- R-035: メニューバーのアイコンは、ライトとダークのメニューバー、メニューバーの色付け、アイコンの選択状態に追従して表示される。

# Assumptions / Open Questions

- Assumption（自動判断）: R-015 が列挙する失敗の段階のうち「起動後の異常終了」だけ、段階と理由の表示を判定する受入条件が無かった。同じ状況を扱う B-020 へ、段階と理由の表示を追加した。
- Assumption（自動判断）: R-025 が列挙する 3 つの失敗のうち「接続先またはリリースの不一致」だけ、終了できることを判定する受入条件が無かった。同じ状況を扱う B-023 と B-024 へ、終了できることを追加した。
