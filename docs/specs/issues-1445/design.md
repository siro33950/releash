# Design

関連: #1445（milestone 84「Agentチャット安定化」／ Phase 0 ／ D1 ／依存なし）

## 概要

本 Issue は **design gate**（正本ドキュメントの再定義・確定）である。runtime code / 永続化 / UI / typed fixture / compatibility table / `LocalEventTransactionStore` の実装は含まず、後続の実装 Issue（#1446 F7 / #1447 S13 / #1448 S14 / #1449 S15 / #1450 L12 / #1451 P4）が担う。

確定する設計は、Agent 実行設定（mode / Goal / Reasoning effort / launch / permission）を **Rust-owned state** として再定義し、2026-07-07 に決定した旧 `PermissionMode { Ask, Edit, Full } + plan_mode: bool` を、migration / 復旧方針を伴って正式に supersede することである。この設計を milestone 84 の正本 4 文書（audit / vocabulary / lifecycle / presentation）とマイルストーン説明・Phase 依存に、相互に矛盾なく固定する。

正本 4 文書は現時点で該当設計の骨格（vocabulary §9 / §9.5 / §10〜§12、lifecycle I14〜I16、presentation S9 / 確定事項）を既に保持している。したがって本 gate の実質は、**新規大規模執筆ではなく、requirements / behavior に列挙された確定内容が 4 文書・マイルストーン説明・トレーサビリティ表の全体で一貫し欠落なく固定されているかを検証・補完し、閉じること**である。本 `design.md` はその確定内容と検証観点を single source として記述する。

> 仮定（本文中で明示）: 「typed fixture に固定される」「compatibility table」「fail-closed で検出する」等は、本 Issue では**設計としての表現・確定**を意味する。実 fixture・実行時検証・Rust 型のコンパイル可能な実装は #1446 以降で行う（requirements 仮定・2026-07-16 確認済み）。

## 変更対象

本 Issue の成果物は正本ドキュメント群の作成・更新である。runtime code は対象外。

| ドキュメント | 変更種別 | 確定・整合させる内容 |
|---|---|---|
| `specs/milestone-84-agent-chat-stabilization/agent-chat-instability-audit.md` | 追補・整合 | feature 追補、工数（Reasoning effort）の定義、runtime schema drift の実例（Codex 0.139.0 / 0.144.2、Claude SDK 0.3.x / Claude Code 2.1.195）、対応する監査問題 ID（CL/CX/SD/OB/RT/FE/RG/ST）とトレーサビリティ整合 |
| `specs/milestone-84-agent-chat-stabilization/agent-chat-ideal-vocabulary.md` | 確定 | V-D5、V-D10〜V-D12 と、domain / event / read model 型（§9 / §9.5 / §10 / §11 / §12）、トレーサビリティ表、確定事項 |
| `specs/milestone-84-agent-chat-stabilization/agent-chat-ideal-lifecycle.md` | 確定 | I14〜I16、turn 状態機械、queue / workflow / turn 順序、確定事項 |
| `specs/milestone-84-agent-chat-stabilization/agent-chat-ideal-presentation.md` | 確定 | S9a（launch）/ S9b（Session / Goal）/ S9c（workflow）、Agent 実行設定 UX、frontend 実装規約、確定事項 |
| milestone 84 説明・Phase 依存 | 更新 | D1（#1445）を Phase 0 の gate として位置づけ、#1446〜#1451 の依存を D1 完了後に設定 |
| `docs/specs/issues-1445/{requirements,behavior,design}.md` | 本 Issue の spec | design gate 自体の要求・振る舞い・設計。正本 4 文書とは別物（requirements 仮定） |

後続 Issue #1446〜#1451 の normative source は上表の正本 4 文書（audit / vocabulary / lifecycle / presentation）であり、GitHub Issue 本文はtask記述であってcanonical sourceではない。

`docs/specs/issues-1445/` の `requirements.md` / `behavior.md` は正本の設計境界と相互に整合させる。

## アーキテクチャと責務分割

### 所有者の原則

