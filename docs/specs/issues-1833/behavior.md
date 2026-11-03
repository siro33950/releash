## B-001: メニューバーの項目

GIVEN Swift.app が起動している
WHEN 開発者がメニューバーを開く
THEN Tauri 版の項目のうち New Terminal、Back to Kanban、View の Theme と Font Size を除いたものがある

## B-002: Open Folder…

GIVEN Swift.app がサーバに接続している
WHEN 開発者が File の Open Folder… を選び、git repository のフォルダを選んで確定する
THEN その repository がサイドバーに現れる

## B-003: Stage All

GIVEN worktree を選択していて、その作業ツリーに変更がある
WHEN 開発者が Git の Stage All を選ぶ
THEN その worktree の変更がすべて index に入っている

リンク先が存在しない symlink も対象に含む。
skip-worktree が設定されたパスは全件ステージの対象から除外する。
サーバは同じ worktree のステージ・アンステージ処理による index 更新を排他で実行する。

## B-004: Unstage All

GIVEN worktree を選択していて、その index に変更が入っている
WHEN 開発者が Git の Unstage All を選ぶ
THEN その worktree の index から変更がすべて外れている

通常ファイルから symlink への型変更も対象に含む。
変更のない index エントリの skip-worktree を保持する。

## B-005: Create Branch…

GIVEN worktree を選択している
WHEN 開発者が Git の Create Branch… を選び、branch 名を入力して確定する
THEN その worktree にその名前の branch が作られ、worktree がその branch に切り替わっている

作成先はシートを開いた時点の worktree とする。
別ウィンドウで選択を変えても作成先は変わらない。
接続状態が変わってもシートと入力を保持し、未接続中は作成を無効にする。

## B-006: terminal へのドロップ

GIVEN terminal が入力を待っている
WHEN 開発者が Finder からファイルをその terminal の上に落とす
THEN その terminal の入力にファイルの path が現れる

attachment が未確立の terminal と終了済みの terminal は、ドラッグ中からドロップを拒否する。
受理後、送信前に attachment が変わった場合や terminal が終了した場合は、送信せずに対象のパスと理由を表示する。
送信後に RPC が失敗した場合は、入力結果を確認できないことと対象のパスを表示する。
入力済みの可能性があるため自動再送せず、端末の内容を確認するよう利用者に伝える。
この通知は再接続後も保持し、利用者が閉じる。

## 要件IDとBehavior IDの対応表
| Requirement ID | Behavior ID |
| --- | --- |
| R-001 | B-001, B-002, B-003, B-004, B-005 |
| R-002 | B-006 |
