# Design

## The actual design

### Architecture

#### 表示名の3段規則は `workspace_tree` の domain が所有する

R-001 の「手動 rename ＞ provider のセッションタイトル ＞ 既定値」は、Workspace ツリーの行が何を見せるかの規則であり、`domain/workspace_tree/` が所有する。行の表示名の規則は既にこの domain にある（`WorkspaceTreeNode.title` を Node 名で決める `entities/mod.rs:483`、ルート行を owner の表示名で差し替える `services.rs:49`）。3段規則をここに置くことで、表示名の決定者はこの domain 一つのままになる。`docs/architecture/DOMAIN.md`「規則は domain が所有する」「一つの概念に一つの表現」に従う。

解決規則は次の一つである。

```text
1. 手動 rename された名前
2. provider のセッションタイトル（単独 Session 実行木の root Session Node だけが参照する）
3. Node 名（単独 Session 実行木では合成定義の Node 名 `session`）
```

R-004 の既定値 `session` は新しい定数にしない。単独 Session 実行木の合成定義は唯一の Node 名を `session` にしている（`domain/workflow/value_objects/node_fact.rs:159`）ため、第3段の「Node 名」がそのまま既定値になる。R-002 は、workflow 実行木の Session Node で第2段を参照しないことによって成立する（第3段の Node 名が常に出る）。

#### 単独 Session 実行木の root 行かどうかは domain の述語で判定する

`WorkspacePublicRoot::public_title()` は現在すべての実行木のルート行に owner（Workflow node）の表示名を返す（`services.rs:49`、Issue #1662 で確定）。単独 Session 実行木では owner の表示名が `session` であるため、この経路を通す限り R-003 の provider タイトルは行に出ない。

`public_title()` を「public root Node が単独 Session 実行木の root Session Node であれば Node 自身の表示名、そうでなければ owner の表示名」に変える。判定は domain の述語で行う。

Issue #1662 の design は「単独 AgentSession を launch 種別で分岐しない」を採ったが、その理由は「domain が表現していない区別に基づく表示規則が gateway 側に生まれ、規則の所有者が二つに割れる」ことだった。本件の分岐は domain 側の述語であり、gateway の `workflow_execution_ids`（launch 種別の集合）を判定に使わない。#1662 の R-005（単独 AgentSession の root 行の表示名が変わらない）は、本件の R-003 が上書きする。

述語は集約が既に持っている条件と同一である。`apply_node_started` は Node id の払い出しのために `kind == Session && parent.is_none() && node_execution_id == execution_id` を計算している（`entities/mod.rs:385`）。この条件に名前を与えて `WorkspaceTreeNode` の述語にし、id 払い出しと表示名解決の両方がその一つを読む。新しい field は増やさない。

#### rename 可否も同じ場所で決める

R-007 の対象外（Sequence 行、Fanout 行、workflow 実行木のルート行）は、「その行が Node 自身の名前を見せているか」と同値である。workflow 実行木のルート行は owner の表示名を見せるので対象外、Sequence / Fanout は Session Node ではないので対象外、単独 Session 実行木のルート行は Node 自身の名前を見せるので対象になる。

`can_rename` は `WorkspaceTreeNode` が持ち、既存の `can_approve` / `can_retry` / `can_close` と同じく集約の recompute で決める。判定に必要な「public root 行かどうか」は `WorkspacePublicRoot::all()` が同じ node 列から導けるため、集約内で完結する。gateway は `node.can_rename` を写すだけで、行種別ごとの分岐を持たない。

rename には bound 済みの AgentSession が要る（後述）ため、`session_id` が未 bind の Session Node は `can_rename` を false にする（R-015）。retry 履歴行に特例は設けない（各 attempt は別の NodeExecution であり、名前も別に決まる）。

#### bind されていない Session Node の状態は分類として表す

R-016 の「bind 前と bind 後を区別できる状態」は、行が今どういう状態かの規則であり、`WorkspaceNodeStatusClassification` が既に所有している（`domain/workspace_tree/value_objects/mod.rs:58-63`、`同:163-191`）。ここに 5 つ目の値を足す。