- Agent 実行設定・Goal・Reasoning effort・permission・launch の判断は全て **Rust-owned state** が所有する。frontend は selected / effective / pending / sync projection の mirror に留まり、domain decision と action enablement を所有しない（rust-first-logic、presentation frontend 実装規約 1〜6）。
- 読み取りモデルは Tauri / WebSocket / 将来の native client が同一 backend-owned state を読める形にする（vocabulary §11 の `get_session` / `get_agent_launch` / paged history / watch）。
- full-retention を避ける。Goal 履歴・queue terminal 履歴・configuration snapshot は event log からの projection / paged query とし、current projection に全履歴を保持しない。
- AgentSession / Workflow間のcross-context相関の所有者は**shared kernel**とする。Workflow contextが意味を所有する`workflow_execution_id / node_execution_id / node_name / node_attempt`はshared-kernelの`NodeExecutionRef`で運び、AgentSession domainはWorkflow domainの`NodeDefinitionName`やentityを直接importしない。逆方向もWorkflow domainはAgentSession aggregate/entityを直接importせず、mode / effort / Goal spec / configuration resolutionのshared-kernel contract valueを参照する。両sibling domainが互いのdomain型を直接importする双方向配置は採用しない（`docs/architecture/DOMAIN.md`の別domain関心をsibling domain型へ常設しない規約に従う）。具体的なmoduleファイル配置は#1446以降で確定する。

### aggregate / saga の分離

正本上で mutation aggregate と query read model を次のように分離する（要件: Goal と configuration、および表示都合の分離）。

- **configuration aggregate**: `AgentSessionConfiguration { provider_model_ref, mode, reasoning_effort, revision }`。Goal 参照を含めず、configurationのmutation authorityを所有する。`SessionConfigurationSelected / SessionConfigurationActivated`がdurable authorityである。
- **configuration command / migration load state**: `AgentSessionConfigurationState::Ready(AgentSessionConfigurationLoadState { selected, effective, pending_update, sync_state }) | NeedsConfigurationResolution(..)`。command、legacy migration、send block、rechallengeだけが参照し、query専用`available_actions`を持たない。
- **configuration query read model**: `AgentSessionConfigurationProjection { selected, effective, pending_update, sync_state, available_actions }`。canonical event / projection dataからqueryが直接構築し、mutation authorityにしない。`available_actions`はmodel / mode / effort候補ごとに、Idle、configuration / Goal sync、pending update / transition、control-operation lease、runtime capability、managed policyをRust queryが評価した`enabled / reason / update_timing / requires_bypass_challenge`である。provider runtime gatewayの`AgentRuntimeEvent`はprovider-neutralな`ProviderConfigurationStateObserved(ProviderConfigurationObservation)`までを入力し、full projectionはclient-facingなquery/watch DTOの`SessionConfigurationChanged`として別境界で生成する。
- **Goal aggregate / mutation state**: `AgentGoalState { current_goal, pending_transition, sync_state }`。`AgentGoal` の `goal_id / goal_revision` と pending / sync の不変条件を所有し、configuration revision から独立する。表示・転送用 field は持たない。
- **Goal query read model**: `SessionGoalProjection { current_goal, pending_transition, sync_state, available_actions, latest_transition }`。`query_service_impl` が pinned snapshot lease 上の committed Goal event / projection data と現在の runtime capability / managed policy data から直接組み立てる。Query 側では `AgentGoalState` を再構築・経由せず、`available_actions` と `latest_transition` を domain aggregate へ逆流させない。
- **launch aggregate**: Session 確立前の New Agent。reserved `attempt_id` 単位の durable launch event stream。Session read model に押し込まない。
- **permission / reconciliation saga**: configuration / Goal / launch / turn-start / permission に共通する write-ahead intent → observation → reconciliation の解決 saga。
- **cross-stream port / 基盤**: domainの`LocalEventTransactionRepository`と、その背後の`LocalEventTransactionStore`。`LocalAtomicParticipant::{AgentSession, Workflow}`がdomain-ownedの`AgentSessionDomainEvent / WorkflowDomainEvent`を保ったままheterogeneous participantを表し、launch / Session / Goal / workflow / queueを跨ぐatomicityを単一transaction / CAS / idempotency / global commit barrierで保証する（vocabulary §9.5）。context-specific `LocalCommittedReadRepository`はrequired source群のdurable projection watermarkの共通下限へpinしたsnapshot leaseを受ける`read_at`でevent / versioned projectionを読み、未追随なら`ProjectionBehind`を返し、active leaseがGC horizonを定める。`LocalWatchRepository`はsnapshot/replay判定、common watermark lease取得、subscription/receiver登録を同じcommit境界で行い、live `receive`ではnotice commitへ厳密にpinした`LocalWatchUpdateFence`を返す。同じportの`finish_bootstrap / finish_update / close_watch`がopaque handle単位のbounded buffer、lease解放、受信終了を所有する。巨大な共通event enum、usecaseでの既存persistence schema/JSON/type erasure、独立JSON logへの逐次appendをatomic扱いする経路を設計として禁止する。

