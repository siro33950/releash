## B-001: シートが 1024×800 で開く

GIVEN 開発者が Node 面を開いている
WHEN 開発者が「…」から「History…」を選ぶ
THEN シートが 1024×800 の大きさで開く

## B-002: 見出しの横にブランチ名が出る

GIVEN 対象の worktree のブランチが `feature/thread-ui` である
WHEN シートが開く
THEN 見出し「History」の横に `feature/thread-ui` が出る

## B-003: 下端に「閉じる」と案内の文が出る

GIVEN シートが開いている
WHEN 開発者がシートの下端を見る
THEN 「閉じる」ボタンと、「行を選ぶと操作が出ます。ダブルクリックで復元・再開します。」が出ている

## B-004: 「閉じる」でシートが閉じる

GIVEN シートが開いている
WHEN 開発者が「閉じる」を押す
THEN シートは閉じる

## B-005: 検索欄と切り替えが出る

GIVEN 開発者が Node 面を開いている
WHEN シートが開く
THEN シートの上部に、「名前・最初の指示で検索」と案内する空の検索欄が出る
AND 「すべて」「Archive 済み」「Provider の履歴」の切り替えが出て、「すべて」が選ばれている

## B-006: 2 区画と見出しが出る

GIVEN Archive 済みの Session が 2 件、Archive 済みの Workflow が 2 件、Provider の履歴が 1 件以上ある
WHEN シートが開く
THEN 「ARCHIVE 済み」の区画と「PROVIDER の履歴」の区画が出る
AND 「ARCHIVE 済み」の見出しに「4」が出る
AND 「PROVIDER の履歴」の見出しに、件数は出ず、「このディレクトリで provider が残した Session」が出る

## B-007: 「ARCHIVE 済み」に Session と Workflow が時刻の新しい順に並ぶ

GIVEN 2 時間前に Archive した Session A、更新時刻が昨日 18:42 の Archive 済みの Workflow W、昨日 15:10 に Archive した Session B がある
WHEN シートが開く
THEN 「ARCHIVE 済み」の区画に、A、W、B の順に行が出る

## B-008: 「PROVIDER の履歴」が最終更新の新しい順に並ぶ

GIVEN 最終更新が 3 時間前の Provider の履歴 P と、最終更新が昨日 09:21 の Provider の履歴 Q がある
WHEN シートが開く
THEN 「PROVIDER の履歴」の区画に、P、Q の順に行が出る

## B-009: 「さらに読み込む」で続きが加わる

GIVEN 「PROVIDER の履歴」の区画に続きがあり、区画の末尾に「さらに読み込む」が出ている
WHEN 開発者が「さらに読み込む」を押す
THEN 既に出ていた行は残り、その後ろに続きの行が加わる

## B-010: Archive 済みの Session の行

GIVEN provider が Claude で、Session の名前が「Thread 面を GitHub の Conversation の見せ方にする」の Session を、2 時間前に Archive している
WHEN シートが開く
THEN その行の 1 段目に「Thread 面を GitHub の Conversation の見せ方にする」が出る
AND 2 段目に「Session · Claude · 2 時間前に Archive」が出る

## B-011: 完了した Archive 済みの Workflow の行

GIVEN タイトルが「development-v4」で、状態が完了、更新時刻が昨日 18:42 の Archive 済みの Workflow がある
WHEN シートが開く
THEN その行の 1 段目に「development-v4」が出る
AND 2 段目に「Workflow」、緑の点と「完了」、「昨日 18:42」が出る

## B-012: 中止した Archive 済みの Workflow の行

GIVEN 状態が中止の Archive 済みの Workflow がある
WHEN シートが開く
THEN その行の 2 段目に、赤の点と「中止」が出る

## B-013: 題のある Provider の履歴の行

GIVEN provider が Claude で、題が「Source Control 面の Commit メニューに Push を足したい」、最終更新が 3 時間前の Provider の履歴がある
WHEN シートが開く
THEN その行の 1 段目に「Source Control 面の Commit メニューに Push を足したい」が出る
AND 2 段目に「Claude · 3 時間前」が出る

## B-014: 題が無い Provider の履歴の行

GIVEN 題が無く、最初の指示が「PR 面の CHECKS の並び順を相談したい」の Provider の履歴がある
WHEN シートが開く
THEN その行の 1 段目に「PR 面の CHECKS の並び順を相談したい」が出る

## B-015: 題も最初の指示も無い Provider の履歴の行

GIVEN provider が Claude で、ID が `1a2b3c4d` で始まり、題も最初の指示も無い Provider の履歴がある
WHEN シートが開く
THEN その行の 1 段目に「Claude 1a2b3c4d…」が出る

## B-016: 今日の時刻の表記

GIVEN 今日、30 秒前、25 分前、2 時間前に Archive した Session がそれぞれある
WHEN シートが開く
THEN それぞれの行の時刻は「たった今」「25 分前」「2 時間前」と出る

## B-017: 昨日の時刻の表記

GIVEN 開発者が午前 1 時にシートを見ていて、前の日の 23:00 に Archive した Session がある
WHEN シートが開く
THEN その行の時刻は「昨日 23:00」と出る

## B-018: 昨日より前で今年の時刻の表記

GIVEN 今日が 11月15日で、同じ年の 11月12日に Archive した Session がある
WHEN シートが開く
THEN その行の時刻は「11月12日」と出る

## B-019: 去年以前の時刻の表記

GIVEN 今日が 2026年11月15日で、2025年11月12日に Archive した Session がある
WHEN シートが開く
THEN その行の時刻は「2025年11月12日」と出る

## B-020: 検索で名前に語を含む行だけになる

