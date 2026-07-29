# Context

- 要求の正本: https://github.com/siro33950/releash/issues/1491 「[Agentチャット安定化] F8: Workspace / Session一覧queryのbounded化」
- 位置づけ: milestone 84「Agentチャット安定化」Phase 1。監査基準commitは `f0c63d3cacfbb65925c20e6338de70586801b3b8`。
- 依存(完了済み):
  - https://github.com/siro33950/releash/issues/1499 — 恒久SQLite store、bounded reducer projection、history-independent commit は導入済み(PR #1531)。physical store / migration / transaction core は本変更の前提であり対象外。
  - https://github.com/siro33950/releash/issues/1559 — `WorkflowExecution` 集約が workflow lifecycle の owner として確立済み(PR #1565)。
- 後続(本変更の成果を利用する):
  - https://github.com/siro33950/releash/issues/1521 — frontend lifecycle 増幅解消。cache validation に必要な snapshot identity は #1521 の側で定義する。
  - https://github.com/siro33950/releash/issues/1561 — session ライフサイクルの Domain 整理。
- 関連(対象外として分離): https://github.com/siro33950/releash/issues/1497 — queue pause projection の bounded read。
- 要求元が確定した実現方針の制約(後続のBehavior・Designが従う):
  - Workspace tree の構造規則(親子関係、木への参加可否、整合)は domain の集約が所有する。ただし集約専用の永続スナップショットは持たせず、`WorkflowExecution` と Session の永続 record から再構成する。
  - 表示は集約の内部構造から切り離した DTO で返す。
  - 保存済み record からの tree 取得で event 履歴を replay しない。
  - 既存の永続 record が答えられる問いに対して、派生 record を別テーブルへ複製しない。索引可能な列を既存 record の行へ足して解決する。
  - SQLite に正本を持たない状態(workflow execution)についてのみ、新しい永続 record を置く。
  - Session 一覧と workflow execution 一覧で、DB 内の全 `session_projection` 行を走査しない。
  - Workspace tree の取得で、recovery fence の読みを Session 数に比例させない。
  - query の実行で、呼び出しごとに thread、tokio runtime、DB 接続を新規生成しない。
  - 同一の永続 record から組み立てた Workspace tree は、組み立てを何度繰り返しても同じ内容になる。
- 現行実装の主要所在: `src-tauri/src/usecase/workflow/workspace_tree.rs`、`src-tauri/src/usecase/workflow/query_service.rs`、`src-tauri/src/adaptor/controller/command/workspace_tree.rs`、`src-tauri/src/adaptor/gateway/agent_session/session_storage/`(`session_projection` 走査経路)。

# Outcome

- 対象者: Releash デスクトップアプリで Workspace tree、Node detail、Session 一覧、workflow execution 一覧を操作する開発者(利用者)、および同じ backend query を利用する Tauri / loopback API / 将来の client surface。
- 現在の問題: 対象 ID だけで完結するはずの表示が、対象と無関係に蓄積した Session・event・workflow execution の量に比例して重くなり、Workspace 経路の操作遅延として現れている。
- 変更後の状態: Workspace tree、Node detail、Session 一覧、workflow execution 一覧の各表示が対象だけで完結し、無関係な蓄積量が増えても操作の応答が劣化しない。表示に必要な情報は一度の query で揃い、Tauri / loopback API / 将来の client が同じ backend query から同じ内容を得られる。

# Current Behavior

監査基準commit `f0c63d3cacfbb65925c20e6338de70586801b3b8` 系譜の現行コードで、次を確認した。

- 再現手順と観測結果: Session・event・execution が蓄積された Workspace で tree 表示、Node 選択、Session 一覧表示、workflow execution 一覧表示を行うと、対象と無関係なデータ量の増加に比例して待ち時間が伸び、操作遅延として観測される。
- Workspace tree の構築ロジックは `src-tauri/src/usecase/workflow/workspace_tree.rs` の手続きに埋没しており、表示のたびに組み立て直される。木を表現する domain 上の主体は存在しない。
- Workspace tree / Node detail / Session binding の query は、対象を読む前に worktree 内の全 Session を収集する。
- tree 表示は execution ごとに event 全件を replay する。
- Session 一覧と workflow execution 一覧は、DB 内の全 `session_projection` 行を走査する(`src-tauri/src/adaptor/gateway/agent_session/session_storage/` 配下)。`session_projection` は `session_id` を主キーとする JSON blob で、workspace・公開list区分・sort key のいずれも列になっていない。
- workflow execution の状態は SQLite に正本を持たず、event stream の畳み込みとしてのみ存在する。一覧は畳み込みを経由するため、読む量が event 総数に比例する。
- Workspace tree の取得で、recovery fence の読みが Session 数に比例する。

# Scope / Non-goals

## Scope

- Workspace tree、Node detail、Session に対応する Node、Session 一覧、workflow execution 一覧の各取得を、対象だけで完結させる。
- Workspace tree の表示に必要な情報を一度の query で返す。
- workflow execution とその実行ノードを、SQLite 上で対象だけ引ける永続 record にする。
- 既存 `session_projection` の行に、一覧が必要とする索引可能な列を足す。
- 再起動をまたいだ Workspace tree の観測内容を維持する。

## Non-goals

- physical store、migration、cutover、transaction core、history-independent commit(#1499 で完了済み)。
- workflow lifecycle の意味論(#1559 で完了済み)。
- session ライフサイクルの状態型と受理判断の domain 整理(#1561)。
- queue pause projection(#1497)。
- Workspace subtree 保持、listener lifecycle、Session page cache、event coalescing などの frontend 側変更(#1521)。
- 取得結果への snapshot identity / revision の付与。cache validation の契約は #1521 の側で定義する。
- close / finalize / Workflow / Session の意味論変更。
- manual archive の永続化形式とその write authority の変更。

# Requirements

- R-001: Workspace tree の表示に必要な情報が、一度の query で返る。
- R-002: 性能要件 — Workspace tree、Node detail、Session に対応する Node、Session 一覧、workflow execution 一覧の各取得の応答が、対象と無関係な Session・event・workflow execution の蓄積量が増えても劣化しない。
- R-003: 性能要件 — 同じ対象への取得を繰り返し実行しても、応答が実行回数に伴って劣化しない。
- R-004: アプリケーションの再起動をまたいでも、Workspace tree として観測できる内容が変わらない。
- R-005: 互換性要件 — Tauri、loopback API、将来の client が、同じ対象について同じ backend query の契約から同じ内容を取得できる。

# Assumptions / Open Questions

なし。