### レイヤ責務（実装 Issue が従う境界）

- **domain**: 上記 mutation aggregateの型と不変条件に加え、serialization非依存の`AgentSessionDomainEvent / WorkflowDomainEvent` closed enum、batch、非再帰なcommit receipt、有効期限付きsnapshot lease型、provider-neutralなpermission dimension / effect / residual protection / evidence refと`LocalEventTransactionRepository` portを定義する。commit receiptはどちらのevent variantにも含めず、schema versionやinfrastructure依存を持たない。GLOSSARYで使用禁止の`WorkflowEvent`をdomain語として採用しない。
- **usecase**: discriminated patch の受理、write-ahead intent、canonical event commit、activation、reconciliation、lease / CAS、challenge / gate 検査。cross-stream commitはdomainの`LocalEventTransactionRepository.commit_batch`、一貫queryはrequired sourceを列挙して1回取得したcommon-watermark snapshot leaseを全context-specific `LocalCommittedReadRepository.read_at`へ渡す。usecase-owned `AgentSessionWatchService / AgentLaunchWatchService`は`LocalWatchRepository`とquery serviceを協調させ、bootstrap leaseまたはlive `LocalWatchUpdateFence.snapshot`からclient-facing typed snapshot/deltaをmaterializeして`finish_bootstrap / finish_update`する。SQLite / WAL / serializationを知らず、snapshot取得後にlatest readを混ぜない。lease失効・`ProjectionBehind`時は部分結果を捨ててquery/watch全体を再取得し、disconnect/cancel/正常終了では`close_watch`する。
- **adaptor/gateway**: `LocalEventTransactionRepository`、context-specific `LocalCommittedReadRepository`、`LocalWatchRepository`を実装し、domain eventを`PersistenceEventEnvelope { event_type, schema_version, serialized_payload }` command modelへ変換してinfrastructureのSQLite/WAL clientを呼ぶ。現行`usecase/agent_session/event_log/events.rs::AgentSessionEvent`と`adaptor/gateway/workflow/event.rs::WorkflowEvent`はlegacy persistence schemaとしてgatewayでlazy upcast/importし、domain portへ流用しない。各projectorは無関係なcommitもskip済みとして順番にconsumeし、処理・skip済みの連続上限だけをsourceごとのdurable `applied_through_global_commit_seq`として進める。common readable watermark、snapshot leaseのpin/expiry/GC horizon、watch subscriptionのbounded buffer/receiver/live fence/closeもgatewayが所有する。provider capability・wire型・`ProviderCliCommand`（Claude `/goal`）・Claude/Codex固有permission snapshot・`ClaudeGoalCommandEvidence`・protocol identity照合をservice / command modelへ閉じ、provider-neutralなdomain value / eventへ変換する。完全なprovider evidenceが必要な場合だけ、secret plaintextをredactして暗号化at rest・per-session quota・単一object size上限・TTL・参照認可を持つbounded evidence storeへ置き、domain/eventからは`ProviderEvidenceRef`で参照する。quota超過時にfull bodyをevent/logへfallbackしない。
- **infrastructure**: SQLite/WAL transaction client、CLI/SDK wire、event log など機械的 I/O を提供する。usecase から直接参照しない。
- **frontend**: projection の mirror と表示のみ。

## データモデルまたは型

正本の権威的な型定義は vocabulary §9 / §9.5 / §10 に置く。本節はその確定要点を再掲する（新規型の定義ではなく、gate で固定する内容の一覧）。

### Agent mode

- `AgentMode = Ask | Edit | Plan | Auto | Bypass` の排他的 5 値 enum。独立した `plan_mode: bool` は存在しない。
- 旧 `PermissionMode + plan_mode` を supersede する。

