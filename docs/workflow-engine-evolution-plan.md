# ワークフローエンジン発展計画

この文書は、Releash のワークフローエンジンをこれからどう発展させるかを定義する。戦略・採用判断・マイルストーンの一次 Owner である。

語彙は [`architecture/GLOSSARY.md`](./architecture/GLOSSARY.md) を正とする。本文書は GLOSSARY 正規語で記述する。

## 目的

Releash は workflow を決定論的な実行レールとして扱う。開発者が WorkflowDefinition を定義し、WorkflowExecution として実行し、観測し、承認できるようにする。`gate: approval` の session は対話式で、承認するまで人間が指示し直せる。

workflow engine は状態遷移の唯一の権威である。Agent / UI / CLI / API は action を要求できるが、workflow state を直接決めない。状態変更は typed command を唯一の入口として engine に届く。

## プロダクト方針

```text
User / UI / CLI / API / Agent action
        |
        v  typed command（状態変更の唯一の入口）
        v
Workflow Engine（状態遷移の唯一の権威・WorkflowExecution を所有）
        |
        v
NodeExecution
  - command   (非対話の一回実行)
  - session   (agent 実行)
  - fanout    (子 NodeExecution 群の展開)
```

- NodeExecution の種別は `command` / `session` / `fanout` の3つ。完了判定（自動 / 人間）は session の `gate`（`auto` / `approval`）で表す。種別ではない。
- WorkflowExecution は typed command boundary から起動される。タイマー・外部イベント連携などの起動設定は本マイルストーン外で扱う。
- human checkpoint を第一級に扱う。`gate: approval` の session で止まり、人間が Artifact（diff / 出力 / 検証結果）を見て承認する。承認しなければ対話で指示し直す（session は対話式・却下や再実行という別操作は無い）。
- UI / CLI / API は同じ command boundary を共有する。画面操作と外部操作が別世界にならない。
- engine は確率論ではなく決定論的に動く。NodeExecution の成否・遷移は Contract 検証済み Artifact と exit code で判断する。

## 中核モデル

語彙・状態所有は GLOSSARY を正とする。本節は engine が主語として扱う対象を示す。

### WorkflowDefinition

ユーザーが書く workflow template。`name` / `description` / `builtin` / `nodes` を持ち、YAML deserialize の直接先となる。

### NodeDefinition

WorkflowDefinition 内の実行単位の定義。種別は **kind ブロックをちょうど1つ**持つことで表す（`type:` フィールドは持たず、どのブロックがあるかで自明）。

```text
command: "<shell>"   -> 非対話の一回実行（標準結果 ok / exit_code / stdout 等）
session: { ... }     -> agent 実行。model / permission / gate / facets を持つ
fanout: { ... }      -> 子 NodeExecution 群を展開する（child / items）
```

- 完了判定は session の `gate`（`auto` / `approval`）で表す。approval は種別ではない。
- facet 参照（policy / knowledge / instruction）は session の `facets:` にまとめる。
- 他 node の Artifact を入力に受けるのは `inputs:`。
- 詳細な構文は [`workflow-yaml-syntax.md`](./workflow-yaml-syntax.md) を正本とする。

### WorkflowExecution

WorkflowDefinition の一回の実行。`status` を持つ。起動元、対象 Worktree、現在 node、タイムスタンプ、失敗理由を集約する。

### NodeExecution

NodeDefinition の一回の実行結果。所属 WorkflowExecution・node・反復回で識別し、Contract 検証済み Artifact・session 参照・token 使用量・失敗理由を保持する。

### Fanout

親 NodeExecution から展開された子 NodeExecution 群を束ねる実体。

- `child`: 展開する Node を名前で参照する（普通の Node を1つ、または複数）。子に特別扱いは無い。
- `items`: 任意の配列（リテラル / 前 Node の Artifact 配列）。child を要素ぶん展開する。Artifact 参照なら実行時に件数が決まり、0 件なら展開なし。
- 組合せ: child 複数 = 別 Node を並列 / child 1つ + items = 配列展開 / child 複数 + items = マトリクス（item × child）。
- 各要素は子の `input` に入る（items 要素型 == child の input 型を load 時に検証）。
- fanout の `artifact` は子 Artifact の配列。**集約機構（aggregate / all / any）は持たない**。結果でまとめて分岐したい場合は、配列を畳んで boolean を出す Node（command 等）を挟み、通常の rules で分岐する。

### Task

