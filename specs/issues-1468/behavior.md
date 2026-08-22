## B-001: Worktree 配下の実行を同じ再帰構造の実行木で観測できる

GIVEN 同じ Worktree 配下で単独 Session、workflow、ネストした Sequence、および Fanout の実行が開始されている
WHEN 人間が Workspace の実行木を表示する
THEN 起きた実行は、それぞれの実行木の root から始まる同じ再帰構造の行として同一の一覧に表示される
AND 単独 Session は Session Node 1行を root とする実行木として表示される
AND 単独 Session を実行木から分離した別一覧は表示されない

## B-002: すべての葉 Node を同じ中央表示で観測できる

GIVEN 実行木に単独 Session、workflow 配下の Session、および Command の葉 Node が表示されている
WHEN 人間がいずれかの葉 Node を選択する
THEN 選択した Node の観測内容が同じ中央表示に表示される
AND Node の出自または内容種別によって別の中央表示へ切り替わらない

## B-003: 合成子は子を束ねる branch として表示される

GIVEN 実行木に実行開始済みの Sequence または Fanout が存在する
WHEN 人間が実行木を表示する
THEN その合成子は実行された子を束ねる branch として表示される
AND その合成子自身の中央表示は存在しない

## B-004: 実行開始済み Node だけが表示される

GIVEN workflow 定義に、実行開始済み Node、未実行 Node、分岐で選択されなかった Node、または未展開の Fanout child が存在する
WHEN 人間が実行中または終了後の実行木を表示する
THEN 実行開始済み Node だけが行として表示される
AND 未開始の Node は定義または expected slot から行として再生成されない

## B-005: 実行木から raw metadata が隠される

GIVEN 実行木の行または選択した Node の header が表示されている
WHEN 人間が表示内容を確認する
THEN 内部 ID、attempt 番号ラベル、および item または child の座標は表示されない

## B-006: retry の全 attempt が実行順に表示される

GIVEN 同じ Node が retry によって複数回実行されている
WHEN 人間が実行木を表示する
THEN 起きたすべての attempt がそれぞれ別の行として実行順に並ぶ
AND 各行に attempt 番号のラベルは表示されない

## B-007: 決着済みの過去 attempt は既定で折り畳まれる

GIVEN 同じ Node に複数の attempt があり、過去の attempt が決着済みである
WHEN 人間が実行木を最初に表示する
THEN 決着済みの過去 attempt は折り畳まれた状態で表示される

## B-008: 折り畳まれた過去 attempt を展開できる

GIVEN 決着済みの過去 attempt が折り畳まれて表示されている
WHEN 人間がその attempt を展開する
THEN その attempt の実行を観測できる

## B-009: 単独 Session の既存ライフサイクル操作が維持される

GIVEN 単独 Session に対する archive、削除、または archive 済み Session の一覧表示が変更前の契約で利用できる
WHEN 人間がその操作を行う
THEN 操作は変更前と同じく行える
AND 操作の観測可能な結果は変更前と変わらない

## B-010: workflow 実行木の既存ライフサイクル操作が維持される

GIVEN workflow 実行木に対する stop、resume、abort、または archive が変更前の契約で利用できる状態である
WHEN 人間がその操作を行う
THEN 操作は変更前と同じく行える
AND 操作の観測可能な結果は変更前と変わらない

## B-011: Node の既存操作が維持される

GIVEN Node に対する approve、retry、または close が変更前の契約で利用できる状態である
WHEN 人間がその操作を行う
THEN 操作は変更前と同じく行える
AND 操作の観測可能な結果は変更前と変わらない

## B-012: workflow 構文を一つの正本文書で確認できる

GIVEN workflow 定義を書く開発者が `docs/workflow-yaml-syntax.md` を参照している
WHEN 現行実装が受理する定義形式を確認する
THEN `main` 規約、nodes マップ、Sequence の `entry`・`output`・`children`、children の4形式、および隣接辺が説明されている
AND Interface の `input`・`artifact` と配線の `inputs` の分離が説明されている
AND completion と Session の二信号、Session の `provider`・`model`・`permission`、`on_failure`、`worktree`、および予約語が説明されている
AND Lua による workflow 定義が説明されている
AND トップレベルの entry フィールドを使わないことが説明されている

## B-013: engine 方針文書が現行モデルを説明する

GIVEN 保守者が `docs/workflow-engine-evolution-plan.md` を参照している
WHEN Node の種類、完了の定義、および実行木全体の状態を確認する
THEN Node は Session、Command、Fanout、および Sequence の4種として説明されている
AND 完了の定義は completion として説明されている
AND 実行木全体の状態は Running、Completed、および Aborted の3値として説明されている