### 旧値 → configuration state migration 写像表

| legacy | resolved AgentMode | command / migration load用 `AgentSessionConfigurationState` |
|---|---|---|
| `plan_mode = true`（permission mode は任意） | `Plan` | `Ready` |
| `plan_mode = false`, `Ask`（legacy readonly 含む） | `Ask` | `Ready` |
| `plan_mode = false`, `Edit` | `Edit` | `Ready` |
| `plan_mode = false`, `Full` | 未確定 | `NeedsConfigurationResolution(ConfigurationResolutionProblem { fields: [{ field: Mode, reason: LegacyBypassConfirmationRequired, ... }], ... })` |

- `plan_mode = true` は permission mode より優先するため、`Full` と併存しても結果は Ready state と `AgentMode::Plan` の 1 通りになる。
- migration は **lazy**（command / migration load stateへの読み込み写像で解釈し、永続化された旧値へ自動 write-back しない）。`Ready` payloadにquery専用`available_actions`を要求しない。
- Mode field に `LegacyBypassConfirmationRequired` を持つcommand側`NeedsConfigurationResolution(ConfigurationResolutionProblem)` の Session は解決前の mode を持たず送信を block し、Bypass 相当の再 challenge を要求する。`Edit` 等へ silent fallback しない。
- migration 対象: SessionMeta / queue item / workflow definition / `WorkflowExecutionStarted` snapshot / DTO。

### configuration patch と commit point

- `ConfigurationPatch = SetModel { model, effort } | SetMode { mode, bypass_confirmation? } | SetReasoningEffort(..)`。1 command 1 concern を型で保証する。`SetMode`でBypassを選ぶ場合はchallenge id / nonceの対が必須で、期限・guard・caller scopeと照合する。`SetModel` のみ model-bound effort selection を同一 semantic patch に含め、`target_model.provider_id == session.provider_id` を Rust で必須検証する。provider 変更は通常 patch でなく別 usecase（turn finalize / handoff 判断 / protocol preflight / 新 launch を伴う）。
- durable intent の commit point は `ConfigurationUpdateRequested { update_id, base_selected_revision, target_revision, patch, applies_from }` の event log append。idempotence は `update_id`。
- selected の唯一の durable commit point は `SessionConfigurationSelected`、effective を進めるのは `SessionConfigurationActivated`。SessionMeta は event から再構築可能な projection / cache（更新失敗は `PersistFailure` + 再投影）。
- `AgentSessionConfigurationProjection` は selected intent / effective provider state / 1 件の pending update / sync stateに加え、Rust評価済みの`available_actions`を持つ。provider・modelを同じrevisionと`TurnStarted` auditに含める。frontendはconfiguration selectorのenabled/reason、restart timing、Bypass challenge要否を再計算しない。

### Reasoning effort（工数）

- selected: `ProviderDefault | Explicit(value)`。
- effective: `Known { value, source: ExplicitSelection | ProviderDefault } | Unknown { selected, expected?, reason }`。provider default 由来の解決済み値も独立 variant ではなく `Known { source: ProviderDefault }` とする。default / 未取得 / 非対応 / effective 不明を `Option` 一つに畳まない。
- **TokenUsage / cost / time / turn / 各種 budget と別概念**として確定する。behavioral signal であり、実使用量・上限を保証しない。停止条件や代替値として usage / budget を混ぜない。
- 選択肢・説明・既定値・反映時点は provider/model capability から取得する。runtime capability API が無い provider は protocol identity に pin した Rust compatibility table を source として明示し、検証不能な model/value は `runtime_available=false` と `unavailable_reason` で表す。

### Goal lifecycle

- current Goal 同時最大 1 件。Completed / Failed も clear / replace までは current 保持。
- write-ahead protocol: `GoalTransitionRequested → adapter apply / ack → GoalSet / GoalTransitioned / GoalCleared`（canonical commit point）。
- operation（set / edit / pause / resume / clear）ごとに strategy / scope / effects 付き `Native | Emulated | Unsupported(reason)`。
- automatic continuation は `AgentMode::Auto` と独立。どの mode でも permission / workflow human checkpoint を維持する。

