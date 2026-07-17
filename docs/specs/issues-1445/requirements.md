# Requirements

関連: #1445（milestone 84「Agentチャット安定化」／ Phase 0 ／ D1 ／依存なし）

## Type

design gate（正本ドキュメントの再定義・確定）。runtime code の実装は含まない。Agent 実行設定（mode / Goal / Reasoning effort）を Rust-owned state として再定義し、旧 `PermissionMode { Ask, Edit, Full } + plan_mode: bool` を正式に supersede する設計を、milestone 84 の正本 4 文書とマイルストーン説明に確定する。この gate が閉じてから、実装 Issue（#1446 F7 / #1447 S13 / #1448 S14 / #1449 S15 / #1450 L12 / #1451 P4）へ進む。

## 背景と目的

milestone 84「Agentチャット安定化」は、`specs/milestone-84-agent-chat-stabilization/agent-chat-instability-audit.md` で確定した監査問題を逐一解消する doc 駆動の方式で進める。その中で Agent 実行設定は、`agent-chat-ideal-vocabulary.md` の設計判断 V-D10 として「2026-07-07 の `PermissionMode` 3 値＋`plan_mode: bool` 決定」を supersede し、mode / Goal / Reasoning effort を Rust-owned な一群として扱う方向へ 2026-07-15 に改訂された。

現行の正本には次の未確定・不整合が残っており、実装（#1446 以降）に着手する前に設計として閉じる必要がある。

- 旧 `PermissionMode + plan_mode` を supersede する新 domain（configuration / Goal / Reasoning effort / launch / permission）の境界と、旧値からの migration・復旧方針が、正本全体で一貫して確定していない。
- Goal mutation state と configuration aggregate、および Goal read model の分離、discriminated patch、canonical event commit、activation、reconciliation、`LocalEventTransactionStore` による cross-stream atomicity が、正本で型として表現され切っていない。
- Reasoning effort（工数）が TokenUsage / cost / time / turn / 各種 budget と別概念であることの確定と、selected / effective / unknown の区別が正本に固定されていない。
- Claude / Codex の provider 差（mode 写像、Goal capability、effort capability、Auto review、Bypass gate）が、用途別 capability 型または exhaustive mapping として typed fixture に固定されていない。
- pin した schema と実行 binary の identity 照合（`BackendProtocolIdentity`）と、control-plane drift を `ProtocolIncompatible` として fail-closed にする方針が確定していない。2026-07-15 時点で Codex wire contract 0.139.0 に対し PATH は 0.144.2、Claude wire は SDK 0.3.x 参照に対し Claude Code は 2.1.195 という実 drift がある。

本変更の目的は、これらを milestone 84 の正本 4 文書とマイルストーン説明に**設計として確定**し、以降の実装 Issue が参照できる single source を提供することである。

## スコープ

この Issue の成果物は、次の正本ドキュメント群の作成・更新（design の確定）である。runtime code・永続化・UI の実装は含まない。

1. `agent-chat-instability-audit.md`: feature 追補、工数（Reasoning effort）の定義、runtime schema drift の実例（Codex 0.139.0 / 0.144.2、Claude SDK 0.3.x / Claude Code 2.1.195）を反映する。
2. `agent-chat-ideal-vocabulary.md`: V-D5、V-D10〜V-D12 と、domain / event / read model 型を確定する。
3. `agent-chat-ideal-lifecycle.md`: I14〜I16、queue / workflow / turn 順序を確定する。
4. `agent-chat-ideal-presentation.md`: S9a（launch）、S9b（Session / Goal）、S9c（workflow）を確定する。
5. milestone 84 説明と Phase 依存を更新する。

確定する設計内容（正本に型・表・fixture・migration 表として固定するもの）:

