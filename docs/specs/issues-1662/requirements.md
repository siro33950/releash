# Context

## 入力文書

- 正本: GitHub Issue #1662「[workspace tree] workflow 実行木のルート行が常に node 名 main で表示され workflow 名が出ない」 <https://github.com/siro33950/releash/issues/1662>（label: `bug`、state: OPEN）
- 参照した規約: `docs/glossary/WORKFLOW.md`、`AGENTS.md`
- 参照した実装: `src-tauri/src/domain/workspace_tree/`（`entities/mod.rs`、`services.rs`、`projection.rs`）、`src-tauri/src/adaptor/gateway/workspace_tree/`（`query_service.rs`、`repository.rs`）、`src-tauri/src/domain/workflow/value_objects/node_fact.rs`、`src-tauri/src/adaptor/controller/command/workspace_tree.rs`、`src/components/workspace/WorkspaceList.tsx`、`src/types/workspace-tree.ts`、`workflows/*.yml`
- 配置先: `docs/specs/issues-1662`

## 確定済みの背景

- workflow 定義の root Node 名は定義構文の規約で `main` に固定されている（`docs/glossary/WORKFLOW.md:40`「root は `nodes.main` という規約で決まる」）。リポジトリ同梱の builtin workflow 8本はすべて `nodes.main` を持つ（`workflows/01_author-spec.yml:112` ほか）。したがって workflow 実行の実行木の root Node 名は常に `main` である。
- workflow 名は Workflow ノード（実行木の owner）が保持する。値は workflow 定義の `name` であり（`src-tauri/src/adaptor/gateway/workspace_tree/repository.rs:103`）、Workspace ツリーへは owner の表示名として入る（生成 `src-tauri/src/domain/workspace_tree/entities/mod.rs:282`、更新 `同:747`）。
- 実行木の子ノードの表示名は Node 名である（`src-tauri/src/domain/workspace_tree/entities/mod.rs:475`）。root Sequence の表示名はここで `main` になる。
- UI に「workflow 実行の行」として現れるのは Workflow ノードそのものではなく public root、すなわち owner の最初の可視直下子である（`src-tauri/src/domain/workspace_tree/services.rs:49` の `from_owner`）。read model の投影で Workflow ノードは自身を DTO 化せず子を展開するだけであり（`src-tauri/src/adaptor/gateway/workspace_tree/query_service.rs:380`）、workflow 名を持つノードは UI に届かない。
- public root の行には execution 単位の識別子（execution_id）と workflow 操作の可否（停止 / 中止 / 再開 / アーカイブ）が付与される（`src-tauri/src/adaptor/gateway/workspace_tree/query_service.rs:342-365`）。行の実体は execution 単位である。
- 単独 AgentSession も同じ実行木として表現される。その合成 workflow 定義は `name` と唯一の Node 名がともに `session` である（`src-tauri/src/domain/workflow/value_objects/node_fact.rs:155-192`）。owner の表示名と public root の表示名はどちらも `session` で一致する。
- frontend は read model から受け取った表示名をそのまま描画する。加工は行わない（`src/components/workspace/WorkspaceList.tsx:408`、型定義 `src/types/workspace-tree.ts`）。
- `AGENTS.md`「Rust がロジックを所有する」により、frontend に許されるのは表示とレイアウト制御、入力受付、`invoke` 呼び出し、表示用フォーマットのみである。表示名の決定はアプリケーションロジックであり Rust が持つ。
- Workspace ツリーの read model は現状 Tauri command `list_workspace_worktree_nodes` と `get_workspace_tree_selection_reconciliation`（`src-tauri/src/adaptor/controller/command/workspace_tree.rs`）からのみ公開されている。loopback local API には Workspace ツリーの route が存在しない。

## Issue 記載と現行コードの差異

Issue が根拠として挙げる `domain/workflow/services/fact_replay.rs:203` の `standalone_session_definition` は、現行の main（`8e7612f83`）に存在しない。単独 AgentSession の合成定義に相当する現行の実装位置は `src-tauri/src/domain/workflow/value_objects/node_fact.rs:155-192` である。Issue 中の他の行番号も現行コードとは数行ずれている。本文書では現行コードで確認した位置を記載する。Issue が述べる事実関係そのもの（root Node 名が `main` 固定であること、public root が UI 上の workflow 行であること、単独 AgentSession では workflow 名と Node 名が一致すること）は現行コードでも成立している。

