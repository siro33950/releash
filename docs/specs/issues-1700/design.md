# Design

## The actual design

### Architecture

#### 活動状態は AgentSession が所有し、Session Node の事実として記録する

「agent が今どう動いているか」の所有者を `AgentSession` にする（Scope）。値オブジェクトは3値に閉じる。

| 値 | 意味 |
| --- | --- |
| `Working` | agent が動いている |
| `AwaitingAnswer` | permission 承認または質問への回答を待って止まっている |
| `AwaitingInstruction` | 応答を終え、次の指示を待って止まっている |

Session の起動直後とプロセス終了後は `AwaitingInstruction` から始める。活動を示す観測が1件も無い状態を `Working` と断定する根拠が無いためであり、値が常に3値のいずれかであることで分類が total になる（R-020 / B-028）。

この値を `NodeExecution` / `WorkflowExecution` に持たせない。Command Node は Session を持たず活動状態も持てないため、実行木の Node 状態側に置くと Command Node が表現できない値を Node 状態が持つことになる。`docs/glossary/DOMAIN.md`（111行目）の状態所有どおり、NodeExecution は `WaitingApproval` / `Paused` / `Failed` / completion signal を所有し続け、活動状態はその外にある。

永続化は既存の純粋事実ログ（`node_events`）に載せる。AgentSession の他の状態（provider session 参照、lifecycle）が既に Session Node の行として記録されている（`adaptor/gateway/agent_session/agent_session_repository.rs` の `session_event_rows`）ため、活動状態も同じ Session Node の `node_execution_id` 行として追記する。これにより、木の fold と AgentSession の復元が同じ行列を読む形が維持され、R-015 の再起動後の再現が読み側の導出だけで成立する。

値オブジェクト自体は `domain/workflow/value_objects/node_fact.rs` に置く。永続事実の payload であり、`ExecutionTreeLaunch`（AgentSession が `launched_as` として持つが、事実の payload として workflow 側に定義されている値）と同じ位置づけである。`domain/agent_session` から `domain/workflow` への依存は既存（`agent_session.rs`、`services/session_derivation.rs`）であり、逆向きの依存を新設しない。状態の所有者が AgentSession であることは、遷移の受理判定を `AgentSession` 集約だけが持つことで表す。

#### 遷移の受理と記録を集約の1操作にし、事実を遷移ごとに1件へ閉じる

`AgentSession` に活動状態の観測操作を追加する。現在値と同じ観測は `AlreadyApplied` を返して未コミットイベントを増やさず、異なる観測だけが `Applied` になり、活動観測イベントを未コミットイベント列へ積む。repository はそのイベントを1事実行へ写像する。「観測は受け取ったが記録していない」中間状態を型として作らない（`docs/architecture/DOMAIN.md`）。これが R-014 / B-021 の根拠であり、判定は集約の外に複製しない。Stop による `AwaitingInstruction` への遷移だけは、完了信号として既に記録する `StopReceived` 事実を同じ導出関数へ入力し、活動観測の事実を重ねて記録しない。

活動状態は lifecycle を置き換えない。`archive` / `restore` / `delete` / GC / initial instruction の受理判定は `open` / `paused` / `archived` を読み続け、活動状態を読まない（R-018 / B-026）。同様に completion signal の記録と `decide_node_completion_handshake` は活動状態を入力にしない（R-016 / B-023）。

#### provider hook を活動の観測点として追加し、両 provider で同じ集合にする

`adaptor/gateway/provider_lifecycle/launch_spec.rs` が登録する hook を、Claude / Codex の双方が持つ event に揃えて拡張する。Claude は plugin の `hooks/hooks.json`、Codex は `-c hooks.<Event>=[...]` で登録する既存の形をそのまま使う。

| hook event | Claude | Codex | 追加/現行 |
| --- | --- | --- | --- |
| `SessionStart` | あり | あり | 現行 |
| `Stop` | あり | あり | 現行 |
| `StopFailure` | あり | なし | 現行（Claude のみ） |
| `UserPromptSubmit` | あり | あり | 追加 |
| `PreToolUse` | あり | あり | 追加 |
| `PostToolUse` | あり | あり | 追加 |
| `PermissionRequest` | あり | あり | 追加 |

