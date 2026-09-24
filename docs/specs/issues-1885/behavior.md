## B-001: Workspaces の表示の状態が購読で届く

GIVEN client が Workspaces の表示に使う対象を購読する
WHEN 購読を開始する
THEN その対象の現在の状態が届く
AND 以後、その対象が変わるたびに、変わった後の状態が届く

## B-002: 購読へ移した読み取りの単発の呼び出しの不在

daemon は、購読へ移した Workspaces の表示の状態を返す単発の呼び出しを提供しない。

## B-003: 購読の単位は読み取り結果 1 つ

GIVEN Workspaces の一つの表示が、複数の状態を組み合わせた読み取り結果を使う
WHEN client がその対象を購読する
THEN 組み合わせ済みの読み取り結果が 1 つ届く
AND client は届いた結果をそのまま表示に使える

## B-004: 状態の変化が取り直しなしで表示に届く

GIVEN Workspaces が購読で得た状態を表示している
WHEN daemon が持つその状態が変わる
THEN 変わった後の状態が購読で届く
AND client は、その状態を取り直す呼び出しを行わない

## B-005: client の取り直しと再試行の不在

client は、Workspaces の表示のために、定期的な取り直し、変更通知を受けての取り直し、取得の失敗を受けての再試行を行わない。

## B-006: 変更通知の不在

daemon は、`branch-list-sync`、`workflow-execution-changed`、`agent-session-changed`、`workspace-list-changed` の変更通知を送らない。

## B-007: 購読がある間の監視

GIVEN 監視を必要とする対象を購読している client がある
WHEN その対象の監視対象のファイルまたは git のディレクトリが変わる
THEN 変わった後の対象が購読で届く

## B-008: 購読が無くなったときの監視

GIVEN 監視を必要とする対象を購読している client がある
WHEN その対象の購読が全て終わる
THEN その対象のためのファイルと git のディレクトリの監視は終わる

## B-009: client からの監視の要求の不在

client は、Workspaces の表示のために、ファイルと git のディレクトリの監視の開始と停止を要求しない。

## B-010: PR の状態の配信

GIVEN client が PR の状態を含む対象を購読している
WHEN PR の状態が変わる
THEN 変わった後の状態が購読で届く
AND client は PR の状態を取りに行く呼び出しを行わない

## B-011: issue の配信

GIVEN client が issue の一覧を含む対象を購読している
WHEN issue の一覧が変わる
THEN 変わった後の一覧が購読で届く
AND client は issue を取りに行く呼び出しを行わない

## B-012: 取得に失敗したときの一覧の保持

GIVEN Workspaces が Repository と worktree の一覧を表示している
WHEN daemon による一覧の取得が失敗する
THEN Workspaces は最後に得られた一覧を表示し続ける

## B-013: 失敗した範囲の表示

GIVEN 複数の Repository のうち一部の取得が失敗する
WHEN Workspaces が一覧を表示する
THEN 成功した範囲は更新後の内容を表示する
AND 失敗した Repository と worktree には、取得に失敗したことと前回の情報を表示していることが示される

## B-014: 初回の取得の失敗と項目なしの区別

GIVEN ある Repository の一覧をまだ一度も取得できていない
WHEN その Repository の取得が失敗する
THEN Workspaces はその Repository を「項目なし」として表示しない
AND 取得に失敗したことが示される

## B-015: branch の一覧が購読で届く

GIVEN client が Workspaces または Settings の base branch 選択肢で使う branch の一覧を購読する
WHEN 購読を開始する
THEN 現在の branch の一覧が届く
AND 以後、branch の一覧が変わるたびに、変わった後の一覧が届く

## B-016: 使われなくなるコードの不在

この変更で使われなくなったコードは残らない。

## B-017: 外部の情報の鮮度と取り直しの要求

GIVEN client が PR の状態または issue を含む対象を購読している
WHEN 最後に取りに行ってから 30 秒が過ぎる
THEN daemon はその情報を取りに行く
AND 利用者が取り直しを要求したときは、30 秒を待たずに取りに行く

## B-018: Workspaces の一覧の再走査の要求

GIVEN Workspaces が一覧を表示している
WHEN 利用者が一覧の再走査を要求する
THEN daemon は Repository の走査をやり直す
AND やり直した結果が購読で届く
AND 一覧の取得に失敗している状態でも再走査を要求できる

## B-019: agent session の履歴の表示件数

GIVEN client が agent session の履歴を購読している
WHEN 利用者が表示件数を増やす
THEN 増やした後の件数の履歴が届く

## B-020: Session の Node の引き当てが購読で届く

GIVEN client が worktree と session を指定して対象を購読する
WHEN 購読を開始する
THEN その session に対応する Node の id が届く

## B-021: 任意のパスから解決した repository のルートが購読で届く

GIVEN client が任意のパスを指定して対象を購読する
WHEN 購読を開始する
THEN そのパスから解決した repository のルートが届く

## B-022: 起動ディレクトリから解決した repository のルート

GIVEN client が起動時に表示する repository を決める
WHEN その対象を購読する
THEN daemon の起動ディレクトリから解決した repository のルートが届く
AND daemon の作業ディレクトリそのものを返す呼び出しは無い

## B-023: 更新を要求する呼び出しの戻り値

GIVEN client が値の更新を要求する呼び出しを行う
WHEN その呼び出しが成功する
THEN 呼び出しは更新後の状態を返さない
AND 更新後の状態は購読で届く

## 要件IDとBehavior IDの対応表
| Requirement ID | Behavior ID |
| --- | --- |
| R-001 | B-001 |
| R-002 | B-002 |
| R-003 | B-003 |
| R-004 | B-004 |
| R-005 | B-005 |
| R-006 | B-006 |
| R-007 | B-007, B-008, B-009 |
| R-008 | B-010, B-011, B-017 |
| R-009 | B-012, B-013, B-014 |
| R-010 | B-018 |
| R-011 | B-019 |
| R-012 | B-015 |
| R-013 | B-020, B-021 |
| R-014 | B-022 |
| R-015 | B-016 |
| R-016 | B-023 |