# Outcome

- 対象者: Releash で workflow を実行し、Workspace ツリーで実行状況を観測・操作する開発者。
- 現在の問題: どの workflow を起動しても実行木のルート行が `main` と表示され、行から workflow を判別できない。さらに、同じ実行がアーカイブ済み履歴では workflow 名で表示されるため、実行中と履歴で同じ実行の名前が食い違う。
- 変更後に実現する状態: Workspace ツリーの workflow 実行ルート行に、その実行の workflow 名が表示される。同一 workspace で複数の workflow を走らせても行から実行中の workflow を判別でき、実行中の表示名とアーカイブ済み履歴の表示名が一致する。

# Current Behavior

## 再現手順

1. Releash を起動し、任意の worktree を選択する。
2. builtin workflow `01_author-spec` を起動する。
3. 同じ worktree で builtin workflow `03_full-review` を起動する。
4. Workspace パネルの実行木を見る。
5. 手順 2 で起動した実行を停止し、アーカイブする。
6. workflow 履歴の表示を見る。

## 実際の出力

- 手順 4: 実行木のルート行が 2行とも `main` と表示される。どちらの行がどの workflow の実行かを行の表示名から判別できない。
- 手順 6: 履歴には `01_author-spec` と表示される。手順 4 で同じ実行が `main` と表示されていたものと食い違う。

## 確認方法と根拠

上記は実行木の投影経路をコード上で追って確認した。

- ルート行の表示名: public root は owner の最初の可視直下子（`domain/workspace_tree/services.rs:49`）であり、workflow 実行ではこれが root Sequence `main` になる。その表示名は Node 名（`domain/workspace_tree/entities/mod.rs:475`）であり、read model の投影は public root の表示名を差し替えない（`adaptor/gateway/workspace_tree/query_service.rs:395-406`）。frontend は受け取った値をそのまま描画する（`src/components/workspace/WorkspaceList.tsx:408`）。
- 履歴行の表示名: `adaptor/gateway/workspace_tree/query_service.rs:239` が workflow 名を使う。
- 表示名の差異は表示名だけに閉じており、行の識別子（execution_id）と workflow 操作の可否は public root に正しく付与されている（`adaptor/gateway/workspace_tree/query_service.rs:342-365`）。

単独 AgentSession については、合成定義の `name` と唯一の Node 名がともに `session` であるため（`domain/workflow/value_objects/node_fact.rs:155-192`）、owner の表示名と public root の表示名はいずれも `session` で一致している。

# Scope / Non-goals

## 変更するもの

- Workspace ツリーの read model が返す、workflow 実行ルート行（public root）の表示名。

## 変更しないもの

- 実行木の Node が保持する表示名そのもの。Node 詳細など、Node の表示名を使う他の経路の表示は変えない。
- ルート行以外の行（子 Sequence、Fanout、Session / Command Node）の表示名。
- 単独 AgentSession の実行木の表示名。
- ルート行の識別子、状態表示、workflow 操作（停止 / 中止 / 再開 / アーカイブ）の可否。
- アーカイブ済み workflow 履歴の表示名。既に workflow 名である。
- workflow 定義構文。root Node 名を `main` に固定する規約は変えない。
- 表示名の決定を frontend に置くことは、`AGENTS.md` の「全てのアプリケーションロジックは Rust に置く」により選択肢に含めない。

# Requirements

- R-001: Workspace ツリー上で workflow 実行を表すルート行の表示名が、その実行の workflow 名になる。
- R-002: R-001 は、ルート行にあたる Node の種別（Sequence / Fanout / Session / Command）に依らず成り立つ。
- R-003: 同一 workspace で複数の workflow 実行が同時に並ぶとき、各ルート行の表示名からどの workflow の実行かを判別できる。
- R-004: 同一の実行について、実行中に表示されるルート行の表示名と、アーカイブ済み workflow 履歴に表示される名前が一致する。
- R-005: 単独 AgentSession の実行木のルート行の表示名は、本変更の前後で変わらない。
- R-006: ルート行以外の行の表示名は、本変更の前後で変わらない。
- R-007: Node 詳細に表示される名前は、本変更の前後で変わらない。
- R-008: ルート行の識別子、状態表示、および workflow 操作（停止 / 中止 / 再開 / アーカイブ）の可否は、本変更の前後で変わらない。

# Assumptions / Open Questions

## Assumptions

なし。

## Open Questions

なし。
