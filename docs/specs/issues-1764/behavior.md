## B-001: メニューバーのアイコンの常時表示

GIVEN Swift.app が動いている
WHEN 開発者がウィンドウをすべて閉じる
THEN メニューバーのアイコンは表示されたままである

## B-002: 集約状態の表示

GIVEN Swift.app がサーバに接続し、ある worktree に人の番（attention）の実行木がある
WHEN 開発者がメニューバーのアイコンを見る
THEN アイコンは attention の色を示す
AND すべての実行木が idle になると、アイコンは idle の色を示す
AND バックグラウンド失敗があるノードの詳細には、ツリー一覧と同じ状態色と失敗理由を表示する

## B-003: 未接続時の表示

GIVEN Swift.app がサーバに接続していない
WHEN 開発者がメニューバーのアイコンを見る
THEN アイコンは状態を示さない

## B-004: メニューバーの明暗と選択状態への追従

GIVEN Swift.app が動いている
WHEN 開発者が macOS の外観を明るい表示と暗い表示で切り替え、メニューを開く
THEN アイコンはどちらの外観でも見え、メニューを開いている間は選択状態の見た目になる

## B-005: 実行中の実行木の一覧

GIVEN Session の実行木と workflow の実行木がそれぞれ別の worktree で動いている
WHEN 開発者がメニューバーのアイコンのメニューを開く
THEN 各実行木が worktree 名、Session なら provider の表示名、workflow なら workflow 名、状態とともに一覧される
AND 一覧の項目を選ぶと、ウィンドウが前面に出てその worktree が選ばれる
AND 一部の実行木を読み取れない場合も正常な木を表示し、読めなかった木の ID と失敗理由を明示して集約状態を attention にする
AND 個別の失敗が残る間は定期的に再取得し、回復後は失敗表示を消す
BUT DB アクセスや同期全体の失敗は購読全体の失敗として返す

## B-006: メニューの項目

GIVEN メニューバーのアイコンのメニューが開いている
WHEN 開発者が「開く」を選ぶ
THEN ウィンドウが前面に出る。ウィンドウが無ければ開く
AND 「設定」を選ぶと Settings のウィンドウが開く
AND 「About」を選ぶと Swift.app の名前と版が表示される
AND 「Quit」を選ぶと Swift.app のプロセスが終了する

## B-007: ログイン項目の登録

GIVEN Swift.app が登録を許される場所で実行されている
WHEN 開発者が Settings で「ログイン時に起動」を有効にする
THEN Swift.app がログイン項目として登録され、登録は 1 つだけである
AND 次のログインで Swift.app が起動する

## B-008: ログイン項目の解除

GIVEN Swift.app がログイン項目として登録されている
WHEN 開発者が Settings で「ログイン時に起動」を無効にする
THEN 登録が解除され、次のログインで Swift.app は起動しない

## B-009: 登録の復元

GIVEN 「ログイン時に起動」が有効で、ログイン項目の登録が失われている
WHEN 開発者が Swift.app を起動する
THEN Swift.app がログイン項目として登録し直される

## B-010: 登録できない場所

GIVEN Swift.app が AppTranslocation または読み取り専用の場所で実行されている
WHEN 開発者が Settings で「ログイン時に起動」を有効にする
THEN Swift.app はログイン項目を登録しない
AND Settings にサーバが返す理由が表示される

## B-011: 承認待ちの表示

GIVEN Swift.app がログイン項目を登録し、macOS が承認待ちにしている
WHEN 開発者が Settings のログイン項目の節を見る
THEN 承認待ちである旨と、システム設定のログイン項目を開く導線が表示される

## B-012: .app の削除

GIVEN Swift.app がログイン項目として登録されている
WHEN 開発者が .app を削除してログインし直す
THEN macOS は Swift.app の起動を試みない

## B-013: 最小化状態での起動

GIVEN 「ログイン時に起動」と「最小化状態で起動」がともに有効である
WHEN macOS がログイン時に Swift.app を起動する
THEN ウィンドウは開かず、メニューバーのアイコンだけが表示される
BUT サーバに接続できない場合は、ウィンドウを開き、エラーと再試行操作を表示する

## B-014: 開発者による起動

GIVEN 「ログイン時に起動」と「最小化状態で起動」がともに有効である
WHEN 開発者が Swift.app を起動する
THEN ウィンドウが開く

## B-015: ウィンドウを閉じる

GIVEN Swift.app がサーバに接続し、workflow が動いている
WHEN 開発者がウィンドウを閉じる
THEN Swift.app のプロセス、サーバ、workflow は続く

## B-016: 終了

GIVEN Swift.app がサーバに接続し、workflow が動いている
WHEN 開発者がアプリメニューの Quit、⌘Q、Dock アイコンの終了、メニューバーの Quit、AppleScript の quit のいずれかで終了する
THEN ログイン項目の登録・解除中なら完了を最大 5 秒待ち、失敗または期限超過でも終了する
AND Swift.app のプロセスだけが終了し、サーバと workflow は続く

## B-017: 終了後の再起動

GIVEN Swift.app を終了した後もサーバが動いている
WHEN 開発者が Swift.app を起動する
THEN Swift.app は動いているサーバに接続し、サーバは起動し直されない

## B-018: 二重起動

GIVEN Swift.app が動いている
WHEN 開発者が Swift.app をもう一度起動する
THEN 既存の画面が前面に出る
AND 2 つ目の画面と 2 つ目のサーバは起動しない

## B-019: CLI の設置の契機

GIVEN Swift.app が動いていて、`/usr/local/bin/releash` が無い
WHEN 開発者が Swift.app を起動し直し、設定を変更する
THEN `/usr/local/bin/releash` は作られない
AND Settings の「CLIを設置」を押したときだけ作られる

## B-020: 回帰の E2E

GIVEN 回帰の E2E が、隔離した環境でサーバを起動し workflow を開始している
WHEN E2E が Swift.app を終了し、Swift.app を起動し直す
THEN 終了の後もサーバと workflow は続き、起動し直した Swift.app は同じサーバに接続する

## B-021: 購読の形

GIVEN サーバが全 worktree の集約状態を配信している
WHEN どの client がその購読を開始しても
THEN 同じ形で集約状態を受け取る
AND 実行概要の読み取り失敗中は定期的に再取得し、取得した状態の配信に成功すると変更通知待ちに戻る

## 要件IDとBehavior IDの対応表

| Requirement ID | Behavior ID |
| --- | --- |
| R-001 | B-001 |
| R-002 | B-002、B-003 |
| R-003 | B-004 |
| R-004 | B-005 |
| R-005 | B-006 |
| R-006 | B-007、B-008、B-009 |
| R-007 | B-010 |
| R-008 | B-011 |
| R-009 | B-012 |
| R-010 | B-013、B-014 |
| R-011 | B-015 |
| R-012 | B-016 |
| R-013 | B-017 |
| R-014 | B-018 |
| R-015 | B-019 |
| R-016 | B-020 |
| R-017 | B-021 |