GIVEN 「ARCHIVE 済み」の区画に、名前が「Thread 面の見せ方」の Session、タイトルが「development-v4」の Workflow があり、「PROVIDER の履歴」の区画に、題が「Thread の並び順」の行と、題が「Push を足したい」の行がある
WHEN 開発者が検索欄に「thread」と入れる
THEN 「ARCHIVE 済み」の区画には「Thread 面の見せ方」の行だけが出て、見出しに「1」が出る
AND 「PROVIDER の履歴」の区画には「Thread の並び順」の行だけが出る

## B-021: Provider の履歴は最初の指示でも検索できる

GIVEN 題が「Push を足したい」で、最初の指示に「Commit メニュー」を含む Provider の履歴がある
WHEN 開発者が検索欄に「Commit メニュー」と入れる
THEN その行が「PROVIDER の履歴」の区画に出る

## B-022: まだ出していない Provider の履歴も検索できる

GIVEN 題が「古い相談」の Provider の履歴があり、「さらに読み込む」を押すまで一覧に出ていない
WHEN 開発者が検索欄に「古い相談」と入れる
THEN その行が「PROVIDER の履歴」の区画に出る

## B-023: 検索欄を空にすると元に戻る

GIVEN 検索欄に語が入っていて、一覧が絞り込まれている
WHEN 開発者が検索欄を空にする
THEN 一覧は絞り込む前の行に戻る

## B-024: 切り替えで「Archive 済み」を選ぶ

GIVEN シートが開いていて、切り替えが「すべて」である
WHEN 開発者が切り替えで「Archive 済み」を選ぶ
THEN 「ARCHIVE 済み」の区画だけが出る

## B-025: 切り替えで「Provider の履歴」を選ぶ

GIVEN 検索欄に「thread」が入っていて、切り替えが「すべて」である
WHEN 開発者が切り替えで「Provider の履歴」を選ぶ
THEN 「PROVIDER の履歴」の区画だけが出る
AND その区画には、「thread」を含む行だけが出る

## B-026: 切り替えで「すべて」に戻す

GIVEN 切り替えが「Archive 済み」である
WHEN 開発者が切り替えで「すべて」を選ぶ
THEN 「ARCHIVE 済み」の区画と「PROVIDER の履歴」の区画が出る

## B-027: 行を一度押すと選ばれるだけである

GIVEN シートに、Archive 済みの Session の行が出ていて、どの行も選ばれていない
WHEN 開発者がその行を一度押す
THEN その行が選ばれ、その行に「復元」と削除のボタンが出る
AND ほかの行には操作のボタンは出ない
AND 復元、再開、削除のどれも行われない

## B-028: 選んだ Archive 済みの Workflow の行に「復元」が出る

GIVEN シートに、Archive 済みの Workflow の行が出ている
WHEN 開発者がその行を一度押す
THEN その行に「復元」が出て、削除のボタンは出ない
AND 復元は行われない

## B-029: 選んだ Provider の履歴の行に「再開」が出る

GIVEN シートに、Provider の履歴の行が出ている
WHEN 開発者がその行を一度押す
THEN その行に「再開」が出る
AND 再開は行われない

## B-030: 「復元」で Session が復元される

GIVEN 開発者が Archive 済みの Session の行を選んでいる
WHEN 開発者がその行の「復元」を押す
THEN その Session が Node 面に出る
AND その Session は「ARCHIVE 済み」の区画から消える

## B-031: ダブルクリックで Workflow が復元される

GIVEN シートに、Archive 済みの Workflow の行が出ている
WHEN 開発者がその行をダブルクリックする
THEN その Workflow が Node 面に出る
AND その Workflow は「ARCHIVE 済み」の区画から消える

## B-032: 「再開」で Session が再開される

GIVEN 開発者が Provider の履歴の行を選んでいる
WHEN 開発者がその行の「再開」を押す
THEN その Session が Session タブで開く

## B-033: ダブルクリックで Session が再開される

GIVEN シートに、Provider の履歴の行が出ている
WHEN 開発者がその行をダブルクリックする
THEN その Session が Session タブで開く

## B-034: 削除のボタンで確認のアラートが出る

GIVEN 開発者が、名前が「Thread 面の見せ方」の Archive 済みの Session の行を選んでいる
WHEN 開発者が削除のボタンを押す
THEN 「「Thread 面の見せ方」を削除しますか？」、「削除した Session は History から消え、復元できません。」、「削除」ボタン、「キャンセル」ボタンを持つアラートが出る
AND その Session は削除されない

## B-035: アラートの「削除」で削除される

GIVEN Archive 済みの Session の削除を確かめるアラートが出ている
WHEN 開発者が「削除」を押す
THEN その Session は削除され、「ARCHIVE 済み」の区画から消える

## B-036: アラートの「キャンセル」で削除されない

GIVEN Archive 済みの Session の削除を確かめるアラートが出ている
WHEN 開発者が「キャンセル」を押す
THEN アラートが閉じる
AND その Session の行は「ARCHIVE 済み」の区画に残る

## 要件IDとBehavior IDの対応表

| Requirement ID | Behavior ID |
| --- | --- |
| R-001 | B-001 |
| R-002 | B-002 |
| R-003 | B-003, B-004 |
| R-004 | B-005 |
| R-005 | B-006, B-020 |
| R-006 | B-007 |
| R-007 | B-008, B-009 |
| R-008 | B-010 |
| R-009 | B-011, B-012 |
| R-010 | B-013, B-014, B-015 |
| R-011 | B-016, B-017, B-018, B-019 |
| R-012 | B-020, B-021, B-022, B-023 |
| R-013 | B-024, B-025, B-026 |
| R-014 | B-027, B-028, B-029 |
| R-015 | B-030, B-031 |
| R-016 | B-032, B-033 |
| R-017 | B-034 |
| R-018 | B-035, B-036 |
