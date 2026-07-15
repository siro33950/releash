# Agent チャット表示（presentation）の理想形

作成日: 2026-07-07
更新日: 2026-07-15（Agent 実行設定 UI を追加）

milestone 84「Agentチャット安定化」のドキュメント群:

- [agent-chat-instability-audit.md](agent-chat-instability-audit.md) — 問題点インベントリ（全 66 件、要求リスト）
- [agent-chat-ideal-vocabulary.md](agent-chat-ideal-vocabulary.md) — 正規化語彙・データ構造の理想形
- [agent-chat-ideal-lifecycle.md](agent-chat-ideal-lifecycle.md) — ライフサイクルの理想形（不変条件）
- **agent-chat-ideal-presentation.md（本書）** — UI 表示の理想形

本書は「backend が持つ情報を、どの surface に、どう表示するか」の正本を定義する。監査で確定した presentation 問題群（FE 群）と、語彙拡張で新たに表示対象になる要素（thinking / plan / notice / stop reason / usage 等）の表示先を確定する。frontend は Rust backend が所有する read model の mirror であり（CLAUDE.md 原則 / ST-8）、本書は「何を映すか」を決める文書であって frontend にロジックを置く根拠にはしない。

## 表示原則

- **P1 (live / reload 等価)**: 画面は read model のみから描画できる。transient event は read model 更新を早く届ける手段であり、live と reload 後で表示が変わってはならない。streaming delta は seq で順序・欠落を検出し、欠落時は snapshot 再取得で自己修復する（FE-3、seq 契約は語彙文書 §11）。唯一の例外は `Notice(PersistFailure)` — 永続化自体の故障を知らせるため transient 表示を許容する（lifecycle I8）。
- **P2 (表示先の一意性)**: 各語彙要素に primary surface を 1 つ定める。同一情報の二重描画を禁止し、補助 surface は要約・導出値のみを表示する。
- **P3 (無言遷移の禁止)**: ユーザーに観測可能な状態変化（turn 終了・失敗・中断・permission 失効・queue 変化・Agent 設定変更）は、必ず画面上の変化を伴う。「スピナーが消えただけ」「reload したら突然エラーが現れる」（FE-2）を許さない。
- **P4 (スコープの一致)**: バナー・エラー表示は対象スコープ（session / turn / app）の surface にのみ表示する。session を跨いだグローバル状態での表示を禁止する（FE-5）。
- **P5 (入力の保全)**: 送信失敗・queue 操作・stall のいずれでも、入力欄の内容と添付は消えない（lifecycle I6 の表示面）。
- **P6 (監査可能性)**: 後からセッションを開いた読者が「何が実行され、どの mode / Goal / 推論レベルだったか、なぜ止まり、何が拒否され、いくら使ったか」を durable read model と履歴から読み取れる（RG-4 / FE-7 / CL-1 effective / RG-9 / #1445〜#1451）。

## Surface 定義

| ID | surface | 実体（現行） | 責務 |
|---|---|---|---|
| S1 | transcript | `ChatSessionView` | parts の時系列描画。会話の正本表示 |
| S2 | activity log | `ActivityLog` | 実行中 turn の tool 進行の要約ストリーム |
| S3 | todo フッター | todo 表示部 | 最新 `TodoListSnapshot` の常時表示 |
| S4 | permission UI | `PermissionDialog` ＋ transcript 内 permission part | 回答の受付と履歴表示 |
| S5 | 入力エリア | `MessageInput` ＋ queue chips | 入力・送信・queue 操作 |
| S6 | session バナー | チャットパネル上部（session-scoped） | セッション単位の警告・エラー |
| S7 | usage indicator | 入力エリア上部（`MessageInput` 上縁、新設） | token / context / cost の常時要約 |
| S8 | session バッジ | セッション一覧・タブ | セッション状態（Running / Waiting / Error）の要約 |
| S9a | launch configuration | New Agent dialog（新設） | preflight→prepared reservation/challenge→durable `AgentLaunchAttempt`。provider-ack済みSession stateとは分離 |
| S9b | Session 設定 strip | `MessageInput` 上部（新設） | selected/effective configuration、current Goal、各pending/reconciliation、protocol compatibilityのprimary surface |
| S9c | workflow Agent 設定 | workflow step editor（新設） | model/modeのInherit/Setとeffort/initial GoalのInherit/Set/Clearを編集。Session stateとは別artifact |

