# Context

- 正本: [#1864 `Delegate配下のSessionの終了通知が失敗し、Workflowがレビュー完了待ちで停止する`](https://github.com/siro33950/releash/issues/1864)
- 最初の周の調査基準は branch `feat/issues/1864` の `f3e55b13`（`origin/main` と同一）である。Issue が調査対象とした commit `81ec380b` はこの祖先であり、該当箇所は調査基準でも同じである
- ドメイン語彙と状態所有の正本: `docs/glossary/DOMAIN.md`。Session は delegate の child を部分木として持つ Node であり、child の NodeExecution は親 Session の部分木である
- workflow 定義構文の正本: `docs/glossary/WORKFLOW.md`。`completion.delegate` の child には Command / Sequence / Fanout、または `artifact` 宣言か `worktree: isolated` を持つ Session を指定できる
- 現行実装の確認先: `src-tauri/src/adaptor/gateway/workflow/worktree_context.rs`、`src-tauri/src/adaptor/gateway/agent_session/session_facts.rs`、`src-tauri/src/adaptor/gateway/agent_session/agent_session_repository.rs`、`src-tauri/src/adaptor/gateway/agent_session/agent_session_query_service.rs`、`src-tauri/src/usecase/provider_lifecycle/ingress.rs`、`src-tauri/src/domain/workflow/entities/workflow_execution/worktree.rs`、`src-tauri/src/adaptor/gateway/workflow/workflow_host.rs`、`src-tauri/src/usecase/workflow/control_plane.rs`
- Node の作業場所の導出は二か所にある。実行側は `ExecutionTree::execution_worktree_path`（domain）で、祖先の Node 種別を問わず最も近い隔離祖先から決める。読み側は adaptor gateway の `execution_worktree_path` で、保存された事実から同じ値を導出する
- 対象となる実行木の構造は、`impl`（Sequence）→ `implement`（Session）→ `completion.delegate` → `inner_check`（Sequence）→ `select_round`（Command）/ `inner_review`（Fanout）→ `reviewer`（Session）× 6 /`inner_adjudicate`（Session）である。この経路の Node に個別の worktree 指定はない

# Outcome

対象者は、delegate を含む workflow を Releash で実行する利用者である。

現在、delegate の child 部分木にある Session は、読み側の作業場所解決が「祖先が合成子でない」として失敗するため、provider からの Session 通知を受理できない。結果を提出したレビュー Session でも終了が記録されず、Fanout が完了待ちのまま後続の裁定へ進まない。

変更後は、delegate 親子関係を通る Session でも実行側と同じ作業場所を読み側で解決でき、Session の開始・終了通知を受理・記録できる。結果提出と終了が揃ったレビューは完了し、Fanout の後続 Node が開始する。

# Current Behavior

- 読み側の `execution_worktree_path` は祖先を辿る途中で親の事実行を読み、その Node が Sequence / Fanout でなければ `Corrupt("worktree ancestor is not a composite")` を返す（`worktree_context.rs:73-77`）
- 上記の構造では `reviewer` → `inner_review`（Fanout）→ `inner_check`（Sequence）→ `implement`（Session）と辿るため、delegate 親 Session に到達した時点で拒否される。拒否は親 Session 自身の worktree 指定を評価する前に起きる
- `read_session_context` がこの導出を呼び、`Corrupt` は `AgentSessionRepositoryError::Corrupt` と `AgentSessionQueryError::Corrupt` へ写像される（`session_facts.rs:51-68`、`session_facts.rs:110-121`）
- provider 通知の受信 `ProviderLifecycleIngressUsecase::receive` は最初に `begin_session_mutation` で対象 Session を読むため、`SessionStarted` / `StopObserved` / `ActivityObserved` / `report_unavailable` のいずれもこの時点で失敗し、事実は記録されない（`ingress.rs:120-123`、`ingress.rs:319-335`）
- 実行側は `ExecutionTree::execution_worktree_path` を使い、祖先の Node 種別を判定せず最も近い隔離祖先から作業場所を決めるため、同じ Node の起動は成功する（`worktree.rs:4-25`、`workflow_host.rs:1208-1220`）。Command Node の作業場所も同じ実行側の導出を使う（`control_plane.rs:435`、`control_plane.rs:493`）ため、Command だけではこの差異が現れない
- Issue の実測では、Claude / Codex 各3本のレビューで結果は提出されているが終了通知が記録されず、裁定が開始されない
- 既存の読み側テスト `worktree_context_test.rs` の fixture は Sequence または Fanout の直下に Session を置く構造だけで、delegate 親 Session を祖先に持つ構造がない

# Scope / Non-goals

## 変更する対象

- 読み側の作業場所解決が受理する祖先関係。delegate 親 Session を経由する経路を含める
- delegate の child 部分木にある Session に対する、Session 開始・終了通知の受理と記録
- delegate 構造での作業場所解決と Session 通知の受理を確認する回帰テストの追加

## 変更しない対象

- 実行側（domain、workflow_host、control_plane）の作業場所の導出
- Sequence / Fanout 配下の継承規則と、隔離 worktree の継承規則そのもの
- delegate の発火条件、続行条件、反復上限など delegate の仕様
- 既に終了が記録されず停止している実行木を先へ進める手段。この変更が対象とするのは、修正後に開始する実行である

# Requirements

- R-001: delegate 親子関係を含む祖先経路でも、Session の作業場所を読み側で解決できる
- R-002: 読み側が導出する Session の作業場所は、その Session の起動に使われた作業場所と一致する
- R-003: delegate の child 部分木にある Session の開始・終了通知が受理され、事実として記録される
- R-004: 結果提出と終了が揃った delegate 部分木の Session Node は完了し、Fanout の後続 Node が開始する
- R-005: Sequence / Fanout 配下の Session の作業場所解決と、隔離 worktree の継承は、変更前と同じ結果を返す
- R-006: 循環した祖先関係、および別の実行木の Node を含む祖先関係は、変更前と同じく読み取りエラーとして拒否される

# Assumptions / Open Questions

なし
