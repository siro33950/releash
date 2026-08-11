# ワークフローエンジン発展計画

この文書は Releash workflow engine の戦略、採用判断、実行モデル、マイルストーンを定義する一次 Owner である。語彙は [`architecture/GLOSSARY.md`](./architecture/GLOSSARY.md)、WorkflowDefinition の grammar は [`workflow-yaml-syntax.md`](./workflow-yaml-syntax.md)、実行時ライフサイクル不変条件は [`../specs/workflow-lifecycle/workflow-ideal-lifecycle.md`](../specs/workflow-lifecycle/workflow-ideal-lifecycle.md) を正とする。

## 目的

Releash は workflow を決定論的な実行レールとして扱う。開発者は WorkflowDefinition を定義し、WorkflowExecution として実行し、NodeExecution と Artifact を観測し、人間の判断点で承認・追加指示・中断・再開できる。

workflow engine は state transition の唯一の権威である。Agent / UI / CLI / API は action を要求できるが、workflow state を直接決めない。状態変更は typed command を唯一の入口として engine に届く。

```text
User / UI / CLI / API / Agent action
        |
        v  typed command
Workflow Engine（WorkflowExecution を所有）
        |
        v
NodeExecution
  - command   非対話の一回実行
  - session   agent session 実行
  - fanout    子 NodeExecution 群の展開
```

## プロダクト方針

- WorkflowExecution を第一級の状態として扱い、定義・実行・観測・承認・中断・再開を同じ workbench に置く。
- NodeExecution の kind は `command` / `session` / `fanout` の三つ。人間の承認待ちは session の `gate: approval` であり、別 kind ではない。
- `gate: approval` の session は対話式で、承認しない間は同じ session に追加指示できる。
- NodeExecution の成否と遷移は exit code と Contract 検証済み Artifact から決定する。自然言語や frontend の条件判定で state を動かさない。
- UI / CLI / API / Agent action は同じ typed command boundary と backend-owned read model を共有する。
- 起動時刻、周期、外部イベント購読は operation surface / integration 側が所有し、WorkflowDefinition grammar には含めない。
- full-retention を避け、event log、projection、summary、page、id-based operation で必要な状態だけを扱う。

## 中核モデル

### WorkflowDefinition

管理される workflow template。`name` / `description` / `builtin` / `schemas` / `nodes` を持つ。YAML schema が直接 deserialize 先となり、別 grammar からの normalization layer を持たない。

### NodeDefinition

WorkflowDefinition 内の実行単位。`command` / `session` / `fanout` の kind block をちょうど一つ持つ。`artifact` / `input` / `inputs` / `rules` は共通 field で、kind ごとの制約を load-time Diagnostic が検査する。

### WorkflowExecution

WorkflowDefinition の一回の実行。`execution_id`、status、対象 Worktree、現在 Node、起動元、時刻、失敗・中断理由を持つ。status は `running` / `waiting_approval` / `interrupted` / `completed` / `failed` / `aborted`。

### NodeExecution

NodeDefinition の一回の実行。`node_execution_id`、kind、attempt、status、session 参照、Artifact、token usage、失敗情報を持つ。同名 Node が loop や fanout で複数実行されても実行個体を一意に扱える。

### Fanout

親 NodeExecution と子 NodeExecution 群を束ねる derived view。

- `child` は通常の command / session NodeDefinition を一つまたは複数参照する。
- `items` は literal 配列または Artifact 配列 field。child × items を展開する。
- 各 item は child の単一 `input` に入り、型一致を load 時に検証する。
- child の rules は fanout 実行中に評価しない。
- 空 items は子を起動せず、空配列 Artifact で完了する。
- fanout の Artifact は子 Artifact の配列。配列を畳む場合は後続の通常 Node を使う。
- fanout 固有の failure policy は持たない。resume は完了済み child Artifact を再利用し、未確定 child だけを再実行する。

### Artifact / Contract

Artifact は WorkflowExecution / NodeExecution 間で生成・参照される検証済みデータで、独立した lifecycle state を持たない。Contract は Artifact validation の補助語彙である。

- `schemas` は限定した JSON Schema subset。
- routing field は required boolean または required string enum。
- `request` は起動時入力由来の String scalar Artifact。
- command は `ok` / `exit_code` / `stdout` / `stderr` / `duration` の予約 field と stdout-JSON の Contract field を単一 Artifact 名前空間に合成する。
- session と CLI / API submit は同じ Contract engine を使い、session は検証済み提出まで完了しない。

### Diagnostic

WorkflowDefinition / NodeDefinition の parse、shape、resolve、typecheck、control-flow の validation result。lifecycle state ではない。Rust backend が code / stage / span / message を返し、frontend は表示だけを担当する。

## 状態変更と event log

状態変更は次の typed command usecase に集約する。

- start
- approve
- Artifact submit
- abort
- stop
- resume

engine は状態遷移を append-only event log に記録する。event log は永続化・projection・観測の adapter 語彙であり、domain entity ではない。現在状態は event replay から構築した WorkflowExecution / NodeExecution read model として読む。

`interrupted` は再開可能な checkpoint である。resume は最後に確定した NodeExecution までを replay し、未確定 Node を新しい attempt として実行する。session は再アタッチせず新しく開始する。

## 採用するもの

