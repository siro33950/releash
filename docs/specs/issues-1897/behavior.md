## B-001: 表示は購読から届く値だけで決まる

GIVEN 画面が daemon の状態（workflow の定義、AgentSession の状態、Provider の実行ファイルの設定、Notion の設定等）を表示している
WHEN 利用者がその状態を変える操作（保存、open・restore・delete、更新、削除等）を行う
THEN 画面の表示は、購読から届いた変更後の状態で変わる
AND 操作の戻り値や操作の成否から画面側で推定した値は、表示に使われない

## B-002: 状態を変える操作は識別子だけを返す

GIVEN 画面が、作ったもの・対象を選ぶ・移る必要のある操作（Session の restore・resume・create、worktree の作成）を呼び出す
WHEN 操作が成功する
THEN 戻り値は、作ったもの・対象の識別子だけを持つ
AND 選んだもの・移った先の表示の中身は、購読から届く値で決まる

## B-003: 作った Session Node を選ぶ

GIVEN 利用者が Workspaces の一覧で Session を restore・resume・create する
WHEN 操作が成功する
THEN 操作は Session Node の ID を返し、その node は購読に流れる daemon の状態に既に入っている
AND 画面はその node を選び、購読に現れた node を表示する
AND 「Session Node was not found」の失敗は出ない

## B-004: 起動時に開くタブとリポジトリの一覧

GIVEN 起動時のリポジトリの worktree が 1 つだけである
WHEN Releash が起動する
THEN 起動時のリポジトリが、daemon によってリポジトリの一覧に入り、購読で画面に届く
AND 購読で届いた worktree（path・branch・リポジトリ名）のタブが、起動時に 1 回だけ開く

## B-005: リポジトリを足すときの root

GIVEN 利用者がリポジトリを足すためにディレクトリを選ぶ
WHEN 画面がそのディレクトリの root を求める
THEN root は単発の呼び出しの結果として、root がある・無いのどちらかで返る
AND 呼び出しが失敗したときは、失敗が画面に出る

## B-006: 使い手の無い購読の対象と画面の状態

GIVEN この開発の変更が入っている
WHEN 購読の対象と画面の状態を一覧する
THEN 使い手の無い購読の対象（`session-node`・`repository-root` を含む）は無い
AND 値を増やすだけで読まれない画面の状態（`gitRefreshKey`・`refreshGit`）は無い

## B-007: プロセスが動いていない terminal

GIVEN terminal のプロセスが動いていない
WHEN 利用者がその terminal で打鍵する
THEN terminal は入力を受け付けない
AND プロセスが動いていないことが terminal に表示されている

## B-008: 起動前にためた入力を送れないとき

GIVEN 利用者が terminal の起動前に入力した
WHEN 起動の完了時にプロセスが動いていない
THEN 起動できなかったことが画面に出る

## B-009: 起動前の入力のバッファの上限

GIVEN terminal の起動前の入力が、バッファの上限を超える
WHEN 上限を超えた分が捨てられる
THEN 捨てたことが画面に出る

## B-010: 利用者の操作の失敗

GIVEN メニューの有効・無効の切り替え、パスのコピー、レビューのスレッドの削除、terminal のリンクを開く操作、worktree の作成のいずれかが失敗する
WHEN 利用者がその操作を行う
THEN 失敗が原因とともに画面に出る

## B-011: 更新の確認の失敗

GIVEN 更新の確認が失敗する
WHEN 画面が更新を確認する
THEN 更新の確認に失敗したことが、更新が無いときと区別できる形で画面に出る

## B-012: 購読の読み取りの失敗

GIVEN workspace の状態の購読の読み取りが失敗する
WHEN 画面がその購読の値を受け取る
THEN 画面は失敗を表示し、「状態が無い」ときと同じ扱いにしない

## B-013: 読まれない失敗を記録しない

GIVEN `repository_scan`・`terminal_checkpoint`・`provider_session_title_list`・`provider_session_title`・`provider_lifecycle_append`・`provider_lifecycle_resolution`・`workflow_recovery_list`・`client_request_limit` のいずれかの操作が、要対応になる種類の失敗で終わる
WHEN daemon がその失敗を扱う
THEN 失敗は失敗の記録に書かれず、ログに残る
AND 画面の表示は変更前と同じである

## B-014: 起動時の回復の失敗は要対応に出る

GIVEN 起動時の workflow の実行木の回復が、要対応になる種類の失敗で終わる
WHEN Workspaces の一覧を表示する
THEN その実行木の node に要対応が出る