WorkflowExecution 内で NodeExecution 間を跨ぐ作業情報。main / sub の区別を持たない。WorkflowExecution に属する状態として所有する。

固定 schema の global な作業リスト `tasks[]`（要素 `{ id, description, done }`）として持つ。書き込みは CLI のみ、workflow からは read（`fanout` の `items: tasks`）のみ。workflow YAML では定義しない。`tasks` は予約語。`start` の `"<task>"` 文字列は `tasks` には入らず、初回 Artifact `request`（String・予約名）になる。

### Artifact / Contract

Artifact は NodeExecution の間で生成・参照される判断材料・成果物・中間出力で、状態を持たない。Contract は Artifact の validation 語彙。全 node 種別（command / session / CLI submit）が同一の Contract 機構で検証済み Artifact を出す。routing が見る値は Contract に宣言された boolean / enum であること。起動時の `"<task>"` は初回 Artifact `request`（String・予約名）として扱う。

### 状態変更と event log

- 状態変更は typed command（domain entity ではない実装機構）を唯一の入口とする。UI button / CLI / API / Agent action はすべてこの command に落とす。
- engine の状態遷移は append-only な event log として積む。event log は projection / resume / 観測の adapter 語彙であり、domain entity ではない。
- 現在状態は WorkflowExecution / NodeExecution から読み、履歴は event log から辿る。

### Diagnostic

WorkflowDefinition / NodeDefinition の構文・参照・validation error は lifecycle state ではなく Diagnostic として扱う。

## 採用するもの

| 項目 | 方針 |
| --- | --- |
| Node 種別 | kind ブロック（`command` / `session` / `fanout`）で種別を表す。`type:` は持たない。完了判定は session の `gate`（`auto` / `approval`）。必要になれば `loop` を追加する。 |
| Command / validation gate | test / lint / validation を決定論的な command node として実行し、標準結果（`ok` / `exit_code`）と stdout-JSON の Contract 検証済み Artifact で分岐する。 |
| Fanout | 並列を Fanout に統一する。`child`（Node 参照、単一/複数）と `items`（配列）で展開。集約 node は持たず、結果の畳みは command 等の node で行う。 |
| 構造化出力（Contract 検証済み Artifact） | 全 node 種別が同一 Contract 機構で typed な Artifact を出す。CLI/API からも提出できる。`schemas:` で Contract を宣言する。 |
| Routing / Diagnostic | 遷移は `rules`（`when` / `switch` / `next` / `loop_guard`）。順序非依存で、網羅・排他・ループ健全性を load 時に検証する。式言語は持たない。 |
| CLI/API | UI / Agent / Remote が共有する typed command boundary にする。 |
| WorkflowExecution 管理 | 実行を execution id で扱い、status / logs / approval / abort の主語を WorkflowExecution にする。 |
| Workflow Panel | 右パネルを Review / Workflow で切り替え、active execution・timeline・node 詳細・conversation・承認・logs・Artifact を置く。 |

## 採用しないもの

| 項目 | 理由 |
| --- | --- |
| テンプレート / Skill / Main Agent 仲介 | 不要と判断し採用しない。 |
| Worktree isolation | Releash は Worktree を選択してから task を渡す設計。workflow 起動時の自動生成は扱わない。 |
| Chat router | 自然文からの workflow 自動選択は不要。CLI で十分。 |
| PR/Issue の直接 lifecycle 連携 | 直接 API 連携ではなく workflow template の操作として表現する。 |
| Workflow marketplace / defaults | curated built-in に絞る。 |
| Per-node MCP | 現在の workflow boundary では不要。 |
| Workflow map / DAG 表示 | timeline と node 詳細で足りる。loop があり厳密な DAG 表示は誤解を生む。 |

## CLI/API の形

CLI は typed protocol として、Agent / UI / remote / engine をつなぐ。

```sh
# 観測
releash workflow list
releash workflow executions
releash workflow status <execution-id>
releash workflow logs <execution-id>

# 操作（状態変更は typed command 経由）
releash workflow start <workflow-name> "<task>"
releash workflow approve <execution-id> --node <node-name> --comment "LGTM"
releash workflow abort <execution-id>

# Contract 検証済み Artifact の提出
releash workflow output submit <execution-id> --node <node-name> --type <contract> --json '{"key":"value"}'
releash workflow output validate <execution-id> --node <node-name> --file output.json
releash workflow output get <execution-id> --node <node-name>

# Task（global 作業リスト・書き込みは CLI のみ）
releash task list <execution-id>
releash task add <execution-id> --description "..."
releash task done <execution-id> --id <task-id>
```

