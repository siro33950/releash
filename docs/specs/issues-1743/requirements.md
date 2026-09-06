# Context

- Primary source は GitHub Issue #1743「fix(workspace): Sequence が Command child の Artifact を引き継ぐと Workspace node の不変条件に当たり tree 読み出しが corrupt_stored_state で全滅する」（https://github.com/siro33950/releash/issues/1743 、state: OPEN、label: bug、milestone なし、comment なし）である。Issue 外の自由文指示はない。
- 追加資料は次の現行コードである。`src-tauri/src/domain/workspace_tree/projection.rs`（`command_result_from_value` / `runtime_snapshot_nodes`）、`src-tauri/src/domain/workspace_tree/entities/mod.rs`（`node_shape_is_valid` / `WorkspaceTree::restore` / `WorkspaceTreeProjector::project`）、`src-tauri/src/domain/workspace_tree/value_objects/mod.rs`（`WorkspaceCommandResult` / `WorkspaceStructureFact::NodeArtifactProduced`）、`src-tauri/src/adaptor/gateway/workspace_tree/repository.rs`（`tree_nodes` / `workspace_tree_from_folded` / `invariant_query_error`）、`src-tauri/src/adaptor/gateway/workspace_tree/query_service.rs`、`src-tauri/src/adaptor/controller/api/error.rs`、`src-tauri/src/domain/workflow/entities/workflow_execution/mod.rs`（`complete_scope`）、`src-tauri/src/domain/workflow/services/contract_schema.rs`、`src-tauri/src/domain/workflow/services/validation.rs`、`src-tauri/src/adaptor/gateway/workflow/workflow_host.rs`（`build_command_artifact`）、および `docs/specs/issues-1729/`。
- 本作業ブランチの base は commit `2467b1d0f`「feat(workflow): Sequence の Artifact を children の統合 map にする (#1729) (#1742)」であり、#1729 は取り込み済みである。したがって Issue が観測した v0.4.10 の値の形（Sequence の Artifact が Command 結果そのもの）は、この base では成立しない。Issue が問題とするのは、node の kind を見ずに Artifact の値の形だけで Command 結果を判定する構造が残っていることである。
- Workspace ツリーの node 形状は kind ごとに固定されており、Command 実行結果（`command_result`）を持てるのは Command node だけである（`node_shape_is_valid`）。
- Workspace ツリーは event store の fact log を fold して読み側で導出する（`AGENTS.md`「永続化は event store」）。Issue は fact log 自体が健全であること、すなわち composite node（sequence / fanout）に対する `artifact_produced` イベントが1件も存在しないことを確認済みとして示している。
- 全てのアプリケーションロジックは Rust に置く（`AGENTS.md`）。
- 現在状態の確認は、Issue、上記の現行コード、既存テストの読解によって行った。ビルド・テスト・lint は実行していない。
- `docs/specs/issues-1743` は未作成であり、本 Issue に対応済みの実装は確認できなかった。

# Outcome

対象は、Releash で workflow を実行し Workspace ツリーで実行状態を観測する開発者である。

現在、Workspace ツリーの読み側は Command 実行結果を node の kind ではなく Artifact の値の形で判定する。そのため Command 以外の kind の node が Command 結果と同じ形の Artifact を持つと、その node に Command 実行結果が入り、node 形状の不変条件に反する。読み側は workspace 内の全実行木のノードを1つの木に結合してから検証するため、1ノードの違反で当該 worktree のツリー全体が `corrupt_stored_state` になり、健全な実行木も健全なノードも1つも表示されない。実行自体は継続するため、「running のまま観測できない」状態になる。

変更後、Command 実行結果は Command node にだけ現れる。Sequence / Fanout / Session の Artifact がどんな値の形であっても Workspace ツリーは読め、実行の観測が Artifact の値の形に左右されない。

# Current Behavior

## 判定の構造

`runtime_snapshot_nodes`（`src-tauri/src/domain/workspace_tree/projection.rs:126-133`）は、node の kind を見ずに Artifact の値を `command_result_from_value` に渡し、その戻り値を `NodeArtifactProduced` fact の `result` にする。

`command_result_from_value`（`projection.rs:1-10`）は `exit_code`（整数）、`duration`（非負整数）、`stdout`（文字列）、`stderr`（文字列）が揃っていれば Command 実行結果を返し、揃っていなければ返さない。

projector はこの fact を受けて `has_artifact` を真にし、`command_result` に上記の戻り値を設定する（`src-tauri/src/domain/workspace_tree/entities/mod.rs:931-945`）。

`node_shape_is_valid`（`entities/mod.rs:1031-1105`）は Workflow / Fanout / Sequence / WorkflowSession に `command_result.is_none()` を要求し、WorkflowCommand にだけ `command_result` を許す。

## 観測された事象（v0.4.10）

Issue が示す実例は次のとおりである。worktree `releash-worktrees/feat-issues-1730`、execution `424af9c3-3402-415e-b791-a109b5fea3e0`。

- Workspace ツリーの読み出し全体が `corrupt_stored_state: store corrupt (correlation_id=...)` で拒否され、UI には worktree 名とエラー行だけが残ってツリーが出ない。
- ログに `Workspace indexed record invariant failure [5a7af598-f0a8-420e-89b2-66c80171c05c]: invalid Workspace node: branch-e545aadcc3e85b81e2fabb806ef957eb` が繰り返し出る。
- 対象は Sequence ノード `impl`（node_execution_id `41ee3cdb-8da1-4dad-8fa6-28c195c94cae`）である。Issue はハッシュ再現で対象を確定済みとしている。

v0.4.10 の Sequence は `output` が指す child の Artifact をそのまま引き継ぐため、Command 結果の形が次の順に伝播した。

| ノード | kind | Artifact の top-level キー |
| --- | --- | --- |
| `initial_done` | command | `duration, exit_code, ok, outcome, reason, stderr, stdout, unresolved` |
| `initial`（`output: initial_done`） | sequence | 同上（そのまま引き継ぎ） |
| `impl`（`output: initial`） | sequence | 同上（さらに引き継ぎ） |

結果として Sequence node に Command 実行結果が入り、Sequence の node 形状に反した。

発現条件は、Sequence の `output` が Command child（または Command 結果の形の Artifact を返す child）を指し、その Sequence が完了することである。Sequence の Artifact は完了時に導出されるため、実行の途中までは表示でき、完了した時点から以後の読み出しが失敗する。

## 現行 base（`2467b1d0f`）での状態

- Sequence の Artifact は、通った children の Artifact を child 名をキーとする map である（`complete_scope`、`src-tauri/src/domain/workflow/entities/workflow_execution/mod.rs:1476-1499`）。top-level に `exit_code` は現れないため、上記の値の形では再発しない。
- Fanout の Artifact は children の実行順配列である（同 `1502-1523`）。配列に対する名前引きは成立しないため、この経路でも成立しない。
- Session の Artifact は宣言した Contract に従う。Contract の Command 予約フィールド検査（`schema_declares_command_reserved_field`、`src-tauri/src/domain/workflow/services/contract_schema.rs:276-285`）は Command node にだけ適用される（`src-tauri/src/domain/workflow/services/validation.rs:1511-1519` の `if is_command`）。したがって Session node の Contract は `exit_code` / `duration` / `stdout` / `stderr` を宣言でき、その4つを揃えた Artifact を submit して完了した Session node には Command 実行結果が入る。WorkflowSession は `command_result.is_none()` を要求するため、同じ不変条件違反が成立する。これはコード読解で確認した経路であり、実行による再現は行っていない。
- すなわち、kind を見ずに Artifact の値の形で Command 結果を判定する構造は残っており、Artifact の値の形に依存して Workspace ツリーが壊れる状態は解消していない。

## 影響範囲

- `tree_nodes`（`src-tauri/src/adaptor/gateway/workspace_tree/repository.rs:87-118`）は `runtime_snapshot_nodes` を呼び、その内部の `WorkspaceTreeProjector::project` が `validate()` を行う（`entities/mod.rs:1004`）。失敗は `invariant_query_error`（`repository.rs:283-289`）で corrupt 扱いになり、local API では `corrupt_stored_state` に写る（`src-tauri/src/adaptor/controller/api/error.rs:84-87`）。
- `workspace_tree_from_folded`（`repository.rs:120-138`）は workspace 内の全実行木のノードを集めて1つの木として復元・検証するため、1ノードの違反で workspace 全体が読めない。`load_node`、`load_node_by_node_execution_id`、`node_id_for_session` も同じ `tree_nodes` を通る。
- 読み側の入口は Tauri command と loopback local API の双方が同じ query service を経由する。

## fact log

fact log（`node_events`）は健全である。composite ノード（sequence / fanout）に対する `artifact_produced` イベントは1件も存在せず、Sequence の Artifact は fold の導出結果である。判定を kind ベースにすれば、同じ fact log から正しく再構築できる。

## 既存テスト

`projection.rs` の既存テストは、失敗した node に Command 結果の形でない Artifact を与えて Command 実行結果が付かないことを確認する（`projection.rs:326`、`projection.rs:377`）。node の kind と Artifact の値の形が食い違う場合を対象にしたテストはない。

# Scope / Non-goals

## Scope

- Workspace ツリー projection における Command 実行結果の導出を、Artifact の値の形ではなく node の kind に基づかせること。
- 非 Command node の Artifact が Command 結果と同じ形であっても Workspace ツリーが読めること。
- 上記を対象とするテスト。

## Non-goals

- Sequence / Fanout / Session の Artifact 構造そのもの。#1729 で確定した Sequence の統合 map、Fanout の実行順配列、Session の Contract に従う Artifact は変えない。
- Command node の Artifact の組み立て（`build_command_artifact`）と、Command 実行結果として保持・表示する項目。
- Session node の Contract が Command 予約フィールドを宣言できる現行規則。kind ベースの判定で不変条件違反は成立しなくなるため、宣言規則は変えない。
- 1ノードの不変条件違反で workspace 全体の読み出しが失敗する結合構造の見直し。Issue はこれを影響の説明として挙げるだけで、修正方針として示していない。
- 保存済み fact log の移行・修復。fact log は健全である。
- 不変条件違反時のエラー表示および UI の見せ方。
- Workspace ツリー以外の読み側。

# Requirements

- R-001: Workspace ツリーの node の読み出し結果に Command 実行結果（exit code、duration、stdout、stderr）が現れるのは、Command node だけである。
- R-002: Command node の Command 実行結果の導出は現行と変わらない。Artifact に `exit_code`、`duration`、`stdout`、`stderr` が揃っていればその値が結果になり、揃っていなければ結果を持たない。
- R-003: 非 Command node の Artifact が Command 実行結果と同じ形であっても、その workspace の Workspace ツリー読み出しおよび node 単体の読み出しは `corrupt_stored_state` にならず、同じ workspace の全実行木のノードが読める。
- R-004: Artifact を産出したことを表す表示は、Command 実行結果の有無と独立に、node の kind に関わらず Artifact があるとき真になる。
- R-005: 既存の fact log を書き換えることなく、同じ fact log から R-001 から R-004 を満たす Workspace ツリーを再構築できる。データ移行を必要としない。

# Assumptions / Open Questions

Assumption はない。Open Question はない。