### Provider identity / protocol drift

- `BackendProtocolIdentity { executable_version, schema_tag, commit_sha, schema_hash, experimental_flags, initialize_capabilities_hash }`。
- initialize 時に compiled generated schema と spawn した CLI/flags/capabilities を照合する。control-plane drift は `ProtocolIncompatible` として fail-closed。Session 確立後は session-level、確立前は durable launch attempt（`ObservedProtocolIdentity` + expected hash + raw control ref）として保存する。
- parse可能なcontent-plane unknown、既知variantのdecode failure、content/controlを分類できないmalformed frame、size上限超過のdurable event / 構造化ログには、payload長・digest・分類（content/control/unclassifiedとdecode failure種別）・固定上限以下のsecret-redacted sampleだけを記録し、full bodyを恒久保存しない。content-plane unknownだけは低強調`Notice(UnsupportedMessage)`でSessionを継続し、それ以外は取得済みの部分identityとbounded summaryを`ProtocolIncompatible`へ保存して新規turnをfail-closedでblockする。完全evidenceが必要な場合だけ上記bounded evidence storeへ保存し`ProviderEvidenceRef`で参照する。具体的な暗号方式・quota値・object size上限値・TTL値は後続Issueで決め、各分類の件数をparityテスト（ST-7）で別々に検証する。

### cross-stream atomicity

- `LocalEventTransactionRepository.commit_batch` は `Vec<LocalAtomicParticipant>` を持つ domain batch を受け、背後の `LocalEventTransactionStore` が実行する。`LocalAtomicParticipant` は `AgentSessionDomainEvent` と `WorkflowDomainEvent` の各 closed event 型をそれぞれ typed append のまま包み、同じ batch へ混在させる。全 participant の head を CAS し、per-stream seq と global commit seq を割当て、typed payload / batch id / idempotency key / head 更新を単一 durable transaction で commit する（vocabulary §9.5）。
- commit 前 batch はどの query / projector / watch にも見せない。crash が commit 前なら 0 件、commit 後なら全 participant が見える。`batch_id / idempotency_key` の再実行は同じ結果、異なる payload は conflict。
- `LocalAtomicBatchCommitted`はstream、previous/committed head、event countだけを返す非再帰なcommit receiptで、`AgentSessionDomainEvent` / `WorkflowDomainEvent`のどちらにもappendしない。NodeExecution系eventは`WorkflowDomainEvent`だけに置く。
- usecase から見えるwrite境界はdomainの`LocalEventTransactionRepository` portとdomain-facing batch / participant / event型だけである。adaptor/gateway実装がparticipant variantごとのclosed event encoderでpersistence envelopeへserializationし、infrastructureのSQLite/WAL transaction clientへ委譲する。
- query serviceは必要なevent/projection sourceを列挙し、committed global headと各sourceのdurable `applied_through_global_commit_seq`の最小値であるcommon readable watermark以下へ固定した1つの有効期限付き`LocalSnapshotLease`を取得する。全context-specific repositoryへ同じ`read_at(snapshot, query)`を渡してfinally相当でreleaseし、source watermarkがbarrier未満なら`ProjectionBehind`でquery全体を破棄・bounded retryする。active leaseの最小barrierがGC horizonをpinするため、全過去versionは保持しない。watchは`open_watch`が同じstorage transaction / commit lock内でcursorのreplay可否、common-watermark lease取得、barrier後subscription/receiver登録を行う。live `receive`はrequired sourceがnotice commitまで追随してから、そのcommitへ厳密にpinしたlease付きfenceを返す。usecase-owned watch serviceがそのfenceからtyped updateを構築し、同じportの`finish_update`でleaseを解放する。上限超過/lag/追随timeout/lease失効では登録を解放してwatch全体を再取得し、disconnect時は`close_watch`でsubscriptionとleaseを回収する。

### Workflow / UI 型