現在の `classify_own_status` は Session Node の分類を `activity` から決めるが、`activity` は行の生成時に既定値 `AwaitingInstruction` が入る（`entities/mod.rs:482`、`domain/workflow/value_objects/node_fact.rs:271-272`）ため、bind 前の行は観測していない状態を `Attention` として出している。Session Node で `session_id` が未 bind のときは、`activity` を見る前に新しい分類を返す。判定は `can_rename` と同じ `session_id` の有無であり、新しい field も新しい事実も増やさない。

R-017 の集約は `severity()` の順序で決まる（`value_objects/mod.rs:75-89`）。新しい分類を最弱に置き、既存 4 値の相対順は変えない。bind 前は行が主張する情報が最も少なく、`most_severe` は「最も強い状態を親へ上げる」規則であるため、他の子がいる限り親行には出ない。

bind 前の状態が現れるのは workflow 実行木の Session Node だけである。単独 Session 実行木は root started と `session_attached` を同時に書く（`domain/workflow/value_objects/node_fact.rs:196-206`、`SessionExecutionTreeRootFacts::into_facts`）。

#### 名前の2つの入力は AgentSession が所有する

手動 rename の名前と、観測した provider のセッションタイトルは、どちらも `AgentSession` 集約の状態にする。

- provider のセッションタイトルは provider 側の属性であり、`AgentSession` が既に所有している provider・provider session identity・transcript reference と同じ性質のものである（`docs/glossary/DOMAIN.md`「状態所有 / AgentSession」）。
- 手動 rename は Session Node の行に対する人間の行動だが、Session Node と AgentSession は 1 対 1 に bind される。専用の書き込み port を新設せず、既存の `AgentSessionRepository` の書き込み経路に載せる。

どちらも `AgentSessionLifecycleEvent` として集約から出て、`agent_session_repository.rs:89` の `session_event_rows` が NodeFact へ写す。事実は Session Node の `node_execution_id` 行として `node_events` に載り、読み側の fold が導出する（`docs/architecture/README.md`「Agent TUIの状態所有」、AGENTS.md「永続化は event store」）。

表示名の解決規則（3段）は `workspace_tree` に、名前の入力の状態は `agent_session` に置く。前者は「行が何を見せるか」、後者は「Session が何と名乗っているか」であり、同じ概念の二重化ではない。

#### 事実から表示名までの配線は活動状態と同じ経路に載せる

`AgentSessionActivity` は既に「1 tree 分の事実列 → `FoldedTree.session_activities` → `RuntimeSnapshotNodeProjection` → `WorkspaceStructureFact::NodeActivityProjected` → 集約の apply」という経路で Node へ届いている（`domain/workflow/services/fact_replay.rs:36,87`、`domain/workspace_tree/projection.rs:94`、`entities/mod.rs:854`）。

表示名の入力も同じ経路に載せる。`fold_execution_tree` の同一走査で Session Node ごとの「手動 rename 名」と「観測済み provider タイトル」を導出し、projection が Node ごとの fact として渡し、集約が 3 段規則で `title` を決める。表示名の投影のためだけの別経路は作らない。

`WorkspaceTreeNode.title` を書き換えるので、Node 詳細（`load_node` → `node_detail`）にも同じ表示名が出る。#1662 が `title` を据え置いたのは workflow 名を Node の名前へ混ぜないためであり、本件で書き換えるのは Node 自身の名前なので抵触しない。

#### rename の入口は Workspace Node action に載せる

不透明な node id を受け取って Node への操作へ解決する入口は `WorkspaceNodeCommandUsecase` が既に持っている（`usecase/workflow/workspace_node_command.rs`、`approve_workspace_node` / `retry_workspace_node`）。rename も同じ形にする。resolver が node id から対象の AgentSession id を解決し、usecase が rename の実行を委譲する。

公開入口は Tauri command だけにする。Workspace ツリーの read model 自体が Tauri command からしか公開されていないため、local API に rename だけを生やすと入口の非対称が生まれる。