Claude だけが持つ `Notification`（`permission_prompt` / `elicitation_dialog` 等）や `PostToolUseFailure` は使わない。片方にしか無い観測点で分類を決めると、同じ活動状態の Session が provider によって違う色になり R-011 / B-018 が破れる。

`PreToolUse` を含めるのは、permission が拒否されたときに `PostToolUse` が届かず、agent が動いているのに黄のまま残る経路（R-001）を、両 provider が持つ観測点で塞ぐためである。

#### 受理経路は provider lifecycle ingress に合流させ、Stop だけが実行木へも記録する

活動の signal は既存の Provider Hook 経路（`releash hook receive` → local API → `ProviderLifecycleIngressUsecase::receive`）をそのまま通す。`ProviderLifecycleBinding` は既存の `StopObserved` と同じく binding / provider / scope / capability / provider session の一致だけを検証し、活動そのものは binding の状態にしない。binding は活動 signal に対して lifecycle event を新設せず、transcript 参照の更新だけを事実として返す。

`ActivityObserved` と `StopFailed` は AgentSession の operation lock の下で集約へ観測を渡し、`Applied` のときだけ AgentSession の commit を行う。`StopObserved` は別の活動観測 commit を行わず、binding が受理した provider lifecycle event と実行木の `StopReceived` を `ProviderExecutionTreeStopTransaction::commit_provider_stop` の1 commit で確定する。受理順序は「binding で signal を検証して lifecycle event を準備する → `commit_provider_stop` で lifecycle event と `StopReceived` を永続化する」であり、永続化された `StopReceived` 自体から `AwaitingInstruction` を導出する。これにより、完了判定に効く信号（R-016）と Stop 後の表示状態が同じ事実から再現される。

活動 signal は実行木の control plane を通らないため、`recover_active_executions` のような復旧経路を tool 呼び出しごとに起動しない。

#### Session Node のプロセス終了を、正常終了と異常終了に分ける

`domain/workflow/services/fact_replay.rs` の `NodeFact::ProcessExited` の Session 分岐が、現在は `failure_reason` を見ずに一律 `Paused` を導出している。これを Command 分岐と同じく異常判定で分ける。

- 正常終了（`exit_code == Some(0)` かつ `failure_reason` / `failure_kind` なし）→ 実行中（`Running`）の Session Node に限り、現行どおり `Paused`（緑）。
- それ以外（非0終了、突合で発見した喪失を表す `exit_code: None` を含む）→ `Failed`（赤）。

異常かどうかの判定は `ProcessExitedFact` の1つの述語として定義し、`derive_session_facts` の `last_exit_abnormal` と fold の分岐が同じ述語を読む。同じ規則を二か所に書かない。

承認待ち（`WaitingApproval`）の Session Node では、正常終了しても Node 状態を動かさない。プロセス終了によって活動状態は `AwaitingInstruction` へ戻り、後述する表示分類の段3によって黄になる。承認待ちを `Paused` へ変えないのは、承認操作の受理条件を維持し、R-005 が定める承認操作の可否と Node 詳細による判別手段を保つためである。

Node 完了に伴う Releash 側の停止（`AgentSession::stop_for_terminal_execution_tree_node`）は `last_exit_abnormal = false` を立てて `exit_code: Some(0)` の事実を書くため緑になる（B-015）。ユーザーが provider CLI を閉じた場合は PTY の exit code がそのまま事実になり、実行中の Session Node なら正常終了は緑、そうでなければ承認待ちを含めて赤になる（B-013 / B-014）。

#### 正常終了しなかった Session Node に resume を提示する

`recompute_workflow_recovery_capabilities`（`domain/workspace_tree/entities/mod.rs`）は leaf の状態が `Paused` であることを resume の条件にしている。Session Node が `Failed` を導出するようになるとこの条件から外れ、workflow の実行として起こされた実行木は retry へ移れる一方、Session の起動として起こされた実行木は #1696 R-009 が explicit retry を受理しないため復旧操作が無くなる。

