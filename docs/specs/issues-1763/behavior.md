## B-001: Settings を開く

GIVEN Swift.app がサーバに接続している
WHEN 開発者が app メニューの Settings 項目を選ぶか ⌘, を押す
THEN Settings のウィンドウが開く

## B-002: 外部エディタの選択

GIVEN Settings が開いている
WHEN 開発者が検出済みの外部エディタの一覧から一つを選ぶ
THEN 選択がサーバに保存され、購読 `external-editor` の選択中の値がそれになる
AND ファイルタブの「外部エディタで開く」がそのエディタで開く

## B-003: Repository の削除

GIVEN Settings に登録済みの repository が一覧されている
WHEN 開発者が一つの repository の削除を選び、確認に応じる
THEN その repository が Settings の一覧とサイドバーから消える
AND ディスク上の repository は残る

## B-004: Base branch の選択

GIVEN Settings に repository が一覧されている
WHEN 開発者がその repository の base branch を branch の一覧から選ぶ
THEN 選択がその repository の git config `releash.base` に保存され、購読 `releash-base` の値がそれになる

## B-005: Base branch を自動に戻す

GIVEN repository の base branch が branch に設定されている
WHEN 開発者が base branch を自動にする
THEN git config `releash.base` が消え、購読 `releash-base` の値が空になる

## B-006: Notion の接続テスト

GIVEN Settings に repository が一覧されている
WHEN 開発者がその repository の API token と Database ID を入力して接続テストを行う
THEN 結果（成功、token 不正、database 不正、network の失敗、未設定）が表示される
AND 成功したときは database の property から title、labels、branch 名、branch 名の prefix を選べる

## B-007: Notion 設定の保存

GIVEN repository の Notion の接続テストに成功している
WHEN 開発者が property の対応を選んで保存を確定する
THEN 設定がサーバに保存され、購読 `notion-config` の値がそれになる
AND その repository の Worktree 作成で Notion task を取得できる

## B-008: Notion 設定の削除

GIVEN repository に Notion の設定が保存されている
WHEN 開発者がその設定を削除する
THEN 購読 `notion-config` の値が空になる

## B-009: Provider の可用性の表示

GIVEN Settings が開いている
WHEN 開発者が provider の項目を見る
THEN provider ごとに可用性、既定の実行ファイル、有効な実行ファイル、解決された実行ファイルまたは利用できない理由が表示される

## B-010: Provider の実行ファイルの上書き

GIVEN provider の項目が表示されている
WHEN 開発者が実行ファイルの上書きを入力して確定する
THEN 上書きがサーバに保存され、購読 `provider-availability` の有効な実行ファイルがそれになる

## B-011: Provider の実行ファイルのリセット

GIVEN provider の実行ファイルが上書きされている
WHEN 開発者が上書きをリセットする
THEN 購読 `provider-availability` の有効な実行ファイルが既定に戻る

## B-012: Provider の可用性の再確認

GIVEN provider の項目が表示されている
WHEN 開発者が可用性の再確認を行う
THEN 購読 `provider-availability` の可用性が再確認後の値になる

## B-013: 承認の自動化の切り替え

GIVEN Settings が開いている
WHEN 開発者が承認の自動化を切り替える
THEN 値がサーバに保存され、購読 `workflow-config` の値がそれになる

## B-014: Terminal の起動コマンドの設定

GIVEN Settings が開いている
WHEN 開発者が Terminal の起動コマンドを入力して確定する
THEN 値がサーバに保存され、購読 `terminal-startup-command` の値がそれになる
AND 以後に開く Terminal タブでそのコマンドが実行される

## B-015: Diff base の既定の選択

GIVEN Settings が開いている
WHEN 開発者が Diff base の既定を Branch base にする
THEN 以後に開くファイルタブの比較が Branch base になる

## B-016: ログイン時に起動の切り替え

GIVEN Settings が開いている
WHEN 開発者が「ログイン時に起動」を切り替える
THEN 値がサーバに保存され、購読 `desktop-settings` の `autoLaunch` がそれになる

## B-017: 最小化状態で起動の切り替え