#### provider タイトルの取り込みは対象を絞った定期読み取りにする

R-010 のとおり hook payload はタイトルを含まないため、取り込みは Releash 側の定期読み取りで行う。設計上の要点は3つある。

**駆動は composition root が持つ。** `docs/architecture/USECASE.md` は usecase に時刻を持ち込まないことを求める。tick を刻むタスクは controller（composition root）が spawn し、usecase の1メソッドを呼ぶ。既存の `run_startup_recovery`（`lib.rs:1010` 付近）と同じ形にする。tick の間隔と「今回の tick で読むか」の規則は domain が持つ（後述 Algorithm）。

**対象の選別は AgentSession の lifecycle が決める。** R-011 の「活動中」は `AgentSession` の lifecycle が `Open`（= 後続の attach / resume が無い process_exited が無く、archive もされていない）であることと同値であり、既存の `SessionFactsView::is_open()`（`fact_replay.rs:385`）がその規則を持つ。新しい判定は作らない。

**実行木の fold を tick ごとに走らせない。** AGENTS.md は full-recompute 経路を増やさないことを求める。対象の列挙は、`node_events` から lifecycle 関連の event type 集合を全 Node 分まとめて読み（新設する `event_type` 索引で引く。Database 参照）、`node_execution_id` ごとの事実列を `derive_session_facts` に掛ける。provider session id を持ち lifecycle が `Open` と判定された対象にだけ、実行木 root と最新タイトルの追加読みを行う。実行木全体の fold も、workflow 集約の復元も行わない。

既存の `find_for_activity`（`agent_session_repository.rs:478`）は同種の絞り込み読みだが、root・attachment・最新 activity しか読まないため `exited` / `archived` を導出できず、R-011 の判定には再利用できない。取り込み用の読み取りは lifecycle 関連の事実（`session_attached` / `process_exited` / `archive_requested` / `resume_requested` / `restore_requested`）と最新のタイトル事実を読む。

#### Provider history の行ラベルは domain が決め、読み取りは2段階にする

R-014 の「provider のセッションタイトル → 最初のユーザープロンプト → provider 名と短縮 id」という3段のフォールバックは表示規則であり、`domain/agent_session/` の状態を持たない規則として置く。frontend は解決済みの文字列を描画するだけにする（AGENTS.md「Rust がロジックを所有する」）。

読み取りは2段階にする。候補の列挙（`AgentSessionHistoryGateway::list_metadata`）は現在どおり file の stem と mtime、Codex の行だけを見る安価な走査のままにし、ラベル入力の取得は、ページングと所有済み除外を通過した可視分（`limit` 件）に対してだけ行う。現在の列挙は provider あたり最大 201 件を走査する（`agent_session_history_query_service.rs:15-16`）ため、列挙時にラベル入力を読むと 1 ページの表示で最大 201 transcript を開くことになる。Claude は可視候補の transcript の先頭 64KiB だけから最初の `isMeta` でない `type: "user"` のテキストを読み、末尾 64KiB のタイトル窓と合わせても全走査しない。Codex は可視候補に対する既存の `threads` クエリの同じ行から `name` と `first_user_message` を取得し、追加のクエリや rollout 読み取りを行わない。

#### 検証の置き場所

B-001〜B-004、B-006〜B-011、B-019〜B-021 は、`workspace_tree` 集約の単体テスト（表示名の解決、`can_rename`、bind 前の分類と集約順位）と gateway の投影テストで判定できる。手段が自明でないのは次の3点である。