CLI は local API 経由で engine を操作する。headless engine（server-client 化）は別系列（GitHub #77/#78/#79）で扱い、後段で統合する。

## UI 方針

右パネルを Review / Workflow で切り替える。Workflow panel には次を置く。

- active WorkflowExecution summary と execution 履歴
- event timeline
- NodeExecution 詳細と conversation transcript
- approval actions（CLI と同じ command boundary）
- logs と Contract 検証済み Artifact

main agent は user-facing narrator として残す（進捗報告・承認依頼・失敗 summary）。state transition は所有しない。approve / abort / output 提出は typed command として engine に戻す。

## マイルストーン

構文の正本は [`workflow-yaml-syntax.md`](./workflow-yaml-syntax.md)、完成形の例は [`examples/full-pipeline.yml`](./examples/full-pipeline.yml)。

### Workflow Engine 新モデル移行

目的: 現行実装の旧 workflow 表現（`type: agent/bash/approval/parallel`、`output_contract`、`pass_output_from`、`parallel_children`、`aggregate`、step/run 語彙）を、`command` / `session` / `fanout`、Contract 検証済み Artifact、WorkflowExecution / NodeExecution 主語へ移行する。

方針:

- マイルストーンは一つにまとめる。
- 配下 issue は表現単位で切る。各 issue は schema 置換だけでなく、実行、validation、event log / projection、CLI/API、UI、built-in workflow の移行までを完了条件に含める。
- 新旧互換を長く持たない。issue 内で一時 adapter が必要な場合も、完了時点では旧表現を残さない。

配下 issue:

1. NodeDefinition を kind block へ移行する

   - `type:` を廃止し、`command` / `session` / `fanout` の kind block をちょうど1つ持つ schema にする。
   - `session.facets` に `policy` / `knowledge` / `instruction` を集約する。
   - `artifact` / `inputs` / `input` / `rules` を共通フィールドとして扱う。
   - kind block が0個 / 2個以上、`tasks` など予約語との衝突、未定義 Contract / node 参照は load 時 Diagnostic にする。

2. 文法健全性 / Diagnostic front-end を独立させる

   - WorkflowDefinition の検証を parse / shape、resolve、typecheck、control-flow の段階に分ける。
   - parse / shape は YAML 構文、unknown field、kind block 個数、kind ごとの許可 field を検査する。
   - resolve は node 名、Contract 名、Artifact path、予約名（`request` / `tasks` / `item`）を解決する。
   - typecheck は `rules.when.on` の boolean、`switch.on` の enum、fanout `items` と child `input` の型一致、`artifact` / `input` の Contract 存在を検査する。
   - control-flow は終端 node、到達不能 node、cycle、`loop_guard`、rules の排他・網羅を検査し、任意の Artifact 値から遷移先が一意に定まることを保証する。
   - Diagnostic は lifecycle state ではなく validation result として返す。UI / CLI は Rust が返す Diagnostic code / span / message を表示するだけにし、frontend に validator を再実装しない。

3. `approval` node を `session.gate=approval` に移行する

   - `approval` を node 種別から削除し、session の完了 gate として扱う。
   - `gate: auto` / `gate: approval` を session 必須 field にする。
   - `gate: approval` は承認まで完了しない対話式 session とし、承認しなければ同じ session で人間が指示を続ける。
   - `reject` / reject rule / rerun 操作は廃止する。abort は WorkflowExecution に対する別 typed command として扱う。
   - approval chat / UI action / CLI approve / stale target validation を `session.gate=approval` に合わせる。

4. Contract / Artifact を `schemas:` と `artifact:` に統一する

   - 旧 `output_contract` / `input_contracts` を廃止し、YAML 内の `schemas:` と node の `artifact:` / `input:` を正にする。
   - 各 NodeExecution は最大1つの Contract 検証済み Artifact を産出し、Node 名で参照できるようにする。
   - command は stdout-JSON、session / CLI submit は typed command 経由で Artifact を提出する。
   - routing が参照する field は Contract に宣言された boolean / enum に限定する。

5. Artifact 入力と参照規約を実装する

   - 旧 `pass_output_from` / `pass_previous_response` / `workflow_variables` 依存の入力注入を `inputs:` に置き換える。
   - `request` を起動時 `"<task>"` 由来の初回 Artifact として扱う。`request` は Node 名ではなく予約 Artifact 名にする。
   - fanout child では `item` / `item.<field>` を使えるようにする。
   - template 補間は `{{ request }}` / `{{ node.field }}` / `{{ item.field }}` の参照規約に統一する。