- launch draft / durable launch attempt / Session projection / Goal projection / workflow template / resolved launch config / queue snapshot を別型にする。
- workflow template: 必須 model/mode は `RequiredOverride::Inherit | Set`、optional effort / initial Goal は `OptionalOverride::Inherit | Set | Clear`。`baseline → WorkflowExecution default → NodeDefinition override` を Rust が解決し `ResolvedLaunchConfiguration`（provenance / revision / canonical hash 付き）を作る。
- Bypass template 保存は権限付与でなく、NodeExecutionごとに既存の`WorkflowExecution.id + NodeExecution.id + NodeDefinition.name（node_name）+ NodeExecution.attempt + resolution_id + resolved hash`へ束縛したone-time challengeとprovider gateを検証する。新しい`NodeDefinitionId`やmigrationは導入しない。

## 処理フロー

以下は正本が保証すべきフローの規範（lifecycle I14〜I16 / turn 状態機械の要約）。

### configuration 更新

1. 初期実装では execution-affecting な user 更新は **Idle 限定**。Session 共通 control-operation lease を取得し `base_selected_revision` を CAS 検証する。非 Synced / Goal transition pending / 別 update 中は provider I/O 前に conflict。
2. `ConfigurationUpdateRequested` を append（durable intent commit point）。
3. live ack または next-turn / restart staging の adapter acceptance 後に `SessionConfigurationSelected` を append（selected の commit point）。
4. live は `SessionConfigurationActivated` append 後だけ effective を進める。`AwaitingNextTurn` / `AwaitingRestart` は activation ack + event 後に `TurnStarted` で反映する（selected と effective を分ける）。
5. `SessionConfigurationSelected` 前に provider が reject を確定した場合だけ `ConfigurationUpdateRejected` を記録し、旧 selected / effective を維持する。selected commit 後の NextTurn / Restart activation reject・timeout は new selected / old effective を維持して `ConfigurationReconciliationRequired` とし、selected を巻き戻さない。partial apply / ack 後の canonical append 失敗 / provider conflict も reconciliation とし、新規 turn / queue drain / workflow resume を block する。

### Goal 遷移

1. Idle かつ configuration / Goal とも Synced・pending 無しの場合だけ受理し lease + `base_goal_revision` CAS。
2. `GoalTransitionRequested` を append。
3. ack 後 `GoalSet / GoalTransitioned / GoalCleared` を canonical append。Claude set/edit は Goal event + `TurnStarted` を単一 atomic batch。
4. reject は旧 current 維持、timeout / 部分成功 / ack 後 append 失敗 / provider conflict は Goal 専用 reconciliation。
5. `ProviderGoalStateObserved` は append 時点で sync を `ObservationPending` にし、同一 observation id を canonical / no-change / reconciliation event が consume するまで新規 turn を block する。

### launch（New Agent / NodeExecution）

`preflight → reservation → (Bypass challenge) → start → provider resource → Session seed → initial Goal handoff` の順で durable に進める。

1. `get_agent_launch_preflight` が `Checking | Compatible(capabilities) | ProtocolIncompatible(partial identity)` を返す。
2. `prepare_agent_launch` が attempt id / canonical draft hash を reserve。Bypass は `AgentLaunchDraftPrepared + BypassChallengeIssued` を同一 local batch、non-Bypass は Prepared 単独。
3. `start_agent_launch` が draft hash / preflight context / policy / gate / provider identity を再検証し、`BypassChallengeConsumed + AgentLaunchAttemptStarted` を atomic append してから provider I/O。
4. provider resource 作成と initial configuration apply/readback 後、`SessionCreated + SessionConfigurationSelected(revision=1) + SessionConfigurationActivated(revision=1) + LaunchStageAdvanced(LocalSessionCommitted)` を launch/session stream 横断の local atomic batch で append し seed する。initial Goal が無い場合は `AgentLaunchCompleted` もこの同じ batch に含め、attempt を durable terminal にする。
5. initial Goal がある場合は launch/Goal stream 横断で `GoalTransitionRequested` を write-ahead し、canonical Goal event / Claude `TurnStarted` / `LaunchStageAdvanced(InitialGoalCommitted)` / `AgentLaunchCompleted` を同一 atomic batch で確定する。initial Goal reject は Goal stream の `GoalTransitionRejected` と launch stream の `LaunchInitialGoalRejected` を同一 atomic batch で write-ahead terminal event として append し、`RetryGoal / ContinueWithoutGoal / CancelSession` を CAS + write-ahead で排他する。

### protocol identity 照合

