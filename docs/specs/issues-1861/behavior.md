## B-001: 手動更新中の一覧保持

GIVEN Repository・Worktree・Session・Workflow の一覧を取得済みの Workspaces が表示されている
WHEN Workspaces 行の更新の操作を実行する
THEN 更新が完了するまでの間、直前に取得できた Repository・Worktree・Session・Workflow の一覧が表示され続ける

## B-002: 自動更新中の一覧保持

GIVEN Repository・Worktree・Session・Workflow の一覧を取得済みの Workspaces が表示されている
WHEN 自動更新が開始される
THEN 更新が完了するまでの間、直前に取得できた Repository・Worktree・Session・Workflow の一覧が表示され続ける

## B-003: 更新をまたぐ表示状態の維持

GIVEN Repository と Worktree を展開し、Worktree 配下の Session を選択して表示している
WHEN 自動更新または手動更新が実行され、展開中の Repository・Worktree と表示中の Session が更新後も存在する
THEN それらの展開状態と選択が維持される
AND Workspaces 一覧のスクロール位置が維持される
AND 表示中の Session が切り替わらない

## B-004: Workspaces 行の更新の操作

GIVEN Workspaces が表示されている
WHEN Workspaces 行を見る
THEN 更新の操作が一つある
AND どの Repository 行にも更新の操作がない

## B-005: 取得失敗中の更新の操作

GIVEN 一覧の取得に失敗し、更新失敗が示されている
WHEN Workspaces 行の更新の操作を見る
THEN 更新の操作を実行できる

## B-006: 折りたたまれた Repository を含む全体の更新

GIVEN 折りたたまれた Repository と、展開された Repository が登録されている
WHEN Workspaces 行の更新の操作を実行し、更新が完了する
THEN 登録 Repository 一覧、各 Repository の Worktree 一覧、各 Worktree 配下の Session・Workflow の一覧が更新される
AND 折りたたまれていた Repository を展開すると、更新後の Worktree 一覧と、その配下の Session・Workflow の一覧が表示される

## B-021: 自動更新の対象範囲

GIVEN 折りたたまれた Repository と、展開された Repository が登録されている
WHEN 自動更新が実行され、更新が完了する
THEN 登録 Repository 一覧、各 Repository の Worktree 一覧、各 Worktree 配下の Session・Workflow の一覧が更新される
AND 折りたたまれていた Repository を展開すると、更新後の Worktree 一覧と、その配下の Session・Workflow の一覧が表示される

## B-007: 更新による走査の再実行

GIVEN Repository の走査結果が保存されている
WHEN 手動更新または自動更新が実行される
THEN 保存済みの結果をそのまま反映するのではなく、Repository の走査が再実行され、その結果が一覧へ反映される

## B-008: 自動更新の失敗からの復旧

GIVEN 自動更新が失敗し、対象に更新失敗が示されている
WHEN アプリを再起動せずに Workspaces 行の更新の操作を実行し、取得に成功する
THEN その対象の一覧が更新後の内容で表示される
AND その対象の更新失敗の表示が解消する

## B-009: 一部失敗時の部分更新

GIVEN 複数の Repository が登録されている
WHEN 更新を実行し、一部の Repository の取得に失敗し、残りの Repository の取得に成功する
THEN 取得に成功した Repository の一覧は更新後の内容で表示される
AND 取得に失敗した Repository には、直前に取得できた一覧が表示され続ける

## B-010: 失敗した対象の表示

GIVEN 一覧を取得済みの Repository・Worktree がある
WHEN 更新を実行し、その Repository・Worktree の取得に失敗する
THEN その Repository・Worktree に、一覧の更新に失敗したことが示される
AND 表示しているのが前回取得の情報であることが示される

## B-011: 全件失敗時の一覧保持

GIVEN 一覧を取得済みの Workspaces が表示されている
WHEN 更新を実行し、すべての対象の取得に失敗する
THEN 直前に取得できた Repository・Worktree・Session・Workflow の一覧が表示され続ける