resume の条件を、leaf が `Paused` であることに加えて、Session Node が正常終了しなかったことでも成立するように広げる。正常終了しなかったことは、`ProcessExited` の `exit_code` が非0または `None` である事実から同定し、fold した `RuntimeNodeExecutionFailure` に `ProviderProcessExit` の由来として保持する。session activation 失敗など runtime が確定した `WorkflowEvent::NodeFailed` は `runtime_failure_observed` 事実へ分け、同じ `Failed` でも resume 対象にしない。提示側と受理側はどちらも `RuntimeNodeExecution::can_resume()` だけを読み、由来の判定を複製しない。AgentSession の lifecycle は R-018 により `paused` のままで resume を受理できる状態にあり、実行木側の提示をそれに合わせる形になる（R-021 / B-029）。retry の受理条件と、resume 不能理由の導出は変えない。

受理側の `WorkflowRuntimeHost::resume_workflow_execution` は、対象抽出の前に durable facts から aggregate を再構成する。対象の Session Node について `WorkflowAgentSessionPort` の provider 復旧操作を先に呼び、`ProviderWorkflowAgentSessionPort` は `AgentSessionLifecycleUsecase` へ委譲する。AgentSession が `paused` なら既存の provider session id を使う `ProviderSessionLaunch::resume`（CLI の `--resume`）で process を起動し、`open` なら provider が動作中なので再起動しない。復旧に失敗した場合は `NodeResumed` を commit せず、Node を resume 前の `Failed` / `Paused` に保つ。復旧が成立した場合にだけ Node を `Running` へ遷移させる。

Node の commit 後に initial instruction の配送が失敗した場合に備え、resume 対象ごとに直前の `Failed` / `Paused` を保持する。補償は一律 `Paused` へ落とさず、`Paused` には正常終了、provider process の異常終了に由来する `Failed` には同じ失敗内容の `ProcessExited` を追記して元の状態を再構成する。runtime 由来の `Failed` は `can_resume()` が対象外にするため、この補償経路へ入らない。

#### 表示分類の入力を Node 状態と活動状態にし、completion signal を外す

`domain/workspace_tree` が分類を所有する構図（#1683）は変えない。`WorkspaceTreeNode::classify_own_status` の入力を、Node の kind、詳細状態、活動状態、recovery fence の有無に変更し、completion signal を入力から外す（R-006）。`completion_signals` field 自体は `can_stop`、`can_retry`、詳細 DTO の `submitReceived` / `stopReceived` / `waitingFor` が読み続けるため保持する。

親行（Sequence / Fanout / Workflow owner）の重大度集約（赤 > 黄 > 青 > 緑）と `recompute_status_classifications` の走査は変更しない（R-010 / B-017）。

#### 主要な変更対象