- B-005（供給源が Claude の `ai-title` / Codex の thread name であること）: 供給源の判定を持つのは `ProviderSessionTitleGateway` の実装であり、infrastructure の primitive は byte 列を返すだけで `ai-title` を知らない。temp 上に実 transcript と実 SQLite を用意し、gateway を通して得たタイトルを固定する。Provider CLI 自体は起動しない（`docs/architecture/TEST.md` モック方針）。
- B-012 / B-013（終了・paused・archived で再取得しない）: 対象選別が事実列から導出されることの検証であり、事実列を組んだ repository のテストで、列挙結果に当該 session が現れないことを固定する。
- B-014 / B-015（次の取り込みでの反映）: 実時間で待たない。R-012 の 20 秒 / 5 分は、tick 番号と「タイトル取得済みか」から読み取り要否を返す domain 規則の単体テストで固定する。反映そのものは、タイトル事実を含む事実列を組んだ集約のテストで固定する。

### Interface

#### 追加する純粋事実（`domain/workflow/value_objects/node_fact.rs`）

`NodeFact` に2つの variant を足す。event_type と detail は `node_events` の永続形そのものである。

```rust
/// 人間の行動: Session Node の表示名を指定した。
SessionNodeRenamed(SessionNodeRenamedFact),   // event_type: "session_node_renamed"
/// 観測: provider が当該 session に付けているセッションタイトルを読み取った。
ProviderSessionTitleObserved(ProviderSessionTitleObservedFact), // event_type: "provider_session_title_observed"
```

detail はそれぞれ `{ "name": String }` と `{ "title": String }` の1 field。どちらも遷移や導出結果ではなく、人間の行動と外部の観測であり、既存の事実語彙の分類に収まる。`fold_execution_tree` の `apply_record` では実行状態を変えない（既存の `AgentActivityObserved` 等と同じ no-op 群に入る）。

#### AgentSession 集約（`domain/agent_session/aggregates/agent_session.rs`）

- `rename(SessionDisplayName)` — 表示名を確定し `AgentSessionLifecycleEvent::Renamed` を出す。同値なら `AgentSessionMutationOutcome::AlreadyApplied`。
- `observe_provider_session_title(&str) -> AgentSessionMutationOutcome` — `observe_activity` と同じ形。同値なら `AlreadyApplied` を返し、事実を作らない（5 分間隔の再読で同じ事実を積まない）。
- 読み取り用の accessor を2つ足す（手動 rename 名、観測済み provider タイトル）。

`SessionDisplayName` は `domain/agent_session/` の値オブジェクトにする。前後の空白を落とし、空文字を拒否する。空文字を「rename の取り消し」として扱う経路は作らない（Requirements が定めていない）。

#### AgentSession の永続化 port（`domain/agent_session/repository.rs`）

`AgentSessionRepository` に2つ足す。

- `list_open_for_provider_session_title() -> Vec<VersionedAgentSession>` — provider session id を持ち lifecycle が `Open` の AgentSession を、実行木の fold を経ずに返す。`Open` の判定は `derive_session_facts` の規則をそのまま適用する。
- `save_provider_session_title(session, caller_request_id)` — `ProviderSessionTitleObserved` の1件だけを受け付ける軽量 commit。`save_activity`（`agent_session_repository.rs:514`）と同じ形。

rename は `save` を使う。既存の CAS 経路のままでよい（人間の操作であり頻度が低い）。

#### provider タイトルの読み取り port（`domain/agent_session/`）

```rust
pub(crate) struct ProviderSessionTitleRequest {
    pub provider: ProviderKind,
    pub provider_session_id: String,
    pub worktree_path: String,
    pub transcript_ref: Option<String>,
}

#[async_trait::async_trait]
pub(crate) trait ProviderSessionTitleGateway: Send + Sync {
    async fn read_title(
        &self,
        request: ProviderSessionTitleRequest,
    ) -> Result<Option<String>, ProviderSessionTitleGatewayError>;
}
```

引数は AgentSession が所有する語彙だけで書く。file path や SQLite の語は現れない。`transcript_ref` が無い Claude session では gateway が worktree path と provider session id から transcript を引き当てる（既存の `claude_project_directory` の規則、`agent_session_history_gateway.rs:93`）。

`ProviderSessionTitleGatewayError` は `Unavailable` / `Corrupt` の2値にする。呼び出し側はどちらでもその tick の読み取りを諦める。

#### Provider history の port と DTO

