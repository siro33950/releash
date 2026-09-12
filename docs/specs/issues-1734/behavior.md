## B-001: `delegate` の受理

GIVEN `artifact` を宣言した Session Node の `completion` に `delegate` map（`child` に同じ定義の Artifact を持つ Node 名、`when` に述語、`max_iterations` に1以上の整数、child が `input` を宣言していれば `inputs`）を書いた定義がある
WHEN `delegate` だけを持つ `completion` の定義と、`require: approval` と `delegate` を並べた `completion` の定義をそれぞれ load する
THEN どちらも Error Diagnostic なく load できる

## B-002: `delegate` の必須 field 欠落・未知キー・存在しない child

GIVEN Session の `delegate` から `child` を省いた定義、`when` を省いた定義、`max_iterations` を省いた定義、`child` に存在しない Node 名を書いた定義、`child` / `inputs` / `when` / `max_iterations` 以外のキーを書いた定義がある
WHEN それぞれの定義を load する
THEN いずれも Error Diagnostic になり、その定義は実行できない

## B-003: Session 以外での `delegate` の宣言

GIVEN Command、Fanout、Sequence のいずれかの Node の `completion` に、必須 field をすべて備えた `delegate` を書いた定義がある
WHEN その定義を load する
THEN Error Diagnostic になり、その定義は実行できない

## B-004: 提出時に述語が成立する場合

GIVEN `delegate` を宣言した Session が実行中であり、`when` が親の Artifact の required boolean field `done` を指す
WHEN 親 Session が `done: true` を含む Artifact を `releash workflow output submit` で提出し、provider が Stop する
THEN child は起動されない
AND 親 Session は delegate による保留なしに完了する

## B-005: 提出時に述語が成立しない場合

GIVEN `delegate` を宣言した Session が実行中であり、`when` が親の Artifact の required boolean field `done` を指す
WHEN 親 Session が `done: false` を含む Artifact を提出する
THEN child の NodeExecution が開始される
AND 親 Session は完了せず child の完了を待つ
AND 提出以外に child を起動する操作は UI、CLI、local API のどこにも存在しない

## B-006: child 完了時に述語が成立する場合

GIVEN `delegate` を宣言した Session の提出により child が起動しており、`when` が `child.passed` を指す
WHEN child が `passed: true` を含む Artifact を産出して完了する
THEN 親 Session は完了する
AND 親 Session の Artifact の `child` キーは child の Artifact である

## B-007: child 完了時に述語が成立しない場合

GIVEN `delegate` を宣言した Session の提出により child が起動しており、`when` が `child.passed` を指す
WHEN child が `passed: false` を含む Artifact を産出して完了する
THEN child の Artifact が親 Session の Artifact の `child` キーとして注入される
AND 親 Session は同じ NodeExecution、同じ attempt、同じ AgentSession のまま続行し、注入された結果を受け取る
AND 親 Session は再び Artifact を提出でき、その提出は B-004 / B-005 と同じ規則で評価される

## B-008: 述語の受理形と型検査

GIVEN `delegate` の `when` に、親の Artifact の field `done` と child の Artifact の field `child.passed` を要素にした `and` の map、および `child.passed` と `child.skipped` を要素にした `or` の map を書いた定義がある
WHEN それぞれの定義を load する
THEN 各参照の末端が required boolean であれば Error Diagnostic なく load できる
AND 末端が boolean でない参照、末端が直上 Object の `required` に含まれない参照、解決できない段を含む参照、要素が空の `and` / `or` を含む定義は Error Diagnostic になり、実行できない

## B-009: 参照先が未確定の参照の評価

GIVEN `delegate` の `when` が `child.passed` を指す Session が実行中である
WHEN 親 Session が最初の Artifact を提出する
THEN `child` はまだ `null` であるため `child.passed` は false として評価される
AND child が起動される

## B-010: 上限到達での完了と辺での区別

GIVEN `max_iterations: 2` の `delegate` を宣言した Session `implement` が Sequence の child であり、その辺が `when: { on: child.passed, then: done }` と `next: give_up` を持つ
WHEN 親が提出し、child が `passed: false` で完了して注入され、親が再提出し、child が再び `passed: false` で完了して注入され、親が3回目の提出をする
THEN 3回目の提出では child は起動されず、`when` は評価されず、`implement` は完了する
AND `implement` の Artifact の `child` は2回目の child の Artifact である
AND 辺は `child.passed` が false であるため `give_up` へ進む

## B-011: 親 Artifact の `child` キー

GIVEN `delegate` を宣言した Session が実行中である
WHEN 親 Session が Artifact を提出する
THEN その時点の親 Session の Artifact には Contract の field に加えて `child` キーがあり、値は `null` である
AND child が完了して注入されると `child` の値は child Node の Artifact そのもの（child 名を挟まない）になる
AND 2回目以降のラウンドでは `child` は最後の child の Artifact で上書きされる
AND 親 Session の完了後、local API の Artifact 取得と CLI の `releash workflow output get` で `child` キーを含む Artifact を読める

