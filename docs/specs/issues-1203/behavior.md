## B-001: .app への daemon の同梱

GIVEN `Releash.app` だけを配置した macOS 環境であり、`.app` の外に Releash の実行ファイルを置いていない
WHEN `Releash.app` を起動する
THEN UI は `.app` に含まれる daemon を起動する
AND UI は利用可能になる

## B-002: 起動完了までの待機

GIVEN daemon が動作していない
WHEN `Releash.app` を起動する
THEN UI は daemon を子プロセスとして起動する
AND backend が Ready に達し、認証済みのクライアント ws 接続が成立するまで、UI は通常の操作を受け付けない
AND 成立した後に UI は利用可能になる

## B-003: ウィンドウを閉じる操作

GIVEN UI と daemon が動作し、workflow が実行中である
WHEN ウィンドウを閉じる操作を行う
THEN メニューバーのアイコンは残る
AND UI プロセス、daemon、実行中の workflow は継続する
AND 再びウィンドウを開ける

## B-004: daemon の異常終了と再起動

GIVEN UI と daemon が動作している
WHEN daemon のプロセスが異常終了する
THEN UI は待機間隔と試行上限の範囲で daemon を再び起動する
AND 再接続した後は現在の状態が画面へ反映される

## B-005: UI の終了検知による daemon の終了

GIVEN UI と daemon が動作している
WHEN UI プロセスが終了する
THEN daemon はそれを検知して自ら終了する
AND 一括停止（workflow runtime / terminal surface / provider exit observer / local API）は行われない
AND daemon が起動した terminal と provider のプロセスも終了する
AND UI のない daemon とその子プロセスは残らない

## B-006: 利用者の終了操作による UI と daemon の終了

GIVEN UI と daemon が動作している
WHEN 利用者が終了操作（アプリケーションメニューの Quit、Cmd+Q、Dock アイコンの終了、メニューバーのアイコンの Quit、AppleScript の quit）を行う
THEN daemon の一括停止（workflow runtime / terminal surface / provider exit observer / local API）のうち利用者の判断を必要としない段階が行われる
AND その完了の後に UI と daemon の両方が終了する
AND 一括停止が有限の期限内に完了しない場合も、UI と daemon は終了する

## B-007: Quit 後の再起動

GIVEN Quit によって UI と daemon が終了している
WHEN `Releash.app` を起動する
THEN UI と daemon の両方が起動する
AND UI は backend が Ready に達し、認証済みのクライアント ws 接続が成立するまで待ってから利用可能になる

## B-008: ログイン時に起動が有効なログイン

GIVEN ログイン時に起動の設定が有効である
WHEN ログインする
THEN メニューバーのアイコンと daemon が起動する

## B-009: ログイン時に起動が無効なログイン

GIVEN ログイン時に起動の設定が無効である
WHEN ログインする
THEN UI も daemon も起動しない

## B-010: 最小化状態で起動

GIVEN ログイン時に起動と最小化状態で起動の設定が有効である
WHEN ログインする
THEN ウィンドウは表示されない
AND メニューバーのアイコンが出る

## B-011: LaunchAgent の単一登録

GIVEN ログイン時に起動の設定が有効である
WHEN LaunchAgent の登録を確認する
THEN Releash の LaunchAgent の登録は 1 つだけである

## B-012: `.app` を削除した後のログイン

GIVEN ログイン時に起動の設定を有効にしていた
WHEN `Releash.app` を削除した後にログインする
THEN Releash の起動は試みられない

## B-013: UI のクラッシュ

GIVEN UI と daemon が動作している
WHEN UI プロセスがクラッシュする
THEN daemon も終了する
AND `Releash.app` を起動するまで、UI も daemon も起動し直されない

## B-014: daemon の単一インスタンス

GIVEN ある data_dir を使う daemon が動作している
WHEN 同じ data_dir を使う daemon をもう一つ起動しようとする
THEN その data_dir に対して動作する daemon は 1 つのままである

## B-015: daemon 未起動時の CLI と provider hook

GIVEN daemon が動作していない
WHEN `releash workflow` / `releash review` / `releash hook` を実行する
THEN 変更前と同じ結果が返る

## B-016: daemon の単独起動手段の非公開

Releash の CLI のヘルプと利用者向けガイド（`docs/guide/`）には、daemon を単独起動する手段が現れない。

## B-017: spawn の失敗

GIVEN daemon の実行ファイルを起動できない
WHEN `Releash.app` を起動する
THEN UI は失敗した段階が spawn であることと、その理由を表示する
AND 自動再起動は行われない

## B-018: backend の初期化の失敗

