# Milestone 82「Workflow Engine 新モデル移行」実装計画

https://github.com/siro33950/releash/milestone/82

## 1. Context

現行 workflow engine は旧表現（`type: agent/bash/approval/parallel`、`output_contract`/`input_contracts`、`pass_output_from`/`pass_previous_response`/`workflow_variables`、`parallel_children`、`aggregate`/`all_match`/`any_match`、regex `rules.match`、`cycle_guard`、`run_id`/`WorkflowRun`/step 語彙）で実装されている。これを `command`/`session`/`fanout` の kind block、Contract 検証済み Artifact（`schemas:`/`artifact:`/`input:`/`inputs:`）、順序非依存 rules（`when`/`switch`/`next`/`loop_guard`）、WorkflowExecution/NodeExecution 主語の read model、typed command boundary、resume へ移行する。

- 実装対象: 14 issues（#1322 #1323 #1324 #1325 #1326 #1327 #1328 #1329 #1330 #1331 #1332 #1335 #1337 #1454）。**#1333 は撤回済み（CLOSED）**: Task Entity / WorkflowExecution-owned `tasks[]` / `releash task ...` は実装しない。#1454 は Goal 1〜13 完了後に追加された Workspace UI の follow-up であり、完了済み goal を再実装しない。
- 仕様の正: `docs/workflow-engine-evolution-plan.md`（戦略）、`docs/workflow-yaml-syntax.md`（構文）、`docs/examples/full-pipeline.yml`（完成形例・整合確認用）、`docs/architecture/GLOSSARY.md`（語彙）。
- 実装は Codex /goal で行う。1 issue = 1 goal = 1 PR、wave 順に逐次実行する。

## 2. 確定済み設計判断（D1〜D7: 2026-07-07、D8: 2026-07-16）

| # | 論点 | 決定 |
|---|---|---|
| D1 | #1332 の local API | **最小 local API を新設**。Tauri アプリ内に localhost バインドの API サーバ（認証トークン付き）を立て、CLI の start/executions/status/logs/approve/abort/output をこれ経由に移行。file-direct/pending file は必要最小の adapter に縮退。#77-79 のデーモン化の土台。 |
| D2 | `schemas:` の Contract dialect | **JSON Schema subset を自前実装**。`type`/`properties`/`required`/`items`/`enum`/`additionalProperties` に限定。routing 参照 field は required かつ boolean/enum を load 時 Diagnostic で強制。`request` 用に scalar（string）Contract を許可。配列要素型は inline 不可、名前付き Contract 参照（`items: <名前>`）。 |
| D3 | command 標準結果と Artifact | **単一名前空間＋予約 field**。command node の Artifact は常に予約 field `ok`/`exit_code`/`stdout`/`stderr`/`duration` を持ち、`artifact:` Contract の field はそれに合成される。Contract が予約名を宣言したら load 時 Diagnostic。rules の `on:` は `ok` も Contract field も同じ規則で参照。 |
| D4 | Codex /goal 分割 | **issue 単位で 14 goal**。wave 順（§5）に 1 goal = 1 PR で逐次実行。 |
| D5 | session `permission` | **現行 3 値（ask/edit/full）のまま**。docs（full-pipeline.yml / syntax doc）の `permission: read` は #1337 の正本化で修正する。 |
| D6 | 旧 template 変数 | **全廃し Artifact 参照に統一**。`{{ project_name }}`/`{{ path_alias.* }}`/`{{ vars.* }}`/`{{ task }}`/workflow の `variables:` セクションを廃止（#1326）。built-in の spec_dir 連携（contract 出力からの抽出ハードコード）は「authoring が spec_dir field を持つ Artifact を産出 → 後続が `inputs:` + `{{ <node>.spec_dir }}` で参照」に書き換え。 |
| D7 | Automation 編集 UI | **YAML 直接編集 + Diagnostic 表示に簡素化**。StepEditor/WorkflowEditor のフォーム編集を廃止し、Monaco での YAML 編集 + Rust が返す Diagnostic のインライン表示に寄せる（#1322 で置換、#1323 で code/span 表示化）。 |
| D8 | Workspace 観測 UI | **Node 中心の再帰ツリー + 単一 NodeContentView**。NewSession は Workflow 非所属の単独 Node、Workflow と Fanout は Node を束ねる branch、Node content は Session または Command とする。Workflow/Fanout は独自の中央 view を持たない。backend の NodeExecution は保持し、Rust が実行occurrenceをevent projectionの実行順どおりWorkspace UI専用read modelへ投影する。同じ定義Nodeの反復も実行ごとに別行とする（#1454）。 |