`AgentSessionHistoryGateway`（`domain/agent_session/provider_history_gateway.rs`）に、可視分だけを対象にする取得を足す。

```rust
async fn list_session_titles(
    &self,
    provider: ProviderKind,
    worktree_path: &str,
    provider_session_ids: &[String],
) -> Result<Vec<ProviderSessionTitleEntry>, AgentSessionHistoryGatewayError>;
```

`ProviderSessionTitleEntry` は `provider_session_id`、`session_title: Option<String>`、`first_user_prompt: Option<String>` を持つ。最初のユーザープロンプトだけを上限付きで読み、会話本文を全走査しない。`AgentSessionHistoryMetadata` は変えない（列挙は安価なままにする）。

行ラベルの解決は `domain/agent_session/` の3段規則に置く。`AgentSessionHistoryCandidateDto`（`usecase/agent_session/agent_session_history.rs:9`）に `label: String` を足す。`provider` と `provider_session_id` は残す（React key と resume 操作が使う）が、行には `label` だけを描画する。

#### 取り込み usecase（`usecase/agent_session/`）

`ProviderSessionTitleIngestionUsecase` を足す。`ingest_due()` の1メソッドで、対象の列挙 → タイトル読み取り → 集約への観測 → 変化時のみ保存と通知を行う。tick 番号は usecase 内のカウンタで進める（実行制御 state であり、時刻ではない）。

#### Tauri command

`adaptor/controller/command/workspace_tree.rs` に `rename_workspace_session_node(worktree_path, node_id, name)` を足す。戻り値は既存の command と同じ `Result<(), String>`。`WorkspaceNodeCommandUsecase` に `rename_workspace_session_node` を足し、resolver に node id → AgentSession id の解決を、実行の委譲先に rename の port を足す。

#### 公開 DTO / 型の互換

`WorkspaceNodeCapabilitiesDto` に `can_rename: bool` を足す。`AgentSessionHistoryCandidateDto` に `label: String` を足す。どちらも field 追加のみで、既存 field の意味は変えない。frontend の `WorkspaceNodeCapabilities` / `AgentSessionHistoryCandidate` にも対応する field を足す。

行の状態を表す公開値（`WorkspaceNodeStatusClassification::as_public_str`、`value_objects/mod.rs:66-73`）が 4 値から 5 値になる。frontend の union（`src/types/workspace-tree.ts:30-34`）と色表（`src/components/workspace/WorkflowNodeStatusIcon.tsx:15-23`）に同じ値を足す。既存 4 値の文字列と意味は変えない。

### Data Model

- `NodeFactRecord` の追加2種。identity は既存と同じ `(tree_id, node_execution_id, seq)`。すなわち手動 rename の名前も観測タイトルも **NodeExecution（attempt）単位**で持つ。retry で新しい attempt が始まれば名前は引き継がれない（各 attempt は別の Node 行であり、別の AgentSession を持つ）。
- `AgentSession` 集約に2つの値を足す（手動 rename 名、観測済み provider タイトル）。どちらも `Option<SessionDisplayName>` 相当で、事実列から導出される。
- `FoldedTree`（`fact_replay.rs:29`）に Session Node ごとの名前入力の map を足す。`session_activities` と同じ走査・同じ寿命。
- `WorkspaceTreeNode` に `can_rename: bool` を足す。`title` は既存 field を使い、Session Node では3段規則の結果が入る。bind 前の状態分類は `session_id` の有無から導出するため、専用の field は持たない。
- 保持しないもの: transcript の本文、provider 側のカスタム名、タイトルの観測履歴（最新の観測だけを読む）、Provider history で一時的に読んだ最初のユーザープロンプト。
- versioning: `node_events` の event_type は追記のみで、既存 event_type の detail 形は変えない。旧いログは新しい event_type を含まないだけで、そのまま読める。

### Database