GIVEN 「ログイン時に起動」が有効である
WHEN 開発者が「最小化状態で起動」を切り替える
THEN 値がサーバに保存され、購読 `desktop-settings` の `startMinimized` がそれになる

## B-018: 最小化状態で起動の変更の抑止

GIVEN 「ログイン時に起動」が無効である
WHEN 開発者が「最小化状態で起動」を見る
THEN 変更できない状態で表示される

## B-019: ログイン項目の承認待ちの表示

GIVEN macOS のログイン項目が承認待ちである
WHEN 開発者が Settings のログイン項目の節を見る
THEN 承認待ちである旨と、システム設定のログイン項目を開く導線が表示される

## B-020: 登録できない場所での表示

GIVEN Swift.app が AppTranslocation または読み取り専用の場所で実行されている
WHEN 開発者が Settings のログイン項目の節を見る
THEN サーバが返す理由が表示される

## B-021: CLI の設置

GIVEN Swift.app が設置を許される場所で実行されている
WHEN 開発者が CLI の設置を行う
THEN `/usr/local/bin/releash` が同梱の CLI を指し、設置した旨と設置先のパスが表示される

## B-022: CLI の設置の失敗

GIVEN Swift.app が設置を許されない場所で実行されている
WHEN 開発者が CLI の設置を行う
THEN 失敗の理由が表示される

## B-023: telemetry の切り替え

GIVEN Settings が開いている
WHEN 開発者が performance telemetry または crash reporting を切り替える
THEN 値がサーバに保存され、購読 `desktop-settings` の対応する値がそれになる

## B-024: 変更の保持

GIVEN 開発者が Settings のある項目を変更して確定した
WHEN Settings を閉じて開き直す、または Swift.app を再起動して Settings を開く
THEN 変更後の値が表示される

## B-025: 保存の失敗

GIVEN Settings が開いている
WHEN 開発者が項目を変更して確定し、サーバが保存を拒否する
THEN 失敗の理由が表示され、項目の表示は保存済みの値に戻る

## B-026: close_to_tray の削除

GIVEN Tauri シェルで Swift.app と同じサーバに接続している
WHEN 開発者がウィンドウを閉じる
THEN ウィンドウは隠れる
AND `close_to_tray` はサーバの設定ファイル、proto、Tauri シェル、React のどこにも無い

## B-027: UpdateAppSettings の形

GIVEN サーバが起動している
WHEN クライアントが `start_minimized` だけを持つ `UpdateAppSettings` を送る
THEN サーバの `app.start_minimized` がその値になる

## B-028: last_root_path の削除

GIVEN `app.last_repo_paths` に repository が登録されている
WHEN workflow がその repository の worktree を解決する
THEN `app.last_repo_paths` だけから解決する
AND `last_root_path` はサーバの設定ファイルと proto のどこにも無い

## B-029: 旧キーが残る設定ファイルの読み込み

GIVEN `releash.toml` に `close_to_tray` または `last_root_path` のキーが残っている
WHEN サーバが設定を読む
THEN 読み込みに成功し、他の設定の値が保たれる

## B-030: React の Settings の継続

GIVEN React の画面がサーバに接続している
WHEN 開発者が React の Settings で「Start minimized」を切り替えて保存する
THEN 値がサーバに保存される
AND React の Settings に「Minimize to tray on close」の項目は無い

## 要件IDとBehavior IDの対応表

| Requirement ID | Behavior ID |
| --- | --- |
| R-001 | B-001 |
| R-002 | B-002 |
| R-003 | B-003 |
| R-004 | B-004、B-005 |
| R-005 | B-006、B-007、B-008 |
| R-006 | B-009、B-010、B-011、B-012 |
| R-007 | B-013 |
| R-008 | B-014 |
| R-009 | B-015 |
| R-010 | B-016、B-017、B-018、B-019、B-020 |
| R-011 | B-021、B-022 |
| R-012 | B-023 |
| R-013 | B-024、B-025 |
| R-014 | B-026、B-027 |
| R-015 | B-028 |
| R-016 | B-029 |
| R-017 | B-030 |