6. `rules` を順序非依存の Diagnostic 対象にする

   - 旧 regex `match` / `next` と node 直下 `cycle_guard` を廃止し、`when` / `switch` / `next` / `loop_guard` に移行する。
   - `when` は boolean、`switch` は enum、`next` は catch-all として型付きに検証する。
   - 排他、網羅、無防備な loop を load 時 Diagnostic にする。
   - 式言語は持たず、比較・集約は command / session が boolean / enum Artifact に落としてから routing する。

7. `bash` を `command` node に移行して実行可能にする

   - 旧 `type: bash` を廃止し、`command: "<shell>"` kind block にする。
   - command 実行は `ok` / `exit_code` / `stdout` / `stderr` / `duration` を標準結果として持つ。
   - `artifact:` 指定時は stdout を JSON parse / Contract validation し、Artifact として保存する。
   - command result と Artifact field の両方で routing できるようにする。

8. `parallel_children` を `fanout.child` に移行する

   - 旧 `parallel` / `parallel_children` を廃止し、fanout が普通の NodeDefinition を名前で参照する形にする。
   - fanout child は leaf として Artifact を返すだけにし、child 自身の `rules` は fanout 実行中は無視する。
   - child 複数、child 1つ + `items`、child 複数 + `items` のマトリクスを実行できるようにする。
   - fanout child を個別 NodeExecution として event log / projection / UI に出す。

9. `aggregate` を廃止し、畳み込みは通常 node に移す

   - 旧 `aggregate` / `all_match` / `any_match` を廃止する。
   - fanout の Artifact は子 Artifact 配列とする。
   - 配列をまとめて分岐したい場合は command 等の通常 node で boolean / enum Artifact を作り、通常 `rules` で分岐する。
   - built-in workflow の aggregate は reducer command / session node に移す。

10. WorkflowExecution / NodeExecution read model へ移行する

   - 現在状態は WorkflowExecution / NodeExecution から読み、履歴は event log projection で辿る。
   - 旧 `StepHistoryEntry` / `StepOutput` / `ParallelStepState` / `WorkflowStateSnapshot` の公開語彙を NodeExecution / Artifact / Fanout に寄せる。
   - `run_id` / `WorkflowRun` / `runs` の外部語彙を `execution_id` / WorkflowExecution / `executions` に揃える。内部互換名を残す場合も外部 API では露出しない。

11. CLI/API command boundary を新語彙に揃える

    - `releash workflow start <workflow-name> "<task>"` を CLI から起動できるようにする。
    - `releash workflow executions` / `status <execution-id>` / `logs <execution-id>` を正にする。
    - `output submit|validate|get` は `--node` / `--type` を使い、step 語彙を出さない。
    - UI / CLI / API / Agent action は同じ typed command boundary に落とす。CLI は local API 経由を正とし、file-direct / pending file 経路は必要最小の adapter に縮退する。

12. WorkflowExecution-owned `tasks[]` を実装する

    - 固定 schema の `tasks[]`（`{ id, description, done }`）を WorkflowExecution に属する状態として持つ。
    - `releash task list|add|done <execution-id>` を追加し、書き込みは CLI に閉じる。
    - workflow からは read のみ許可し、`fanout.items: tasks` で展開できるようにする。
    - workflow YAML から `tasks` へ書き込めないこと、Node 名 `tasks` を拒否することを保証する。

13. Resume を abort-only recovery から移行する

    - 中断状態を再開可能な checkpoint として WorkflowExecution / NodeExecution に表現する。
    - event log から最後に確定した NodeExecution までを再構築し、次の NodeExecution から再開できるようにする。
    - orphan recovery は強制 abort ではなく、abort / resume を typed command として選べる形にする。

横断完了条件:

- `docs/examples/full-pipeline.yml` が新 schema で load / 実行できる。
- built-in workflow は新 schema に移行済みで、旧 `type` / `output_contract` / `input_contracts` / `pass_output_from` / `parallel_children` / `aggregate` / step 語彙を含まない。
- Automation UI / Workflow panel は新語彙で表示・操作し、domain behavior を frontend に持たない。
- Tauri / CLI / Remote / Agent action が同じ backend-owned WorkflowExecution state と typed command boundary を使う。
- 旧 workflow state / old NDJSON / old YAML 互換は保持しない。必要な移行 adapter はこのマイルストーン内で撤去する。