## 語彙 × 表示先マトリクス

vocabulary 文書の語彙要素ごとに primary / 補助 surface と表示規則を定める。

| 語彙要素 | primary | 補助 | 表示規則 |
|---|---|---|---|
| `Text` | S1 | — | 現行通り。streaming 追記 |
| `Thinking` | S1 | — | 折りたたみブロック（streaming 中は自動展開、完了で自動折畳）。**backend 共通**（CX-3/RG-1: Codex でも表示）。`redacted` は「非公開の思考」プレースホルダ。Task 配下（`parent_tool_use_id` あり）も Task 展開内に描画する（FE-6） |
| `ToolCall` | S1 | S2 | kind 別アイコン、`status` バッジ（Running spinner / Succeeded / Failed / **Denied / TimedOut / Interrupted を色・文言で区別**（RG-4））、`exit_code` バッジ（RG-8）、output は text ＋ **image**（CL-6/RG-7）を描画。Running 中の出力は追記表示（SD-5）。WebSearch は query と結果要約を表示（CX-11） |
| `TodoListSnapshot` | S3 | S1 | `in_progress` 項目をハイライト（スピナー付き）、priority 表示（RG-5）。Claude / Codex 共通（CX-5/RG-2） |
| `Notice` | kind による（下表） | — | 下記「Notice 振り分け」 |
| `Error`（part） | S1 | S6 | `retryable=true` は「再試行中」表示にし、`resolved=true` へ更新されたら成功扱いに畳む（CX-8: 恒久の赤エラーにしない） |
| `Permission` | S4 | S1 | 下記「permission UX」 |
| `TaskStatus` / Task 配下 | S1 | S2 | Task 展開内に thinking / tool / 未 pair の tool result も描画。要約の 200 字切りは全文展開可能にする（FE-6） |
| `TurnResult` | S1 | S8 | 下記「turn 終端の表示」 |
| `TokenUsage` | S7 | — | context 使用率バー＋ token 数＋ cost（FE-4/RG-9）。turn 中は `TokenUsageUpdated` で逐次更新 |
| `AgentMode` | S9b | S8 | `Ask / Edit / Plan / Auto / Bypass` を排他的 selector で表示。Plan toggle は置かない。Auto は provider reviewer の範囲、Bypass は危険性を常時明示 |
| `ReasoningEffort` | S9b | — | UI名は「工数（推論レベル）」。selected ProviderDefault/Explicit、effective Known(value/source)またはUnknown(selected/expected/reason)、runtime availability、option/description/defaultを広告順で表示し、TokenUsage/cost/budgetと同じcontrolにしない |
| `SessionGoalProjection` | S9b | S6/S8 | current Goal、pending transition、sync state、latest evidence、Rust評価済みavailable actionsとstrategy/scope/effectsを表示。Completed/Failedは根拠付きoutcome |
| `AgentSessionConfigurationProjection / TurnStartState` | S9b | S6 | selected/effective provider/model/mode/effort revision、provider generation、pending/observation/recovery/resolution、StartingTurn/ReconciliationRequiredを描画。activation前をeffective表示しない |
| `AgentLaunchPreflight / PreparedAgentLaunch / AgentLaunchAttempt` | S9a | S6 | Checking→Compatible→draft reservation/challenge→startを表示。draft変更でreservation失効。full projection＋after_seq watchでreload/reconnect復元 |
| `AgentConfigurationTemplate / ResolvedLaunchConfiguration` | S9c | — | baseline/Run/Nodeのoverride、revision、field provenanceを表示し、解決previewはRust queryの結果だけを描画 |
| `BackendProtocolIdentity` | S9b | S6 | 互換なら通常は詳細内。schema/binary/flag不一致はProtocolIncompatibleとしてerror表示しsendをdisable |
| turn configuration revision | S1（turn 詳細） | — | TurnStartedのimmutable effective snapshotからprovider/model/mode/effective effort/unknown reason/Goal/protocol identityに加え、当時のprovider permission/effects/residual protections/context hashを展開し、後から監査可能にする |
| turn_phase / stall | S1 末尾 ＋ S8 | S2 | StartingTurnは「開始を確定中」、Streamingはspinner、Interruptingは「停止中（最大10秒）」。stall診断で観測停止を表示し、turn-start reconciliation中はsendを止める |
| queue | S5 | — | chips: Queued / AwaitingBypassConfirmation / Starting / Started / Paused / Failed / Cancelled / NeedsResolutionを可視化。snapshot/goal ref/current差とexecution idを表示し、Bypass確認の期限切れは再prepare、その他の解決不能差はCAS付きrebase/取消を提示 |
| session 状態 | S8 | S6 | Error 時は理由（最後の Fatal / TurnError の要約）を tooltip で表示（RT-6）。reload 後も理由が残る（durable Notice 由来） |

