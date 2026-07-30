> **要求は requirements.md が正、配置は規約が正。**
> 本書が示す設計方針は要求を満たすためのものである。一方、本書に現れる個別のファイル・型の配置や分類は判断の結果ではなく参考にすぎない。どのコードをどの層へ置くかは `docs/architecture/` の規約（DOMAIN / USECASE / GATEWAY / INFRASTRUCTURE / CONTROLLER / TEST）を各コードに当てて決めること。本書の配置記述が規約と食い違う場合は規約に従う。

# Design

## The actual design

### Architecture

本設計は `docs/architecture/DOMAIN.md`、`docs/architecture/USECASE.md`、`docs/architecture/GATEWAY.md`、`docs/architecture/INFRASTRUCTURE.md`、MS84 正本群、`docs/specs/issues-1499/design.md`、および workflow 側の先行実装 `src-tauri/src/domain/workflow/entities/workflow_execution/` を根拠とする。#1499 が確立した operation / obligation / terminal / fixed SQLite の authority は維持し、session lifecycle の判断主体だけを domain へ移す。

#### 集約境界

`src-tauri/src/domain/agent_session/aggregates/session/` に `Session` 集約を新設し、session identity、canonical `SessionState`、現在の Turn、bounded な queue state、現在の permission state、provider recovery の既知事実を所有させる。集約は全 message、全 part、全 turn history を保持せず、現在の遷移と受理判定に必要な bounded state だけを保持する。

集約内外の関係は次に固定する。

| Concept | Owner / relation |
| --- | --- |
| Session lifecycle | `Session` 集約 root。open / closed / archived、現在の turn と queue の組合せ、不変条件、操作の受理可否を所有する |
| Turn | `Session` 配下の entity。identity、canonical `TurnPhase`、terminal、現在の permission / tool settlement を所有し、単独では永続化しない |
| Queue | `Session` 配下の bounded child state。accepted item の順序と identity、pause latch、次 item の開始可否を所有する。input 本文の authority は既存 accepted operation / obligation のまま |
| Permission | 対象 Turn 配下の entity。request identity と現在の応答状態を所有し、別 Session や別 Turn へ late result を適用しない |
| Operation / obligation | `src-tauri/src/domain/local_event/` が所有する独立集約。session 集約へ吸収せず、既存 identity、閉じた遷移、recovery action を維持する |
| Configuration / Goal | 今回の集約境界変更の対象外。既存state / persistence契約を維持し、Session集約は受理判断に必要なrevision / capability factだけを参照してI14〜I16の意味を再定義しない |
| Message / history / presentation | SQLite の event・message projection と QueryService が所有する read model。Session 集約へ full-retention しない |

operation / obligation の存在を必要とする操作では、`AgentSessionLifecycleRepository` が同じ SQLite snapshot の Session projection と owner-scoped obligation view を取得し、domain recovery service が後者を「未解決 recovery があるか」という fact に分類する。usecase はその fact を Session 集約へ渡し、Session 集約だけが自身の lifecycle / quiescence と組み合わせて最終的な受理可否を決める。domain service は独立集約を跨ぐ recovery 分類に限定し、操作の受理可否を返さない。

obligation の状態語彙は現在二つの表現を持つ。`domain/local_event/record.rs` の `ObligationStateRecord`（Prepared / Pending / EffectReserved / Running / WaitingApproval / OutcomeUnknown / ReconciliationRequired / Failed / Completed / Cancelled）と、`domain/agent_session/events.rs` の `ObligationState`（Pending / EffectReserved / Completed / ReconciliationRequired / Cancelled）である。後者は `adaptor/gateway/agent_session/session_storage/stored_event_v1.rs` の `StoredObligationStateV1` と相互写像され、永続 schema V1 に含まれる。同じ概念の粒度違いの二重表現であり `docs/architecture/DOMAIN.md`「一つの概念に一つの表現」に反するが、一本化は永続 schema の evolution を伴うため本変更では扱わない（Non-goals 参照）。本変更では両者の意味と写像を変更せず、Session 集約はどちらも保持しない。

`src-tauri/src/domain/agent_session/entities/session.rs` の DTO 型 `Session` と未使用の `entities/message.rs` は削除する。`entities/turn.rs` と `entities/permission_request.rs` は集約 child として実行経路に接続し、Turn result、interrupt、token usage などの値語彙は `value_objects/` の一つの定義へ統合する。`SessionState` は `value_objects/session_state.rs`、`TurnPhase` は `value_objects/turn_phase.rs` を唯一の semantic definition とし、usecase の `SessionState` / `TurnPhase` と `RuntimeSessionPhase` は廃止する。persistence record と public DTO は異なる名前を持つ境界表現として残してよいが、受理判定や遷移を持たせない。

