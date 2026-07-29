> **要求は本書が正、配置は規約が正。**
> 本書が定める要求・スコープ・受け入れ条件は満たすべきものである。一方、本書に現れる個別のファイル・型の配置や分類は判断の結果ではなく参考にすぎない。どのコードをどの層へ置くかは `docs/architecture/` の規約（DOMAIN / USECASE / GATEWAY / INFRASTRUCTURE / CONTROLLER / TEST）を各コードに当てて決めること。本書の配置記述が規約と食い違う場合は規約に従う。

# Context

- 入力: [Issue #1561](https://github.com/siro33950/releash/issues/1561) — [Agentチャット安定化] session ライフサイクルの Domain 整理 — 集約確立・usecase 3分解・再監査と audit 再構築（OPEN）。要求の正本。
- 補助資料:
  - `specs/milestone-84-agent-chat-stabilization/agent-chat-ideal-lifecycle.md` — MS84 正本。設計原則 L-P1〜L-P6 と不変条件 I1〜I17 を定義。本変更で原則を追加する対象。
  - `specs/milestone-84-agent-chat-stabilization/agent-chat-instability-audit.md` — 監査台帳 66 件（CL 7 / CX 11 / SD 7 / OB 8 / RT 8 / FE 7 / RG 9 / ST 9）。本変更で再構築する対象。
  - `specs/milestone-84-agent-chat-stabilization/phase-plan.md` — MS84 計画。本 Issue を Phase 2 として配置済み。Phase 3 以降の Issue（約 30 件）は Phase 2 の再分類が出るまで凍結中。
  - `docs/architecture/DOMAIN.md` — 「モデルが実行を担う」全体規約。port を内側に置く理由と、domain 層の port がドメインの言語で書かれること。
  - `docs/architecture/INFRASTRUCTURE.md` / `docs/architecture/GATEWAY.md` — infrastructure と gateway の境界。判定は「変換しているか」。
  - `specs/workflow-lifecycle/workflow-ideal-lifecycle.md` — #1559 / #1565 で確立した workflow 側の先行事例 spec。
- 背景: #1559 で診断され #1565 で是正された workflow 側の構造問題（ライフサイクルを表現する集約が domain に不在で、遷移判断が手続きに散在する）と同型の問題が agent session 側にも存在する。workflow 側は #1565 が「集約 + 決定サービス群を domain に確立し、runtime host を gateway へ移設する」形で実装を完了しており、本変更はその session 版である。MS84 Phase 1（#1491）完了後、Phase 3 以降の行動修復 Issue 群の開始前に、壊れた構造へパッチを積み続けないための構造整理として位置づけられている。
- 制約:
  - #1499 で確立した operation / obligation / SQLite authority の契約（受理済み operation の意味論）は変えない。変えるのは表現主体の所在である。
  - MS84 の行動保証（I1〜I17）は維持する。構造の再配置であり、保証水準の変更ではない。
  - #1565 が実装済みの接点を二重定義しない: obligation（`src-tauri/src/domain/local_event/record.rs` の閉じた遷移）と WorkflowTurn 送信可否述語（`src-tauri/src/domain/agent_session/services/workflow_turn_admission.rs`）。後者は集約設計へ統合する。
  - Phase 3 以降の Issue の契約内容は、再監査で判断が出るまで変更しない。

# Outcome

- 対象者: Releash の開発者（MS84 Phase 3 以降の行動修復 Issue 群を実施し、agent session の安定化を進める者）。
- 現在の問題: session ライフサイクル（状態・遷移・受理・不変条件）を表現する主体が domain に存在せず、状態型が usecase / domain に二重定義され、ビジネスルールが巨大な usecase 手続きに埋没している。この構造のままでは行動修復の各修正が回帰しやすく、audit 台帳の finding も構造要因と行動問題が未分離のまま残る。
- 変更後に実現する状態: session のライフサイクルは domain の集約が表現し、遷移・受理判断は domain に一本化される。MS84 spec に表現主体の原則が明文化され、整理後の構造で再監査した結果に基づき audit 台帳と Phase 3 以降の計画が再構築される。

# Current Behavior

2026-07-29 時点の worktree（branch `feature/1561`）で確認した現状。コード参照は `src-tauri/src/` からの相対。

- `domain/agent_session/entities/` には Session / Turn / Message / MessagePart / PermissionRequest / Attachment が存在するが、`#[allow(dead_code)]` が付いた DTO 的残骸であり、ライフサイクル（状態・遷移・受理・不変条件）をメソッドとして表現する集約になっていない。`docs/architecture/DOMAIN.md` の「モデルが実行を担う」規約に反する。
- `SessionState` が usecase（`usecase/agent_session/session/mod.rs:157`）と domain（`domain/agent_session/entities/session.rs:25`）の両方に定義され、adaptor（`adaptor/controller/agent_session_operation_wiring.rs`）が変換している。`TurnPhase`（`usecase/agent_session/status.rs`）/ `RuntimeSessionPhase`（`usecase/agent_session/runtime/session_state.rs`、`pub(crate)`）は usecase 層にのみ存在する。
- `SessionState` はイベントログからの導出 projection（`usecase/agent_session/event_log/projector.rs` の `project_status`）であり、「このコマンドをこの状態で受理してよいか」を答える集約がない。#1565 が導入した決定サービス `decide_workflow_turn_admission` は WorkflowTurn 送信ゲートを是正したが、facts の組み立て（active turn / pending queue / unresolved recovery の判定）は adaptor に残っており、受理判定一般を表現する主体は不在。
- `usecase/agent_session/runtime/usecase.rs`（23,891 行）と `usecase/agent_session/session/store.rs`（10,729 行）に、状態機械・provider I/O・通知・復旧・shutdown が埋没している。
- `agent-chat-ideal-lifecycle.md` の設計原則（L-P1〜L-P6）は行動保証を定義するが、表現主体（誰がライフサイクルを表現するか）の原則がなく、Entity・集約の語彙は登場しない。L-P5 は Rust vs frontend の境界であり、Rust 内部で domain が空洞であることは規定外。
- `agent-chat-instability-audit.md` は 66 件の finding を保持するが、main `b3f9f54c` 付近の実装を基準としており、#1565 の是正（WorkflowTurn 送信ゲート関連等）や構造整理の効果は反映されていない。
- provider 実装が層規約に反している。`docs/architecture/INFRASTRUCTURE.md` は infrastructure を「外部世界の都合をその形のまま扱い、変換しない層」と定め、変換は gateway の責務としている。しかし `infrastructure/agent_session/` には変換が多く含まれる（wire とドメインイベントの相互変換、ドメインの permission entity と wire の相互変換、ドメインサービスの呼び出し等）。どのコードがどちらに属するかは規約の一問を各コードに当てて判定する必要があり、本書では個々の配置を決めない。判定がファイル単位で割り切れないことは実在の反例で確認済みである（`claude/models.rs` は `impl AgentBackend` を持つ一方、中身の大半は提供モデルの一覧・backend id・表示名・capability という外部世界の事実の宣言である）。
- obligation の状態語彙が二重表現になっている。`domain/local_event/record.rs` の `ObligationStateRecord`（10 値）と `domain/agent_session/events.rs` の `ObligationState`（5 値）が同じ概念を異なる粒度で表す。後者は `adaptor/gateway/agent_session/session_storage/stored_event_v1.rs` の `StoredObligationStateV1` と相互写像され、永続 schema V1 に含まれる。

# Scope / Non-goals

## Scope

- `agent-chat-ideal-lifecycle.md` の設計原則への「ライフサイクルの集約表現とエンジンの domain 所在」の追加（Phase A）。
- session ライフサイクルの集約境界の設計: Session / Turn / queue / permission / obligation の集約関係、導出 projection と受理判断の関係、既存 entities の dead_code 残骸の扱いと SessionState 二重定義の解消方針の確定（Phase A）。
- `usecase/agent_session/runtime/usecase.rs` / `session/store.rs` の3分解 — (i) 遷移・決定を domain の集約 + 決定サービスへ、(ii) 駆動手順・トランザクション境界を usecase へ、(iii) 外部世界との接触を gateway と infrastructure へ（変換は gateway trait 実装、外部世界の都合をその形のまま扱う部分は infrastructure）（Phase B）。
- 状態型（SessionState / TurnPhase / RuntimeSessionPhase）と受理述語の domain への一本化、`workflow_turn_admission` の集約設計への統合、adaptor に残る facts 組み立ての置き場所見直し（Phase B）。
- provider 実装の層違反の解消 — 対象範囲は `infrastructure/agent_session/` 配下の全コード。`docs/architecture/INFRASTRUCTURE.md` の一問「変換しているか」を各コードに当てて配置を決める。判定はコード単位であり、ファイル単位では割り切れないためファイル内での切り分けを伴う（Phase B）。
- 整理後の構造での不安定性再監査と、audit 台帳 66 件の再分類（「構造整理で解消」「残存」「新規発見」）を含む台帳再構築（Phase C）。
- 再分類結果に基づく Phase 3 以降の Issue 群の再編（吸収・close・再スコープ）の phase-plan.md への反映（Phase C）。

## Non-goals

- MS84 の行動保証（I1〜I17）の水準変更。本変更は構造の再配置であり、外部から観測される session の振る舞いを変えない。
- #1499 で確立した operation / obligation / SQLite authority 契約の変更。
- obligation の閉じた遷移（#1565 実装済み）および WorkflowTurn 送信可否述語の再実装・二重定義。
- obligation 状態語彙の二重表現（`ObligationStateRecord` 10 値と `ObligationState` 5 値）の一本化。後者は永続 schema V1（`StoredObligationStateV1`）に含まれ、一本化は schema evolution を伴う。本変更は「domain record の新しい永続 version を導入しない」ため対象外とし、別 Issue で扱う。本変更では両者の意味と写像を変更しない。
- provider 固有の process / wire 実装そのものの変更。移設は層の分割に限り、Claude / Codex が要求する形式・手順は infrastructure に閉じたままとする。
- `local_event_store` の層配置の是正。`docs/architecture/INFRASTRUCTURE.md` に照らすと、SQLite の接続・DDL・トランザクション機構と `StoredEventV1` のような保存形式の表現は infrastructure に属し、SQL（DML）と codec が gateway に属する。しかし現状はいずれも `adaptor/gateway/local_event_store/` にある。本変更は「既存 schema と table をそのまま使用し、schema evolution を行わない」ことを前提としており、この再配置は #1499 の commit authority に触れるため対象外とする。別 Issue で扱う。
- Phase 3 以降の行動修復 Issue 群の実施。各 Issue の契約内容の変更（再編は phase-plan.md への反映に留める）。
- frontend の変更。

# Requirements

- R-001: `agent-chat-ideal-lifecycle.md` の設計原則に、ライフサイクルの集約表現とエンジンの domain 所在の原則（session のライフサイクルは domain の集約がメソッドとして表現し、遷移は集約経由でのみ起きる。usecase は駆動手順とトランザクション境界、gateway は外部世界の都合と内側の言語の相互変換、infrastructure は外部世界の都合をその形のまま扱うことに限る。adaptor / usecase の手続きに受理判定の独自解釈を置かない）が追加されている。
- R-002: session のライフサイクル状態型と受理判断が domain に一本化されている。SessionState の usecase / domain 二重定義が解消され、usecase / controller に SessionState の独自解釈（直接の `matches!` 等による受理判定）が残っていない。#1565 導入の WorkflowTurn 送信可否述語は集約設計へ統合され、二重定義されていない。
- R-003: `usecase/agent_session/runtime/usecase.rs` / `session/store.rs` が3分解され、遷移・決定（受理判定、状態遷移、terminal 収束、recovery 判断）が domain の集約メソッドの単体テストで検証されている。
- R-004: 互換性 — MS84 の行動保証（I1〜I17）に対応する既存テストが引き続き通り、#1499 の operation / obligation / SQLite authority 契約が変更されていない。
- R-005: 整理後の構造での再監査が完了し、`agent-chat-instability-audit.md` が再構築されている（66 件の各 finding が「構造整理で解消」「残存」「新規発見」に再分類され、#1565 是正済みの finding が反映されている）。
- R-006: 再分類の結果に基づく Phase 3 以降の Issue 群の再編（吸収・close・再スコープ）が `phase-plan.md` に反映されている。
- R-007: provider 実装の層違反が解消されている。`infrastructure/agent_session/` に変換が残っておらず（ドメインの語彙と wire の相互変換、port の実装、ドメインサービスの呼び出しがない）、それらは `adaptor/gateway/agent_session/{codex,claude}/` にある。プロセス起動・stdio transport・wire 契約の定義は infrastructure に残り、adaptor 層へ引き上げられていない。

# Assumptions / Open Questions

- なし。
