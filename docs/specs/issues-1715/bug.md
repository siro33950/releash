# 周回再入した Sequence の終端評価が output child の Artifact を間欠的に見失い、成功した実行に偽の failed が付く

## 現象

外側ループで Sequence を再入する workflow（dev-cycle の review ⇄ fix）で、再入した Sequence が間欠的に次の validation_failure を付けて failed になる。

```
sequence 'review' reached its terminal without an artifact from output child 'make_plan'
```

実際には output child の Artifact は存在し、下流へも供給されている。

- execution `e061ad68-2590-4970-a3fd-1f85ad1c9a35`: `review` attempt=2（node_execution `87a713c5`）が上記 reason で failed。子は `make_plan`（`103ce226`）まで全て succeeded で、その `artifact_produced` は fact log seq 451 に `parentId=87a713c5` で実在する。後続の `judge(2)` / `fix(2)` は round 2 の `make_plan` の内容を入力として開始しており、その開始は `review(2)` の failed 確定より先行している。
- execution `f00637c0-f0da-46d8-a7d6-e77457c39368`: `review` は attempt 1・2・4 が succeeded、attempt 3 だけが同一 reason で failed。実行自体は completed に達した。

実行は継続するため機能は失われない。観測される実害は、実行木に残る偽の failed と、当該 Sequence に立つ誤った `canRetry=true` の 2 点。

## 原因

Artifact 付き Submit より先に Stop が記録された Sequence の output child を fact log から再生するときは、`artifact_produced` を親 scope へ反映してから二信号成立による終端評価を行うべきところ、`submit_received` を適用した時点で終端評価しており、直後の `artifact_produced` を読む前に Sequence を validation_failure にしている。

所在:

- `src-tauri/src/usecase/workflow/control_plane.rs:281-341`
- `src-tauri/src/adaptor/gateway/workflow/fact_log.rs:187-192, 259-277, 320-340`
- `src-tauri/src/domain/workflow/services/fact_replay.rs:71-73, 290-311`
- `src-tauri/src/domain/workflow/entities/workflow_execution/mod.rs:1470-1517, 1709-1803, 2278-2408, 3134-3149`

証拠:

1. live の submit 経路は、Submit signal を記録して `NodeSubmitReceived` を push した後に `apply_submitted_output` で Artifact を scope へ格納し、その後で completion handshake を行う（`control_plane.rs:281-341`）。live state では output の Artifact を読めており、後続 Node を開始できる。一方、永続化する events の並びは `NodeSubmitReceived` → `ArtifactProduced` になる。
2. `fact_log` はこの順で `submit_received` / `artifact_produced` に写像し（`fact_log.rs:187-192, 259-277`）、store は配列順に seq を採番し（`local_event_store/commit.rs:888-892`）、read は `ORDER BY seq`（`local_event_store/node_events.rs:102-111`）、fold は逐次適用する（`fact_replay.rs:71-73`）。
3. Stop が既に適用済みだと、fold は `submit_received` を読んだ時点で `derive_session_settlement` を呼び、handshake が Ready なので `apply_leaf_completion` へ進む（`fact_replay.rs:290-297`、`mod.rs:3134-3149`）。この時点で `artifact_produced` は未適用のため、親 scope へ Artifact なしで記録して終端評価に入り（`mod.rs:1709-1803`）、`complete_scope` が output child の Artifact 不在として当該 validation_failure を付け、scope を `runtime.scopes` から除去する（`mod.rs:1470-1517`）。
4. 続く `artifact_produced` は `replay_artifact_produced` → `record_pending_result` に入り、子の `NodeExecution.artifact` は設定されるが、除去済みの親 scope を引けず `NotApplicable` になる（`mod.rs:2278-2317, 2365-2408`）。「子は succeeded で Artifact も見えるが、親 Sequence は failed」が決定論的に成立する。
5. 合成子の failed は事実として記録されない（`fact_log.rs:320-340`）。この偽の failed は永続化された事実ではなく、読むたびに fold が導出している。live 側は正しく前進しているため、後続 Node が先に開始し後から親だけ赤くなるという現象の時系列と整合する。
6. Stop が Submit より後着する周回では `artifact_produced` が先に適用されるため正常になる。間欠性も同じ条件で説明できる。

## 棄却した仮説

- 周回再入した同名 Sequence が前周回の scope と identity 衝突し、別インスタンスの artifacts を参照している: `ExecutionParentRef.parent_id` は親合成子の `node_execution_id` を直接保持し、ループ中の同名 Node でも一意になる（`node_execution.rs:28-36`）。`replay_node_started` も合成子の開始ごとに `node_execution_id` をキーとする fresh な `SequenceScopeRuntime::default` を push する（`mod.rs:3482-3494, 3608-3638`）。`complete_scope` は渡された `scope_id` でその実体を取得するため、周回間の scope 共有は確認できなかった。
- `complete_scope` と下流入力束縛が異なる artifact store を読み、終端評価だけ誤った store を参照している: 下流の `scope_resolution_space` も routing も `complete_scope` も、同一の `sequence.artifacts` を読む（`mod.rs:1300-1317, 1383-1393, 1472-1479`）。差は store ではなく、fold が `artifact_produced` 適用前に settlement と scope 除去を行う時点にある。
- submit と Stop の live mutation が mutex 外で競合して runtime scope を破壊している: control-plane commit は `executions` mutex 下で current を取得し、`PreparedWorkflowTransaction` の stale candidate 検査を通して永続化する（`workflow_host.rs:1054-1113`）。別の required-event 経路も append 完了まで同 mutex を保持する（同 `:2125-2165`）。到着順は直列化されており、その有効な順序の一方を fold が誤解釈することが問題で、未直列化 mutation は原因ではない。
- Issue #1702 / #1703 の Workspace node id 修正が workflow completion を回帰させた: commit `092d3fb09` の変更対象は `docs/specs/issues-1702` と workspace_tree の domain / gateway / query test のみで、`workflow_execution` / `control_plane` / `fact_replay` に変更はない。本件は workflow fact fold 内の `complete_scope` が導出しており、Workspace projection の id 導出とは別経路である。

## 再現

`src-tauri/src/domain/workflow/services/fact_replay_test.rs::test_sequence_stop後のartifact付きsubmitをoutput子の成果として完了導出する`

## 期待

- E-001: Artifact 付き Submit より先に Stop が記録された output child を持つ Sequence は、その Artifact を自身の成果として完了する。`sequence '<name>' reached its terminal without an artifact from output child '<child>'` の validation_failure は付かない。
- E-002: 既に永続化済みの fact log を読み直したときも E-001 が成立する。再実行やデータ移行を要しない。
- E-003: 親 scope が完了する前に到着した Artifact は、その Node 自身の完了より後に到着した場合も、Node の成果として親 scope に反映され、下流の入力束縛と Sequence の output 解決が同じ値を見る。
- E-004: output child が Artifact を出さないまま Sequence が終端へ到達した場合は、従来どおり当該 validation_failure で失敗する。

## 修正方針

- P-001: 修正は fact log の読み手（fold）側に置く。永続化する events の順序と live 経路の submit 処理は変えない。
- P-002: fold は、親 scope が完了する前に到着した Artifact を、その Node の成果として親 scope に反映する。二信号成立による決着が親 scope の完了へ進む前に、同一提出の Artifact を反映しなければならない。
- P-003: 完了済み Node への再提出を live で受け付ける変更は本修正に含めない（#1720）。