既存 `src-tauri/src/domain/agent_session/services/workflow_turn_admission.rs` の述語は `Session` 集約の workflow turn admission に統合し、同ファイルと facts DTO を削除する。workflow 側は送信する実行 checkpoint の正当性を引き続き判断し、session 集約は open、quiescent、未解決 recovery なしだけを判断する。

#### 3層への分解

主要な変更対象と責務は次のとおり。

| Path | 変更後の責務 |
| --- | --- |
| `src-tauri/src/domain/agent_session/aggregates/session/` | send / workflow turn / Stop / permission / close / archive / backend switch の受理、Turn と queue の遷移、terminal 収束、late observation の fencing |
| `src-tauri/src/domain/agent_session/services/` | 独立 operation / obligation の view を Session が消費する recovery fact へ分類する。Session 操作の最終受理判断は持たない |
| `src-tauri/src/domain/agent_session/repository.rs` | versioned な bounded Session 集約を読み、aggregate change を既存 atomic batch の CAS participant として準備する repository 境界 |
| `src-tauri/src/usecase/agent_session/operation/` | domain decision、operation / obligation participant、transaction boundary、effect dispatch の順序を調停する command usecase |
| `src-tauri/src/usecase/agent_session/runtime/` | provider observation を domain command へ変換する駆動手順、recovery / shutdown の orchestration、post-commit effect の調停だけを持つ |
| `src-tauri/src/usecase/agent_session/session/` | session の QueryService と read DTO。lifecycle state machine と persistence codec を持たない |
| `src-tauri/src/adaptor/gateway/agent_session/` | `AgentBackend` / `AgentSessionRuntime` と `AgentSessionLifecycleRepository` の具体実装。local-event の `AgentSessionProjectionRecord`・owner-scoped obligation view と Session restore / change set の写像 |
| `src-tauri/src/adaptor/gateway/local_event_store/` | local-event record と Stored V1 / SQLite の写像、および既存 `LocalEventTransactionRepository` の snapshot query・atomic commit 実装。Session 集約との写像は持たない。本変更では現在の層配置を維持する（下記の既知の乖離を参照） |
| `src-tauri/src/infrastructure/agent_session/{codex,claude}/` | 外部世界の都合をその形のまま扱うコード（[INFRASTRUCTURE.md](../../docs/architecture/INFRASTRUCTURE.md)） |
| `src-tauri/src/adaptor/gateway/agent_session/{codex,claude}/` | 変換するコード（[GATEWAY.md](../../docs/architecture/GATEWAY.md)） |
| `src-tauri/src/adaptor/gateway/notification/` | commit 後の既存 notification 配送 |
| `src-tauri/src/adaptor/controller/agent_session_operation_wiring.rs` | transport 入力の変換と usecase / gateway の composition のみ |

**対象範囲**: `src-tauri/src/infrastructure/agent_session/` 配下の全コード。

**判定**: 配置は本書では決めない。[INFRASTRUCTURE.md](../../docs/architecture/INFRASTRUCTURE.md) の一問——**変換しているか**——を、対象範囲の各コードに当てて決める。この一問以外の基準（port を実装しているか、ドメイン型を参照しているか、外部ライブラリを呼ぶか）を判定に持ち込まない。それらは規約が層を分ける基準ではない。

**単位**: 判定はコード単位であり、ファイル単位ではない。一つのファイルに外部世界の事実の宣言と変換が同居することがあり、その場合はファイル内で切り分ける。ファイルを丸ごと移すことで判定を代替しない。

この点は実在の反例で確認されている。`claude/models.rs` は `impl AgentBackend` を持つが、中身は `CLAUDE_FIXED_MODELS`（Claude が提供するモデルの一覧）、backend id、表示名、`capabilities`、CLI パスの既定値という**外部世界の事実の宣言**が大半で、変換は `available_models()` の写像だけである。port を実装しているという理由でファイルごと gateway へ移すと、外部世界の事実が gateway に居座る。`skills.rs` も同様にファイルシステムへの接触を含む。

**結果の検証**: 移設後、対象範囲の各コードについて、そこに置いた理由を「変換しているか」で説明できること。説明できないコードが残っていれば判定が未了である。