- `node_events` のテーブル定義は変えない。既存カラム（`event_type` / `detail` / `session_id`）に新しい event_type の行が増えるだけである。
- 取り込み対象の候補出しのために索引を1つ新設する。既存の索引は `(node_execution_id, seq)` / `(kind, tree_id)` / `(session_id, tree_id, seq)` の3つで、`event_type` を含むものが無い（`local_event_store/schema.rs:214-221`）。`event_type` を先頭に持つ索引を足さない限り lifecycle 関連の event type 集合を条件とする読みは `node_events` の全表走査になる。手本にできる既存クエリは無い（`list_tree_roots` の `parent_id IS NULL AND event_type = ?1` も索引に載っていない）。索引の追加はスキーマ変更であり、`schema.rs` の版と `require_index` の一覧に加える。
- 新設する access path は2つ。
  - lifecycle 関連の event type 集合（`session_attached` / `process_exited` / `archive_requested` / `resume_requested` / `restore_requested`）を指定し、全 Node の該当行を `node_execution_id`・`seq` 順で返す取得。上の新設索引で引き、この1回の結果から provider session id を持ち lifecycle が `Open` の候補を出す。
  - `Open` と判定済みの対象1件について、実行木 root と `node_execution_id` を指定した最新の `provider_session_title_observed` を取得する読み。既存の `first_row_of_tree` と `latest_row_for_node_with_event_types` を使い、判定で捨てる対象には発行しない。
- Codex の `state_5.sqlite` は読み取り専用の外部 DB である。既存の `threads` 走査（`infrastructure/provider_history.rs:66`）に加えて、id を指定して `name` と `first_user_message` を同じ行から引く読み取りを足す。`name` は Codex の thread name、`first_user_message` は Provider history の第2段だけに使う。rollout は読まない。

### UI/UX

- Session Node の行に rename の入口を出す。`capabilities.canRename` が真のときだけ、既存の Archive / Delete / Close と同じ hover 表示のアイコンボタン（Pencil）を出す（`src/components/workspace/WorkspaceList.tsx:213-283` の並び）。
- bind 前の Session Node の行は、新しい状態分類の色（灰、`text-muted-foreground`）で描画する。色表（`src/components/workspace/WorkflowNodeStatusIcon.tsx:15-23`）に1値足すだけで、アイコンの形は既存どおり `status` から決まる（`同:49-58`）ため、bind 前は灰の回転ローダーになる。`capabilities.canRename` は bind 前に偽なので、灰の行では rename の入口が出ない。
- 押すと行の表示名が単行の入力欄に変わる。初期値は現在の表示名、全選択状態にする。Enter で確定して `rename_workspace_session_node` を invoke し、Escape と blur で取り消す。空白のみの入力は invoke せずに取り消す。
- 確定後の行の再描画は backend からの通知で行う。既存の worktree 単位の変更通知が Workspace ツリーの再取得を起こす（`src/hooks/useWorkspaceTreeNodes.ts:303` の `subscribeAgentSessionChanged`）。frontend 側で表示名を組み立てたり先読みで書き換えたりしない。
- Provider history の行は `candidate.label` だけを描画する（現在の `{candidate.provider} {candidate.providerSessionId}`、`WorkspaceList.tsx:1257` を置き換える）。表示上の切り詰めは既存の `max-w-52 truncate` のままにする。
- archive 済み AgentSession の一覧（`agentSessionLabel`、`WorkspaceList.tsx:105`）は変更しない。

### Algorithm

#### 定期取り込みの間隔（R-012）

session ごとの最終読み取り時刻を持たず、単一の tick カウンタと剰余で決める。

- 基準 tick = 20 秒。
- タイトル未取得の AgentSession は毎 tick 読む → 読み取りの間隔は 20 秒。
- タイトル取得済みの AgentSession は 15 tick ごと（= 5 分）に読む → 読み取りの間隔は 5 分。

規則（`tick`、`has_title`）→ 読むか、と2つの定数は `domain/agent_session/` に置く。session ごとの scheduling state を持たないため、再起動やセッション増減で状態を復元する必要がない。

#### Claude の `ai-title` の読み取り（R-013 / B-016）