| Path | 変更の要旨 |
| --- | --- |
| `src-tauri/src/domain/workflow/value_objects/node_fact.rs` | 活動状態の値オブジェクトと活動観測の事実を追加し、`ProcessExitedFact` の異常判定述語を定義する |
| `src-tauri/src/domain/workflow/services/fact_replay.rs` | Session の `ProcessExited` を正常/異常で分け、事実列から活動状態を導出する単一の関数を持ち、`SessionFactsView` と fold の双方がそれを読む |
| `src-tauri/src/domain/workflow/entities/workflow_execution/mod.rs` | resume 可否の単一述語を所有し、Node の resume 遷移も同じ述語で受理する |
| `src-tauri/src/domain/agent_session/aggregates/agent_session.rs` | 活動状態を保持し、観測の受理判定（同値なら未適用）と復元を集約の操作にする |
| `src-tauri/src/usecase/agent_session/agent_session_lifecycle.rs` | workflow resume 時に、Open の provider は維持し、Paused の provider は既存 resume 経路で復旧する |
| `src-tauri/src/domain/agent_session/services/session_derivation.rs` | 事実から復元する AgentSession の field に活動状態を加える |
| `src-tauri/src/domain/provider_lifecycle/value_objects/provider_lifecycle_signal.rs` | 活動観測の signal 種別を追加する |
| `src-tauri/src/domain/provider_lifecycle/entities/provider_lifecycle_binding.rs` | 活動観測を binding 状態を変えずに検証・受理する |
| `src-tauri/src/domain/workspace_tree/value_objects/mod.rs` | `WorkspaceTreeNode` が活動状態を持ち、分類規則の入力を差し替える |
| `src-tauri/src/domain/workspace_tree/entities/mod.rs` | 活動状態を projection fact として受け取り分類再計算へ渡し、resume の条件に正常終了しなかった Session Node を加える |
| `src-tauri/src/domain/workspace_tree/projection.rs` | Node ごとの活動状態を projection の入力に加える |
| `src-tauri/src/usecase/provider_lifecycle/ingress.rs` | 活動 signal の受理経路を追加し、Stop は実行木の `StopReceived` だけを1 commit で記録する |
| `src-tauri/src/adaptor/gateway/workflow/node_session_boundary.rs` | workflow resume の provider 復旧操作を AgentSession lifecycle usecase へ委譲する |
| `src-tauri/src/adaptor/gateway/workflow/workflow_host/lifecycle_commands.rs` | resume の対象抽出前に durable facts から実行 aggregate を再構成し、provider 復旧後だけ Node を Running へ確定し、失敗時は直前の Node 状態へ補償する |
| `src-tauri/src/usecase/agent_session/usecase.rs` | 活動観測の手順（load → 集約の判定 → 変化時のみ保存）を追加する |
| `src-tauri/src/adaptor/gateway/provider_lifecycle/launch_spec.rs` | 追加 hook を両 provider の起動仕様へ登録する |
| `src-tauri/src/adaptor/gateway/provider_lifecycle/payload.rs` | hook event 名と tool 名から signal への写像を拡張し、payload から tool 名を読む |
| `src-tauri/src/adaptor/gateway/agent_session/agent_session_repository.rs` | 活動観測イベントを事実行へ写像し、活動状態の bounded な読みで集約を復元する |
| `src-tauri/src/adaptor/gateway/workspace_tree/repository.rs` | fold から得た Node ごとの活動状態を projection へ渡す |
| `src-tauri/src/adaptor/protocol/provider_lifecycle.rs` | local API の signal request に活動観測を追加する |
| `src-tauri/src/usecase/agent_session/agent_session_read.rs` | 既存 `activity` の導出（`with_activity`）を削除する |
| `src-tauri/src/usecase/agent_session/agent_session_query.rs` | `AgentSessionItemDto` から `activity` を削除する |
| `src-tauri/src/domain/terminal_surface/value_objects/terminal_activity.rs` | 削除する |
| `src/types/agent-session.ts` | `activity` を型から削除する |

#### 検証境界

- domain: 分類の優先順位（失敗・recovery fence → 完了/中止/停止 → 活動状態）、活動観測の同値時未適用、`ProcessExited` の正常/異常分岐、活動状態の導出が集約復元と木読みで同一であること。
- gateway: 両 provider の hook 登録内容、payload の event 名から signal への写像（subagent payload の除外を含む）、活動事実の追記と復元の往復、活動状態がツリー DTO の色へ現れること。
- usecase: 活動 signal の経路分岐、`StopObserved` が `commit_provider_stop` の1 commit だけで完了して第二の活動状態 commit を行わないこと、`StopFailed` と `ActivityObserved` の単独 commit が維持されること、活動状態が操作可否の入力にならないこと。
- 質問系 tool の `PreToolUse` が `AwaitingAnswer` へ、それ以外の `PreToolUse` が `Working` へ写ること（B-006）は、両 provider の tool 名を入力にした payload 写像で検証する。
- 活動が一度も観測されていない Session Node が黄になること（B-028）と、正常終了しなかった Session Node に resume が立つこと（B-029）は、事実列から導出した分類と操作可否で検証する。B-029 の受理経路は provider の resume 起動、復旧失敗時に `ResumeRequested` が記録されないこと、および instruction 配送失敗時に直前の `Failed` / `Paused` が事実列から再現されることまで検証する。
- CLI: 登録した全 event で hook が終了コード 0 と空 JSON を返すこと（R-013）。
- 検証手段が自明でないもの: 事実件数が遷移回数と一致すること（B-021）は、同一活動を繰り返し観測した後の `node_events` 行数で検証する。再起動後の色（B-022）は、書き込み後に store から再 fold した分類で検証する。