### Notice 振り分け

| NoticeKind | 表示先 | 規則 |
|---|---|---|
| Compaction | S1 inline | 進行 → 完了 / **失敗**（SD-7）まで単一ブロックで遷移 |
| ModelRerouted | S1 inline ＋ S6（一時） | 「モデルが X に変更された」を明示（CX-7） |
| ConfigWarning / Deprecation / GuardianWarning | S6 ＋ S1 inline | セッション開始時警告はバナー、turn 中発生は inline（CX-7/RG-6） |
| RateLimit | S7 に反映 ＋ S1 inline（閾値超過時のみ） | 最新値は usage indicator の導出値として表示（RG-6） |
| McpServerStatus | S6 ＋ S1 inline | 接続失敗・認証切れを明示（CL-5） |
| UnsupportedMessage / classified OversizeDropped | S1 inline（低強調） | content-planeと確定できた「未対応の応答をN件受信」など。未分類/malformed/oversize frameはS6のProtocolIncompatible（V-P1） |
| ProtocolIncompatible | S6（error）＋S1 inline | control-planeのschema/binary/flag/capability drift。protocol identityと対象を表示し、新規sendをdisableする |
| PersistFailure | S6（error） | 保存失敗はバナーで明示（I8）。永続化故障を知らせる性質上、transient 表示を許容する唯一の例外（P1） |
| Diagnostic | S1 inline | stall 等。転記される診断はユーザーが症状報告に使える文面にする |

### turn 終端の表示

`TurnResult` の表示は stop_reason / 失敗理由を区別する（CL-3/RG-3 の解消）:

| 終端 | 表示 |
|---|---|
| Completed{EndTurn} | 現行通り（明示表示なしで Idle に戻る） |
| Completed{MaxTurns} | 「ターン上限で停止。続行できます」＋続行アクション（送信欄に定型文） |
| Completed{MaxTokens} | 「出力上限で途切れました」＋続行アクション |
| Completed{Refusal} | 「モデルが応答を拒否しました」を明示（workflow へは failure_signal） |
| Failed{TurnError} | S1 に Error block（**live で即時**。reload 後にだけ現れる状態を禁止 — FE-2）＋ S8 バッジ |
| Interrupted{UserAbort} | 「停止しました」チップ |
| Interrupted{Timeout/Crash/SessionClosed} | 理由付き中断チップ（I1/I2 の回収結果を含む） |
| stats | duration / cost / num_turns を終端チップの詳細（hover / 展開）に表示 |

## Agent 実行設定 UX