| 項目 | 方針 |
| --- | --- |
| Node kind | `command` / `session` / `fanout` の kind block。完了 gate は session 内に置く。 |
| deterministic validation | test / lint / validation を command として実行し、標準結果と typed Artifact で routing する。 |
| human checkpoint | `gate: approval` の session で止まり、Artifact を見て承認または追加指示する。 |
| Fanout | 通常 Node 参照と items matrix。child も NodeExecution として観測する。 |
| typed Artifact | 全 kind と外部 submit が同じ Contract engine を使う。 |
| typed routing | `when` / `switch` / `next` / `loop_guard` を load 時に排他・網羅・型検査する。 |
| shared operation surface | UI / CLI / local API が同じ usecase と read model を使う。 |
| resume | interrupted checkpoint と event log から確定済み結果を再利用する。 |

## 採用しないもの

| 項目 | 理由 |
| --- | --- |
| workflow 起動時の Worktree 自動生成 | Releash は Workspace / Worktree を選んでから起動する。 |
| 自然文による workflow router | 起動対象は operation surface から明示する。 |
| PR / Issue の直接 lifecycle 所有 | 外部 system の操作は workflow command / Artifact として扱う。 |
| Workflow marketplace | curated built-in に絞る。 |
| per-node MCP 設定 | 現行 boundary には不要。 |
| graph を state owner にする UI | timeline と NodeExecution 詳細を正とし、表示が state を所有しない。 |
| Releash core の Task Entity / global task input | task 的な値はユーザー定義 Artifact field として表現する。 |
| routing 式言語 | 比較・計算・集約は通常 Node で boolean / enum Artifact に畳む。 |

## CLI / Local API

CLI の更新操作は Tauri アプリ内の token-authenticated localhost API を介して、UI と同じ usecase を呼ぶ。read-only query はアプリ未起動時に backend-owned read model を直接読む fallback を許可する。

```sh
releash workflow status <execution-id>
releash workflow output submit <execution-id> --node <node-name> [--node-execution <id>] --type <contract> --json '<json>'
releash workflow output get <execution-id> --node <node-name>
```

fanout で同名 NodeExecution が複数 active な場合は `node_execution_id` が必要。session 内の CLI は engine が注入する実行 ID を既定値に使う。

## UI 方針

Workflow panel は次を backend read model から表示する。

- active WorkflowExecution summary と execution 履歴
- event timeline
- NodeExecution 詳細、fanout group、参照先AgentSessionのTerminal Surface
- approval、abort、stop、resume action
- logs と Artifact

Automation editor は YAML 直接編集と Rust Diagnostic 表示を提供する。frontend に grammar validator、routing 判断、resume 判断を実装しない。

main agent は user-facing narrator として進捗・承認依頼・失敗 summary を伝えるが、state transition は所有しない。

## マイルストーン

### Workflow Engine 新モデル移行（完了）

milestone 82 は表現単位の縦切りで実施し、2026-07-15 の最終 cleanup / 文法正本化まで完了した。撤回された Task Entity 案は実装対象に含めていない。

| wave | issue | 完了結果 |
| --- | --- | --- |
| 1: grammar foundation | #1322 / #1325 / #1326 / #1327 / #1323 | kind block、Contract / Artifact、参照、rules、Diagnostic pipeline |
| 2: runtime expression | #1328 / #1324 / #1329 / #1330 | command、session approval gate、fanout、通常 Node reducer |
| 3: state / command boundary | #1331 / #1332 | WorkflowExecution / NodeExecution read model、local API / CLI typed command |
| 4: resume | #1335 | interrupted、stop / resume、partial fanout checkpoint |
| 5: final canon | #1337 | 残存互換経路の掃討、built-in / example の整合、文法正本化 |

完了後の不変条件:

- built-in workflow と [`examples/full-pipeline.yml`](./examples/full-pipeline.yml) は正本文法だけで load / 実行できる。
- YAML loader、event log、workflow state、外部 API に廃止形式の reader / converter / feature flag を持たない。
- Tauri / CLI / API / Agent action は同じ backend-owned WorkflowExecution state と typed command boundary を使う。
- Automation editor と Workflow panel は backend の Diagnostic / read model を表示し、domain behavior を持たない。
- grammar、load-time validation、runtime behavior、execution trigger の責務は分離されている。

## テスト方針

新しい behavior test と、受理してはならない shape の regression test を対にする。新ロジックは Rust の domain / usecase / adaptor test を主とし、frontend test は表示・操作・invoke 境界に限定する。

継続する品質ゲート:

- YAML fixture: valid / invalid を分け、Diagnostic code と stage を固定する。
- parse / shape: YAML syntax、unknown field、kind / rule shape、kind 別 field。
- resolve: Node / Contract / Artifact path、`request` / `item` scope、fanout child。
- typecheck: Contract subset、routing field、command 予約 field、fanout items / child input。
- control-flow: 終端、到達性、排他、網羅、cycle / loop guard、fanout child leaf。
- command: 標準結果、stdout-JSON、Contract violation、missing-field routing、cancellation。
- session: gate、Artifact 提出、approval、追加指示、stale target。
- fanout: child / items matrix、空 items、child rules 無視、配列 Artifact、partial resume。
- projection: event replay から WorkflowExecution / NodeExecution / Artifact / Fanout を再構築する。
- operation surface: UI / CLI / API が同じ command usecase と read model を使う。
- built-in / example: 全定義が Diagnostic ゼロで load でき、代表的な全遷移を engine test で実行できる。
- property test: valid と判定した rules は任意の routing 値に対して遷移先がちょうど一つになる。

CI と同じ `cargo fmt --check` / `cargo clippy -- -D warnings` / `cargo test`、`pnpm lint` / `pnpm test` / `pnpm build` / integration test を通す。
