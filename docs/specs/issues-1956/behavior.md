## B-001: 購読の読み取りの失敗の表示

GIVEN 画面がある状態を購読している
WHEN daemon がその状態を読めない
THEN 画面は、その状態を読めなかったことを表示する
AND 画面は、その状態が無い・空であるときと同じ表示をしない

## B-002: 購読中に読めなくなったとき

GIVEN 画面がある状態を購読していて、その値を表示している
WHEN その状態が変わり、daemon が読み直しに失敗する
THEN 画面は、その状態を読めなかったことを表示する
AND 画面は、前の値を今の値として表示しない

## B-003: 外部の情報の取り直しに失敗したとき

GIVEN 画面が Issue の一覧を購読している
WHEN daemon が Issue の取り直しに失敗する
THEN 画面は、Issue の一覧を読めなかったことを表示する

## B-004: 端末の表示の取り直しに失敗したとき

GIVEN 画面が端末を購読している
WHEN daemon が端末の表示の取り直しに失敗する
THEN 画面は、端末の表示を取り直せなかったことを受け取る

## B-005: 最初の読み取りに失敗した後の回復

GIVEN 画面がある状態の購読を始め、最初の読み取りが失敗した
WHEN その状態が変わり、daemon が読めるようになる
THEN 画面は、その状態の値を表示する

## B-006: 読めていない対象を後から購読する

GIVEN ある状態を読めず、それを購読している画面がある
WHEN 別の画面が同じ状態を購読する
THEN 後から購読した画面も、その状態を読めなかったことを表示する

## B-007: 読み込めない workflow の定義

GIVEN Automation で workflow を選んでいる
WHEN その定義を読み込めない
THEN 画面は、定義を読み込めなかったことを表示する
AND 画面は、定義が無いときと同じ表示をしない

## B-008: workflow の定義の形式を読めない

GIVEN Automation で workflow を選んでいる
WHEN その定義の形式を読めない
THEN 画面は、定義を読み込めなかったことを表示する
AND 画面は、その定義を既定の形式として表示しない

## B-009: Review で差分を読めない

GIVEN Review の画面で worktree を開いている
WHEN daemon が git の変更の状態を読めない
THEN 画面は、差分を読めなかったことを表示する
AND 画面は「No changes」を表示しない

## B-010: Review で一度読めた後に差分を読めなくなる

GIVEN Review の画面で worktree の差分を表示している
WHEN daemon が git の変更の状態を読み直せない
THEN 画面は、差分を読めなかったことを表示する
AND 画面は、前の差分を今の差分として表示しない

## B-011: Workspaces の未コミット数を読めない

GIVEN Workspaces の一覧を表示している
WHEN ある worktree の未コミット数を読めない
THEN その worktree について、未コミット数を読めなかったことを表示する
AND 0 や前の数を正しい値として表示しない

## B-012: Workspaces の一覧を集められない

GIVEN Workspaces の一覧を購読している
WHEN daemon が一覧を集める処理に失敗する
THEN 画面は、一覧を読めなかったことを表示する
AND 画面は、リポジトリが無いときと同じ表示をしない

## B-013: Workspaces の PR の状態を取れない

GIVEN Workspaces の一覧を表示している
WHEN あるリポジトリの PR の状態を取れない
THEN 画面は、そのリポジトリの PR の状態を取れなかったことを表示する
AND PR が無いときと同じ表示をしない

## B-014: 保存されたワークスペースの状態が無い

GIVEN worktree の保存された状態のファイルが無い
WHEN その worktree を開く
THEN 既定の配置で開く

## B-015: 保存されたワークスペースの状態を読めない

GIVEN worktree の保存された状態のファイルが、読めないか壊れている
WHEN その worktree を開く
THEN 画面は、保存された状態を読めなかったことを表示する
AND そのファイルは上書きされない

## B-016: Issue の一覧を取れない

GIVEN worktree 作成の画面で Issue の一覧を表示しようとしている
WHEN `gh` が失敗するか、出力を解析できないか、origin の URL を読めない
THEN 画面は、Issue の一覧を読めなかったことを表示する
AND 画面は、Issue が無いときと同じ表示をしない

## B-017: branch base・releash base を読めない

GIVEN branch base または releash base を表示する画面を開いている
WHEN git の設定を読めない
THEN 画面は、設定を読めなかったことを表示する
AND 画面は、未設定のときと同じ表示をしない

## B-018: 既定ブランチを探せない

GIVEN base ブランチを表示する画面を開いている
WHEN 既定ブランチを探すときの git の読み取りが失敗する
THEN 画面は、読めなかったことを表示する
AND 画面は、既定ブランチが無いときと同じ表示をしない

## B-019: まだコミットの無いブランチ

GIVEN まだコミットの無いブランチの worktree がある
WHEN その worktree の状態を読む
THEN 読み取りは失敗にならず、今までどおり表示される

## B-020: 実体の無くなった worktree