- **5 mode**: S9bは`Ask / Edit / Plan / Auto / Bypass`の単一selectorとし、旧`permission mode + Plan toggle`を廃止する。schema上の存在とruntime availabilityを区別し、availability source/checked at/unavailable reasonをRust capabilityから表示する。別modeへsilent fallbackしない。
- **危険性**: `Bypass`は影響範囲を説明し、Rustが返すexecution固有guard（Session revision、launch reservation＋draft hash、queue execution＋snapshot hash、workflow run/node/execution attempt＋resolution＋resolved hash、またはscope/action/targetまで含むreconciliation attempt）、target、期限、nonceへ束縛したone-time challengeに対する明示確認を要求する。通常承認を最大限迂回してもexplicit rules、MCP user interaction、provider circuit breaker等の`residual_protections`が残ることを列挙し、「全保護無効」とは表示しない。waiting projectionまたは`get_bypass_challenge`からIssued/Consumed/Expired/Cancelledをreload後も復元し、Issued時だけnonceを確認操作へ使う。確認中に設定やattemptが変わったらchallengeを失効表示して再取得する。provider側launch opt-inが必要ならrestart-requiredを表示する。template保存は権限付与でなく、S9a/S9cもexecution時に新しいchallengeを通す。managed policyによる禁止はUIで解除できない。
- **Auto**: 「無制限」「Releashが承認」とは表現せず、provider側classifier/reviewerがeligibleな要求を審査し、sandboxやmanaged policyは広がらないことを示す。typed ModeEffectから、Claudeのclassifier delegation＋keep-working/質問削減nudgeと、Codexのreviewer swapのみという差を表示する。approved/deniedだけをAuto解決として履歴表示し、inProgressはactivity、timedOut/aborted/manual fallbackは未解決・fallback状態として表示する。
- **工数（推論レベル）**: model選択後にcapabilityからoption、説明、default、schema/runtime availability、source/context/checked at、反映時点を描画する。selectedのProviderDefault/ExplicitとeffectiveのKnown(value/source)/Unknown(selected/expected/reason)を区別し、providerの広告順を維持する。model変更patchではtarget modelとeffortを一緒にpreviewする。Claudeのorganization limit等を含むauthoritative validation/readbackができない明示値は、tableのpreviewとdisabled理由を表示し、effective確定とは見せない。
- **使用実績との分離**: S9b の工数は model の応答・推論強度の signal、S7 は token / context / cost の使用実績である。工数は厳密な token 上限ではなく、時間・turn 数・token / cost / time budget の入力欄を追加しない。
- **Goal**: objective/statusだけでなくpending transition/sync state/latest evidence/provider snapshotをS9bに置く。controlはRustが実行contextで評価した`available_actions`だけを描画し、schema/runtime availability、source/context/checked at、strategy/application scope/effectsを操作前に示す。Claude set/editとclear後の再setが伴うStartsTurn、progress reset、identity replacementを確認文に含め、`--resume / --continue`によるSession復元をGoal Resume/turn開始と表示しない。Completed/Failed/Blockedはevidence付きoutcomeとし、根拠なしのcomplete/fail controlは置かない。historyはpaged queryでkind/result/time/before/after/evidenceを展開し、turnのgoal id/revisionから当時のobjectiveをlookupする。atomic batch失敗時はprovider turn/interrupt状態もreconciliation詳細に表示する。
- **scope 分離**: S9aはlaunch preflight/draft reservation＋start後のdurable launch attempt、S9bは実行Sessionのauthoritative configuration/Goal/turn-start projection、S9cはworkflow definition templateである。見た目を再利用してもstateとcommandを共有しない。S9aはserverのsnapshot/replay＋subscription barrierを使う`after_seq` watchで小さなfull projectionを受け、launch成否不明時は最後に完了したstage、provisional provider resource、local Session、initial Goal handoff/reconciliation、観測値、部分protocol identityとRustが返すcleanup/readback/reuse/recreate操作を表示する。providerが安全なlookup/idempotent createを持たない場合はRecreateを表示しない。initial Goal待ち/明示rejectを区別し、reject時はRetry Goal / Goalなしで続行 / Session取消のRust許可済みactionだけを出す。Session確立前のProtocolIncompatibleもS9aに復元表示する。S9cはrequired/optional overrideとfield provenanceを表示し、実行Sessionのack済み結果と区別する。
- **確定表示**: frontendはdraft/requested/selected/effectiveと、Goal current/pending/syncを分ける。pending中は要求値と旧effective/currentを併記し、next-turn/restart activation前にeffectiveとして見せない。canonical commit失敗や結果不明は旧値表示へ戻さず`ReconciliationRequired`とRustが返す`reconciliation_id`・起点request/observation（存在する場合だけ）・解決操作を表示し、sendをdisableする。SessionMeta cacheだけの失敗はPersistFailure＋再投影として区別する。

## permission UX

