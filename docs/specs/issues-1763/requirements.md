# Context

- 要求の正本: [siro33950/releash#1763](https://github.com/siro33950/releash/issues/1763)
- 補助資料: マイルストーン [#78](https://github.com/siro33950/releash/milestone/78) の「確定した設計」と「UI の決まり」、[#1764](https://github.com/siro33950/releash/issues/1764)、[#1765](https://github.com/siro33950/releash/issues/1765)、[#1766](https://github.com/siro33950/releash/issues/1766)、[#2011](https://github.com/siro33950/releash/issues/2011)、[#2036](https://github.com/siro33950/releash/issues/2036)
- 対象の commit: main `226c9038`

確定済みの制約は次のとおりである。

- Swift.app の画面は SwiftUI の標準の部品と標準の style を使い、色はシステムの色、文字はシステムの文字の段階を使う（マイルストーン #78「UI の決まり」）。
- Swift.app は自前でロジックを持たず、設定の読み書きは proto の RPC と購読で行う（`AGENTS.md`「サーバがロジックを所有する」）。
- Settings の各項目は、macOS の標準に従い、利用者が変更を確定した時点で反映と保存を行う。一括保存の操作は置かない。
- ログイン項目の登録、解除、起動時の復元は #1764 が行う。この変更では、設定の保存と状態の表示だけを行う。承認待ちの表示は、登録が入る #1764 までは実機で観測できないため、状態を差し替えた単体テストで確かめる。
- React の変更は `close_to_tray` の削除に伴う分に限る。React の Crash reporting の表示元は localStorage のまま #1765 の撤去まで残す。
- `UpdateAppSettings` の RPC 自体を消すかは #2036 が決める。この変更では `start_minimized` だけを持つ形にする。
- proto の field の削除は互換性を壊す変更であり、PR に `buf skip breaking` ラベルが要る（`AGENTS.md`「ビルド・テスト・Lint」）。

# Outcome

対象者は Swift.app を使う開発者である。

Swift.app には Settings が無いため、開発者は外部エディタ、repository ごとの base branch と Notion、provider の実行ファイル、承認の自動化、Terminal の起動コマンド、ログイン時の起動、telemetry のどれも Swift.app から変えられない。
Diff base の既定を変える画面はどのクライアントにも無い。
サーバの設定ファイルと proto には、どのクライアントも使わない `app.last_root_path` と、Swift.app に選ぶ余地が無い `app.close_to_tray` が残っている。

変更後、開発者は Swift.app の Settings で上記の項目を見て変えられ、変えた値はサーバの設定か Swift.app の UserDefaults に保存され、開き直しても残る。
`last_root_path` と `close_to_tray` は、サーバの設定ファイル、proto、Tauri シェル、React のどこにも残らない。

# Current Behavior

Swift.app の Settings scene は空の view で、app メニューの Settings 項目も消している（`clients/macos/Sources/ReleashApp.swift:8-10`）。
⌘, を押しても何も開かない。

Swift.app が設定を読む箇所は次の 3 つだけである。

- 外部エディタで開く操作は RPC `OpenInEditor` を呼び、サーバが `app.external_editor` を読む（`clients/macos/Sources/FileTabPane.swift:144-151`、`server/src/usecase/external_editor/open_usecase.rs:5-12`）。
- Diff base の既定は UserDefaults の `diffBase` を読み、無ければ `head` にする。値は `head` と `branch-base` である（`clients/macos/Sources/AppModel.swift:606`、`clients/macos/Tests/Integration/FileTabPaneTests.swift:102`）。書く画面は無い。
- Worktree 作成の Notion task は購読 `notion-tasks` で取り、サーバが `releash.toml` の `notion` 節を読む（`clients/macos/Sources/CreateWorktreeView.swift:160`、`server/src/usecase/state_subscription/reads.rs:454-457`）。

Swift.app が設定を書く箇所は、サイドバーからの repository の追加だけである（`clients/macos/Sources/AppModel.swift:632-645`）。

React の Settings は下書きを一括で保存する作りで、各項目を次の RPC と購読で読み書きしている（`clients/desktop/src/components/panels/SettingsModal.tsx`）。

| 項目 | 読み | 書き |
| --- | --- | --- |
| External editor | 購読 `external-editor`（選択中と検出済みの一覧） | `UpdateExternalEditor` |
| Repository の削除 | 購読 `workspaces` | `RemoveRepoPath` |
| Repository ごとの Base branch | 購読 `releash-base`、`branches` | `SetReleashBase`（空は自動） |
| Repository ごとの Notion | 購読 `notion-config` | `ValidateNotionConfig`、`SaveNotionConfig`、`DeleteNotionConfig` |
| Provider の可用性と実行ファイルの上書き | 購読 `provider-availability` | `UpdateProviderExecutable`、`ResetProviderExecutable`、`RefreshProviderAvailability` |
| 承認の自動化 | 購読 `workflow-config` | `UpdateWorkflowConfig` |
| Terminal の起動コマンド | 購読 `terminal-startup-command` | `UpdateTerminalStartupCommand` |
| Close to tray、Start minimized | 購読 `desktop-settings` | `UpdateAppSettings` |
| Launch at login | Tauri コマンド `get_login_item_status` | Tauri コマンド `set_login_item_enabled` と `UpdateLoginItemPreference` |
| CLI の設置 | なし | `InstallCli` |
| Performance telemetry | 購読 `desktop-settings` | `UpdatePerformanceTelemetry` |
| Crash reporting | client の localStorage | `UpdateCrashReporting` |

React の「Start minimized」は「Launch at login」が無効のとき変更できない（`SettingsModal.tsx:786-797`）。

`close_to_tray` の読み手は Tauri シェルだけで、ウィンドウを閉じたとき隠すか最小化するかを決めている（`clients/desktop/src-native/infrastructure/platform/window_lifecycle.rs:66-92`）。
サーバ側は domain（`server/src/domain/app_config/value_objects/mod.rs:16`）、設定ファイルの model（`server/src/adaptor/gateway/app_config/config_models.rs:133-134`、既定は真）、usecase（`server/src/usecase/app_config/usecase.rs:82-92`）、query service（`server/src/usecase/app_config/query_service.rs:9`、`:54`）、presenter と controller（`server/src/adaptor/presenter/client/mod.rs:152`、`:168`、`server/src/adaptor/controller/client/app_config/commands.rs:29-34`）、proto の `WindowSettings.close_to_tray` と `DesktopSettings.close_to_tray`（`proto/client.proto:2270`、`:2638`）が持つ。

`last_root_path` を書く箇所はテスト以外に無く、読むのは workflow の worktree 解決だけである（`server/src/adaptor/gateway/workflow/worktree_gateway.rs:12-18`）。

サーバの設定ファイルの model は未知のキーを拒否しないため、削除したキーが残る `releash.toml` も読める（`config_models.rs` に `deny_unknown_fields` は無い）。

CLI の設置は RPC `InstallCli` がサーバ側で完結し、サーバの実行ファイルと同じディレクトリの `releash` を `/usr/local/bin/releash` に symlink する（`server/src/usecase/installation.rs:22-40`、`server/src/domain/installation/mod.rs:85-96`）。
Swift.app は `releashd` と `releash` を `Contents/Helpers` に同梱する（`clients/macos/project.yml:58-73`）。
置いてよい条件の判定と理由は購読 `daemon-info` の `cli_installation` で届く。
ログイン項目を登録してよいかの判定は RPC `CheckLoginRegistration` が返し、理由は AppTranslocation と読み取り専用の 2 つである（`server/src/domain/installation/mod.rs:46-60`）。

# Scope / Non-goals

変更する対象:

- Swift.app の Settings 画面。引き継ぐ項目は Requirements の R-002 から R-012 のとおり。
- Diff base の既定を変える項目（Swift.app の UserDefaults `diffBase`）。
- `app.close_to_tray` の削除。サーバの domain、設定ファイルの model、usecase、query service、presenter、controller、proto の `WindowSettings` と `DesktopSettings`、Tauri シェル、React、各テスト。
- `app.last_root_path` の削除。サーバの domain、設定ファイルの model、worktree 解決、テスト。
- `UpdateAppSettings` を `start_minimized` だけを持つ形にする変更と、それに伴う React の変更。
- 「設定を変更すると保存され、開き直しても残る」を確かめる回帰の E2E（XCUITest）。

変更しない対象:

- Theme と Font size。明暗は macOS の外観の設定に従い、文字はシステムの文字の段階を使う。
- Diff mode の既定と Diff only mode。Swift.app ではファイルタブの帯の `diffStyle` と `diffFull` が担う。
- Auto update と更新の節（#1766）。
- 定義と Facet の画面（#2011）。
- ログイン項目の登録、解除、復元と、最小化状態での起動の実際の動作（#1764）。
- CLI の設置で置いてよい条件の判定と配置の規則（サーバの `domain/installation/` のまま）。
- `UpdateAppSettings` の RPC 自体の削除（#2036）。
- React の localStorage の項目の削除と、React の Crash reporting の表示元の変更（#1765）。
- Repository の追加（サイドバーの既存の操作のまま）。
- Notion task の取得と Worktree 作成の仕様。

# Requirements

- R-001: 開発者は、app メニューの Settings 項目と ⌘, で Swift.app の Settings を開ける。
- R-002: 開発者は、検出済みの外部エディタの一覧とシステム既定から外部エディタを選べる。選択はサーバの `app.external_editor` に保存され、ファイルタブの「外部エディタで開く」はその選択に従う。
- R-003: 開発者は、登録済みの repository を一覧で見て、一覧から削除できる。削除の前に確認があり、ディスク上の repository は消えない。削除した repository は Settings の一覧とサイドバーから消える。
- R-004: 開発者は、repository ごとに base branch を branch の一覧から選ぶか、自動（main または master）にできる。選択は repository の git config `releash.base` に保存される。
- R-005: 開発者は、repository ごとに Notion の API token と Database ID を入力し、接続テストの結果（成功、token 不正、database 不正、network の失敗、未設定）を見られる。接続テストに成功すると、database の property から title、labels、branch 名、branch 名の prefix を選べる。設定は保存と削除ができ、保存した設定は Worktree 作成の Notion task の取得に使われる。
- R-006: 開発者は、provider ごとに可用性（利用可能か否か）、既定の実行ファイル、有効な実行ファイル、解決された実行ファイルまたは利用できない理由を見られる。実行ファイルの上書きの入力、上書きのリセット、可用性の再確認ができ、上書きはサーバの `agents.<provider>.cli_path` に保存される。
- R-007: 開発者は、承認の自動化を有効または無効にできる。値はサーバの `workflow.approval_auto_approve` に保存される。
- R-008: 開発者は、Terminal の起動コマンドを入力できる。値はサーバの `app.terminal_startup_command` に保存され、以後に開く Terminal タブで実行される。
- R-009: 開発者は、Diff base の既定を HEAD または Branch base から選べる。選択は Swift.app の UserDefaults `diffBase` に保存され、以後に開くファイルタブの比較はそれに従う。
- R-010: 開発者は、「ログイン時に起動」と「最小化状態で起動」を有効または無効にできる。値はそれぞれサーバの `app.auto_launch` と `app.start_minimized` に保存される。「最小化状態で起動」は「ログイン時に起動」が有効なときだけ変更できる。macOS のログイン項目が承認待ちのときはその旨とシステム設定のログイン項目を開く導線が表示され、Swift.app が登録できない場所で実行されているときはサーバが返す理由が表示される。
- R-011: 開発者は、CLI を `/usr/local/bin/releash` に設置できる。結果（設置した、または設置済み）と設置先のパス、失敗したときはその理由が表示される。
- R-012: 開発者は、performance telemetry と crash reporting を有効または無効にできる。表示の正は購読 `desktop-settings` の値で、変更はサーバの `telemetry.performance_telemetry` と `telemetry.crash_reporting` に保存される。
- R-013: R-002 から R-012 の各項目は、開発者が変更を確定した時点で保存され、Settings を閉じて開き直しても、Swift.app を再起動しても、保存した値が表示される。保存に失敗したときは理由が表示され、表示は保存済みの値に戻る。
- R-014: `app.close_to_tray` は、サーバの設定ファイル、proto、Tauri シェル、React のどこにも残らない。Tauri シェルはウィンドウを閉じると常に隠す。`UpdateAppSettings` は `start_minimized` だけを受け取る。
- R-015: `app.last_root_path` は、サーバの設定ファイルと proto のどこにも残らない。workflow の worktree 解決は `app.last_repo_paths` だけを使う。
- R-016: 削除したキーが残る既存の `releash.toml` を、サーバは引き続き読める。
- R-017: React の Settings は、`close_to_tray` の項目が消えた状態で、Swift.app と同じ RPC と購読で引き続き動く。

# Assumptions

- Settings の各項目は macOS の標準に従い、変更を確定した時点で反映と保存を行う。一括保存の操作は置かない。
- ログイン項目のトグルは設定の保存だけを行い、登録と解除は行わない。
- React の変更は `close_to_tray` の削除に伴う分に限る。

# Invariants

なし。

# Risks

なし。
