## B-001: Lua 定義と YAML 定義の同一性

GIVEN ある workflow が Lua で定義されており、同じ内容の YAML 定義が存在する
WHEN それぞれを実行する
THEN 実行木の構造、Node の完了条件、辺の進行、事実ログ、read model は同一になる
AND 定義が Lua で書かれたことを示す区別は実行木にも事実ログにも現れない

## B-002: Lua の評価は load 時に限られる

GIVEN Lua で定義された workflow が load されている
WHEN その workflow を実行し、Node が起動・完了・再開する
THEN 実行中に Lua は評価されない

## B-003: require による合成は単一定義になる

GIVEN Lua 定義が require で他のファイルの部品を取り込んでいる
WHEN その定義を load する
THEN engine は単一の workflow 定義を受け取る
AND 部品は実行木に子 sequence として現れ、折り畳みと承認の単位になる

## B-004: 部品の複数回利用

GIVEN 部品が sequence を返す関数として書かれている
WHEN 同一定義の中でその部品を複数回使う
THEN 使用ごとに独立した Node 群が定義に現れる
AND それぞれが別々の実行として実行木に並ぶ

## B-005: 同一の値を複数の children へ置く記述

GIVEN Lua 定義が同一の node の値を複数の children エントリへ置いている
WHEN その定義を load する
THEN その定義は拒否される

## B-006: 存在しない参照

GIVEN Lua 定義が、宣言されていない facet キー、artifact を宣言していない node の field、または schema に存在しない field を参照している
WHEN その定義を load する
THEN その定義は拒否される
AND 拒否の理由は、その参照を書いた Lua のファイル名と行番号とともに報告される

## B-007: スコープ外の値の参照

GIVEN Lua 定義の合成子が、自分の children でも自分の input パラメータでもない値を配線の供給元にしている
WHEN その定義を load する
THEN その定義は拒否される
AND 拒否の理由は、その配線を書いた Lua のファイル名と行番号とともに報告される

## B-008: YAML と同じ誤りに対する診断

GIVEN ある誤りを含む定義が Lua と YAML の両方で書かれている
WHEN それぞれを検証する
THEN 同じ診断コード、同じ段、同じ趣旨のメッセージが報告される

## B-009: 編集支援

GIVEN workflows ディレクトリを LuaLS が有効なエディタで開いている
WHEN workflow 定義を編集する
THEN ビルダーの引数、node の参照、facet キーに対して補完と型検査が働く
AND node と facet の参照は定義元へジャンプでき、リネームが全参照へ追従する

## B-010: facet 本文への到達

GIVEN Lua 定義が facet を参照している
WHEN その参照から定義元へジャンプする
THEN その facet の本文である md ファイルへ到達できる

## B-011: 生成物が欠けている、または古い場合

GIVEN 型定義スタブまたは facet インデックスが存在しない、あるいは現在の facet と一致していない
WHEN Lua 定義を load して実行する
THEN load 結果と実行結果は、生成物が最新である場合と変わらない

## B-012: 評価の決定性

GIVEN 同じ Lua ファイル群が存在する
WHEN 一覧表示、診断、実行開始など、異なる契機で繰り返し load する
THEN 常に同じ workflow 定義が得られる
AND 時刻、環境変数、外部ファイルの読み取りによって結果が変わらない

## B-013: 終了しない定義

GIVEN Lua 定義の評価が終了しない、または過大なメモリを要求する
WHEN その定義を load する
THEN 評価は打ち切られ、その定義は拒否される
AND 打ち切られたことが診断として観測できる
AND Releash の他の操作は継続できる

## B-014: require の探索範囲

GIVEN Lua 定義が workflows ディレクトリの外にあるファイルを require している
WHEN その定義を load する
THEN その定義は拒否される
AND ディレクトリ外のファイルは読み込まれない

## B-015: アプリ内での Lua 定義の扱い

GIVEN Lua で定義された workflow が存在する
WHEN 人間が Releash の Automation UI でその workflow を開く
THEN 定義のソースは表示されない
AND 定義を編集する手段は外部エディタでの起動だけである

## B-016: Lua 定義の診断の提示

GIVEN Lua で定義された workflow が検証で誤りを含む
WHEN 人間が Releash の Automation UI を表示する
THEN その workflow の誤りの件数が一覧で分かる
AND 詳細では、各誤りがファイル名・行番号とともに一覧できる

## B-017: 既存 YAML 定義の互換性

GIVEN 既存の YAML で定義された workflow が存在する
WHEN 一覧表示、取得、保存、診断、実行を行う
THEN 変更前と同じ結果になる

## B-018: builtin workflow の互換性

GIVEN builtin workflow が存在する
WHEN 一覧表示、取得、実行、編集、削除を試みる
THEN 変更前と同じく一覧・取得・実行でき、編集と削除は拒否される

## B-019: workflow 名の重複

GIVEN 複数のファイルが同じ workflow 名を宣言している
WHEN その名前で実行を開始しようとする
THEN 開始は拒否される

## B-020: ファイル名と workflow 名の不一致

GIVEN Lua 定義が宣言する workflow 名がファイル名と一致しない
WHEN その定義を load する
THEN その定義は拒否される

## 要件IDとBehavior IDの対応表

| Requirement ID | Behavior ID |
| --- | --- |
| R-001 | B-001 |
| R-002 | B-002 |
| R-003 | B-003 |
| R-004 | B-004, B-005 |
| R-005 | B-006 |
| R-006 | B-007 |
| R-007 | B-008 |
| R-008 | B-009, B-010 |
| R-009 | B-011 |
| R-010 | B-012 |
| R-011 | B-013 |
| R-012 | B-014 |
| R-013 | B-015 |
| R-014 | B-016 |
| R-015 | B-017 |
| R-016 | B-018 |
| R-017 | B-019, B-020 |