**既知の乖離（本変更では是正しない）**: 同じ判定基準を `local_event_store` に当てると、SQLite の接続・DDL・トランザクション機構と `StoredEventV1` のような保存形式の表現は infrastructure に属し、SQL（DML）と codec が gateway に属する。しかし現状はいずれも `adaptor/gateway/local_event_store/` にある。本変更は既存 schema と commit authority を変えないことを前提とするため、この再配置は対象外とする（requirements.md Non-goals）。provider を是正して local_event_store を残すのは一貫性を欠くが、#1499 の atomic commit 契約に触れる変更を本フェーズへ持ち込まない判断による。

`usecase/agent_session/runtime/usecase.rs` は monolith として廃止し、同 directory の turn 駆動、provider event 適用、recovery、streaming、shutdown の各 orchestration moduleへ分割する。`usecase/agent_session/session/store.rs` も廃止し、domain decision、QueryService、SQLite repository / codec へ上表どおり移す。既存の `AgentSessionRuntimeUsecase` と `SessionStore` を参照する composition は、互換 facade を一時的に置く場合も lifecycle 判断を持たず、同一変更内で新 owner へ接続する。

`RuntimeSessionState` に残せるのは provider handle、stream buffer、timer、process-local lease と canonical identity / revision fence だけである。session / turn / queue / permission の状態を別 enum や独自 boolean の組合せで判断しない。`AgentStatusCenter` も backend-owned read model mirror として維持し、admission authority にはしない。

#### Transaction と concurrency

各 command は、Session 集約と owner-scoped pending obligation view を一つの bounded SQLite snapshot から復元する。domain recovery service が view を recovery fact に分類し、Session 集約がその fact と exact revision に対して domain command の最終受理可否と change set を決める。domain が返す event / state change は、operation receipt、obligation、terminal、message projection など同じ利用者操作の participant とともに既存 `LocalAtomicBatch` へ入り、`LocalEventTransactionRepository::commit_batch` の一回の CAS で確定する。

provider、notification、workflow callback などの effect は commit 成功後にだけ解放する。commit outcome が不明なら effect を新 identity で開始せず、既存 operation identity の readback / reconciliation へ戻す。provider I/O 待機中は SQLite transaction と session lock を保持せず、結果適用時に session revision、turn / operation identity、runtime generation を再確認する。late result は一致した元 operation にだけ適用する。

normal completion、interrupt、Fatal、Session lifecycle、shutdown の競合は、Session 集約が canonical terminal candidate と queue / permission settlement を一つの decision として作り、既存 terminal CAS が winner を一つにする。既存 `domain/local_event/record.rs` の obligation 遷移は変更せず、terminal winner と同じ batch で必要な obligation participant を確定する。

#### Phase A と Phase C の文書更新

`agent-chat-ideal-lifecycle.md` の設計原則へ `L-P7（domain lifecycle authority）` を追加し、Session 集約、usecase、gateway、infrastructure、controller の責務と「遷移は集約経由のみ」を定める。既存 L-P1〜L-P6 と I1〜I17 の意味は変更しない。

`agent-chat-instability-audit.md` は再監査 baseline の commit、調査日、各 finding の stable ID、再分類、根拠、残存 owner を持つ台帳へ再構築する。既存 66 ID は変更・再利用せず各一回だけ残し、新規 finding は `NF-001` から別の stable 連番を付ける。分類は「構造整理で解消」「残存」「新規発見」の閉じた値とする。

`phase-plan.md` は再監査台帳を入力として、Phase 3 以降の各 Issue に「維持」「#1561へ吸収してclose」「残存 finding に合わせて再スコープ」「新規 finding のowner」のいずれか、根拠 finding ID、結果の phase / hard dependency を記録する。Issue 本文の契約はこの変更では編集せず、計画上の routing だけを更新する。

### Interface

この構造変更は既存 public contract を壊さず、public API version の更新を行わない。domain 型を直接 serialize せず、gateway / presenter / protocol の既存境界表現へ明示的に写像する。

内部境界は次に固定する。