- **正本**: pending permission は backend state（`get_session` の `pending_permission_request`、#1379）と transcript の permission part の 2 経路で届くが、描画の正本は read model。二重表示はしない（現行方針を維持）。
- **回答確定**: click後はdurable `Responding`として操作不能にし、provider ack後だけResolvedへ進める。timeout/restartで実効性不明ならpermission reconciliationを表示し、同じ回答を再送させない。
- **失効**: `Cancelled` への遷移（CLI 取り下げ CL-1 / interrupt / turn 終端）を受けたら、dialog は即座に操作不能にし「取り下げられました」チップへ差し替える（FE-1: 押せるのに効かないダイアログを残さない）。
- **実効性**: `effective=false` の解決は「回答は届きませんでした（ツールは実行されていません）」と表示し、Allowed/Denied と視覚的に区別する（P6）。
- **整形表示**: `ApprovalDisplay` により command / diff / 対象ファイルを整形表示する。生 JSON の直接表示は fallback のみ（SD-6）。tool 名は transcript と dialog で一致させる。
- **Question**: question ごとに描画し、`is_secret` はマスク入力、`is_other_allowed` は自由記述欄、multi_select は複数選択 UI（CX-1 の語彙前提）。secret plaintextは再表示・history保存せず、解決後は「回答済み」のredacted markerだけを表示する。crash後に必要なら再入力を求める。
- **解決済みチップ**: decision（Allowed / Denied / Cancelled）に加え、`decision_reason` / `description`（ルール名・CLI の説明文）を表示する（FE-7）。

## エラー・バナーのスコープ規則

- バナー state は session_id をキーに保持し、他 session のイベントで消える・混ざることを禁止する（FE-5）。
- turn に紐づくエラーは S1 の part（durable）が正本。バナーは「操作の失敗（送信・切替等）」という session スコープの一時通知に限定する。
- app スコープの通知（更新通知等）は本書の対象外。

## frontend 実装規約（ST-8 の解消方針）

1. reducer は「read model 断片の適用」に限定する。順序・欠落・合成の判断（seq 検証、snapshot との整合）は backend が read model / delta の契約として保証し、frontend は契約違反を検出したら snapshot 再取得する（FE-3）。
2. 表示ロジックは「part → component」の写像に限定し、状態機械（turn phase の解釈・permission の有効性判定・queue の遷移判断）を frontend に持たない。
3. 描画の正本は get_session で復元される read model / runtime state（語彙文書 §11。#1379 の pending permission 復元を含む）とする。transient event の蓄積から frontend が独自に再構成した状態を正本にせず、初期化経路は get_session の完全復元に一本化する。
4. Agent設定は`base_selected_revision`を持つtyped `ConfigurationPatch`の1variantでbackend usecaseへ要求し、selected/effective/pending/sync stateのmirrorだけを保持する。Goal commandは独立`base_goal_revision`を使う。turn送信payloadのfrontend値からcurrent stateを再構成しない。
5. 5 modeのprovider写像、runtime availability、Goal strategy/effects/transition、reasoning effortのselected/effective/互換判定、reconciliation、workflow override解決、protocol compatibilityをfrontendに置かない。Rust adapter/usecase/queryの結果を表示する。
6. mode/Goal/reasoning effortの選択肢とGoal action enabled判定はread model駆動とし、UIにprovider/model固有値やlifecycle表をhard-codeしない。Bypass challenge、Goal emulation effects、Unsupported/ProtocolIncompatible reasonもbackend応答をそのまま表現する。

## トレーサビリティ（本書が解消する問題）

