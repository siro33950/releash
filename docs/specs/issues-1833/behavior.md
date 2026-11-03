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

## B-004: Unstage All

GIVEN worktree を選択していて、その index に変更が入っている
WHEN 開発者が Git の Unstage All を選ぶ
THEN その worktree の index から変更がすべて外れている

## B-005: Create Branch…

GIVEN worktree を選択している
WHEN 開発者が Git の Create Branch… を選び、branch 名を入力して確定する
THEN その worktree にその名前の branch が作られ、worktree がその branch に切り替わっている

## B-006: terminal へのドロップ

GIVEN terminal が入力を待っている
WHEN 開発者が Finder からファイルをその terminal の上に落とす
THEN その terminal の入力にファイルの path が現れる

## 要件IDとBehavior IDの対応表
| Requirement ID | Behavior ID |
| --- | --- |
| R-001 | B-001, B-002, B-003, B-004, B-005 |
| R-002 | B-006 |