| Interface | Responsibility |
| --- | --- |
| `Session` aggregate methods | canonical state と recovery fact / observation を受け、全 Session 操作の最終的な typed admission / transition outcome と change set を返す。I/O は行わない |
| `AgentSessionLifecycleRepository` | 既存 `LocalEventTransactionRepository` の一つの bounded snapshot query を使い、`AgentSessionProjectionRecord`・owner-scoped obligation view を同一 snapshot から取得する。前者を versioned Session restore input へ写像し、後者は domain recovery service の分類入力へ写像して返す。Session change set は CAS participant に写像し、単独 commit は行わない。**port はドメインの層に置く以上ドメインの言語だけで書く**（[DOMAIN.md](../../docs/architecture/DOMAIN.md) port）。`AgentSessionProjectionRecord` や obligation view のような保存形式側の語彙を、引数・戻り値・エラーに出さない。これらは実装の内側で消費し、境界を越えるのはドメインの型だけである |
| `LocalEventTransactionRepository` | session、operation、obligation、terminal、projection participant の既存 atomic commit / readback authority |
| `AgentBackend` / `AgentSessionRuntime` | `adaptor/gateway/agent_session/{codex,claude}/` が `infrastructure/agent_session/{codex,claude}/` の provider client を domain gateway contractへ変換し、provider session の open、turn start、interrupt、permission response、event stream という外部作用だけを提供する |
| `AgentSessionNotificationGateway` | durable commit 後の notification を配送し、domain decision を変更しない |

現在の `SendAdmissionGate`、`StopAdmissionGate`、`SessionLifecycleGate`、`PermissionResponseGate` は lifecycle decision interface として廃止する。snapshot / mutation preparation / effect execution が混在しているため、snapshot と CAS participant の準備は `AgentSessionLifecycleRepository`、受理と遷移は `Session`、provider effect は `AgentSessionRuntime` に分ける。

domain rejection は invalid lifecycle、not quiescent、unresolved recovery、stale target などの型を保って usecase まで運び、usecase が既存の rejection / `SafeOperationFailure` へ写像する。storage failure、provider failure、commit outcome unknown の既存公開分類は維持し、controller や gateway が error string を解析して分類しない。

### Data Model

| Record | Identity / owner | Retention |
| --- | --- | --- |
| `Session` aggregate | `session_id`。domain owner | 現在の lifecycle、current / last turn fact、queue head / order、pause、current permission、recovery fenceだけを保持 |
| `Turn` entity | Session 内の `turn_id` | 現在の turn と terminal 判定に必要な bounded fact。過去 turn body は保持しない |
| Queue child | accepted `queue_item_id` と対応 operation / reserved turn identity | pending item の identity / orderだけ。canonical input は既存 obligation recordを参照 |
| Permission child | `(turn_id, request_id)` | 現在の request と解決事実。transcript 表示は message projectionへ分離 |
| Operation / obligation | #1499 の既存 identity と `domain/local_event` owner | 既存 retention、pending index、recovery resultを変更しない |
| Session / status DTO | QueryService / protocol owner | domain state と同型の表示値を持つ非権威 read model |

`SessionState` の既存六値と公開表現、`TurnPhase` の既存公開表現は維持する。`SessionState::Done` / `Error` は直近 terminal の projection でもあるため、それ単独を admission gate に使わず、集約が current Turn、queue、recovery と組み合わせて判断する。

domain record の新しい永続 version は導入しない。既存 persistence record と canonical mutation identity を維持し、`adaptor/gateway/agent_session/` の写像だけを domain aggregate restore / change set に対応させる。

### Database

fixed SQLite の既存 schema と table をそのまま使用し、schema evolution は行わない。Session 集約の復元は `session_projection` の identity lookup と、既存 owner index を使う未解決 obligation の有無を同じ reader transaction で取得する bounded access path にする。全 event history、全 Session、全 pending recovery pageへの fallbackは持たない。

write は既存 session stream、session / message projection、operation、obligation、terminal、pending index を同じ `LocalAtomicBatch` で更新する。aggregate repository が独自 transaction や第二の store を作らず、SQLite commit 後の read model だけを公開する。旧 file-store は引き続き探索・参照・変換・変更しない。

### UI/UX

該当なし。

### Algorithm

aggregate restore は、`adaptor/gateway/local_event_store/` が一つの reader snapshot から検証済み `AgentSessionProjectionRecord` と owner-scoped obligation view を返し、`adaptor/gateway/agent_session/` が前者を domain の `SessionRestore` へ写像する。domain recovery service は後者を recovery fact に分類し、`Session` が restore invariant を検証して生成された後、その fact を含む command の最終受理可否を決める。message projection はこの restore に含めない。event-log projector は message / history read model の構築に限定し、session lifecycle の受理規則は呼び出さない。

command 適用は「同一 snapshot の recovery fact を exact revision の集約 candidate に渡して domain command の最終受理判断を行い、change set を既存 operation participant と一括 commitし、成功後だけ effect を解放する」という共通 transaction procedureを使用する。workflow turn も通常 send と同じ procedureを通り、controller が active turn / queue / recovery factsを別々に組み立てない。