## B-012: 初回取得の失敗と項目なしの区別

GIVEN 一度も一覧を取得できていない対象がある
WHEN その対象の取得に失敗する
THEN 初回取得に失敗したことが示される
AND 「項目なし」としては表示されない

## B-013: 初回取得中と項目なしの区別

GIVEN 一度も一覧を取得できていない対象がある
WHEN その対象の取得が進行中である
THEN 取得が進行中であることが示される
AND 「項目なし」としては表示されない

## B-014: 正常に取得した空の結果の反映

GIVEN 対象の一覧を取得済みである
WHEN 更新を実行し、その対象の取得に成功して結果が空である
THEN その対象は「項目なし」として表示される

## B-015: 再取得成功による失敗表示の解消

GIVEN 更新失敗が示されている対象がある
WHEN その対象の再取得に成功する
THEN その対象の更新失敗の表示が解消する

## B-016: 手動更新の進行表示と重複操作の抑止

GIVEN Workspaces 行の更新の操作が実行され、更新が進行中である
WHEN 同じ更新の操作をもう一度実行しようとする
THEN 更新が進行中であることが示され、重複した更新の要求は受け付けられない
AND 更新が成功または失敗で終わった後は、再び更新の操作を実行できる

## B-017: 応答順序が入れ替わった場合の反映

GIVEN 自動更新と手動更新が同じ対象に対して重なって実行されている
WHEN より古い取得の応答が、より新しい取得の応答よりも後に返る
THEN 表示される一覧はより新しい取得結果のままであり、より古い取得結果で上書きされない

## B-018: 削除の反映

GIVEN 一覧に表示されている対象がある
WHEN 更新を実行し、取得に成功した結果からその対象が失われている
THEN その対象は一覧から取り除かれる

## B-019: 更新をまたぐ Session と Workflow の継続

GIVEN Session を表示し、Workflow が実行中である
WHEN 更新が開始され、成功または失敗で終わる
THEN 表示中の Session は中断されずに表示され続ける
AND 実行中の Workflow の実行は中断されない

## B-020: 登録 Repository 一覧の取得失敗の表示

GIVEN 登録 Repository 一覧を取得済みの Workspaces が表示されている
WHEN 更新を実行し、登録 Repository 一覧の取得に失敗する
THEN Workspaces 一覧全体に、一覧の更新に失敗したことが示される
AND 表示しているのが前回取得の情報であることが示される

## B-022: 登録 Repository 一覧の表示の一致

GIVEN Workspaces に登録 Repository 一覧が表示されている
WHEN 登録 Repository 一覧を表示する画面を開く
THEN その画面に表示される登録 Repository は、Workspaces に表示されている登録 Repository と一致する

## B-023: 更新後の登録 Repository 一覧の一致

GIVEN 登録 Repository 一覧を表示する画面と Workspaces が同じ登録 Repository を表示している
WHEN 更新が実行され、Workspaces に表示される登録 Repository 一覧が変わる
THEN 登録 Repository 一覧を表示する画面に表示される登録 Repository は、更新後に Workspaces が表示している登録 Repository と一致する

## 要件IDとBehavior IDの対応表
| Requirement ID | Behavior ID |
| --- | --- |
| R-001 | B-001, B-002 |
| R-002 | B-003 |
| R-003 | B-004 |
| R-004 | B-005 |
| R-005 | B-006, B-021 |
| R-006 | B-007 |
| R-007 | B-008 |
| R-008 | B-009, B-011 |
| R-009 | B-010, B-020 |
| R-010 | B-012, B-013, B-014 |
| R-011 | B-008, B-015 |
| R-012 | B-016 |
| R-013 | B-017 |
| R-014 | B-014, B-018 |
| R-015 | B-019 |
| R-016 | B-022, B-023 |