## テスト方針

各 issue は、旧表現を削る regression test と新表現の behavior test を同じ PR に置く。新ロジックは Rust 側の domain / usecase / adaptor test を主にし、frontend test は表示・操作・invoke 境界に限定する。

必須テスト:

- Diagnostic front-end: parse / shape、resolve、typecheck、control-flow の各段階が structured Diagnostic（code / span / message）を返すこと。
- Fixture suite: `valid/` と `invalid/` の YAML fixture を用意し、invalid fixture は期待 Diagnostic code を固定して検証すること。
- Parse / shape: YAML 構文、unknown field、kind block が0個 / 2個以上、kind ごとの不許可 field、旧 `type:` / `output_contract` / `parallel_children` / `aggregate` / `rules.match` の拒否。
- Resolve: 予約語 `tasks` の node 名拒否、未定義 node / Contract / Artifact path、`request` / `tasks` / `item` のスコープ違反、fanout child 参照の解決失敗を Diagnostic にすること。
- Typecheck: `when.on` が boolean field、`switch.on` が enum field、`artifact` / `input` が既存 Contract、fanout `items` の要素型と child `input` 型が一致すること。
- Control-flow: 終端 node、到達不能 node、rules の排他・網羅、switch enum の抜け、cycle に到達可能な `loop_guard` が無い場合の拒否、任意の Artifact 値で遷移先が1つに定まること。
- Session gate: `gate` 必須、`gate: auto` の自動完了、`gate: approval` が承認まで完了しないこと、同じ session で追加指示できること。
- Approval command: approve は通る、stale / unauthorized target は拒否される、reject command / reject rule が受理されない。
- Artifact / Contract: `schemas:` の validation、`artifact:` 産出、Contract validation success / failure、routing 対象 field が boolean / enum 以外なら Diagnostic。
- CLI submit: session / CLI submit が同じ Artifact 機構に書き込むこと、`workflow output submit|get|validate` が `--node` / `--type` 語彙で動くこと。
- Input / reference: `inputs: [request]`、`inputs: [node]`、`{{ request }}`、`{{ node.field }}`、`{{ item.field }}` が展開されること。旧 `{{task}}` / `pass_output_from` 依存が残っていないこと。
- Routing: `when` / `switch` / `next` の排他・網羅検証、switch enum の抜け検出、cycle に到達可能な `loop_guard` が無い場合の拒否。
- Command node: `ok` / `exit_code` / `stdout` / `stderr` / `duration` の標準結果、exit code routing、stdout-JSON の Contract 検証、validation failure から fix node への route。
- Fanout: child 複数、child 1つ + `items`、child 複数 + `items` のマトリクス展開、`items` 0件、items 要素型と child `input` 型の一致検証。
- Fanout semantics: fanout child の `rules` が fanout 実行中に無視されること、子 Artifact 配列が fanout Artifact になること、旧 `aggregate` が受理されないこと。
- Reducer node: fanout 結果を command / session node で boolean / enum Artifact に畳み、通常 `rules` で分岐できること。
- Property test: 小さな Contract / rules / enum を生成し、validator が valid と判断した workflow では任意の routing 対象値に対して遷移先がちょうど1つになること。
- Execution projection: WorkflowExecution / NodeExecution / Artifact / Fanout が event log から再構築され、UI / CLI / Remote が同じ read model を読めること。
- CLI/API naming: `executions` / `execution-id` / `--node` の語彙で status / logs / approve / abort / output が動き、旧 `runs` / `run_id` / `--step` が外部 API に残っていないこと。
- Start request: `workflow start <workflow-name> "<task>"` が `request` Artifact を作り、`tasks[]` には書かないこと。
- Task: `releash task list|add|done`、`tasks[]` の `{ id, description, done }` schema、workflow から read-only、`fanout.items: tasks` 展開、workflow YAML からの書き込み不可。
- Resume: crash / stale / explicit stop 後に event log から再構築し、最後に確定した NodeExecution の次から resume できること。orphan recovery で abort / resume を typed command として選べること。
- Built-in / example: `docs/examples/full-pipeline.yml` と built-in workflow が新 schema で load でき、旧 field を含まないこと。
- Remote sync: remote workflow state sync が WorkflowExecution / NodeExecution / Artifact read model を配信し、frontend 側で domain decision を再実装していないこと。