- identity 一致 → Session 確立。
- control-plane mismatch → `ProtocolIncompatible` で fail-closed（確立後 session-level / 確立前 durable launch attempt）。
- parse 可能かつ content-plane と分類できた unknown message / part → payload 長・digest・content 分類・固定上限以下の secret-redacted sample と、必要時だけ bounded evidence store の `ProviderEvidenceRef` を durable event / 構造化ログへ記録した低強調 `UnsupportedMessage` で継続。full body は event / log へ恒久保存しない。
- 既知 variant の decode failure、未分類・malformed frame、size 上限超過 → payload 長・digest・分類 / decode failure 種別・固定上限以下の secret-redacted sample と、必要時だけ bounded evidence store の `ProviderEvidenceRef`、取得済みの部分 identity を保存し `ProtocolIncompatible` で fail-closed。full body は event / log へ恒久保存しない。

### Auto / Bypass と workflow checkpoint

- Auto / Bypass はいずれも workflow checkpoint を自動で越えない（human 判断を要求する checkpoint は維持）。
- Bypass は Rust の execution-scoped one-time challenge と provider launch gate の両方を経て初めて有効。template 保存は権限付与でない。

## エラー処理

- **reconciliation**: 確定できない結果（timeout / partial apply / ack 後 canonical append 失敗 / provider conflict）は各 aggregate の `ReconciliationRequired` に落とし、新規 turn / queue drain / workflow resume を block する。各 reconciliation に `reconciliation_id` を発行し、local request 由来のときだけ originating id、provider 観測があるときだけ observation id を関連付ける。provider-originated drift のために架空の update / transition を作らない。
- **fail-closed**: control-plane の protocol drift / decode 失敗は `ProtocolIncompatible`。新規 turn を block し、旧値表示へ戻さない。
- **legacy Full**: Mode field に `LegacyBypassConfirmationRequired` を持つ `NeedsConfigurationResolution(ConfigurationResolutionProblem)` は AgentMode 未確定のまま送信 block + 再 challenge。復元不能な legacy / unknown 設定も同じ state（scope / field / raw payload / resolution id / actions 付き）で block し fallback しない。
- **persist と drift の区別**: SessionMeta cache のみの失敗は `PersistFailure` + 再投影で回復し、canonical provider drift と区別する。
- **half-commit 禁止**: cross-stream の途中失敗では participant の一部だけを公開せず launch / turn-start reconciliation へ入る。

## テスト方針

本 Issue は design gate であり、runtime コード・自動テストは書かない。検証は**ドキュメントの整合性検証**として行う（受け入れ基準の充足確認）。

- 旧 V-D10（`PermissionMode` 3 値＋`plan_mode`）が supersede され、migration 写像表・lazy migration・legacy Full の send block と再 challenge・migration 対象一覧が vocabulary / lifecycle / presentation に矛盾なく揃っていることを確認する。
- `AgentGoalState` aggregate と configuration aggregate が完全分離され、`query_service_impl` が `AgentGoalState` を経由せず committed event / projection data から `SessionGoalProjection` を直接組み立てること、Goal が configuration revision から独立した id / revision / pending / sync lifecycle を持つことを確認する。
- discriminated patch / canonical event commit / activation / reconciliation が型・event 表として表現されていることを確認する。
- Reasoning effort が usage / budget と分離され selected / effective / unknown を区別していることを確認する。
- Claude / Codex 差が用途別の型に固定されていることを確認する。Goal は `GoalCapabilitySupport::Native | Emulated | Unsupported`、mode / Bypass は `ModeCapabilitySupport::Native | Composed | Unsupported`、effort は `ReasoningEffortCapability` の schema/runtime validation/readback fields、Codex Goal status と Auto review は exhaustive mapping とする。
- parse可能かつ分類済みのcontent-plane unknownだけが継続し、unknown / decode failure / 未分類・malformed frame / size上限超過のevent/logが長さ・digest・分類・bounded redacted sampleだけを持つこと、完全evidenceは暗号化・per-session quota・object size上限・TTL・参照認可付きstoreの`ProviderEvidenceRef`だけで参照され、secret plaintextとfull bodyがevent/logへ恒久保存されないことを確認する。
- heterogeneous batch が `LocalAtomicParticipant::{AgentSession, Workflow}` でdomain-ownedの各context closed event型を保ち、schema version/serializationを含めず非genericな`LocalEventTransactionRepository.commit_batch`へ渡されることを確認する。legacy persistence schemaはgateway変換だけで扱う。
- 全query sourceがcommon projection watermark以下の同じpinned snapshot leaseで`read_at`され、projector遅延時にbatchのeventとstale projectionを合成しないことを確認する。watchはsnapshot/replay判定とsubscription/receiver登録を同じ境界で行い、各live notice commitへpinしたleaseからusecase-owned serviceがtyped updateを構築し、同じportがhandle単位のbounded buffer・受信・bootstrap/update完了・closeを所有することを確認する。
- frontend が domain decision / action enablement を所有しない設計（available_actions 駆動）になっていることを確認する。
- 各設計内容が対応する監査問題 ID（CL/CX/SD/OB/RT/FE/RG/ST）とトレーサビリティ表に整合していることを、vocabulary / lifecycle / presentation の 3 トレーサビリティ表で相互確認する。
- 実 fixture・実行時検証・コンパイル可能な Rust 型・parity テストは #1446 以降で実装・追加する。

