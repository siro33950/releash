# Context

- 要求の正本: [Issue #1468](https://github.com/siro33950/releash/issues/1468)「[統一 Node モデル] 実行木 UI の統一と文法正本化・最終 cleanup」（OPEN・milestone 86 の wave 8・comment なし）。
- 設計の正本: [`specs/unified-node-model/decisions.md`](../unified-node-model/decisions.md)（§実行木、§完了の定義と辺、§採用するもの「実行木 UI の完全性」、§語彙、§既存文書・実装との関係）と [`specs/unified-node-model/syntax.md`](../unified-node-model/syntax.md)（YAML 構文の確定分）、[`specs/unified-node-model/examples/full-cycle-development.yml`](../unified-node-model/examples/full-cycle-development.yml)（新構文の単一定義例・56 node）。
- 補助資料:
  - [milestone 86](https://github.com/siro33950/releash/milestone/86)「統一 Node モデル」— wave 順序（本 Issue は wave 8・最終）と、MS87 から継承する前提（session の完了は Submit + provider Stop の二信号、木全体は Running / Completed / Aborted の3値で詳細状態は Node 所有）。「Standalone AgentSession を実行木と別系統に置く」MS87 の扱いは本 milestone が上書きする。
  - [#1454](https://github.com/siro33950/releash/issues/1454)（CLOSED）— Node 中心再帰ツリー UI の先行実装。維持する原則（合成子は branch / 中央は単一 `NodeContentView` / Node header に内部 ID や番号ラベルを出さない）の出所。
  - [#1512](https://github.com/siro33950/releash/issues/1512)（CLOSED）と [`docs/specs/issues-1512/`](../../docs/specs/issues-1512/) — 「実行開始済み Node だけを表示する」表示契約の出所。
  - [#1591](https://github.com/siro33950/releash/issues/1591)（CLOSED・commit `0f15c1750`）と [`specs/issues-1591/`](../issues-1591/) — Lua による workflow 定義。文法正本への Lua 記述の追加は #1591 の Non-goal であり、本 Issue が所有する。
  - [#1466](https://github.com/siro33950/releash/issues/1466)（CLOSED・commit `ee86b1e91`）— 永続化の事実ログ化と単独 Session の Node 化。単独 Session が既に実行木（`tree_id` 単位の事実の集合）として永続化されている前提。
  - [#1467](https://github.com/siro33950/releash/issues/1467)（CLOSED・commit `6e5c47022`）と [`specs/issues-1467/`](../issues-1467/) — 直前 wave。spec の配置と構成の先例。
  - 改訂対象として本 Issue が名指しする文書: [`docs/workflow-yaml-syntax.md`](../../docs/workflow-yaml-syntax.md) / [`docs/workflow-engine-evolution-plan.md`](../../docs/workflow-engine-evolution-plan.md) / [`docs/workflow-engine-model-boundary.md`](../../docs/workflow-engine-model-boundary.md) / [`specs/workflow-lifecycle/workflow-ideal-lifecycle.md`](../workflow-lifecycle/workflow-ideal-lifecycle.md) / [`docs/architecture/GLOSSARY.md`](../../docs/architecture/GLOSSARY.md) / [`docs/examples/`](../../docs/examples/)。
- 確定済みの背景と制約（後続の Behavior・Design が従う）:
  - 維持する UI 原則（#1454）: 合成子は branch として子を束ねる、中央表示は単一の `NodeContentView`、Node header に内部 ID や番号ラベルを出さない。
  - 維持する表示契約（#1512）: 実行木には起きた実行だけが載る。定義 child や expected child slot から未開始 Node の行を再生成しない。
  - 改訂する既定（#1454）: 「retry で行を増やさない（occurrence の最新 attempt のみ関連付ける）」を廃し、起きた実行はすべて行に出す。retry は attempt ごと、delegate は発火ごとに行が並ぶ。番号ラベル（`attempt N` 等）は表示せず、順序は並びで判別する。決着済みの過去はデフォルト折り畳みで表示する。
  - 実行木の所属は root の Worktree に固定される。したがって監督は Worktree 単位の view で完結し、workspace 横断の監督 view は作らない。
  - 部品化・再利用の記述は Lua（#1591）が担う。定義を跨ぐ参照（`ref`）は #1464 の close により存在せず、example は単一定義1本に統合済みである。
  - `gate` は completion へ改名済みであり、以後は使用禁止語として扱う。
  - 本 Issue 本文は「詳細設計は着手時に `docs/specs/` 配下へ plan / design / goal 方式で作成する」と書くが、直近 wave（#1591 / #1467）の実績は `specs/issues-<issue 番号>/` の `requirements.md` / `behavior.md` / `design.md` である。本 spec は後者に従い `specs/issues-1468/` に置く。
  - 品質ゲートは本リポジトリの既定（`cargo fmt --check` / `cargo clippy -- -D warnings` / `cargo test`、`pnpm lint` / `pnpm test` / `pnpm build`、`pnpm test:integration`）。

# Outcome

- 対象者: Releash で workflow と単独 Session を実行・観測・監督する開発者、workflow 定義を書く開発者、および Releash 本体の保守者。
- 現在の問題:
  - 監督する側から見て、同じ Worktree で起きている実行が2系統に分かれている。単独 Session は実行木とは別のフラットな一覧に並び、中央表示も workflow 配下の Node とは別経路になる。ネストした sequence の実行インスタンスは fanout として表示され、実行木の構造が定義の構造と一致しない。retry した Node は起きた実行がすべて行に出るが、決着済みの過去 attempt も常に展開されたままで、いま見るべき最新の attempt が埋もれる。中央の Node header には `Attempt N` という番号ラベルが出ており、#1454 で決めた「Node header に番号ラベルを出さない」原則から外れている。
  - 定義を書く側から見て、文法の正本（`docs/workflow-yaml-syntax.md`）が旧構文（node のリスト・先頭 node が entry・`gate`・fanout の `child` と `item`）のまま止まっており、実装が受理する構文を説明していない。Lua による定義の説明はどの正本文書にも無い。engine の方針文書・モデル境界文書・ライフサイクル文書も、Node 種別3種・`gate`・6値 status を前提に書かれており、実装（Node 4種・completion・3値）と食い違う。GLOSSARY には Sequence / completion / 実行木 / 辺 が無い。例は `docs/examples/full-pipeline.yml` と `specs/unified-node-model/examples/full-cycle-development.yml` の2本が併存する。
- 変更後に実現する状態:
  - Worktree 配下で起きた実行が、単独 Session も含めて同じ再帰構造の実行木として同一の一覧に並び、どの Node を選んでも同じ中央表示経路で観測できる。起きた実行はすべて行として存在し、決着済みの過去 attempt は既定で折り畳まれ、番号ラベルは出ない。
  - 文法の正本文書だけを読めば、Lua による定義を含む新構文の全てが分かる。engine の方針・モデル境界・ライフサイクル・語彙の各正本が現行実装と一致し、`specs/unified-node-model/` との間に矛盾が無い。例は1本に一本化され、load でき、実行できる。移行のために入れた暫定 adapter と旧経路は実装に残っていない。

# Current Behavior

commit `0f15c1750ec787b6af38e47ec92afa833aa4eced`（branch `feat/issues/1468`）の worktree で、以下をコードと文書の調査により確認した。調査範囲は、Workspace 実行木の read model（`src-tauri/src/domain/workspace_tree/`、`src-tauri/src/adaptor/gateway/workspace_tree/`、`src-tauri/src/usecase/workflow/workspace_tree.rs`）、実行木を描画する frontend（`src/types/workspace-tree.ts`、`src/components/workspace/WorkspaceList.tsx`、`src/screens/MainLayout.tsx`、`src/components/panels/NodeContentView/`）、workflow 定義の検証と example の fixture test（`src-tauri/src/adaptor/gateway/workflow/diagnostics.rs`）、実行状態の語彙（`src-tauri/src/domain/workflow/value_objects/execution.rs`）、および改訂対象として名指しされた文書である。

## 実行木は「Workflow ブランチ」と「standalone Session 一覧」の2系統に分かれている

- read model の snapshot は木と Session 一覧を並べて返す。`WorkspaceTreeSnapshotDto` は `nodes: Vec<WorkspaceTreeItemDto>` と `sessions: Vec<AgentSessionItemDto>` を持つ（`src-tauri/src/usecase/workflow/workspace_tree.rs:21-26`）。
- `sessions` は、事実ログ上 `TreeRootFact::Session` を root に持つ木（= #1466 で Node 化された単独 Session）を、木としてではなく Session item の平坦な一覧として射影したものである（`src-tauri/src/adaptor/gateway/workspace_tree/query_service.rs:103-155`）。
- frontend も2系統のまま描画する。`agentSessions` を `AgentSessionRow` の一覧として出し、その後に `nodes` を `WorkspaceTreeItemRow` として出す（`src/components/workspace/WorkspaceList.tsx:1396-1440`）。
- 中央表示も2経路に分かれる。選択が `CenterSelection.kind === "agent_session"` なら `AgentSessionRoute` を直接描画し、`kind === "node"` のときだけ `NodeContentView` を通る（`src/screens/MainLayout.tsx:144-181`、`src/types/workspace-tree.ts:7-24`）。

## 実行木の DTO に Sequence が無く、sequence の実行インスタンスは fanout として現れる

- domain 側には Sequence がある。`WorkspaceNodeKind` は `Workflow / Fanout / Sequence / WorkflowSession / WorkflowCommand` の5種で、`Sequence` には「部品 sequence の実行インスタンス（実行木の branch）」という位置づけが与えられている（`src-tauri/src/domain/workspace_tree/value_objects/mod.rs:25-32`）。
- 公開 DTO は3種のままである。`WorkspaceTreeItemDto` は `Node / Workflow / Fanout` で（`src-tauri/src/usecase/workflow/workspace_tree.rs:43-47`）、射影は `WorkspaceNodeKind::Fanout | WorkspaceNodeKind::Sequence` を同じ `WorkspaceTreeItemDto::Fanout` へ落とす（`src-tauri/src/adaptor/gateway/workspace_tree/query_service.rs:282-290`）。frontend の型も `WorkspaceNode | WorkspaceWorkflow | WorkspaceFanout` の3種である（`src/types/workspace-tree.ts:81-84`）。
- したがってネストした sequence の部分木は、UI 上 fanout の行として（`FanoutRowStatusIcon` 付きで）現れる（`src/components/workspace/WorkspaceList.tsx:335-338`）。
- root は Workflow branch であり、定義 root（`main`）の実行インスタンスとしては現れない。単独 Session は木に載らない（上記のとおり `sessions` 側）。

## 起きた attempt は行として並ぶが、決着済みの過去 attempt も既定で展開される

- 木の projection は、同じ node 名の実行が起きるたびに occurrence を数え、別の opaque ID を持つ別行を作る（`src-tauri/src/domain/workspace_tree/entities/mod.rs:339-445`）。新しい attempt は新しい `node_execution_id` を伴うため（`src-tauri/src/adaptor/gateway/workflow/workflow_host.rs:188`、`:905`）、attempt ごとに行が増える。同じ node 名の attempt 1 / 2 が別 ID の2行になることは既存テスト `fanout_occurrences_are_distinct_and_children_stay_nested_in_event_order` が固定している（`src-tauri/src/domain/workspace_tree/entities/mod.rs:1850-1940`）。
- 一方、折り畳みの既定は決着状態と無関係である。branch 行の展開状態は frontend が `useState(true)` で持つだけで、backend は折り畳みに関する情報を返さない（`src/components/workspace/WorkspaceList.tsx:320`）。決着済みの過去 attempt を既定で折り畳む仕組みは無い。
- 行に attempt 番号のラベルは出ない。木の Node DTO は `attempt` を持たない（`src-tauri/src/usecase/workflow/workspace_tree.rs:51-59`）。

## Node header に `Attempt N` が表示される

- Node detail DTO は `attempt: Option<u32>` を持ち（`src-tauri/src/usecase/workflow/workspace_tree.rs:104-109`）、中央の `NodeHeader` がそれを `Attempt {n}` として表示する（`src/components/panels/NodeContentView/NodeContentView.tsx:158-162`）。

## backend で廃止された status 語彙が frontend にだけ残っている

- backend の公開 status は `running / paused / failed / waiting / interrupted / aborted / completed` の7語である（`src-tauri/src/domain/workspace_tree/value_objects/mod.rs:35-57`）。`queued` を返す経路は #1512 で除去済みで、Rust 側に文字列 `"queued"` は存在しない。
- frontend の `WorkspaceNodeStatus` は `queued` と `error` を含む9語のままであり（`src/types/workspace-tree.ts:27-35`）、`NodeContentView` には `detail.status === "queued"` の分岐が残っている（`src/components/panels/NodeContentView/NodeContentView.tsx:64-68`）。

## 実行状態は既に3値だが、文書は6値前提のまま

- production の `ExecutionStatus` は `Running / Completed / Aborted` の3値である。`WaitingApproval` と `Interrupted` は `#[cfg(test)]` でのみ存在する（`src-tauri/src/domain/workflow/value_objects/execution.rs:6-14`）。詳細状態は Node 側の `WorkspaceNodeStatus` が持つ。
- `docs/workflow-engine-evolution-plan.md` は「status は `running` / `waiting_approval` / `interrupted` / `completed` / `failed` / `aborted`」と書く（`:46`）。`docs/workflow-engine-model-boundary.md` も同じ6値を書き（`:48`）、構造図は「Workspace 配下に WorkflowExecution / AgentSession が並列」である（`:20-36`）。
- `specs/workflow-lifecycle/workflow-ideal-lifecycle.md` は全体が6値 ExecutionStatus 前提である。§状態語彙（`:23-32`）、§ExecutionStatus 遷移表（`:79-92`）、§操作×状態 受理マトリクス（`:95-111`）、§capability 導出規則（`:162-168`）がいずれも6値に依存する。同書は「現行エンジン(6値 ExecutionStatus)と統一 Node モデルの実行木の両方に同じ invariant を適用する」（`:7`）「milestone 86 移行時は内部表現だけを実行木へ差し替え、本書と契約を維持する」（W-D4）と自認している。

## 文法の正本 `docs/workflow-yaml-syntax.md` は旧構文のまま

同書が説明している構文は、現行実装が受理しないものを含む。

- Root は `nodes` を「必須の非空配列。先頭の Node が entry」と書く（`:35`）。現行は名前をキーにしたマップで、root は `main` 規約である。リスト形式の `nodes` は拒否される（`src-tauri/src/adaptor/gateway/workflow/diagnostics.rs:3568` の invalid fixture）。
- Node kind は `command` / `session` / `fanout` の3つとして書かれ（`:38-49`）、`sequence` の記述が無い。
- session の完了は「`gate`: 必須。`auto` または `approval`」として kind ブロック内に書かれている（`:81`、`:92`、`:97-98`）。現行は Node 共通フィールドの `completion` である。session の `model` / `permission` の記述は無く、「ReleashはProviderのmodel、permission、plan、sandboxを設定せず」と書かれている（`:100`）。
- fanout は `child`（Node 名参照）と `items`、および `item` / `item.<field>` 参照として書かれている（`:102-132`、`:154-155`）。現行は `children` リストと `inputs` による配線で、`{{ item }}` 特殊名は廃止されている。
- `input: <Contract>` は「fanout child として受ける単一 parameter の型」（`:139`）、`inputs: [request | <node>, ...]` は「Artifact 全体への依存宣言」（`:140`）と書かれている。現行の `input` はパラメータのリスト、`inputs` は合成子の children が書く配線マップである。
- children 4形式、隣接辺（rules を持たないエントリの既定）、`entry` / `output`、`on_failure`、`worktree`、予約語一覧、Lua による定義の記述はいずれも無い。

## `gate` は実装からは消えているが正本文書に残っている

- Rust の非テストコードに単語 `gate` は存在しない（`gateway` の部分一致のみ）。YAML schema にも Diagnostic 文言にも `gate` は無い。
- 一方、正本文書には残る。`docs/workflow-yaml-syntax.md`（`:81`、`:92`、`:97`、`:98`、`:292`）、`docs/workflow-engine-evolution-plan.md`（`:27`、`:28`、`:97`、`:99`、`:154`、`:179`）、`docs/workflow-engine-model-boundary.md`（`:104`）。
- `docs/architecture/GLOSSARY.md` の使用禁止語一覧に `gate` は無い。正規語一覧にも Sequence / completion / 実行木 / 辺 は無く、§Workspace の構造図は「Workspace 配下に WorkflowExecution / AgentSession / Command が並列」である（`:57-77`）。Worktree は「外部実体。Releash は所有しない」の1種類だけで（`:25`）、§状態所有（`:120-135`）に隔離 worktree の記述は無い。

## example は2本併存し、実行経路の統合テストが無い

- `docs/examples/full-pipeline.yml`（15 node）は新構文で書かれており、Diagnostic ゼロで load でき、実 loader も通ることを fixture test が固定している（`src-tauri/src/adaptor/gateway/workflow/diagnostics.rs:2170-2212`）。
- `specs/unified-node-model/examples/full-cycle-development.yml`（56 node）も同様に fixture test を持ち、残る Diagnostic が `implement_all` / `fix_all` の `WFU002`（`worktree` の unknown field・#85 で解禁）2件だけであることを固定している（`src-tauri/src/adaptor/gateway/workflow/diagnostics.rs:2110-2138`）。
- 正本文書は前者を指す（`docs/workflow-yaml-syntax.md:3`、`docs/workflow-engine-evolution-plan.md:161`）。`specs/unified-node-model/` は後者を指す（`decisions.md:5`、`syntax.md:5`）。
- Rust から example を参照するのは上記2つの diagnostics test だけで、example を engine の実行経路へ通す統合テストは無い。

# Scope / Non-goals

## Scope

- Workspace の実行木 read model と UI を、Worktree に所属する実行木の再帰構造へ一般化すること。単独 Session、ネストした sequence の部分木、fanout の展開が同じ再帰構造として現れること。
- 中央表示を単一の `NodeContentView` 経路へ統一し、Node header から raw metadata（内部 ID・attempt 番号ラベル）を除くこと。
- retry の attempt を行として並べ、決着済みの過去 attempt を既定で折り畳んで表示すること。
- `docs/workflow-yaml-syntax.md` を現行実装の構文（Lua による定義を含む）の正本へ改訂すること。
- `docs/workflow-engine-evolution-plan.md` / `docs/workflow-engine-model-boundary.md` / `specs/workflow-lifecycle/workflow-ideal-lifecycle.md` / `docs/architecture/GLOSSARY.md` を、統一 Node モデルと現行実装（Node 4種・completion・3値 status + Node 所有の詳細状態・Worktree 配下の実行木）へ改訂すること。
- example の workflow 定義を1本へ一本化し、docs と `specs/unified-node-model/` の双方がそれを参照する状態にすること。一本化した example が load でき、実行できることの検証。
- 統一 Node モデルへの移行のために入れた暫定 adapter・旧構文の読み替え・不要になったロジックの削除。

## Non-goals

- workspace 横断の監督 view を作ること。木の構造所属が root Worktree に固定されるため、Worktree 単位の view で監督が完結する。
- Fanout 結果への承認の表示場所の検討。#1454 系が所有する。
- delegate（親 Session の Submit による child 起動）と `worktree: shared | isolated` の受理・解禁。milestone #85 が所有する。`worktree` を宣言した定義が `WFU002` で拒否される現行の扱いは変えない。
- 定義を跨ぐ参照（`ref`）の導入。#1464 の close により存在しない。
- 部品化・再利用の文法を YAML 側に追加すること。Lua（#1591）が担う。
- builtin workflow の Lua 化、および YAML 編集 UI の変更。
- 過去 milestone の記録文書（`docs/specs/milestone-82/` など、完了した milestone の plan / design / goal）の書き換え。改訂対象は本 Issue が名指しした正本文書に限る。
- 事実ログのスキーマ移行、および既存の永続データの変換。
- Node の実行・完了・遷移の規則そのものの変更。本 wave は観測（UI）と正本文書の整合であり、engine の振る舞いを変えない。

# Requirements

- R-001: Worktree 配下で起きた実行は、その Worktree に所属する実行木として観測できる。単独 Session は Session Node 1個を root とする実行木として、workflow は定義 root（`main`）の実行インスタンスを root とする実行木として、いずれも同じ再帰構造の行で同一の一覧に並ぶ。ネストした sequence の部分木と fanout の展開も同じ再帰構造の行として現れる。実行木と別系統の一覧（standalone Session の別一覧など）は存在しない。
- R-002: ツリーで選択した Node の中央表示は、その Node が単独 Session か workflow 配下の Session か Command かによらず、単一の `NodeContentView` 経路で機能する。
- R-003: 互換性要件 — 合成子（Sequence / Fanout）は子を束ねる branch として現れ、自身の中央表示を持たない（#1454 の原則を維持する）。
- R-004: 互換性要件 — ツリーに現れるのは実行が開始された Node だけである。定義にあるが実行されていない Node、分岐で選択されなかった Node、未展開の fanout child は、定義や expected slot から再生成されない（#1512 の表示契約を維持する）。
- R-005: 互換性要件 — ツリーの行と Node header に、内部 ID・attempt 番号ラベル・item / child 座標などの raw metadata が表示されない（#1454 の原則を維持する）。
- R-006: 同じ Node が retry で複数回実行された場合、起きた attempt がすべて実行順の行として並ぶ。行に attempt 番号のラベルは付かず、順序は並びで判別できる。
- R-007: 決着済みの過去 attempt は既定で折り畳まれた状態で現れ、人間の展開操作によって観測できる。
- R-008: 互換性要件 — 単独 Session に対する既存の操作（選択して中央で観測する、archive する、削除する、archive 済みを一覧する）と、workflow 実行木に対する既存の操作（stop / resume / abort / archive、Node の approve / retry / close）は、変更前と同じく行える。
- R-009: `docs/workflow-yaml-syntax.md` だけで、現行実装が受理する構文の全てが説明されている。説明対象は、`main` 規約（トップレベル entry フィールド無し）、nodes マップ、sequence（`entry` / `output` / `children`）、children の4形式、隣接辺、Interface（`input` / `artifact`）と配線（`inputs`）の分離、completion（session の二信号を含む）、session の `provider` / `model` / `permission`、`on_failure`、`worktree`、予約語、および Lua による定義（#1591）である。
- R-010: `docs/workflow-engine-evolution-plan.md` の記述が現行実装と一致する。Node の種別は Session / Command / Fanout / Sequence の4種として書かれ、完了の定義は `completion` として書かれ、木全体の status は Running / Completed / Aborted の3値として書かれている。
- R-011: `docs/workflow-engine-model-boundary.md` の構造図が「Worktree 配下の実行木」を表し、「Workspace 配下に WorkflowExecution / AgentSession が並列」という構造と6値 status の記述が残っていない。
- R-012: `specs/workflow-lifecycle/workflow-ideal-lifecycle.md` の不変条件・遷移表・操作×状態 受理マトリクス・capability 導出が、「木全体 = Running / Completed / Aborted の3値、WaitingApproval / Paused / Failed は Node が所有」を前提に書かれている。6値 ExecutionStatus を前提とする記述が残っていない。
- R-013: `docs/architecture/GLOSSARY.md` に Sequence / completion / 実行木（execution tree）/ 辺（edge）が正規語として収録され、`gate` が completion の使用禁止語として収録され、Workspace の構造が「Worktree 配下の実行木」として記述され、隔離 worktree（`isolated` 宣言により生まれる実行環境）の状態所有が記述されている。
- R-014: workflow の完了条件を指す語としての `gate` が、正本文書・schema・Diagnostic 文言のいずれにも残っていない。
- R-015: example の workflow 定義は1本に一本化され、`docs/` と `specs/unified-node-model/` の双方がその1本を参照する。複数の example 定義が併存しない。
- R-016: 一本化した example は `worktree` を宣言せず、現行の loader で Diagnostic ゼロで load できる。解禁済み構文（ネストした sequence、fanout の子に置いた合成子）に対する Diagnostic も出ない。
- R-017: 一本化した example は engine の実行経路を通して実行できる。
- R-018: `specs/unified-node-model/`（`decisions.md` / `syntax.md` / `examples/`）と改訂後の正本文書の間に、矛盾する記述が無い。
- R-019: 統一 Node モデルへの移行のために入れた暫定 adapter、旧構文の読み替え、および廃止済みの語彙・状態に対応する不要なロジックが、実装に残っていない。

# Assumptions / Open Questions

なし。