- **Domain / read model 境界の分離**: configuration mutation authority は `AgentSessionConfiguration { provider_model_ref, mode, reasoning_effort, revision }`（Goal 参照を含めない）、durable authority は canonical `SessionConfigurationSelected / SessionConfigurationActivated` event とする。Goal aggregate は `AgentGoalState { current_goal, pending_transition, sync_state }` とし、canonical Goal eventとともにGoalのmutation/durable authorityを所有する。`AgentGoalState` は Command 側だけで利用する。`SessionGoalProjection { current_goal, pending_transition, sync_state, available_actions, latest_transition }` は query が pinned snapshot 上の committed Goal event / projection data と runtime capability / managed policy data から直接組み立てる read model とし、domain aggregate を再構築・経由しない。`AgentSessionConfigurationProjection` もcanonical event / projection dataから組み立てるread modelで、model / mode / effort候補ごとのRust評価済み`available_actions { enabled, reason, update_timing, requires_bypass_challenge }`を持ち、Idle / sync / pending / lease / capability / policyに基づく設定操作の可否をfrontendへ委譲しない。
- **Agent mode**: `Ask / Edit / Plan / Auto / Bypass` の排他的 5 値 enum を採用する。旧 3 値＋`plan_mode: bool` を supersede する。
- **discriminated patch / commit point**: `ConfigurationPatch::{SetModel{model, effort}, SetMode, SetReasoningEffort}` により 1 command 1 concern を型で保証し、event log append を durable intent / canonical commit point、SessionMeta を再構築可能な projection / cache として定義する。projection は selected / effective / pending / sync（reconciliation）state を持ち、provider / model を同じ revision と `TurnStarted` audit へ含める。
- **Goal lifecycle**: current Goal 同時最大 1 件（Completed / Failed も clear/replace までは current 保持）、`GoalTransitionRequested → adapter apply/ack → GoalSet/Transitioned/Cleared` の write-ahead protocol、operation ごとの strategy / scope / effects 付き `Native / Emulated / Unsupported`、automatic continuation を `AgentMode::Auto` と独立させることを定義する。
- **Reasoning effort（工数）**: selected は `ProviderDefault | Explicit(value)`、effective は `Known { value, source: ExplicitSelection | ProviderDefault } | Unknown { selected, expected, reason }`。TokenUsage / cost / time / turn / 各種 budget と別概念として確定し、選択肢は provider API または protocol identity に pin した compatibility table 駆動とする。
- **更新確定と reconciliation**: user 起点の execution-affecting 更新は初期実装で Idle 限定とする。`SessionConfigurationSelected` 前の provider reject だけは旧 selected / effective を維持する。selected commit 後の NextTurn / Restart activation reject・timeout は new selected / old effective を維持して `ReconciliationRequired` とし、selected を巻き戻さない。partial apply / ack 後の canonical append 失敗 / provider conflict も `ReconciliationRequired`、activation ack ＋ event 後に `TurnStarted` とする。
- **cross-stream atomicity / consistent read**: domain側の`LocalEventTransactionRepository` port（domain-facing batch / event型、`commit_batch`、snapshot lease）をusecaseが利用し、adaptor/gateway実装がserializationとinfrastructureのSQLite/WAL clientを呼ぶ。batchはdomain-owned `AgentSessionDomainEvent / WorkflowDomainEvent`を保った`LocalAtomicParticipant::{AgentSession, Workflow}`のheterogeneous participant列を受け取り、巨大な共通event enum、既存のserde persistence schema、JSON/type erasureをusecase境界へ持ち込まない。schema versionはgatewayのpersistence command envelopeだけが所有し、legacy `AgentSessionEvent / WorkflowEvent` schemaはgatewayでupcast/変換する。背後のRust-owned`LocalEventTransactionStore`が単一transaction / CAS / idempotency / global commit barrierでlaunch / Session / Goal / workflow / queue横断のatomicityを保証する。全context-specific `LocalCommittedReadRepository`はrequired sourceごとのdurable projection watermarkの共通下限へ固定した有効期限付き`LocalSnapshotLease`で`read_at(snapshot, query)`を行い、source未追随時は`ProjectionBehind`を返してhalf-visible readを成功扱いしない。active leaseがGC horizonをpinし、失効・追随遅延時は部分結果を捨ててquery全体をfresh leaseで再取得する。`LocalWatchRepository`は`open_watch`でsnapshot/replay判定・common watermark lease取得・subscription/receiver登録を同じstorage transaction / commit lockで行う。live `receive`はnoticeだけでなく当該commitへ厳密にpinしたleaseを持つfenceを返し、`finish_update`が明示解放する。usecase-owned watch serviceがこのfenceとquery serviceからtyped Session snapshot/deltaまたは`AgentLaunchChanged`を構築し、Tauri / WebSocket handlerはtyped streamだけを送る。lag/上限超過/lease失効/追随timeoutではsubscriptionを解放してwatch全体を再取得する。
- **provider 境界**: domain はprovider-neutralなpermission dimensions / effects / residual protections / `ProviderEvidenceRef`とGoal acceptance evidenceだけを所有する。Claude / Codex固有permission snapshot、`ClaudeGoalCommandEvidence`、raw wire field / control refはadaptor/gatewayのservice / command modelとevidence storeへ閉じ、正規化後にdomain value / eventへ変換する。
- **Provider 差の fixture 固定**: Claude `/goal`（`ProviderCliCommand`・set は `StartsTurn` を伴う・pause/resume emulation の effects）、Claude Bypass（dangerous launch opt-in ＋ Rust challenge の追加 policy）、Claude effort（runtime capability / readback が無い場合は pin した version × model compatibility table で送信前検証し、検証不能は `runtime_available=false` と `unavailable_reason` で表す）、Codex Goal status（active/paused/complete/blocked/usageLimited/budgetLimited を全域写像・read-only accounting を `ReasoningEffort` と分離）、Codex Auto review（approved/denied だけ Auto 解決、inProgress/timedOut/aborted は activity/未解決へ）を typed fixture として固定する。
- **Protocol identity**: `BackendProtocolIdentity { executable_version, schema_tag, commit_sha, schema_hash, experimental_flags, initialize_capabilities_hash }` を定義し、compiled generated schema と spawn した CLI/flags/capabilities を initialize 時に照合する。parse 可能かつ content-plane と分類できた unknown message / part だけを低強調 `UnsupportedMessage` へ着地させる。control-plane drift、既知 variant の decode failure、content/control を分類できない malformed frame は raw ref と部分 identity を保存して `ProtocolIncompatible` として fail-closed（Session 確立後は session-level、確立前は durable launch attempt）にする設計を確定する。
- **migration / supersede**: 旧 `permission_mode` / `plan_mode` から `AgentMode` と `AgentSessionConfigurationState` への写像表を確定する。`plan_mode=true` は permission mode より優先して Ready state の `Plan`、`Ask/legacy readonly` は Ready state の `Ask`、`Edit` は Ready state の `Edit`。`plan_mode=false` の `Full` は mode を未確定のまま `NeedsConfigurationResolution(ConfigurationResolutionProblem { fields: [{ field: Mode, reason: LegacyBypassConfirmationRequired, ... }], ... })` とする。lazy migration（自動 write-back しない）、legacy Full Session の send block と再 challenge、migration 対象（SessionMeta / queue item / workflow definition / `WorkflowExecutionStarted` snapshot / DTO）も確定する。
- **Workflow / UI 型**: launch draft、durable launch attempt、Session projection、Goal projection、workflow template、resolved launch config、queue snapshot を別型にし、workflow model/mode を `Inherit/Set`、optional effort / initial Goal を `Inherit/Set/Clear`、baseline → WorkflowExecution default → NodeDefinition override を Rust で解決、Bypass template 保存は権限付与でなく NodeExecution ごとに challenge / provider gate を検証、という設計を確定する。NodeExecution相関は新しいdefinition IDを導入せず、既存の`WorkflowExecution.id / NodeExecution.id / NodeDefinition.name（node_name）/ NodeExecution.attempt`に束縛する。
- **最終設計ゲート追補（2026-07-15）** の各点（工数 = behavioral signal、aggregate/saga 分離と共通原則、launch の preflight→reservation→(Bypass challenge)→start→provider resource→Session seed→initial Goal handoff、initial Goal reject の write-ahead terminal event、`LocalEventTransactionStore` の単一 transaction、compiled schema と実行 binary/flags/capabilities の照合と部分 identity を含む `ProtocolIncompatible` の fail-closed 保存）を正本に反映する。