startup recovery は既存 bounded pending inventory の item ごとに Session と obligation を復元し、domain recovery service が `applied / already-applied / reconciliation / terminal` の既存意味へ分類する。一 item の failure は他 Session を止めず、closed / archived は reopen しない。retry / restart は元の operation・obligation identity と revisionを使い、別 identity の作用へ変換しない。

再監査の判定規則は次のとおり。

| Classification | 判定根拠 |
| --- | --- |
| 構造整理で解消 | finding の原因経路が新 aggregate / gateway 境界へ置換され、対応する既存 behavior と構造検査の双方が通る |
| 残存 | 利用者影響または保証 gap が整理後も再現し、Phase 3 以降の owner Issue を特定できる |
| 新規発見 | 元の66件にない独立した利用者影響または保証 gap を整理後の実行経路で確認できる |

台帳検査は、元の66 IDが各一回存在すること、全 row が閉じた分類を持つこと、残存 / 新規 finding が一つの owner routing を持つこと、#1565で解消済みの WorkflowTurn gate finding が根拠付きで反映されることを確認する。phase plan検査は台帳の残存 / 新規 ID と routing の双方向対応、吸収 / close / 再スコープの重複なし、hard dependency が前 phaseだけを指すことを確認する。

### Infra

該当なし。

## Alternatives Considered

- 既存 `entities/session.rs` の `messages: Vec<Message>` をそのまま集約へ育てる案: lifecycle判断に不要な全履歴を aggregateへ持ち込み、full-retentionと巨大な transaction candidateを作るため不採用。
- operation / obligation を Session 集約へ吸収する案: #1499 の独立 identity、pending recovery、shutdownとの共有、閉じた遷移を変更し、同じ概念を二重実装するため不採用。
- `workflow_turn_admission` を standalone serviceのまま残し、他の操作だけを集約へ移す案: facts assemblyとquiescence解釈が controller / aggregateに分裂するため不採用。
- 新しい aggregate schemaへ一括 migrationする案: 現在の bounded projectionから必要なstateを復元でき、schema変更はoperation replayとcanonical identityに不要な互換riskを加えるため不採用。

## Cross-cutting concerns

- Crash consistency: aggregate changeだけを先行公開せず、既存SQLite CASとoperation identity readbackをcommit authorityにする。
- Concurrency: session単位の短いcandidate作成をrevisionでfenceし、provider I/O中はlockを保持しない。late resultはsession / turn / operation / runtime generationの一致を必要とする。
- Performance: aggregate restoreとadmissionはbounded projectionとowner-indexed obligation existence lookupだけを使い、message countとevent history量から独立させる。
- Security: public failureとnotificationへprovider raw payload、canonical input、path、SQLを追加せず、既存safe failure mappingを維持する。
- Verification: R-003 / R-004 と B-001 は、domain aggregate の遷移・受理・terminal・recoveryの単体検証、serialization互換とrestart / replayの永続化検証、既存behaviorを通るproduction compositionの契約検証で担保する。R-001 / R-002 / R-003 / R-007 は観測可能な振る舞いを持たない構造要件であり、振る舞い定義を持たない（behavior.md 参照）。R-003 は上記の集約単体検証、R-007 は移設後の層配置と production 経路の確認で担保する。semantic state型と受理authorityの一本化は、本変更の完了確認としての source 走査（旧 `SessionState` / `TurnPhase` / `RuntimeSessionPhase` と旧 gate が production 経路から消えたことの確認）と production call-path 検証で確認する。これは本変更に閉じた一回限りの完了確認であり、一般規則を常設検査として encode するものではない（規約の強制手段については `docs/architecture/DOMAIN.md` を参照）。

## Risks

- persistence表現とdomain aggregateの相互写像が既存canonical identityを変える可能性がある。serialization互換とrestart / replayの永続化検証で検出する。
- `SessionState`の履歴projectionとcurrent Turn lifecycleを誤って同一視すると、workflow turnやqueue admissionが退行する。aggregate admissionの単体検証で検出する。
- session、operation、obligation、terminalを別commitにすると、#1499のatomicityとrecoveryが壊れる。repositoryはparticipant準備に限定し、commit authorityを`LocalEventTransactionRepository`一つに保つ。
- process-local runtime mirrorに旧booleanや`RuntimeSessionPhase`の判断が残ると第二のstate machineになる。完了確認としての source 走査とproduction call-path検証で検出する。
- 66件の再分類をコード移動だけで解消扱いにすると行動gapを見落とす。各「構造整理で解消」は対応behaviorの検証結果とcode evidenceの両方を必須にし、根拠が不足する項目は「残存」にする。