## B-015: node の起動の失敗

GIVEN workflow の node の起動が失敗する
WHEN 起動の失敗が一時的でない
THEN node は失敗の状態になり、その理由を持つ
AND Workspace ツリーに、その起動の失敗の要対応は出ない

## B-016: 一時的な node の起動の失敗のやり直し

GIVEN workflow の node の起動が一時的な失敗で終わる
WHEN 起動のやり直しが成功する
THEN Workspace ツリーに、その起動の失敗の要対応は残らない

## B-017: 入口に関係なく同じステータスコード

GIVEN Connect と HTTP local API の両方から呼べる操作（workflow の output の submit 等）が、同じ失敗で終わる
WHEN それぞれの入口から呼び出す
THEN 返るステータスコードは、失敗の分類で決まる同じ種類のコードである

## B-018: HTTP local API の技術的な失敗

GIVEN HTTP local API の呼び出しが、性質を持つ技術的な失敗で終わる
WHEN CLI または hook がその呼び出しを行う
THEN HTTP のステータスコードは、その性質（一時的、時間切れ、取り消し、その他）で決まり、性質に関係ない一律の 503 や 500 ではない

## B-019: 処理の打ち切りによる失敗

GIVEN 画面の呼び出しの処理が、取り消しまたは panic で打ち切られる
WHEN 画面がその呼び出しの結果を受け取る
THEN ステータスコードは打ち切りの性質に従い、取り消しは CANCELED になる

## B-020: 監視のパスを求める失敗

GIVEN Workspaces の監視のパスを求めるときに、worktree の一覧または root の読み取りが失敗する
WHEN Workspaces の購読が更新される
THEN その repository の要素に、監視の失敗が読み取りの失敗として画面に出る

## B-021: 確かめのループの間隔

GIVEN ファイルロック、login shell の終了、前の UI の終了、起動中の UI の socket のいずれかを待つ
WHEN 状態が変わるのを確かめる
THEN 確かめる間隔は、変更前と同じ値（ファイルロックと login shell は 10 ms、前の UI と socket は 20 ms）である
AND 再試行の予算を使い切っても、確かめは止まらない

## B-022: 相手が待ち時間を指定する失敗

GIVEN Notion の API が 429 と Retry-After を返す
WHEN daemon が Notion の呼び出しをやり直す
THEN 次のやり直しまで、Retry-After の値を待つ
AND やり直しは、呼び出しの期限か再試行の予算が尽きるまで続き、回数の上限 2 回では止まらない

## B-023: SQLite の busy の待ち

GIVEN store が SQLite の busy を返す
WHEN 起動時または実行中の読み書きで、daemon が busy の解消を待つ
THEN 待ちは呼び出しの期限で打ち切られる

## B-024: provider lifecycle の event の保存の再試行

GIVEN provider の hook の signal による event の保存が、一時的な失敗で終わる
WHEN daemon が保存をやり直す
THEN やり直しの間隔は、1 件ずつの操作のやり直しの規則で決まり、client のつなぎ直しの規則と同じ値ではない

## B-025: command の完了の監視の寿命

GIVEN workflow の command が、起動した呼び出しの期限より長く走る
WHEN command が終わる
THEN command の完了が観測される
AND workflow の操作で command が止められたとき、または daemon が終了したときは、監視も止まる

## B-026: worktree の削除

GIVEN 利用者が worktree を削除する
WHEN 削除の呼び出しが返る
THEN 呼び出しが成功していれば、worktree は既に削除されている
AND 削除が失敗したときは、失敗が呼び出しの失敗として画面に出る
AND 成功・失敗のどちらでも、購読の上で削除中の表示が解ける

## B-027: Workspaces の取り直し

GIVEN 利用者が Workspaces を取り直す
WHEN 取り直しの呼び出しが返る
THEN 走査と PR の状態の取り直しは終わっている
AND 走査または PR の状態の取り直しが失敗したときは、失敗が読み取りの失敗として購読に載り、画面に出る

## B-028: 処理の先の期限

GIVEN 呼び出しの残りの期限が、処理の先（外部プロセス、HTTP、gh、login shell 等）の自前の期限より短い
WHEN 呼び出しの期限が来る
THEN 処理の先は、自前の期限を待たずに打ち切られる

## B-029: 起動時の処理の期限

GIVEN 呼び出しから始まらない起動時の処理（前の UI の終了の確認、起動中の UI の socket の確認等）
WHEN その処理が待つ
THEN 期限は変更前と同じ値である