transcript は JSONL で、`{"type":"ai-title","aiTitle":...,"sessionId":...}` の行が会話の進行に伴って繰り返し追記される。最後の出現が最新のタイトルである。

読むのは **ファイル末尾の 64KiB だけ**にする。先頭の不完全行は捨て、残りの行を後ろから走査して最初に見つかった `ai-title` を採る。全走査はしない。

64KiB の根拠: ローカルの transcript 100 本で「ファイル末尾から最後の `ai-title` 行までの距離」を測ったところ中央値約 14KiB、最大約 31KiB だった。窓に入らなかった場合はその tick では「未取得」として扱い、未取得の cadence（20 秒）で読み続ける。

`custom-title` 行は読まない（R-009）。`ai-title` 以外の type を解釈しない。

#### Codex の thread name の読み取り

`state_5.sqlite` の `threads` を id で引き、`name` を採る。空文字と NULL は「未取得」とする。rollout ファイルは読まない。

#### Provider history の行ラベル（R-014 / B-017 / B-018）

1. provider のセッションタイトル（Claude は末尾 64KiB の `ai-title`、Codex は `threads.name`）
2. 最初のユーザープロンプト（Claude は transcript 先頭 64KiB にある最初の非 meta user 行、Codex は同じ `threads` 行の `first_user_message`）
3. `Claude 4f3a9b21…` の形（provider 名と provider session id の先頭 8 文字 + 省略記号）

1 と 2 は前後の空白を落とし、空になるものは「無い」として次の段へ落ちる。2 は空白の並びを1個の空白へ畳んで改行を除き、Unicode scalar value で80文字を超えるときは79文字と省略記号にする。この正規化と切り詰めは `provider_history_label` だけが持ち、frontend は既存の CSS `truncate` 以外の規則を持たない。provider session id を短縮せずに載せる経路は作らない（B-018）。Claude は最初のユーザープロンプトを探す先頭 64KiB だけを読み、Codex はタイトルと同じクエリ結果を使うため、いずれも会話本文を全走査しない。

#### 表示名の解決

`WorkspaceTreeNode` が Session Node のとき:

```text
manual_rename
  .or(単独 Session 実行木の root Session Node なら observed_provider_title)
  .unwrap_or(node_name)
```

Session Node 以外はこの規則を通さない。ルート行の投影は `WorkspacePublicRoot::public_title()` が単独 Session 実行木の root だけ Node 自身の表示名を返し、それ以外は従来どおり owner の表示名を返す。

### Infra

- `infrastructure/provider_history.rs` に、ファイル先頭および末尾の指定 byte 数を読む primitive を足す。byte 列と窓境界の byte を返すだけで、JSONL の解釈はしない（`docs/architecture/INFRASTRUCTURE.md`「変換しない」）。行の切り出し、user メッセージの判定、`ai-title` の判定は gateway が行う。
- 同ファイルに、id 集合を指定して `threads` の `name` と `first_user_message` を同じ行から引く読み取りを足す。接続は既存と同じ read-only + no-mutex で開く。
- 定期取り込みのタスクは composition root（`lib.rs`）で `tauri::async_runtime::spawn` する。tick 間隔は domain の定数を読む。CLI 起動経路（`main.rs` の CLI 分岐）では起動しない。
- 新しいプロセス、ネットワーク接続、外部サービスは増えない。

## Alternatives Considered

**provider タイトルを事実として永続化せず、runtime のメモリにだけ持つ。** 採らない。再起動直後は全行が既定値に戻り、paused / archived になった AgentSession は再取得しないため（R-011）タイトルが二度と復元されない。event store に事実として置けば読み側の導出だけで復元できる。

**session ごとに最終読み取り時刻を持って due 判定する。** 採らない。session 単位の scheduling state と、その復元・破棄の面倒が増える。単一 tick と剰余で B-014 / B-015 の上限を満たせる。

**Provider history の列挙時にタイトルも読む。** 採らない。列挙は provider あたり最大 201 件を走査するため、1 ページの表示で 201 ファイルを開くことになる。可視分（`limit` 件）に絞った2段階にする。