GIVEN 実体の無くなった worktree が git に登録されている
WHEN worktree の一覧を読む
THEN その worktree は一覧に出ない
AND 一覧全体は失敗にならない

## B-021: リポジトリではないパス

GIVEN リポジトリではないパスがある
WHEN そのパスについて git の状態を読む
THEN 読み取りは失敗にならず、リポジトリではないものとして扱われる

## B-022: worktree の情報を読めない

GIVEN worktree の一覧を表示する画面を開いている
WHEN ある worktree の名前・lock の状態・ブランチを読めない
THEN 画面は、読めなかったことを表示する
AND その worktree を黙って一覧から消さない
AND lock されていない・ブランチが `"unknown"` として表示しない

## B-023: session の履歴のタイトルを読めない

GIVEN Workspaces で session の履歴を表示している
WHEN ある session のタイトルまたは最初の入力を読めない
THEN 画面は、読めなかったことを表示する
AND タイトルが無い session と同じ表示をしない

## B-024: provider hook の警告の記録を読めない

GIVEN provider hook の警告の記録がある
WHEN その記録を読めないか、記録が壊れている
THEN 画面は、警告の記録を読めなかったことを表示する
AND 警告が無いときと同じ表示をしない

## B-025: workflow の一覧で定義ファイルを読めない

GIVEN workflow の置き場所に、読めない定義ファイルがある
WHEN Automation で workflow の一覧を表示する
THEN その定義は一覧に残る
AND 画面は、その定義を読めなかったことを表示する

## B-026: facet を読めない

GIVEN 利用者が置いた facet のファイルがあり、読めない
WHEN Automation でその facet またはその一覧を表示する
THEN 画面は、facet を読めなかったことを表示する
AND builtin の内容や空の説明を表示しない

## B-027: 診断で置き場所を読めない

GIVEN workflow または facet の置き場所を読めない
WHEN Automation で診断を表示する
THEN 画面は、診断できなかったことを表示する
AND 問題が無いという結果を表示しない

## B-028: 一覧と診断の対象が一致する

GIVEN workflow の置き場所に定義ファイルがある
WHEN Automation で一覧と診断を表示する
THEN 一覧の定義と、診断の対象の定義は同じファイルの組である

## B-029: 画面側で購読の失敗を表示する

GIVEN 次のどれかの画面を開いている: provider hook の警告、Workspaces の session 履歴または provider の選択肢、設定画面のブランチの選択肢、今のブランチ、Issue の一覧、base ブランチとその選択肢、リポジトリの一覧、worktree 作成画面のブランチまたはブランチの状態
WHEN その画面が購読している状態を daemon が読めない
THEN その画面は、読めなかったことを表示する
AND 読み込み中のまま止まったり、空のときと同じ表示をしたりしない

## B-030: archive の後の選択の照合

GIVEN workspace の archive の後に、選択の照合を待っている
WHEN 選択の状態を読めない
THEN 画面は、選択を読めなかったことを表示する

## B-031: 起動時のリポジトリを読めない

GIVEN git リポジトリの中で Releash を起動する
WHEN 起動時のリポジトリの読み取りが失敗する
THEN 画面は、読めなかったことを表示する

## B-032: git リポジトリの外で起動する

GIVEN git リポジトリの外で Releash を起動する
WHEN 起動時のリポジトリを読む
THEN 今までどおり、何も自動で開かず、失敗も表示しない

## B-033: リポジトリの追加で読めない

GIVEN リポジトリを追加しようとしてフォルダを選んだ
WHEN そのフォルダのリポジトリの読み取りが失敗する
THEN 画面は、読めなかったことを表示する
AND そのフォルダを普通のタブで開かない

## B-034: リポジトリではないフォルダを追加する

GIVEN リポジトリを追加しようとして、リポジトリではないフォルダを選んだ
WHEN そのフォルダのリポジトリを読む
THEN 今までどおり、そのフォルダを普通のタブで開く

## B-035: worktree の一覧と作成の応答

GIVEN worktree の一覧の購読、または worktree の作成の応答を受け取る
WHEN その内容を読む
THEN 内容に未コミット数と base ブランチは含まれない

## 要件IDとBehavior IDの対応表
| Requirement ID | Behavior ID |
| --- | --- |
| R-001 | B-001 |
| R-002 | B-002, B-003, B-004 |
| R-003 | B-005 |
| R-004 | B-006 |
| R-005 | B-007, B-008 |
| R-006 | B-009, B-010 |
| R-007 | B-011 |
| R-008 | B-012, B-013 |
| R-009 | B-014, B-015 |
| R-010 | B-016 |
| R-011 | B-017, B-018 |
| R-012 | B-019, B-020, B-021 |
| R-013 | B-022 |
| R-014 | B-023 |
| R-015 | B-024 |
| R-016 | B-025, B-026, B-027, B-028 |
| R-017 | B-029, B-030 |
| R-018 | B-031, B-032, B-033, B-034 |
| R-019 | B-035 |
