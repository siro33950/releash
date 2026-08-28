## B-001: module 本体の評価中に host API を呼ぶ module を参照する定義が有効な定義になる

GIVEN workflows ディレクトリに Lua module があり、その module は本体の評価中に host API を呼んで Node を作り、その Node を返している
AND 同じ workflows ディレクトリに、その module を `require` して返された Node を workflow の Node として参照する Lua workflow 定義がある
WHEN その workflow 定義を取得する
THEN 取得は成功する
AND 取得した定義には、module が返した Node が workflow の Node として含まれる

## B-002: 該当する定義が存在しても workflow 一覧が成功し当該定義が有効な定義として並ぶ

GIVEN B-001 の Lua module と Lua workflow 定義が workflows ディレクトリにある
AND 同じ workflows ディレクトリに他の workflow 定義がある
WHEN workflow 一覧を取得する
THEN 取得は成功し、workflow の配列が返る
AND 当該 workflow は有効な定義として一覧に含まれる
AND 他の workflow も通常どおり一覧に含まれる

## B-003: 一覧取得の成功が Tauri command と local API の双方で成り立つ

GIVEN B-002 の状態である
WHEN Tauri command `list_workflows` と local API `GET /v1/workflows` のそれぞれで workflow 一覧を取得する
THEN いずれの入口でも取得は成功し、workflow の配列が返る
AND いずれの入口でも当該 workflow は有効な定義として一覧に含まれる

## B-004: load に失敗する Lua 定義があっても一覧の取得は成功する

GIVEN workflows ディレクトリに、load に失敗する Lua workflow 定義がある
AND 同じ workflows ディレクトリに、正常に load できる workflow 定義が他にある
WHEN workflow 一覧を取得する
THEN 取得は成功し、workflow の配列が返る
AND load に失敗した定義は不正な定義として一覧に含まれる
AND その一覧上の扱いは、診断エラーを持つ YAML 定義が一覧に現れる扱いと同じである
AND 他の workflow は通常どおり一覧に含まれる

## B-005: 該当する定義が存在しても workflow diagnostics の取得は成功する

GIVEN B-001 の Lua module と Lua workflow 定義が workflows ディレクトリにある
AND 同じ workflows ディレクトリに他の workflow 定義がある
WHEN workflow diagnostics を取得する
THEN 取得は成功する
AND 当該定義について、load の失敗を示す Diagnostic 項目は報告されない
AND 他の定義の診断結果は通常どおり返る

## B-006: load に失敗する定義があっても他の定義の診断結果が返る

GIVEN workflows ディレクトリに、load に失敗する Lua workflow 定義がある
AND 同じ workflows ディレクトリに、正常に load できる workflow 定義が他にある
WHEN workflow diagnostics を取得する
THEN 取得は成功する
AND 失敗した定義について、失敗位置を示す Diagnostic 項目が報告される
AND 他の定義の診断結果は通常どおり返る

## B-007: diagnostics の取得が local API と CLI の双方で成り立つ

GIVEN B-005 の Lua module と Lua workflow 定義が workflows ディレクトリにある
AND 同じ workflows ディレクトリに、load に失敗する Lua workflow 定義がある
WHEN local API `GET /v1/workflow/diagnostics` と CLI `releash workflow diagnostics` のそれぞれで診断を取得する
THEN いずれの入口でも取得は成功する
AND いずれの入口でも、B-005 の定義について load の失敗を示す Diagnostic 項目は報告されない
AND いずれの入口でも、失敗した定義の Diagnostic 項目と他の定義の診断結果が同じ内容で得られる

## B-008: 現在正常に load できる定義の結果が変わらない

GIVEN workflows ディレクトリに、現在正常に load できる Lua workflow 定義と YAML workflow 定義がある
WHEN workflow 一覧、各 workflow 定義、および workflow diagnostics を取得する
THEN 一覧の内容は変更前と同じである
AND 各 workflow 定義の内容は変更前と同じである
AND 診断結果は変更前と同じである

## B-009: require が workflows ディレクトリ配下だけを探索する

GIVEN Lua workflow 定義が、workflows ディレクトリ配下にない module を `require` している
WHEN workflow diagnostics を取得する
THEN 当該定義は load に失敗する
AND 探索範囲の逸脱を示す Diagnostic 項目が報告される

## B-010: Lua 評価のメモリ上限と命令数上限が有効である

GIVEN Lua workflow 定義の評価が、メモリ上限または命令数上限を超える
WHEN workflow diagnostics を取得する
THEN 当該定義は load に失敗する
AND 上限の超過を示す Diagnostic 項目が報告される

## 要件IDとBehavior IDの対応表
| Requirement ID | Behavior ID |
| --- | --- |
| R-001 | B-001, B-002 |
| R-002 | B-002 |
| R-003 | B-003 |
| R-004 | B-005, B-007 |
| R-005 | B-008 |
| R-006 | B-009, B-010 |
| R-007 | B-002, B-004, B-006, B-007 |
