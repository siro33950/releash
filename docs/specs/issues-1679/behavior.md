## B-001: 事実行の追記が file descriptor を増やさない

GIVEN プロセスの open fd 数が観測できる
WHEN node の事実行を1行追記する
THEN 事実行が記録される
AND 追記の実行中および完了後の open fd 数が、追記を理由として追記前より増えない

## B-002: 同時多重の追記でも file descriptor が増えない

GIVEN 複数の workflow が並列に実行され、事実行の追記が同時に発生する
WHEN それらの追記が並行して行われる
THEN すべての事実行が記録される
AND 同時に実行される追記の数が増えても、追記を理由とする open fd 数の増加が起きない

## B-003: fd 上限の直下でも事実行を追記できる

GIVEN プロセスの open fd 数が fd soft limit の直下にある
WHEN node の activation が session_attached の事実行を追記する
THEN 事実行が記録される
AND `failed to create fact append runtime: Too many open files (os error 24)` は発生しない
AND activation は file descriptor を確保できないことを理由に失敗しない

## B-004: 同期文脈からの追記

GIVEN 同期文脈から事実行の追記が行われる
WHEN 事実行を追記する
THEN 事実行が記録される
AND 追記の結果が呼び出し元へ返り、実行が停止しない

## B-005: async 文脈（provider Stop の受理）からの追記

GIVEN provider の Stop を受理する async 経路が事実行の追記を行う
WHEN その事実行を追記する
THEN 事実行が記録される
AND 受理処理が継続し、panic、deadlock、実行の停止が起きない

## B-006: async 文脈（node activation）からの追記

GIVEN node の activation が async 経路で事実行の追記を行う
WHEN その事実行を追記する
THEN 事実行が記録される
AND activation が継続し、panic、deadlock、実行の停止が起きない

## B-007: 事実行の内容と追記順序の維持

GIVEN 同一 node に対して複数の事実が順に発生する
WHEN それらを追記する
THEN 記録された各事実行の内容が変更前と一致する
AND 同一 node の事実行の並びが、事実の発生順と一致する

## B-008: 追記が途中で失敗したときの記録の単位

GIVEN 複数の事実行を続けて追記する
WHEN その途中で、ある事実行の追記が失敗する
THEN 失敗した事実行より前に追記された事実行は記録されたまま残る
AND 失敗した事実行は記録されない
AND 追記の失敗が呼び出し元へ返る

## B-009: 追記先に起因する失敗の扱い

GIVEN 追記先の SQLite に起因して事実行の追記が失敗する
WHEN node の処理がその事実行の追記を行う
THEN 追記の失敗が呼び出し元へ伝わる
AND その node は失敗として扱われる
AND 追記できなかった事実は、記録されたものとして扱われない

## 要件IDとBehavior IDの対応表
| Requirement ID | Behavior ID |
| --- | --- |
| R-001 | B-001, B-002 |
| R-002 | B-003, B-009 |
| R-003 | B-004, B-005, B-006 |
| R-004 | B-007, B-008 |
| R-005 | B-008, B-009 |