## 3. 計画側の決定事項（P1〜P15）

- **P1 (#1322/#1324 境界)**: #1322 で `session.gate`（auto/approval、省略時 auto）を構文導入し、旧 `type: approval` の受け皿にする。gate 必須化・approve typed command・reject/rerun 削除の完遂は #1324。
- **P2 (status 語彙)**: WorkflowExecution.status は現行 running/waiting_approval/completed/failed/aborted を維持（waiting_approval は gate: approval 待ちの derived 状態）。#1335 で interrupted（再開可能）を追加。
- **P3 (fanout child rules)**: fanout 実行中は child の `rules` を無視する（issue #1329 が正。syntax doc「検討事項」の Diagnostic 案は採らない）。
- **P4 (NDJSON 在庫)**: event log / state の schema 変更は各 issue で破壊的に実施し、互換 reader を書かない（在庫破棄前提。milestone 方針どおり）。
- **P5 (fanout 部分失敗の resume)**: 完了済み child の Artifact は再利用し、未確定 child のみ再実行する（#1335）。
- **P6 (予約名)**: 予約 Artifact 名は `request` / `item` のみ。`tasks` は予約しない（#1333 撤回に従う）。
- **P7 (存続する機構)**: stall observation / RetryPolicy / structured output repair は挙動を維持し語彙のみ新モデルに追随。ContractRepair は #1325 の schemas 機構に接続。
- **P8 (issue 未割当の旧表現の引き取り先)**: `inline_prompt` → #1322 で削除。`collect`/`ReduceStrategy`/`OutputCollected` event → #1330 で削除。`variables:`/`{{vars.*}}`/`{{project_name}}`/`{{path_alias.*}}`/`{{task}}`/contract.rs の spec-directory ハードコード → #1326 で削除。`resets_cycle_for` → #1327 で削除。reject 系一式 → #1324 で削除。pending file 経路 → #1332 で撤去（mutation は API 必須、read-only の file-direct fallback のみ最小残置）。
- **P9 (Diagnostic span)**: 二段 parse で取得する。serde_saphyr の typed load に加え、saphyr AST から YAML path → span map を構築（#1323 冒頭で spike。取得不能な階層は近傍 node の span に fallback）。
- **P10 (仕様の正本)**: issue 個別の spec は作らず、milestone 全体の詳細設計 `design.md`（型定義・検証規則・event 語彙・API・削除一覧）を実装の正本とする。
- **P11 (missing-field routing 意味論)**: `when`/`switch` の `on:` が参照する field が実行時に不在（command の artifact validation 失敗等）の場合は no-match とし、catch-all `next` に落ちる。網羅検証は「artifact 検証が失敗しうる node が Contract field を rules で参照する場合、`next` catch-all 必須」を要求する。command の `ok` は `exit_code==0 && (artifact 未指定 || validation 成功)` の合成とする（D3 と併せて #1327/#1328 で実装、#1337 で文書化）。
- **P12 (workflow start の名前解決)**: `releash workflow start <workflow-name>` は WorkflowDefinition.name で解決する（name↔file の対応は loader が所有、名前重複は Diagnostic）。request 未指定は空文字列の request Artifact とする。
- **P13 (session の artifact 提出)**: `session` + `artifact:` は Contract 検証済み提出まで node 完了しない（現行 repair 機構を踏襲、max_attempts 超過で失敗）。よって session node は完了時 artifact 存在が保証され、P11 の missing-field は実質 command node のみ。
- **P14 (NodeExecution のアドレスと fanout leaf、設計検証で追加)**: fanout child は同名 NodeExecution が並走するため、engine 採番の `node_execution_id` を第一級識別子とし、approve / output submit は並走時にこれでアドレスする（session への env `RELEASH_NODE_EXECUTION_ID` 注入で agent 側は通常意識しない）。fanout child は leaf 専用（通常遷移の対象・entry になれない、fanout の入れ子不可 = WFC006）。child の Artifact は親 fanout の配列にのみ格納し node 名 map に載せない。明示 stop は typed command（`StopExecution`）として #1335 で追加。`additionalProperties` の既定は JSON Schema と同じ true。詳細は design.md §5/§6 R7/§8.5。
- **P15 (domain read model と Workspace UI projection の分離)**: WorkflowExecution / NodeExecution / Fanout は engine・event log・CLI/API の正規 read modelとして維持する。Rust が `Node | Workflow | Fanout` の再帰 tree summary と選択 Node detail に投影し、NodeExecutionの発生順を実行occurrenceの並びとして保持する。同じ定義Nodeのretry/loopもoccurrenceごとに別行とし、各行には後続occurrenceの追加で変化しないopaque IDを割り当てる。attempt / fanout 座標 / 内部 ID は UI に露出しない（#1454）。

## 3.5 実装原則（最優先）

出来上がる実装は「最もシンプルで、型の表現力で正しさを保証する」ものにする。詳細は `goal-common.md` の「実装原則」節（全 goal に適用）。要点: (1) 不正状態を型で表現不能に（kind / rule は enum）、(2) パッチではなく置換・旧構造の改名延命禁止、(3) 互換層・feature flag・新旧併存禁止、(4) YAGNI・予約構文禁止、(5) 不要になったコードは同 PR で削除、(6) validation / Contract 検証は一箇所に集約、(7) engine monolith を肥大させず kind 単位の実行経路に切り出す。

## 4. 現状実装マップ（調査結果）

workflow 関連 Rust は 128 ファイル・約 7 万行。clean architecture 移行済み（`docs/workflow-engine-model-boundary.md` の `src-tauri/src/workflow/` 記述は stale）。

| 領域 | 場所 |
|---|---|
| YAML schema | `src-tauri/src/adaptor/gateway/workflow/schema.rs`（Workflow/NodeDefinition/ChildNodeDefinition/NodeType/ParallelAggregate/TransitionRule/CycleGuard/CollectConfig/ResolvedFacets） |
| domain 定義 | `src-tauri/src/domain/workflow/value_objects/definition.rs`（schema の鏡像）、`domain_mapping.rs` が変換 |
| validation | `src-tauri/src/domain/workflow/services/validation.rs`（2,643 行、ValidationError 30+ variant） |
| Diagnostic | `src-tauri/src/adaptor/gateway/workflow/diagnostics.rs`（1,684 行。severity/message のみ、**code/span なし**） |
| loader | `storage.rs`（serde_saphyr）、`definition_repository.rs`、`builtin.rs`（include_str! で 12 YAML 同梱）、`facet.rs` + `builtin_facets/`（policies/knowledge/instructions/contracts の markdown） |
| Contract | `domain/workflow/services/contract.rs`。**JSON Schema ではなく** markdown facet + ` ```contract-validation``` ` JSON メタブロック方式。`spec-directory` 名のハードコード抽出（52-67 行）が built-in の authoring→implement 連携を支える |
| runtime | `adaptor/gateway/workflow/runtime_engine_impl.rs`（4,820 行 + tests.rs 12,136 行）、parallel_runtime.rs、approval_runtime.rs、orphan_recovery.rs（**abort-only**）、runtime_command_gateway.rs |
| 遷移 | `domain/workflow/services/transition.rs`（regex 評価）、parallel.rs（aggregate/reduce）、approval_rules.rs（reject 検証）、variable_renderer.rs（{{task}}/{{vars}} 等）、failure_policy.rs |
| event log | `event.rs`（WorkflowEvent 約 20 variant、run_id 主語。RunStarted は workflow_definition を丸ごと埋め込み task を持たない）、`log.rs`（`workflow_logs/{run_id}.ndjson` append-only）、`event_projection.rs`（3,284 行、event 列→WorkflowState 再構築）、`run.rs`（`workflow_runs/{run_id}.json` メタデータ） |
| read model | `domain/workflow/value_objects/state.rs`+`step_output.rs`（WorkflowStateSnapshot/StepHistoryEntry/StepOutput/ParallelStepState）、`adaptor/protocol/workflow.rs`（WorkflowStateView: current_step_name/step_history/step_outputs/workflow_variables）、presenter/workflow.rs |
| Tauri command | `adaptor/controller/command/workflow/`（29 command: start_workflow/abort_workflow/approve_workflow_step/list_workflow_runs/get_workflow_run*/get_workspace_workflow_step_detail/restore・archive_workspace_workflow_run 等） |
| CLI | `src-tauri/src/cli/`（mod.rs / workflow.rs 1,509 行 / output.rs / workflow_io.rs）。**local API なし**: read は file-direct、mutation は pending file（`workflow_pending/`、`pending_command.rs`、TTL 24h）。`--step` 語彙、Reject subcommand あり。CLI help は agent の system prompt に注入され、contract.rs の repair prompt が旧 CLI 語彙を agent に指示 |
| frontend | `src/types/workflow.ts`（NodeType 等の旧語彙全部）、`src/components/panels/automation/`（WorkflowList/WorkflowEditor/StepEditor/WorkflowDetail のフォーム編集）、`WorkflowView/WorkflowView.tsx`（approve/reject ボタン）、hooks（useAutomation/useWorkflowState/useWorkspaceWorkflowStepDetail）、`WorkspaceList.tsx`、`workspace-tree.ts` |
| 結合部 | `usecase/workflow/workspace_tree.rs`（1,400 行超）、`usecase/agent_session/status.rs`（2,270 行、workflow status 集約と結合） |

**新語彙（fanout/loop_guard/gate/schemas/NodeKind）のコードは 0 件**。完全な置換移行であり、新旧併存フラグは作らない。

## 5. 実行順序（wave）と goal 対応

```
wave 1（構文基盤・加算的）   : Goal 1 #1322 → Goal 2 #1325 → Goal 3 #1326 → Goal 4 #1327 → Goal 5 #1323
wave 2（表現単位の移行）     : Goal 6 #1328 → Goal 7 #1324 → Goal 8 #1329 → Goal 9 #1330
wave 3（state / command 境界）: Goal 10 #1331 → Goal 11 #1332
wave 4（resume）            : Goal 12 #1335
wave 5（最終 cleanup・正本化）: Goal 13 #1337
wave 6（Workspace UI 再設計） : Goal 14 #1454
```

各 issue は「新語彙の実装 → runtime/projection/CLI・API/UI/built-in/tests の移行 → 対応する旧語彙の削除または拒否」までを同一 goal で完了する。まだ移行していない別表現は巻き込まない（例: #1322 の fanout block は暫定的に既存 parallel_children/aggregate を内包し、中身の置換は #1329/#1330）。

Goal 14 は完了済み Goal 1〜13 の個別仕様を変更しない。確定済み domain/event modelを入力として、Workspace UI の投影と表示境界だけを置き換える。

## 6. リスクと注意点

1. **engine monolith**: runtime_engine_impl.rs（4,820 行）+ tests.rs（12,136 行）を全 goal が触る。1 表現の変更が schema → domain → mapper → usecase DTO → presenter/protocol → frontend 型の 6 層を縦断する。
2. **NDJSON/state 破壊**: schema 型を変える goal ごとに既存実行在庫が deserialize 不能になる（P4 で破棄前提と決定済み。互換 reader を書かないこと）。
3. **built-in + facet の書き換え量**: built-in 12 本すべてが旧語彙。instructions facet（markdown 本文）にも旧 CLI 語彙（`<workflow_output>` / `--step` 等）が含まれる可能性が高く、#1325（contract → schemas）、#1327（regex verdict → enum Artifact）、#1332（CLI 語彙）で本文レベルの書き換えが要る。
4. **span spike**: serde_saphyr で span が取れるかは未確認。#1323 冒頭で必ず spike する（P9）。
5. **full-pipeline.yml 自体の不整合**: (a) `fix_one` の `artifact: fix_result` が `schemas:` 未宣言、(b) `permission: read` が現行値に無い（D5 で docs 修正と決定）、(c) **routing 参照 field（`lgtm` / `all_lgtm` / `has_open` / `verdict`）がどの Contract でも `required` 宣言されておらず、D2 の「routing field は required」規則で typecheck を通らない**。syntax doc の Contract 節にも required 要件の明記が無い。いずれも #1337 で正本例・syntax doc ごと修正する（design.md §14）。それ以前の goal（#1323/#1327）が fixture を作る際は required を宣言した形で書くこと。
6. **旧 north star doc**: `docs/workflow-engine-model-boundary.md` は WorkflowRun/type: 語彙・存在しないパスで書かれた stale 文書。#1337 で改訂または削除を提案する。

## 7. Codex /goal の実行手順

1. `goal-common.md`（全 goal 共通の前提・規約）と、実行する `goal-NN-issue-XXXX.md` を objective として Codex /goal に渡す（各 goal ファイルは冒頭で goal-common.md を読むよう指示している）。
2. wave 順に 1 goal ずつ実行し、PR レビュー・マージ後に次の goal へ進む。
3. 前の goal で削除済みの旧語彙が後続 goal に「削除対象」として再掲されている場合は、残存確認（grep）だけ行えばよい。

| Goal | Issue | ファイル |
|---|---|---|
| 1 | #1322 NodeDefinition kind block 移行 | `goal-01-issue-1322.md` |
| 2 | #1325 Contract / Artifact を schemas と artifact に統一 | `goal-02-issue-1325.md` |
| 3 | #1326 Artifact 入力と参照規約 | `goal-03-issue-1326.md` |
| 4 | #1327 rules を順序非依存 Diagnostic に移行 | `goal-04-issue-1327.md` |
| 5 | #1323 文法健全性 / Diagnostic front-end | `goal-05-issue-1323.md` |
| 6 | #1328 bash を command node に移行 | `goal-06-issue-1328.md` |
| 7 | #1324 approval node を session.gate=approval に移行 | `goal-07-issue-1324.md` |
| 8 | #1329 parallel_children を fanout.child に移行 | `goal-08-issue-1329.md` |
| 9 | #1330 aggregate 廃止と reducer node 移行 | `goal-09-issue-1330.md` |
| 10 | #1331 WorkflowExecution / NodeExecution read model | `goal-10-issue-1331.md` |
| 11 | #1332 CLI/API command boundary 新語彙化 | `goal-11-issue-1332.md` |
| 12 | #1335 Resume を abort-only recovery から移行 | `goal-12-issue-1335.md` |
| 13 | #1337 Workflow YAML 文法の最終 cleanup と正本化 | `goal-13-issue-1337.md` |
| 14 | #1454 Workspace UIをNode中心の再帰ツリーに統一 | `goal-14-issue-1454.md` |
