## B-001: 再起動後のworktree出自判定

GIVEN 事実ログからworktreeの出自を隔離実行環境として導出できる
WHEN アプリケーションが再起動する
THEN そのworktreeは再起動前と同じく隔離実行環境として扱われる
AND worktreeの存在、パス、またはbranch名だけを根拠に作業の場へ再分類されない

## B-002: 隔離実行環境の実体喪失

GIVEN 台帳が再開対象のNodeに属する隔離実行環境を記録しており、そのworktreeの実体が存在しない
WHEN 起動時の突合が行われる
THEN そのNodeで隔離実行環境の喪失を観測できる
AND そのNodeの再開要求は理由付きで拒否される
AND 別のworktreeでそのNodeが再開されない

## B-003: 所有者が終了した隔離実行環境

GIVEN 台帳が隔離実行環境を記録しており、そのworktreeの実体が存在し、所有するNodeの実行が終了している
WHEN 起動時の突合が行われる
THEN そのworktreeは掃除候補として人間へ提示される
AND そのworktreeは自動的に削除されない

## B-004: 台帳と隔離用命名規則に該当しないworktree

GIVEN 台帳に記録がなく、隔離実行環境の専用パスおよびbranch命名規則に一致しないworktreeが存在する
WHEN 人間がWorktree管理UIのworktree一覧またはbranchカード一覧を表示する
THEN そのworktreeは作業の場として通常一覧に表示される

## B-005: 隔離用命名規則だけに該当するworktree

GIVEN 台帳に記録がなく、隔離実行環境の専用パスおよびbranch命名規則に一致するworktreeが存在する
WHEN 人間がWorktree管理UIのworktree一覧またはbranchカード一覧を表示する
THEN そのworktreeは「台帳外・掃除候補」として提示される
AND 作業の場の通常一覧には表示されない

## B-006: 所有者が未終了の隔離実行環境

GIVEN 台帳が隔離実行環境を解放されていないものとして記録しており、そのworktreeを所有するNodeの実行が終了していない
WHEN 人間がWorktree管理UIのworktree一覧またはbranchカード一覧を表示する
THEN そのworktreeは作業の場の通常一覧に表示されない

## B-007: 人間の明示操作がないworktree取得

GIVEN 人間がworktreeの作成または取得を明示的に要求していない
WHEN Releashの処理が行われる
THEN Releashは新しいworktreeを作成または取得しない

## B-008: 起動時の突合によるworktree保護

GIVEN worktreeとbranchが存在する
WHEN 起動時の突合が行われる
THEN worktreeの実体とbranchに対して削除、prune、移動、またはその他の変更が行われない
AND 成果が統合されていないworktreeも削除されない

## B-009: 同一worktreeへの2つ目のworkflow実行木

GIVEN あるworktreeにactiveなworkflow実行木が登録されており、その実行木が実行中Nodeを持つかどうかを問わない
WHEN 同じworktreeで別のworkflow実行木の開始が要求される
THEN 別のworkflow実行木の開始は拒否される

## B-010: workflow実行木と同じworktreeでの単独Session

GIVEN あるworktreeにactiveなworkflow実行木が登録されている
WHEN 同じworktreeで単独Sessionの実行木の開始が要求される
THEN その開始は同一worktreeの使用を理由として拒否されない

## B-011: 同一実行木内でworktreeを共有するNode

GIVEN 同一実行木内の複数Nodeがshared worktreeを共有する
WHEN それらのNodeが並走する
THEN その並走は同一worktreeの使用を理由として拒否されない

## B-012: worktreeフィールドを宣言したworkflow定義

GIVEN workflow定義のNodeがworktreeフィールドを宣言している
WHEN そのworkflow定義が検証される
THEN そのworkflow定義はworktreeフィールドが未対応であることを理由として拒否される

## B-013: 解放済みの隔離実行環境

GIVEN 台帳が隔離実行環境を解放済みとして記録しており、そのworktreeの実体が存在する
WHEN 人間がWorktree管理UIのworktree一覧またはbranchカード一覧を表示する
THEN そのworktreeは掃除候補として提示される
AND そのworktreeは自動的に削除されない

## B-014: 台帳を読めないときのworktree一覧

GIVEN 台帳の読み取りに失敗している
WHEN 人間がWorktree管理UIのworktree一覧またはbranchカード一覧を表示する
THEN 隔離実行環境の専用パスおよびbranch命名規則に一致するworktreeは「台帳外・掃除候補」として提示される
AND 一致しないworktreeは作業の場として通常一覧に表示される

## B-015: 台帳を読めないときの起動時突合

GIVEN 台帳の読み取りに失敗している
WHEN 起動時の突合が行われる
THEN 隔離環境の喪失はいずれのNodeについても確定されない
AND ディスク上のworktreeの形だけを根拠にNodeが再開されない

## 要件IDとBehavior IDの対応表

| Requirement ID | Behavior ID |
| --- | --- |
| R-001 | B-001 |
| R-002 | B-002 |
| R-003 | B-003, B-013 |
| R-004 | B-004 |
| R-005 | B-005 |
| R-006 | B-006 |
| R-007 | B-007 |
| R-008 | B-008 |
| R-009 | B-009, B-010, B-011 |
| R-010 | B-012 |
| R-011 | B-014, B-015 |