## 非スコープ

- runtime code の実装。`AgentSessionConfiguration` domain・永続化・migration の実装は #1446（F7）で行う。
- 5 Agent mode の cross-backend mapping 実装は #1447（S13）。
- Reasoning effort（工数）の cross-backend 設定実装は #1448（S14）。
- Agent Goal の cross-backend lifecycle 実装は #1449（S15）。
- Workflow / queue / restart への Agent 設定継承の実装は #1450（L12）。
- Goal・推論レベル・5 モードの UI 実装は #1451（P4）。
- `LocalEventTransactionStore`（SQLite WAL 等）の実装。設計の確定のみ本 Issue に含める。
- provider 固有の Goal budget 設定、token/cost/time budget、厳密な上限、TokenUsage / cost の accounting 追加。
- ACP への載せ替え（内部正規化プロトコルは現行 `AgentRuntimeEvent` の維持・強化とする、既決）。

## 要求事項

- Agent 実行設定の新 domain（configuration / Goal / Reasoning effort / launch / permission）が Rust-owned state として、旧 `PermissionMode + plan_mode` を supersede する形で正本に定義されていること。
- 旧 V-D10（`PermissionMode` 3 値＋`plan_mode`）が、migration / 復旧方針（写像表・lazy migration・legacy Full の send block と再 challenge・migration 対象一覧）を伴って supersede されていること。
- `AgentGoalState` aggregate と configuration aggregate が正本上で完全に分離され、Goal は configuration revision から独立した id / revision / pending / sync lifecycle を持つこと。表示用の `SessionGoalProjection` は domain aggregate ではなく query read model として分離されること。
- discriminated patch（`ConfigurationPatch`）、canonical event commit、activation、reconciliation が型として正本に表現されていること。
- Reasoning effort が TokenUsage / cost / time / turn / budget と別概念として確定し、selected / effective / unknown を区別して正本に表現されていること。
- Claude / Codex の provider 差が、Goal は `GoalCapabilitySupport::Native | Emulated | Unsupported`、mode は `ModeCapabilitySupport::Native | Composed | Unsupported`、effort は `ReasoningEffortCapability` の validation/readback fields、Codex status / Auto review は exhaustive mapping として typed fixture に固定されていること。
- pin した schema と実行 binary の identity mismatch を fail-closed（`ProtocolIncompatible`）で検出する設計が正本に定義され、control-plane と content-plane を区別していること。
- frontend が domain decision / action enablement を所有せず、selected / effective / pending projection の mirror に留まる設計であること。
- Auto / Bypass が workflow checkpoint を越えず、Bypass challenge / provider gate を Rust が強制する設計であること。
- 上記の設計内容が、正本 4 文書（audit / vocabulary / lifecycle / presentation）とマイルストーン説明・Phase 依存に、相互に矛盾なく反映されていること。
- 各設計内容が対応する監査問題 ID（CL-x / CX-x / SD-x / OB-x / RT-x / FE-x / RG-x）およびトレーサビリティ表と整合していること。