## B-014: モデル境界文書が Worktree 配下の実行木を表す

GIVEN 保守者が `docs/workflow-engine-model-boundary.md` を参照している
WHEN Workspace と実行木の構造および状態を確認する
THEN 構造図は Worktree 配下の実行木を表している
AND Workspace 配下に WorkflowExecution と AgentSession が並列する構造は記載されていない
AND 実行木全体を6値で表す status は記載されていない

## B-015: lifecycle 文書が3値と Node 所有状態を前提とする

GIVEN 保守者が `specs/workflow-lifecycle/workflow-ideal-lifecycle.md` を参照している
WHEN 不変条件、遷移表、操作と状態の受理マトリクス、および capability 導出を確認する
THEN 実行木全体の状態は Running、Completed、および Aborted の3値として扱われている
AND WaitingApproval、Paused、および Failed は Node が所有する状態として扱われている
AND 6値の ExecutionStatus を前提とする規則は記載されていない

## B-016: 用語集が統一 Node モデルの語彙と所有関係を示す

GIVEN 保守者が `docs/architecture/GLOSSARY.md` を参照している
WHEN 統一 Node モデルの用語と Workspace の構造を確認する
THEN Sequence、completion、実行木（execution tree）、および辺（edge）が正規語として説明されている
AND `gate` は completion に対する使用禁止語として説明されている
AND Workspace の構造は Worktree 配下の実行木として説明されている
AND `isolated` 宣言により生まれる隔離 worktree の状態所有が説明されている

## B-017: 完了条件の旧語彙が公開面に残らない

GIVEN workflow の完了条件が正本文書、schema、または Diagnostic で扱われる
WHEN 開発者がその表現を参照する
THEN 完了条件は completion の語彙で表現される
AND `gate` は完了条件を表す語として現れない

## B-018: example の workflow 定義が一本化される

GIVEN 開発者が `docs/` または `specs/unified-node-model/` から workflow の example を参照する
WHEN 参照先を開く
THEN どちらからも同じ一つの workflow 定義へ到達する
AND 別内容の example 定義は併存しない

## B-019: 解禁済み構文に対する Diagnostic が example に出ない

GIVEN 一本化した example がネストした Sequence と Fanout の子に置いた合成子を含む
WHEN 現行の loader でその定義を検証する
THEN それらの構文に対する Diagnostic は報告されない

## B-020: 一本化した example は Diagnostic なしで load できる

GIVEN 一本化した example が `worktree` を宣言していない
WHEN 現行の loader でその定義を load する
THEN その定義は受理される
AND Diagnostic は報告されない

## B-021: 一本化した example を実行できる

GIVEN 一本化した example が現行の loader で受理されている
WHEN その定義から engine の実行を開始する
THEN 定義された実行が engine の実行経路で開始される
AND ネストした Sequence および Fanout の子に置かれた合成子が定義どおり実行される

## B-022: 統一 Node モデルの正本と改訂文書が整合する

GIVEN 開発者が `specs/unified-node-model/` と改訂後の正本文書で同じ概念を参照する
WHEN Node の種類、実行木、構文、completion、辺、状態所有、Worktree 所有、または example の記述を確認する
THEN 両方から同じ契約を読み取れる
AND 相互に矛盾する記述は存在しない

## B-023: 旧モデルが現行の公開契約へ読み替えられない

GIVEN 外部インターフェースから Workspace の実行木、workflow 定義、または実行状態を扱う
WHEN 統一 Node モデルへの移行前の別系統 Session 経路、旧構文、または廃止済みの語彙と状態が入力または参照される
THEN 現行の統一 Node モデルの契約だけが公開される
AND 旧構文は現行構文へ暗黙に読み替えられない
AND 別系統の単独 Session 契約および廃止済みの語彙と状態は公開されない

## 要件IDとBehavior IDの対応表

| Requirement ID | Behavior ID |
| --- | --- |
| R-001 | B-001 |
| R-002 | B-002 |
| R-003 | B-003 |
| R-004 | B-004 |
| R-005 | B-005 |
| R-006 | B-006 |
| R-007 | B-007, B-008 |
| R-008 | B-002, B-009, B-010, B-011 |
| R-009 | B-012 |
| R-010 | B-013 |
| R-011 | B-014 |
| R-012 | B-015 |
| R-013 | B-016 |
| R-014 | B-017 |
| R-015 | B-018 |
| R-016 | B-019, B-020 |
| R-017 | B-021 |
| R-018 | B-022 |
| R-019 | B-001, B-017, B-023 |