**手動 rename を `workspace_tree` の新しい書き込み port で永続化する。** 採らない。`WorkspaceTreeRepository` は「save / CAS を意図的に持たない」読み取り専用の port であり（`domain/workspace_tree/repository.rs`）、そこへ書き込みを足すと Workspace の query 集約が書き込み経路を持つことになる。Session Node と AgentSession は 1 対 1 なので、既存の `AgentSessionRepository` の書き込みに載せられる。

**ルート行の表示名を gateway 側の launch 種別（`workflow_execution_ids`）で分岐する。** 採らない。Issue #1662 の design が退けた形であり、表示規則の所有者が domain と gateway に割れる。集約が既に持っている述語で分岐する。

**取り込み対象を単独 Session 実行木の AgentSession に限定する。** 採らない。R-011 は対象を「活動中の AgentSession」と定めており、実行木の種別で絞る条件は Requirements にない。tick ごとの読み取りが実行木の fold を伴わない設計にすることで、対象を絞らずに full-recompute 経路の増加を避ける。

## Cross-cutting concerns

**provider 側への書き込みを行わない（R-009）。** 追加する外部アクセスはすべて読み取りである。Claude の transcript は open + seek + read のみ、Codex の `state_5.sqlite` は read-only flag で開く。provider に名前を書き戻す経路を作らない。

**読み取り量の上限を設計で閉じる。** Claude transcript は、タイトルには末尾 64KiB、Provider history の第2段には先頭 64KiB の窓だけを読み、最初のユーザープロンプト以外の会話本文を求めて全走査しない。Codex は可視候補の既存 `threads` クエリの同じ行だけを読む。Provider history のラベル入力取得は可視分だけに掛ける。取り込みの対象列挙は lifecycle 関連の event type 集合を条件とする索引付きの一括読みで済ませ、provider session id を持ち lifecycle が `Open` と判定された対象1件ごとにだけ実行木 root と最新タイトルを追加で読む。実行木の fold を tick ごとに走らせず、クエリ本数を履歴の総件数ではなく Open な対象数に比例させる。

**失敗は静かに次の読み取りへ送る。** タイトルまたは最初のユーザープロンプトの読み取り失敗（ファイル欠落、SQLite ロック、形式不一致）は「そのとき未取得」として扱い、事実を作らず、他の session や Provider history の候補の処理を止めない。UI に失敗を出す経路は作らない（Requirements に該当する要求が無い）。ログは warn 止まりにする。

**通知は worktree 単位の既存経路に載せる。** rename の確定とタイトルの変化はどちらも Workspace ツリーの再取得を起こす必要がある。既存の worktree 単位の変更通知を使い、新しい event 名を増やさない。変化が無かった観測では通知しない（5 分ごとの再描画を起こさない）。

## Risks

**Codex の `threads.name` は自動生成の thread name と `SetThreadName` で付けた名前を区別できない。** 同じカラムに入るため、供給源の側では分けられない。R-009 のとおり、Codex についてカスタム名を読み取らないことは保証せず、手動 rename が provider タイトルより優先される規則（R-001）によって rename 済みの表示名を保護する。rename されていない行には `SetThreadName` で付けた名前が出うる。

**`ai-title` が末尾 64KiB の窓に入らない transcript がありうる。** ローカル 100 本の実測では最大約 31KiB だったが、末尾に巨大な出力が連続した session では窓の外に出る。その場合タイトルは未取得のままとなり、Workspace ツリーの行は既定値 `session` を出し続ける。Provider history の行は、先頭 64KiB に最初のユーザープロンプトがあればその冒頭を、無ければ `Claude 4f3a9b21…` を表示する。窓を広げれば読み取り量が増える。

**`ai-title` と `threads.name` は provider の内部形式であり、契約ではない。** provider 側の変更で形が変われば、Releash からは「タイトルが取得できない」としか観測できず、行は既定値に戻る。検知手段は設計に含めていない。