### Interface

#### Tauri read contract

公開 command 名、引数、失敗形式を変更しない。`WorkspaceNodeDto` / `WorkspaceSequenceDto` / `WorkspaceFanoutDto` の `status` と `WorkspaceNodeDetailDto` の `statusClassification` は `active | attention | failure | idle` の closed set のまま、値の決まり方だけが変わる。`WorkspaceNodeDetailDto.status` も既存の closed set のままで、Session Node が `failed` を取り得るようになる。活動状態そのものは DTO へ出さない。ツリーの色に関する frontend の型と色の写像は変更しない。

`AgentSessionItemDto` から `activity` を取り除く（R-022 / B-030）。合わせて `usecase/agent_session/agent_session_read.rs` の `with_activity`、`domain/terminal_surface/value_objects/terminal_activity.rs`、gateway の `session_activity`、`src/types/agent-session.ts` の該当 field を削除する。production の component はこの field を参照していないため表示は変わらない。

#### local API

`ProviderLifecycleSignalRequest` に活動観測の variant を追加する。応答（`Applied` / `Duplicate` / `Rejected`）は変えない。この endpoint の client は同一 binary の `releash hook receive` だけであり、Workspace ツリー・Workspace ノード詳細・AgentSession 詳細を返す local API / CLI は存在しないため、R-019 / B-027 の応答は変わらない。

#### Provider Hook

`releash hook receive` は追加 event でも現行と同じく標準出力へ空 JSON を書き、終了コード 0 で終わる。local API が `Rejected` を返した場合も同様とする。`PreToolUse` / `PermissionRequest` は provider 側で hook の非0終了や decision 出力が tool の実行可否を変える event であり、Releash が承認判定へ介入しないという R-013 / B-020 は、この「常に空 JSON・終了コード 0」で担保する。

Claude payload の `agent_id` による subagent 除外は維持する。subagent の tool 呼び出しで親 Session の活動状態が動くと、agent が止まっているのに青になる。

#### 内部境界

活動観測の受理は `AgentSession` 集約の操作として公開する（適用済み / 既適用を返す）。ingress と repository はこの結果に従うだけで、同値判定を自前で持たない。

### Data Model

追加する record は `node_events` の1 event type「活動観測」だけで、payload は活動状態1値。identity は既存の行 identity（`tree_id`, `seq`）で、Session Node の `node_execution_id` 行に属する。hook 種別、tool 名、`tool_use_id` を payload に持たない。tool 名は活動状態への写像の入力に使うだけで、事実には残さない。分類に必要なのは最新の活動状態だけであり、これらを保持すると full-retention 経路になる。versioning は不要（既存 `node_events` の event type 追加と同じ扱い）。

`AgentSession` は活動状態を1 field として持つ。lifecycle（`open` / `paused` / `archived`）とは独立で、片方が他方を導出しない。プロセス終了の事実（`ProcessExited`）と応答終了の事実（`StopReceived`）は活動状態を `AwaitingInstruction` へ戻す。活動状態は生きている provider プロセスの性質であり、終了後の値を残すと resume 後に古い活動が読まれる。

`WorkspaceTreeNode` は活動状態を Option で持つ。Session Node では常に設定され、Command / Fanout / Sequence / Workflow owner では常に未設定である。Option が表すのは Session の有無であって観測の有無ではない。分類と同じく一時的な read state であり、identity も永続 record も持たない。

`FoldedTree` は Session Node ごとの最新活動状態を fold の同一走査で導出して持つ。`isolated_worktrees` と同じく「同じ木の事実から導出した隣接関心の snapshot」であり、読み側が事実行を二度走査しないための形である。

### Database

schema、既存 index、projection record を変更しない。event type は SQL 側に CHECK を持たず domain が語彙を所有するため、事実の追加だけで載る。