## 受け入れ基準の概要

- [ ] 旧 V-D10 が migration / 復旧方針を伴って supersede されている。
- [ ] `AgentGoalState` aggregate と configuration aggregate が分離され、Query 側は aggregate を経由せず `SessionGoalProjection` read model を直接構築する。
- [ ] discriminated patch、canonical event commit、activation、reconciliation が型で表現される。
- [ ] Reasoning effort が usage / budget と分離され、selected / effective / unknown を区別する。
- [ ] Claude / Codex 差が用途別 capability 型または exhaustive mapping として typed fixture に固定される。
- [ ] parse 可能かつ分類済みの content-plane unknown だけを継続し、分類・decode できない payload と schema / binary mismatch を fail-closed で検出する（設計として）。
- [ ] heterogeneous atomic batch がdomain-ownedのbounded context別closed event型を保つ明示的participant sum typeで表現され、schema version/serializationはgateway command modelへ閉じる。read/queryは全required projection sourceのcommon watermark以下へpinされ、projector遅延中にhalf-visible readを返さない。watchは各live commitへpinしたleaseからusecase-owned serviceがtyped frameを構築し、portがhandle単位の受信・bounded buffer・bootstrap/update完了・closeを所有する。
- [ ] frontend が domain decision / action enablement を所有しない設計になっている。
- [ ] Auto / Bypass が workflow checkpoint を越えず、Bypass challenge / provider gate を Rust が強制する設計になっている。
- [ ] 正本 4 文書とマイルストーン説明・Phase 依存が相互に矛盾なく更新されている。

## 仮定

- 本 Issue（#1445 D1）の成果物は**正本ドキュメントの確定（design gate）のみ**であり、runtime code・テスト・Rust 型定義・typed fixture・compatibility table・`LocalEventTransactionStore` などのコード実装は一切含めず、後続の実装 Issue（#1446〜#1451）が担う（2026-07-16 確認済み）。「typed fixture に固定される」「compatibility table」「fail-closed で検出する」等の受け入れ基準は、本 Issue では**設計としての表現・確定**を意味し、コード上の実 fixture / 実行時検証の実装は #1446 以降で行う。
- spec-id は `issues-1445` とし、`docs/specs/issues-1445/` 配下に requirements / behavior / design の 3 文書を置く。正本 4 文書（`specs/milestone-84-agent-chat-stabilization/` 配下）はこの Issue が更新対象とする既存の canonical docs であり、`docs/specs/issues-1445/` とは別物である。
- Agent mode は `Ask / Edit / Plan / Auto / Bypass` の排他的 5 値 enum を採用する（V-D10 改訂で確定済み）。
- provider 仕様の規範入力は正本 V-D10 に列挙された Claude / Codex 公式ドキュメントと、dependency に pin した CLI / SDK tag が生成する schema・fixture とする。living docs は根拠、実装 wire の規範は pin した tag とする。
- 内部正規化プロトコルは現行 `AgentRuntimeEvent` の維持・強化とし、ACP 載せ替えはしない（既決）。Codex は公式クレート pin、Claude は SDK 型定義を正とした typed wire（V-D12）を前提とする。

## Open Questions

なし（2026-07-16 に全て解消済み）。