## B-012: child の kind ごとの `child` キーの形

GIVEN child が Session、Command、Sequence（子 `judge`）、Fanout（`items` なし、子 `scan`）のそれぞれである `delegate` を宣言した4つの Session がある
WHEN それぞれの `when` を `child.passed`、`child.passed`、`child.judge.passed`、`child.scan.passed` と書いた定義を load して実行する
THEN いずれも Error Diagnostic なく load できる
AND child 完了後の親 Artifact の `child` は、Session / Command では child の Artifact、Sequence では子名をキーとする統合 map、Fanout では child 名をキーとする map であり、各参照はその形に沿って child の値に解決される

## B-013: `child` 予約キーの衝突

GIVEN 直下に `child` field を持つ Contract を `artifact` に参照し、`delegate` を宣言した Session がある
WHEN その定義を load する
THEN Error Diagnostic になり、その定義は実行できない
AND delegate を宣言しない Node が同じ Contract を `artifact` に参照する定義は Error Diagnostic にならない

## B-014: 下流からの `child` の参照

GIVEN `delegate` を宣言した Session `implement` が Sequence の child であり、後続の child が配線 `inputs` で `verdict: implement.child.passed` を受け、`implement` の辺が `when: { on: child.passed, then: done }` を持つ
WHEN その定義を load して実行する
THEN その定義は Error Diagnostic なく load できる
AND `implement` の完了後、後続の child の input パラメータには最後の child の `passed` の値が渡り、辺はその値に従って進む

## B-015: `inputs` の配線の受理と検査

GIVEN child が input パラメータ `task` と `spec` を宣言し、親 Session が input パラメータ `spec` を持つ
WHEN `inputs` に `spec: spec`、`task` に親 Session の Node 名で参照する親の Artifact の field、および `request` を供給元とする配線を書いた定義を load する
THEN Error Diagnostic なく load できる
AND child が宣言していないパラメータへの配線、受理形でない供給元、解決できない field path を書いた定義は Error Diagnostic になり、実行できない
AND 親 Session の型付き input パラメータを、異なる Contract を宣言した child の型付き input パラメータへ配線した定義は、children エントリの配線と同じく Error Diagnostic なく load できる

## B-016: `inputs` の実行時の解決

GIVEN `inputs` に親 Session の Node 名で参照する親の Artifact の field と `<親Session名>.child.<field>` を供給元とする配線を書いた `delegate` を宣言した Session が実行中である
WHEN 親が1回目の提出をして child が起動し、その後 child の結果が注入されて親が2回目の提出をして child が再び起動する
THEN 各回の child は、その起動時点の親の直近の提出値と直近の `child` の値を input として受け取る

## B-017: 実行木での child の表示

GIVEN `delegate` を宣言した Session `implement` の提出により child `verify` が3回起動して完了した
WHEN UI で実行木を参照する、local API で execution を取得する、CLI で `releash workflow status --json` を実行する
THEN いずれの経路でも `verify` の NodeExecution は `implement` の NodeExecution を親とする部分木として現れる
AND `verify` は発火ごとに attempt 1、2、3 の行として並ぶ
AND `implement` の NodeExecution は3回の発火の間、同じ id と attempt のまま完了していない

## B-018: 注入前の中断からの resume

GIVEN `delegate` を宣言した Session の child が完了し、その結果が親 Session に注入される前に WorkflowExecution が中断した
WHEN その WorkflowExecution を resume する
THEN child は再実行されない
AND child の結果が親 Session に注入され、親 Session は続行する
AND 例外として、親 Session が結果の送付を受理した後・provider に届く前に中断していた場合は、resume で再送されず、親 Session は入力待ちのまま続行を人間に委ねる（初回指示と同じ扱い）

## B-019: 注入後の中断からの resume

GIVEN `delegate` を宣言した Session の child の結果が親 Session に注入された後に WorkflowExecution が中断した
WHEN その WorkflowExecution を resume する
THEN child は再実行されず、その結果は二重に注入されない
AND 親 Session は続行する

## B-020: provider session を復元できない場合

GIVEN `delegate` を宣言した Session の child が完了し、親 Session の provider session を復元できない
WHEN 注入のための続行、または resume を行う
THEN 親 Session は失敗し、children エントリの `on_failure` と手動 Retry の既存の失敗経路で扱われる
AND delegate 固有の復旧経路は存在しない

## B-021: `require: approval` と `delegate` の併記

GIVEN `completion` に `require: approval` と `delegate` を並べた Session が実行中である
WHEN 述語が成立して完了する条件、または上限到達で完了する条件を満たす
THEN 親 Session は完了せず WaitingApproval になる
AND 人間が Approve すると完了する
AND 述語が成立せず上限にも達していない時点では、Approve の対象にならない