追加する access path は1つ。**Session Node の `node_execution_id` に対し、活動状態の導出入力となる `AgentActivityObserved` / `ProcessExited` / `StopReceived` の最新行を1行返す読み**。既存 index `idx_node_events_node (node_execution_id, seq)` の同一 Node 範囲を降順に読み、対象 event type の最新1行だけを返すため、木全体を fold せず index も追加しない。

必要な根拠: hook が tool 呼び出しごとに届くようになるため、遷移判定（R-014）のたびに木全体を fold すると read の計算量が木の事実数に比例して増える。活動観測の write 経路は、この bounded な読みと、`session_id` からの既存 indexed lookup（`find_session_attachment` と Node の最新行）だけで集約を復元し、木全体の走査を行わない。

### UI/UX

該当なし。4分類と色の対応（青=実行中 / 黄=介入が必要 / 赤=失敗 / 緑=動いていない）、行の形状、詳細ペインの表示規則は #1683 のままで、frontend は backend が確定した分類値を色へ写像するだけである。分類値の再取得契機は `workspace-tree-refresh` / `workflow-execution-changed` / `agent-session-changed` の3つとし、ツリー行と選択中 Node の詳細が同じ backend-owned state を再取得する。

### Algorithm

#### hook event から活動状態への写像

| hook event | 活動状態 |
| --- | --- |
| `UserPromptSubmit` | `Working` |
| `PreToolUse`（tool 名が質問系以外） | `Working` |
| `PreToolUse`（tool 名が質問系） | `AwaitingAnswer` |
| `PostToolUse` | `Working` |
| `PermissionRequest` | `AwaitingAnswer` |
| `Stop` | `AwaitingInstruction` |
| `SessionStart` | `AwaitingInstruction` |
| `StopFailure` | `AwaitingInstruction` |

質問系の tool 名は、英数字以外を除いて小文字化した結果が `askuserquestion` または `requestuserinput` に一致するものとする。Claude の `AskUserQuestion` と Codex 0.145 以降の `request_user_input` が該当する。

`PreToolUse` を tool 名で分けるのは、質問待ちが両 provider とも `PermissionRequest` ではなく `PreToolUse` で届くためである。Claude は `AskUserQuestion` の実行中に `PreToolUse` を出し、新しいビルドは同じ待ちを `PermissionRequest` としても報告する。Codex 0.145 以降の `request_user_input` は auto-allowed で承認フローに入らないため `PreToolUse` だけを出す。event 名だけで写像するとこの間に `Working` が入り、B-006 が成立しない。

最後に観測した事実が現在の活動状態になる。`AwaitingAnswer` を「人が答えるまで解けない」sticky な状態にはしない。要求が黄を求めるのは「agent が待って止まっている」間であり、止まっている間は `Working` を示す観測点が発火しないためである。`SessionStart` を `AwaitingInstruction` へ写像するのは、その時点で agent が動いていることを示す観測が1件も無いためである（R-020 / B-028）。

#### 事実列から活動状態への導出

Session Node の事実列を走査し、`AwaitingInstruction` を初期値として、`ProcessExited` または `StopReceived` でその初期値へ戻し、`AgentActivityObserved` では事実が持つ値へ進める。seq が後の事実を常に適用するため、Stop 後の `Working` は青へ戻り、その往復回数に上限はない。この導出関数を `AgentSessionActivity::after_fact` の1つだけ定義し、AgentSession の復元（`derive_agent_session_fields` 経由）と実行木の fold の双方が呼ぶ。活動観測と `StopReceived` は `SessionAttached` / `ArchiveRequested` / `ResumeRequested` / `RestoreRequested` の解釈に影響せず、lifecycle と completion signal の導出は現行のままである。

#### 表示分類の決定

Node ごとに次の順で決める。上の条件が成立した時点で確定する。

1. recovery fence を持つ、または詳細状態が `Failed` → 赤（Failure）。
2. 詳細状態が `Completed` / `Aborted` / `Paused` → 緑（Idle）。
3. kind が Session の場合、活動状態で決める。`Working` → 青、`AwaitingAnswer` / `AwaitingInstruction` → 黄。活動状態は常に3値のいずれかであり、この段は total である。
4. kind が Session 以外の場合、詳細状態で決める。`Waiting` → 黄、`Running` → 青。