GIVEN daemon の backend を初期化できない
WHEN `Releash.app` を起動する
THEN daemon は起動完了に達せず異常終了する
AND UI はクライアント ws が確立しない状態でも、失敗した段階が backend の初期化であることと、その理由を表示する
AND UI は待機間隔と試行上限の範囲で daemon を再び起動する

## B-019: 起動期限の超過

GIVEN daemon が起動完了に達しない
WHEN 起動待ちの期限を超える
THEN UI は起動した子プロセスを停止し、その終了を確認してから次の起動を行う
AND UI は失敗した段階が起動期限超過であることと、その理由を表示する
AND UI は無限に待機せず、daemon が重複して起動することもない

## B-020: 連続クラッシュと試行上限

GIVEN daemon が Ready に達した直後にクラッシュすることを繰り返す
WHEN UI が自動再起動を行う
THEN 試行回数は試行上限を超えず、UI は無限に再起動しない
AND UI は失敗した段階が起動後の異常終了であることと、その理由を表示する
AND 上限に達した後、UI は自動再起動を止め、理由と終了操作を表示する

## B-021: 自動再起動の対象外の失敗と利用者の再試行

GIVEN daemon の起動が、自動再起動の対象外の失敗または試行上限への到達によって停止している
WHEN 利用者が画面を確認する
THEN 理由と終了操作が表示される
AND 再試行できる場合は、利用者が明示的に再試行できる

## B-022: 再起動の待機中の Quit

GIVEN daemon の自動再起動の待機中である
WHEN Quit を選ぶ
THEN 再起動の予約が解除される
AND daemon の終了を契機に UI が daemon を再び起動することはない

## B-023: 起動対象でない daemon インスタンスへの接続

GIVEN 古い接続情報が残っており、UI が起動した daemon インスタンスとは別のインスタンスを指している
WHEN UI が接続先を確認する
THEN UI は通常の操作を受け付けない
AND 失敗した段階と理由を表示する
AND 利用者は原因を確認して終了できる

## B-024: リリースの不一致

GIVEN 接続先の daemon のリリースが UI のリリースと一致しない
WHEN UI が接続先を確認する
THEN UI は通常の操作を受け付けない
AND 失敗した段階と理由を表示する
AND 利用者は原因を確認して終了できる

## B-025: 実行中 workflow を含む正常な更新

GIVEN UI と daemon が動作し、workflow が実行中である
WHEN 更新を適用する
THEN 必要な利用者の判断を経て daemon の一括停止が完了する
AND 旧 daemon の終了を確認してから新 daemon が起動する
AND 切替の間、UI は通常の操作を受け付けない
AND 新 daemon が Ready に達し、接続先とリリースの一致と認証済みのクライアント ws 接続が確認され、必要な状態が再取得された後に受付が再開される

## B-026: 意図的な停止での再起動の抑止

GIVEN 更新または再起動のために daemon を停止する
WHEN 旧 daemon が終了する
THEN その終了を契機とする自動の再起動は行われない
AND 旧 daemon と新 daemon が同じデータ領域を同時に使用することはない

## B-027: 停止結果が不明な場合の切替

GIVEN 更新のための daemon の停止の結果が不明である
WHEN UI が切替を進めようとする
THEN 停止完了とみなして切替を進めない

## B-028: 更新の適用の失敗

GIVEN 更新の適用に失敗する
WHEN 利用者が画面を確認する
THEN 失敗した段階と理由が表示される
AND 利用者は原因を確認して終了できる

## B-029: 新 daemon の起動の失敗

GIVEN 更新の後、新 daemon の起動が失敗する
WHEN 利用者が画面を確認する
THEN 失敗した段階と理由が表示される
AND 利用者は原因を確認して終了できる
AND 起動待ちの期限、自動再起動、再試行は daemon の起動監督と同じ規則に従う

## B-030: 更新を伴わない再起動

GIVEN UI と daemon が動作している
WHEN 更新を伴わない再起動を行う
THEN 旧 daemon の一括停止の完了と終了が確認される
AND 新しい daemon と UI が起動する
AND 接続先とリリースの一致、認証済みのクライアント ws 接続、必要な状態の再取得の後に利用可能になる

## B-031: 切替をまたぐ結果不明の変更要求

GIVEN 結果を確定できない変更要求がある
WHEN daemon の再起動、更新に伴う切替、または更新を伴わない再起動が行われる
THEN その変更要求は結果不明として表示される
AND 処理失敗としては表示されない
AND 安全を確認できない再実行は、自動でも利用者の再試行でも行われない
AND 利用者が確認するまで、結果不明の記録は自動で破棄されない