## B-030: HTTP local API の期限と取り消し

GIVEN CLI または hook が HTTP local API を呼び出す
WHEN 呼び出しが既定の期限を超える、または呼び出し元が接続を切る
THEN daemon の処理は打ち切られる

## B-031: HTTP local API の同時実行の枠

GIVEN Connect と HTTP local API の呼び出しが同時に来る
WHEN daemon がそれらを受け付ける
THEN 両方の入口の呼び出しが、同じ 64 席の枠を分け合う
AND workflow の output の submit・validate・get と provider の hook の signal は `workflow` の段、それ以外の HTTP local API の呼び出しは `default` の段で受け付けられる

## B-032: block_on とランタイムの作成

GIVEN この開発の変更が入っている
WHEN daemon の実行時のコード（入口の main を除く）を一覧する
THEN sync から `block_on` で async の処理を動かす箇所と、呼び出しごとにランタイムを作る箇所は無い

## B-033: store の入口

GIVEN store が開いた後である
WHEN daemon が store を読む・書く
THEN 読み込みと書き込みは、それぞれ期限と取り消しが付く async の 1 本の入口を通る

## B-034: 監視の失敗の届け方

GIVEN Workspaces の購読で、ある repository または worktree の監視を張れない、または張った後の監視が壊れる
WHEN Workspaces の購読が更新される
THEN その repository または worktree の要素に、読み取りの失敗が出る
AND 他の repository と worktree の値は、今の値のまま出る

## B-035: 監視の失敗が解ける

GIVEN ある要素に監視の失敗が出ている
WHEN 次の張り直しまたは走査が成功する
THEN その要素の失敗は解け、今の値が出る

## B-036: 購読の対象全体に掛かる監視の失敗

GIVEN 購読の対象全体に掛かる監視（workflows のディレクトリ等）を張れない
WHEN その購読の対象が更新される
THEN その購読の対象に読み取りの失敗が届き、画面に出る

## B-037: 通信の原則の文書

GIVEN `AGENTS.md` の「アーキテクチャ原則」を読む
WHEN 通信の原則を確かめる
THEN 「サーバは状態を配信し、client は購読する。client からサーバへの単発の呼び出しは、状態を変える操作と、client の入力に対する計算だけ」が原則として書かれている
AND 型名、定数、呼び出しの一覧、原則から導ける個別の規則、`docs/architecture/`・`docs/glossary/DOMAIN.md` の内容の複製は書かれていない

## B-038: 状態の経路は購読だけ

GIVEN この開発の変更が入っている
WHEN daemon の状態の変化を画面へ届ける経路を一覧する
THEN 経路は購読の stream だけである

## B-039: client ごとの stream

GIVEN 画面が daemon につながっている
WHEN 画面が複数の対象（terminal を含む）を購読する
THEN 画面が daemon に張る stream は 1 本だけである

## B-040: やり直しの回数の上限の実体

GIVEN この開発の変更が入っている
WHEN daemon の中の、単位時間あたりのやり直しの回数の上限を一覧する
THEN その実体は daemon に 1 つだけである

## B-041: 同時実行の枠

GIVEN この開発の変更が入っている
WHEN daemon の同時実行の枠を一覧する
THEN 全体で 1 つの枠は無く、枠は受け手の側の包みとして掛かっている

## 要件IDとBehavior IDの対応表
| Requirement ID | Behavior ID |
| --- | --- |
| R-001 | B-001 |
| R-002 | B-002 |
| R-003 | B-003 |
| R-004 | B-004 |
| R-005 | B-005 |
| R-006 | B-006 |
| R-007 | B-007 |
| R-008 | B-008, B-009 |
| R-009 | B-010 |
| R-010 | B-011 |
| R-011 | B-012 |
| R-012 | B-013, B-014 |
| R-013 | B-015, B-016 |
| R-014 | B-017, B-018 |
| R-015 | B-019 |
| R-016 | B-020 |
| R-017 | B-021 |
| R-018 | B-022 |
| R-019 | B-023 |
| R-020 | B-024 |
| R-021 | B-025 |
| R-022 | B-026 |
| R-023 | B-027 |
| R-024 | B-028, B-029 |
| R-025 | B-030, B-031 |
| R-026 | B-032 |
| R-027 | B-033 |
| R-028 | B-034, B-035, B-036 |
| R-029 | B-037 |
| R-030 | B-038 |
| R-031 | B-039 |
| R-032 | B-040 |
| R-033 | B-041 |