1 と 2 が 3 / 4 より先に来ることが R-007 / B-011 / B-012 に対応する。3 が詳細状態を見ないことが R-005 / B-007（承認待ちでも agent が動いていれば青）に対応する。4 は Command Node の現行規則そのものであり、`Waiting` → 黄 / `Running` → 青 を維持する（R-009 / B-016）。completion signal はどの段にも現れない（R-006 / B-009 / B-010）。

親行の集約は自分自身の分類と配下の子の分類の重大度最大で、現行のまま変更しない（R-010 / B-017）。

### Infra

該当なし。

## Alternatives Considered

- **terminal 出力の recency（既存 `TerminalActivity`）を分類の入力にする**: 採らない。R-014 は活動状態を遷移ごとの事実として記録することを、R-015 は再起動後に記録から同じ色が再現されることを要求する。recency は読み取り時に導出する値で事実にならず、permission 待ちと入力待ちも区別しない（R-004）。
- **completion signal に「再開」遷移を足し、`StopReceived` から `Pending` へ戻す**: 採らない。Submit と Stop の単調な累積は Session Node の完了規則そのものであり、R-016 / B-023 が現行の完了判定の維持を要求する。
- **活動状態を `NodeExecution` / `WorkflowExecution` に持たせる**: 採らない。Command Node が持てない値を Node 状態が持つことになり、要求が定めた所有者（AgentSession）とも食い違う。
- **活動状態を workspace_tree の read time にだけ持ち、永続化しない**: 採らない。R-015 / B-022 が再起動後の再現を要求する。
- **Claude の `Notification`（`permission_prompt` / `elicitation_dialog` / `agent_needs_input`）を人の回答待ちの観測点にする**: 採らない。Codex に同等の event が無く、片方の provider にしか無い観測点で分類を決めると R-011 / B-018 の provider 間の一致が保てない。

## Cross-cutting concerns

- **hook 頻度**: `PreToolUse` / `PostToolUse` の追加で、hook の発火が「session 単位・turn 単位」から「tool 呼び出し単位」に上がる。1回の発火は provider による短命プロセス起動と loopback HTTP 1往復であり、活動状態が変わらない観測では store への書き込みが起きない。読み側も木全体の fold を行わない（Database 参照）。この2点を満たさない実装は full-recompute 経路の追加になる。
- **provider への非介入**: 追加する hook のうち `PreToolUse` と `PermissionRequest` は、hook の非0終了や decision 出力が provider 側の実行可否を変える event である。Releash の hook は全 event で空 JSON と終了コード 0 を返し、decision を出力しない（R-013 / B-020）。
- **秘匿情報**: 活動観測の事実は活動状態1値だけを保持し、prompt 本文、tool 入力、tool 出力を持たない。既存の `secret_masker` の対象を増やさない。
- **排他**: 活動観測は AgentSession の operation lock の下で行う。Node 完了に伴う停止（`stop_for_terminal_execution_tree_node`）と PTY の終了観測（`observe_process_exit`）が同じ lock を取り、先に `Paused` を記録した側が勝つ現行の順序が B-015 の緑を保つ。

## Risks

- **hook の順序に依存する**。同一 tool 呼び出しで `PreToolUse` が `PermissionRequest` より先に発火することを前提にしている。逆順の provider があると、承認ダイアログ表示中に青へ戻り B-005 が破れる。並列 tool 呼び出しで一方が承認待ち・他方が実行中という混在状態は要求が結果を定めていないため、本設計は最後の観測をそのまま採る。
- **permission 拒否後に tool 呼び出しを伴わない応答が続く経路には、両 provider に共通する活動再開の観測点がない**。`PostToolUse` が届かず次の `PreToolUse` も無い場合、最後に観測した `AwaitingAnswer` を次の共通観測点または `Stop` まで維持する。R-001 は観測された活動状態が `Working` である範囲を定めるため、この未観測区間は適用範囲外である。