## B-032: Releash が動作中の再起動操作

GIVEN Releash の UI と daemon が動作している
WHEN `Releash.app` をもう一度起動する
THEN 既に動作している UI がアクティブになる
AND 2 つ目の UI プロセスは終了する
AND 2 つ目の daemon は起動されない

## B-033: OS 起因の終了

GIVEN UI と daemon が動作している
WHEN macOS のログアウト、再起動、またはシステム終了が行われる
THEN B-006 と同じ停止が行われ、UI と daemon の両方が終了する
AND ログアウト、再起動、システム終了は妨げられない

## B-034: ログイン時に起動が承認を必要とする状態の表示

GIVEN ログイン時に起動の登録が macOS の承認を必要とする状態である
WHEN 利用者が設定を確認する
THEN ログイン時に起動の設定は無効として示される
AND 承認が必要であることと、承認を行う場所への導線が示される

## B-035: 承認を必要とする状態での UI の起動

GIVEN ログイン時に起動の登録が macOS の承認を必要とする状態である
WHEN `Releash.app` を起動する
THEN UI の起動は失敗しない

## B-036: 起動完了を待つ間のウィンドウの表示

GIVEN daemon が動作していない
WHEN `Releash.app` を起動する
THEN daemon の起動完了を待つ間もウィンドウが表示される
AND 起動中であることが示される

## B-037: 最小化状態で起動のときの起動完了を待つ間の表示

GIVEN ログイン時に起動と最小化状態で起動の設定が有効である
WHEN ログインする
THEN daemon の起動完了を待つ間もウィンドウは表示されない

## B-038: ログイン項目の登録が失われた場合の再登録

GIVEN ログイン時に起動の設定を有効にしていた
AND OS のログイン項目の登録が失われている
WHEN `Releash.app` を起動する
THEN 現在の場所でログイン項目が登録し直される
AND ログイン時に起動の設定は有効のままである

## B-039: 読み取り専用の一時的な場所での登録の拒否

GIVEN `Releash.app` が読み取り専用の一時的な場所で実行されている
WHEN ログイン時に起動を有効にしようとする
THEN 登録は行われない
AND 登録できない理由が示される

## B-040: 起動時に CLI を設置しないこと

GIVEN `/usr/local/bin/releash` が設置されていない
WHEN `Releash.app` を起動する
THEN `/usr/local/bin/releash` は設置されない
AND 管理者の認証は求められない

## B-041: 利用者の明示操作による CLI の設置

GIVEN `/usr/local/bin/releash` が設置されていない
WHEN 利用者が設定で CLI の設置を操作する
THEN `/usr/local/bin/releash` が設置される
AND 管理者の認証が必要な場合は、この操作の中で求められる

## B-042: メニューバーのアイコンの表示

GIVEN UI が動作している
WHEN メニューバーの外観が変わる（ライトとダーク、メニューバーの色付け、アイコンの選択状態）
THEN メニューバーのアイコンはその外観に追従して表示される

## B-043: 結果不明の変更要求の終端

GIVEN 再起動または切替をまたいで結果不明のまま残った変更要求がある
WHEN 利用者がその結果不明を確認する
THEN その結果不明の記録は破棄される
AND 以後、同じ対象への変更要求は通常どおり受け付けられる

## 要件IDとBehavior IDの対応表
| Requirement ID | Behavior ID |
| --- | --- |
| R-001 | B-001 |
| R-002 | B-001, B-002 |
| R-003 | B-003 |
| R-004 | B-004 |
| R-005 | B-005 |
| R-006 | B-006 |
| R-007 | B-007 |
| R-008 | B-008, B-009 |
| R-009 | B-010 |
| R-010 | B-011 |
| R-011 | B-012 |
| R-012 | B-013 |
| R-013 | B-014 |
| R-014 | B-015, B-016 |
| R-015 | B-017, B-018, B-019, B-020 |
| R-016 | B-019 |
| R-017 | B-020 |
| R-018 | B-017, B-018, B-020, B-021 |
| R-019 | B-022 |
| R-020 | B-023, B-024 |
| R-021 | B-025 |
| R-022 | B-026 |
| R-023 | B-025 |
| R-024 | B-027 |
| R-025 | B-023, B-024, B-028, B-029 |
| R-026 | B-030 |
| R-027 | B-031, B-043 |
| R-028 | B-032 |
| R-029 | B-033 |
| R-030 | B-034, B-035 |
| R-031 | B-036, B-037 |
| R-032 | B-038 |
| R-033 | B-039 |
| R-034 | B-040, B-041 |
| R-035 | B-042 |