## リスクと代替案

- **正本 4 文書間の不整合リスク**: 型・event・トレーサビリティが 4 文書に分散するため、gate 完了後も後続実装で参照する際にズレが残りうる。緩和策として本 design と 3 文書のトレーサビリティ表を突き合わせ、`#1445〜#1451` 行が全表に存在することを確認する。
- **lazy migration の副作用**: 自動 write-back しないため、legacy Full Session は再 challenge を通すまで送信できない。これは意図的な fail-closed で、data 損失より安全側に倒す判断（memory: legacy 全削除方針とは別に、本件は block + 再 challenge）。
- **runtime schema drift（実在）**: Codex wire 0.139.0 に対し PATH 0.144.2、Claude wire は SDK 0.3.x 参照に対し Claude Code 2.1.195 という drift が 2026-07-15 時点で存在する。設計としては `ProtocolIncompatible` fail-closed で扱い、pin した tag を実装 wire の規範とする。実際の照合実装は #1447〜#1449。
- **代替案: ACP 載せ替え**: 内部正規化プロトコルを ACP へ載せ替える案は不採用（既決）。現行 `AgentRuntimeEvent` の維持・強化とし、Codex は公式クレート pin、Claude は SDK 型定義を正とした typed wire（V-D12）とする。
- **代替案: full-retention read model**: Goal / queue / launch 履歴を current projection に全保持する案は不採用。paged query + event log projection とする（full-retention 回避原則）。
- **代替案: 逐次 append による疑似 atomicity**: 独立 JSON log への順次 append を atomic 扱いする案は不採用。`LocalEventTransactionStore` の単一 transaction を必須とする（vocabulary §9.5）。

## 仮定

- 本 Issue（#1445 D1）の成果物は正本ドキュメントの確定（design gate）のみであり、runtime code / テスト / Rust 型定義 / typed fixture / compatibility table / `LocalEventTransactionStore` のコード実装は含まず、後続 Issue（#1446〜#1451）が担う（2026-07-16 確認済み）。
- spec-id は `issues-1445`。`docs/specs/issues-1445/` に requirements / behavior / design の 3 文書を置く。正本 4 文書（`specs/milestone-84-agent-chat-stabilization/`）は本 Issue が更新対象とする既存 canonical docs であり別物である。
- Agent mode は `Ask / Edit / Plan / Auto / Bypass` の排他的 5 値 enum（V-D10 改訂で確定済み）。
- provider 仕様の規範入力は V-D10 に列挙された Claude / Codex 公式ドキュメントと、dependency に pin した CLI / SDK tag が生成する schema・fixture。living docs は根拠、実装 wire の規範は pin した tag。
- 内部正規化プロトコルは現行 `AgentRuntimeEvent` の維持・強化とし、ACP 載せ替えはしない（既決）。Codex は公式クレート pin、Claude は SDK 型定義を正とした typed wire（V-D12）を前提とする。
- 正本 4 文書は本 gate 着手時点で該当設計の骨格を既に保持しており、本 Issue の作業は新規執筆ではなく確定・整合・欠落補完である。

## Open Questions

なし（2026-07-16 に全て解消済み）。