## B-022: child の隔離

GIVEN `delegate` の child が `worktree: isolated` を宣言している
WHEN 親の提出により child が2回起動する
THEN child の各起動は、親の実行 worktree の HEAD から生成された別々の隔離 worktree を cwd として実行される
AND 親 Artifact に注入される child の Artifact には `worktree` キーがある

## B-023: 正本サンプルと builtin の load

GIVEN `implement_task` が `worktree: isolated` と `artifact` を宣言し、`completion.delegate` に `child: verify_task`、`inputs` の `task: task` と `spec: spec`、`when: child.complete`、`max_iterations` を持ち、`implement_all` が `implement_task` を item ごとに直接展開し、Sequence `implement_and_verify` を持たず、`fix_and_verify` と他のループが変更前のままである `workflows/examples/full-cycle-development.yml`、および `workflows/*.yml` の builtin 8本
WHEN それぞれを load する
THEN Diagnostic はゼロである

## B-025: WORKFLOW.md の整合

GIVEN 本変更後の `docs/glossary/WORKFLOW.md`
WHEN 「Session」「completion」「予約語」「Contract / schemas」「Lua」「Lua API」の各節を読む
THEN `delegate` の受理形、`artifact` を宣言した Session だけが宣言できること、child の条件、`max_iterations` が1以上であること、`inputs` の供給元、発火が Artifact 提出であること、評価の時点と規則、上限の意味、`require: approval` との併記が and であること、親 Artifact の `child` キーの形と child の kind ごとの参照形、child の worktree の規則、`child` が delegate を宣言した Session の Artifact の予約キーであることの記述が変更後の挙動と一致する
AND 「`require` 以外のキー（`delegate` を含む）は Error Diagnostic になる」の記述は残らない
AND 「Lua」節と「Lua API」の表は、Lua の `completion` table が `require` だけを受理し `delegate` を受理しないことを記述する

## B-026: DOMAIN.md の整合

GIVEN 本変更後の `docs/glossary/DOMAIN.md`
WHEN 正規語の表と状態所有の記述を読む
THEN `completion` の要求に `delegate` が含まれること、delegate が Session の所有する同一 session 継続機構であること、child の NodeExecution が親 Session の部分木であることが読める

## B-027: `artifact` を宣言しない Session での `delegate`

GIVEN `artifact` を宣言しない Session（`worktree: isolated` の宣言の有無を問わない）の `completion` に、必須 field をすべて備えた `delegate` を書いた定義がある
WHEN その定義を load する
THEN Error Diagnostic になり、その定義は実行できない

## B-028: Artifact を持たない child と親を含む child

GIVEN `child` に `artifact` を宣言せず `worktree: isolated` でもない Session を書いた定義、`child` に親 Session 自身を書いた定義、`child` に親 Session を children に含む Sequence または `main` を書いた定義がある
WHEN それぞれの定義を load する
THEN いずれも Error Diagnostic になり、その定義は実行できない

## B-029: child の共有と delegate だけからの参照

GIVEN ある Node を合成子の child に置き、同時に Session の `delegate` の `child` にも書いた定義、および同じ Node を二つの Session の `delegate` の `child` に書いた定義がある
WHEN それぞれの定義を load する
THEN いずれも Error Diagnostic になり、その定義は実行できない
AND どの合成子の children にも置かれず、`delegate` の `child` としてだけ参照される Node を持つ定義は、到達不能の Diagnostic にならず load できる

## B-030: `max_iterations` の値域

GIVEN `delegate` の `max_iterations` に `0`、負の整数、整数でない値をそれぞれ書いた定義がある
WHEN それぞれの定義を load する
THEN いずれも Error Diagnostic になり、その定義は実行できない

## B-031: `shared` な child の worktree

GIVEN `worktree: isolated` を宣言し `delegate` を宣言した Session の child が `worktree` を省略している
WHEN 親の提出により child が起動する
THEN child は親 Session の attempt の隔離 worktree を cwd として実行される

## 要件IDとBehavior IDの対応表

| Requirement ID | Behavior ID |
| --- | --- |
| R-001 | B-001, B-029 |
| R-002 | B-002, B-003, B-027, B-028, B-029, B-030 |
| R-003 | B-004, B-005 |
| R-004 | B-004, B-005 |
| R-005 | B-006, B-007 |
| R-006 | B-008, B-009 |
| R-007 | B-010 |
| R-008 | B-011, B-012 |
| R-009 | B-013 |
| R-010 | B-012, B-014 |
| R-011 | B-015, B-016 |
| R-012 | B-007, B-017 |
| R-013 | B-018, B-019 |
| R-014 | B-020 |
| R-015 | B-021 |
| R-016 | B-022, B-031 |
| R-017 | B-023 |
| R-019 | B-025, B-026 |