| 問題 ID | 節 |
|---|---|
| FE-1 | permission UX（失効） |
| FE-2 | turn 終端の表示（live 即時） |
| FE-3 | P1 / frontend 実装規約 1 |
| FE-4 | S7 usage indicator |
| FE-5 | エラー・バナーのスコープ規則 |
| FE-6 | マトリクス Thinking / TaskStatus |
| FE-7 | permission UX（解決済みチップ） |
| SD-6（表示面） | permission UX（整形表示） |
| RG-4, RG-8（表示面） | マトリクス ToolCall |
| CX-3, RG-1（表示面） | マトリクス Thinking |
| CX-5, RG-2, RG-5（表示面） | マトリクス TodoListSnapshot |
| CL-3, RG-3（表示面） | turn 終端の表示 |
| CX-7, RG-6, CL-5（表示面） | Notice 振り分け |
| CX-8（表示面） | マトリクス Error |
| CX-11（表示面） | マトリクス ToolCall（WebSearch） |
| RT-6（表示面） | マトリクス session 状態 |
| OB-3, OB-5, OB-6（表示面） | マトリクス queue |
| RG-9（表示面） | S7 / turn 終端 stats |
| ST-8 | frontend 実装規約 |
| #1445, #1446 | S9a/S9b/S9c / frontend 実装規約 4（Rust-owned projection / revision / reconciliation） |
| #1447 | Agent 実行設定 UX（5 mode） |
| #1448 | S9a/S9b/S9c ReasoningEffort / S7 との分離 |
| #1449 | S9a/S9b/S9c AgentGoal |
| #1450 | Agent 実行設定 UX（workflow override / restart 継承） |
| #1451 | S9a/S9b/S9c 全体 / capability-driven UI |

## 設計判断

- **P-D1**: usage indicator は入力エリア上部（`MessageInput` 上縁）に常設（compact: context 使用率バー＋残量、クリックで token 内訳 / cost）。「あとどれくらい送れるか」を送信操作の直前で判断できる。#1150 で削除された旧表示の復活ではなく、「常時は要約のみ・詳細はオンデマンド」に再設計する。
- **P-D2**: Notice は transcript を正本とし、バナーは同一 Notice への参照表示（P2 の一意性を保つ）。
- **P-D3**: 取り消した queue メッセージは transcript に「取り消し」マークで残す（lifecycle L-D4 と対応。非表示にしない）。
- **P-D4**: mode / Goal / 工数のvisual componentは再利用するが、S9a launch draft/attempt、S9b configuration/Goal projection、S9c required/optional workflow templateを別surface/state/commandとする。
- **P-D5**: mode は排他的 5 値 selector とし、Plan toggle は廃止する。Auto / Bypass の意味と危険性は provider capability と共に表示する。
- **P-D6**: 「工数（推論レベル）」を UI 名とし、S7 の TokenUsage / cost と視覚・状態・説明を分離する。
- **P-D7**: GoalはS9bの独立`SessionGoalProjection`としてcurrent/pending/sync/evidenceを表示し、Rust評価済みavailable actionsからset/edit/pause/resume/clearを提供する。completion/failureはevidence付きoutcome、provider strategy/effectsは操作前表示とする。

## 確定事項（2026-07-07、2026-07-15 レビューで確定）

1. **P-D1**: usage indicator は**入力エリア上部**（パネルヘッダは不採用）。常時表示は最小要約（context 使用率バー＋残量）。
2. **Notice 振り分け**: 本書の振り分け表を初期値として確定。ModelRerouted / RateLimit の強調度は実装後の使用感で調整する（調整はこの表の更新として行う）。
3. **P-D3**: 取り消し queue メッセージは**マーク表示**（lifecycle L-D4 と同一決定）。
4. **Thinking**: **streaming 中は自動展開・完了で自動折畳**を既定とする。
5. **P-D4 / P-D5**: S9a/S9b/S9c の state scope を分け、mode は `Ask / Edit / Plan / Auto / Bypass` の排他的 selector とする。
6. **P-D6**: 工数は model の応答・推論強度として表示し、token / cost / budget の表示・入力とは分離する。
7. **P-D7**: GoalはSessionごとにcurrent最大1件の独立projectionとしてS9bから操作し、pending/reconciliation、Native/Emulated/Unsupported、strategy/scope/effectsを明示する。
8. **確定タイミング**: UIはconfigurationのrequested/selected/effectiveとGoalのcurrent/pending/syncを区別し、provider ack/canonical commit/activation前にeffectiveとして表示しない。reconciliation中は送信を止める。
9. **Bypass / Auto**: BypassはRustのexecution-scoped one-time challengeとprovider launch gateを必須とし、Autoはprovider reviewerの全status/fallbackとsandbox非拡張を表示する。どちらもworkflow checkpointを越えない。
10. **protocol compatibility**: compiled schemaと実行binary/flags/capabilitiesの不一致はS6のProtocolIncompatibleとして表示し、新規sendを止める。
