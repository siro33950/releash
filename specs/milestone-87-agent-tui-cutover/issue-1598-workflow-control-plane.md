# Issue #1598 Submit・Stop・Approval Workflow Control Plane Spec

## 1. 目的

Provider CLI TUIを使用するWorkflow Agent Nodeについて、Nodeの完了意思であるSubmitと、Provider Turnの終了を表すStopを独立したsignalとして扱い、同一Node Attemptの両signalが揃った場合だけWorkflowを進行させる。

ReleashはWorkflow ledgerを正本として、signalの受信、Artifact、Approval、Retry、Node lifecycleをdurableに所有する。Provider transcript、Terminal表示、process exitからWorkflow遷移を推定しない。

## 2. 正本と優先順位

本Specは次の正本を実装可能な作業単位へ落としたものである。

1. Issue #1598本文
2. Issue #1598の`Workflow lifecycle 合意仕様`コメント
3. `specs/milestone-87-agent-tui-cutover/acceptance-contract.md`
4. 本Spec
5. 既存の設計文書と実装

上位の正本と既存実装が矛盾する場合は、既存実装を上位の正本へ合わせる。推測による仕様追加は行わない。

## 3. 現在の問題

現行実装には次の不一致がある。

- Submitは必須Artifactを保存する操作であり、Artifactなしの完了意思を表現できない。
- Provider StopはProvider lifecycle ledgerへ保存されるだけで、WorkflowのNode Attemptへ適用されない。
- 旧Agent runtimeのturn completionが、SubmitとStopの合流を経ずにAutoまたはApprovalを進行させる。
- Workflow全体が`WaitingApproval`、`Failed`、`Interrupted`を所有しており、Nodeごとに異なる状態を持つFanoutを正しく表現できない。
- completion signalの片側受信状態、Node単位Retry、受信済みsignalの再起動後復元を表現できない。
- Workspace read modelとTUIに、待機中signalおよびRetry capabilityが存在しない。
- #1594のfixture self-testは存在するが、ATUI-040、ATUI-041、ATUI-042を製品境界へ接続するacceptance testが存在しない。

## 4. 外部仕様

### 4.1 SubmitとStop

- Nodeの完了意思は明示的なSubmitで表す。
- SubmitにはArtifactを任意で添付できる。
- ArtifactなしのSubmitも完了意思として有効である。
- Provider Stopは現在のProvider Turnが終了した事実であり、Submitとは独立して記録する。
- Completion handshakeを適用する対象はProvider CLI TUIを使用するAgent Nodeである。
- Command Nodeは従来どおりprocessの終了結果によってAttempt結果を確定し、Submit / Stop handshakeを要求しない。
- 同一Node AttemptのSubmitとStopが揃うまでNodeを自動完了しない。
- 片方だけを受信した場合は、時間経過をエラーにせず、もう片方を期限なく待つ。
- deadline、timeout、`Stalled`状態、時間経過による自動Retryを設けない。

### 4.2 AutoとApproval

- Auto Agent Nodeは同一AttemptのSubmitとStopが揃った場合だけ成功し、後続へ進む。
- Approval Agent Nodeは同一AttemptのSubmitとStopが揃った場合だけ`WaitingApproval`となる。
- Approveは対象Nodeだけを成功させる。
- Reject操作は設けない。
- Fanoutの兄弟Nodeは独立して進行し、一つのNodeのsignal待機、Pause、WaitingApproval、Failed、Retryによって他の実行中Nodeを停止しない。
- Fanoutは全子Nodeが成功した場合だけ一度だけ完了し、後続へ進む。

### 4.3 Signal待機

- 片方だけを受信した状態はNode Attemptのdurableな事実として保存する。
- TUIは`Submit受信済み・Stop待ち`または`Stop受信済み・Submit待ち`を表示する。
- Signal待機は`Failed`、`Paused`、`WaitingApproval`とは別のcompletion progressであり、Workflowの終端結果ではない。
- 同一AttemptのPause / Resumeおよびアプリ再起動を跨いで、受信済みsignalを維持する。
- Stop受信済みで実行中Turnを持たないNodeには、アプリ再起動によるPauseを重ねない。
- Submit受信済みだがStop未受信で、アプリ再起動によって実行中Turnを失ったNodeは、Submit受信済みの事実を保持したままPausedとなる。
- Paused Agent Nodeを同一AttemptでResumeした場合、同じProvider conversationで新しいTurnを開始し、受信済みsignalを維持する。

### 4.4 Retry

- Failed Nodeとcompletion signal待ちのNodeは、ユーザーが明示的にRetryできる。
- Retryは新しいNode Attemptを作成する。
- Agent NodeのRetryは新しいAgentSessionとPTYを作成し、Node先頭から実行する。
- Command NodeのRetryは新しいprocessを開始し、Node先頭から実行する。
- Retry前のAttemptの状態、signal、Artifact、ログ、Sessionは履歴として保持する。
- 旧AttemptのsignalおよびRetry後に遅延到着したsignalを、新しいAttemptの完了判定へ使用しない。
- Retry開始後に旧Agent Turnを停止しても、そのStopによって旧Attemptまたは新Attemptを進行させない。
- Retry以外の操作でAttemptを暗黙に作り直さない。

### 4.5 Artifact

- Artifactを添付しないSubmitは、Artifact contractの有無にかかわらず受理できる。
- Artifactを添付する場合はcontractとpayloadを一組で指定する。
- Artifact付きSubmitは、redactionとcontract validationに成功した場合だけ、SubmitとArtifactを一体で受理する。
- Artifact validationに失敗した場合は、ArtifactとSubmitのどちらも記録しない。
- ArtifactはNodeExecution IDへ関連付ける。
- Retry前のAttemptのArtifactを、新しいAttemptまたは新しいAttemptの後続Nodeのcurrent outputとして扱わない。
- Artifactなしで成功したAttemptにはcurrent Artifactが存在しない。
- Artifactが存在する場合は、Workflowの判断材料および出力として取得できる。

### 4.6 完了後のAgentSession

- Node完了およびWorkflow完了によってAgentSession、Provider session、PTY、Terminal Surfaceを終了しない。
- Node完了後にReleashが自動入力または自動resumeを行わない。
- ユーザーは完了NodeのAgentSessionへ明示的に追加質問できる。
- 完了後の追加Turnで発生したStopは、完了済みNodeまたは後続Nodeを再度進行させない。
- 完了後にユーザーが明示的に再開したProvider sessionのProvider内部挙動はWorkflow保証外とする。

### 4.7 Failure

- `Failed`はNode Attemptの結果であり、Workflow全体の終端結果ではない。
- Commandの非ゼロ終了、AgentまたはProviderから受理した明示的な実行失敗、process起動失敗をNode AttemptのFailedとして扱う。
- Provider lifecycleの`StopFailure`は診断情報であり、StopまたはNode Failedへ変換しない。
- process exit、terminal表示、signal欠落、signal拒否からStopまたはNode成功を推定しない。
- Workflow定義不正はWorkflowExecution作成前に拒否し、NodeまたはWorkflowをFailedにしない。

### 4.8 Workflow lifecycle

- `WaitingApproval`、`Paused`、`Failed`はNodeまたはNode Attemptが所有する。
- Workflow全体の終端結果は`Completed`と`Aborted`だけとする。
- 未完了Workflowの表示は配下Nodeの状態から導出し、表示値をWorkflow遷移の判断に使用しない。
- Workflowの保管状態`Open` / `Archived`を実行状態と分離する。
- Workflow自体に独立したPause状態を設けない。
- Pause Workflowは実行中Nodeをそれぞれの規則でPausedにする一括操作とする。
- Resume WorkflowはPaused Nodeをそれぞれの規則で再開する一括操作とする。
- WaitingApproval、完了済みNode、Stop受信済みでSubmitを待っているNodeにはPauseを重ねない。
- Node失敗だけでWorkflowをFailedまたはArchivedにしない。
- Abortは実行中Agent TurnとCommand processを停止してWorkflowをAbortedとするが、AgentSessionとPTYをArchiveしない。
- Archive、Restore、Delete、Session復帰不能時の動作はIssueコメントの合意仕様を維持し、受信済みcompletion signalを失わない。

## 5. Signal correlation

- SubmitはWorkflowExecution ID、Node名、NodeExecution IDを必須入力として対象を特定する。
- SubmitのNodeExecution IDは現在対象となるAttemptと一致しなければならない。
- Provider Stopは検証済みAgentSession IDからAgentSession originを読み、WorkflowExecution IDとNodeExecution IDを解決する。
- Standalone AgentSessionのStopはProvider lifecycle ledgerだけへ保存し、Workflowへ適用しない。
- 現在のProvider lifecycle bindingと一致しないStopをWorkflowへ適用しない。
- Node名だけ、Provider session IDだけ、Terminal表示、現在選択中UIからAttemptを推定しない。
- 同一signalの再送は冪等とし、Node遷移、Artifact、Approval、後続起動を重複させない。

## 6. Domain model方針

### 6.1 WorkflowExecution aggregate

WorkflowExecution aggregateを、Node Attempt lifecycleとcompletion handshakeの唯一の状態遷移authorityとする。

各Agent NodeExecutionは少なくとも次の事実を所有する。

- NodeExecution ID
- Attempt番号
- Node種別とFanout親
- Node lifecycle status
- AgentSession ID
- Submit受信有無
- Stop受信有無
- Attemptに属するArtifact
- Failure
- 開始・更新・完了時刻

Submit / Stopの組合せはnullable fieldの分散判定にせず、Domain valueまたはDomain entityの閉じた遷移として扱う。Domain methodは重複、対象外Attempt、終端済みAttempt、Auto、Approvalを判定し、UsecaseやGatewayへ状態機械を漏らさない。

### 6.2 Workflow level

WorkflowExecutionの非終端／終端管理と、Nodeの詳細状態を分離する。Workflow全体にNode状態の複製を持たず、WaitingApproval、Paused、Failedの表示はNode projectionから導出する。

### 6.3 Domain event

少なくとも次の事実をWorkflow event streamで表現する。

- Node AttemptへのSubmit受理
- Node AttemptへのProvider Stop受理
- Optional Artifact生成
- Node WaitingApproval
- Node Approval解決
- Node完了
- Node失敗
- Node Pause / Resume
- Node Retryと新Attempt開始
- Workflow Completed / Aborted

Replayは同じDomain transition規則を使用し、保存DTOまたはGatewayで状態を推測しない。

## 7. Application / transaction境界

### 7.1 Workflow control plane Usecase

新しいWorkflow制御動作はRustのDomainとUsecaseに置く。`workflow_host.rs`、controller、Gateway、React hook、componentへ新しい状態判断を追加しない。

Workflow control plane Usecaseは次を調停する。

- Workflow aggregateのload / lock / candidate作成
- Domain commandの適用
- Domain eventとprojection mutationのcommit
- commit後のruntime side effect
- state notification
- concurrency conflict時のbounded retry

Gatewayは永続化、AgentSession参照、runtime操作、通知の外部mechanicsだけを提供する。

### 7.2 Submit transaction

一回のSubmit受理は、同じWorkflow stream transactionで次をcommitする。

- Submit受理event
- Artifactがある場合のArtifact event
- 両signal成立によるNode CompletedまたはWaitingApproval event
- 必要な後続Node開始event
- WorkflowおよびWorkspace projection mutation

Artifact validation失敗またはcommit失敗時は、いずれの事実もlive stateへ公開しない。

### 7.3 Provider Stop transaction

Provider Stop受理は既存SQLite Local Event Storeのmulti-stream atomic batchを使用し、次を一括commitする。

- Provider lifecycle Stop fact
- Workflow Node AttemptのStop受理event
- 両signal成立によるNode CompletedまたはWaitingApproval event
- 必要な後続Node開始event
- WorkflowおよびWorkspace projection mutation

Provider lifecycle ledgerだけにStopが残りWorkflowへ適用されない中間状態を作らない。別database、JSON ledger、非durable callbackだけの連携、期限schedulerを新設しない。

### 7.4 Commit後のeffect

- live aggregate candidateはdurable commit成功後だけpublishする。
- Node / Workflow completion後にAgentSessionまたはPTYのclose effectを発行しない。
- 次Nodeのruntime起動はcommit後に行う。
- commit済み次Nodeのruntime起動失敗は、そのNode AttemptのFailedとして別のdurable transitionで記録する。
- concurrent Submit / Stopはstream head CASとDomain冪等性で一度だけ収束させる。

## 8. CLI / Local API

### 8.1 CLI

既存の`workflow output submit`をSubmitの唯一のCLI入口として維持し、新しい一般利用者向けCLI commandを増やさない。

Artifactなし:

```bash
releash workflow output submit EXECUTION_ID \
  --node NODE_NAME \
  --node-execution NODE_EXECUTION_ID
```

Artifactあり:

```bash
releash workflow output submit EXECUTION_ID \
  --node NODE_NAME \
  --node-execution NODE_EXECUTION_ID \
  --type CONTRACT \
  --json JSON_VALUE
```

- `--node-execution`は必須とする。
- Artifact指定時は`--type`と`--json`または`--file`を全て要求する。
- Artifact未指定時は`--type`、`--json`、`--file`を要求しない。
- `workflow output get`はArtifact取得のまま維持する。

### 8.2 Local API

Local API requestはArtifact提出ではなくNode Submitを表す。requestは必須NodeExecution IDとoptional Artifactを持つ。ControllerはprotocolをUsecase commandへ変換するだけとし、signal合流、validation結果による遷移、current Attempt判定を行わない。

### 8.3 Initial instruction

通常Agent NodeとFanout Agent Nodeの両方へ、WorkflowExecution ID、Node名、NodeExecution IDを含む正確なSubmit commandを初回指示として渡す。Artifact contractがないNodeにもArtifactなしSubmitの指示を渡す。

## 9. Read model / TUI

Rust read modelはNodeごとに少なくとも次を公開する。

- Node lifecycle status
- Attempt番号
- Submit受信有無
- Stop受信有無
- 待機中signal
- canApprove
- canRetry
- AgentSession ID
- current AttemptのArtifact有無
- error / recovery reason

TUIはread modelを表示し、signalの組合せやRetry可否を再計算しない。

- `Submit受信済み・Stop待ち`を表示する。
- `Stop受信済み・Submit待ち`を表示する。
- WaitingApproval NodeだけにApproveを表示する。
- Failedまたはcompletion signal待ちのcurrent NodeだけにRetryを表示する。
- Workflow rootの表示はNode projectionから導出する。
- 完了NodeのAgentSession surfaceを閉じず、追加質問可能な状態で残す。

## 10. Recovery / restart

- Workflow aggregateはevent replayによってSubmit / Stop受信状態を復元する。
- App restart時に実行中だったAgent NodeはPausedへ移すが、受信済みsignalを消さない。
- Stop受信済みでSubmit待ちのNodeはWaitingApprovalへ進めず、Submit待ちのまま復元する。
- Submit受信済みでPausedとなったNodeをResumeした場合、同じAttemptの新しいTurnが発生する。次のvalid Stopは保存済みSubmitと合流できる。
- Retryを選択した場合だけ新Attemptへ移り、旧Attemptのsignalを切り離す。
- WaitingApprovalとCompletedはrestart後も同じ状態を維持する。
- Provider sessionを復帰できない場合はIssueコメントのSession復帰規則を適用し、他NodeまたはWorkflow全体を巻き戻さない。

## 11. Red-Green-Refactor実施規約

すべてのobservable behavior changeを、次の完全なRed-Green-Refactor cycleで実施する。

### RED

- 受け入れ済み外部仕様を表す最小のtestをproduction codeより先に追加する。
- runnableなblack-box testを書ける場合、未完成testによるcompile failureだけをREDの証拠にしない。
- focused testを実行し、意図した未実装または誤実装によって失敗したことを確認する。
- 失敗理由が意図と異なる場合は、production codeを変更せずtestまたはfixtureを正す。

### GREEN

- REDで示したbehaviorだけを満たすproduction implementationを追加する。
- test期待値を誤実装へ合わせて変更しない。
- focused testと直接関連するtest moduleを実行して成功を確認する。

### REFACTOR

- 重複、旧分岐、dead code、不正なlayer依存、rollback前提のlive state mutationを整理する。
- behaviorを変更せずfocused testと関連testを再実行する。
- REFACTOR完了前に次のbehaviorへ進まない。

Product acceptanceは最初に外側のREDとして追加し、各focused cycleが進むごとに段階的にGreenへ近づける。fixture self-testだけをproduct acceptance成功として扱わない。

## 12. 実装サイクル

### Cycle 1: Product acceptance入口

RED:

- #1594 fixture、実Hook CLI、Local API、Workflow queryを接続する`workflow_control_plane_acceptance_test`を追加する。
- ATUI-040、ATUI-041、ATUI-042が現行production pathの未接続によって失敗することを確認する。

GREEN:

- acceptance compositionが既存production compositionを利用して起動できる最小のtest入口だけを整える。

REFACTOR:

- acceptance専用の状態機械またはproduction bypassを残さない。

### Cycle 2: Domain completion handshake

RED:

- Submit / Stop両順序、重複、片側待機、終端済み、別AttemptをDomain testで表現する。

GREEN:

- Node Attempt所有のcompletion handshakeとDomain transitionを実装する。

REFACTOR:

- 分散boolean判定を閉じたDomain APIへ集約する。

### Cycle 3: Event persistence / replay

RED:

- Submitのみ、Stopのみ、両方、Retry前後、restart replayをevent codec / repository testで表現する。

GREEN:

- Workflow event、codec、snapshot / Workspace projection mutationを実装する。

REFACTOR:

- Gateway独自の状態再構築を削除し、Domain replayへ統一する。

### Cycle 4: Artifact optional Submit

RED:

- Artifactなし、valid Artifact、invalid Artifact、stale NodeExecution、重複SubmitをUsecase、Local API、CLI testで表現する。

GREEN:

- optional Artifact request、必須NodeExecution ID、atomic Submit transaction、CLI option groupを実装する。

REFACTOR:

- Artifact提出とSubmitを混同する命名・分岐を整理し、Artifact read pathは維持する。

### Cycle 5: Provider Stop connection

RED:

- Workflow AgentSession、Standalone AgentSession、別binding、別AgentSession、重複Stop、process exit、StopFailureをUsecase / integration testで表現する。

GREEN:

- AgentSession origin解決とProvider / Workflow multi-stream atomic commitを実装する。

REFACTOR:

- Provider lifecycle ingressからWorkflow状態判断を除き、Workflow control plane Usecaseへ集約する。

### Cycle 6: Auto / Approval / Fanout

RED:

- Auto一度だけ完了、Approval一度だけ要求、Approve対象限定、Fanout兄弟独立、全成功時一度だけ進行をDomain / Usecase testで表現する。

GREEN:

- 両signal成立時のNode遷移、後続routing、Approval、Fanout集約を実装する。

REFACTOR:

- 旧turn completionによるProvider TUI Nodeの直接進行を除去し、二重authorityを解消する。

### Cycle 7: Retry / Failure

RED:

- Failed Retry、Submit待ちRetry、Stop待ちRetry、旧signal遅延、新AgentSession / PTY、旧履歴保持をtestする。

GREEN:

- Node単位Retry commandと新Attempt起動を実装する。

REFACTOR:

- 自動Retry、Workflow全体Failed、stale signal fallbackを新経路から除去する。

### Cycle 8: Pause / Resume / restart

RED:

- partial signalを保持したPause / Resume、App restart、WaitingApproval維持、Stop受信済みNodeへPauseを重ねないことをtestする。

GREEN:

- Node所有Pause / Resumeとrestart reconciliationを実装する。

REFACTOR:

- Workflow全体`Interrupted`を状態authorityとして使用する経路を除去する。

### Cycle 9: Workspace read model / TUI

RED:

- signal待機表示、canRetry、Node Retry interaction、Node由来Workflow表示をRust projection testとReact interaction testで表現する。

GREEN:

- Rust projection、protocol、presenter、TUI表示と操作を接続する。

REFACTOR:

- frontendの派生判断を削除し、backend read modelのmirrorへ限定する。

### Cycle 10: Completion retention

RED:

- Node / Workflow完了後のAgentSession / PTY継続、自動入力なし、明示的追加質問、追加Stopによる再進行なしをproduct acceptanceで表現する。

GREEN:

- completion後のruntime cleanupをAgentSession / PTY retentionと分離する。

REFACTOR:

- Provider TUI経路に残った旧GUI cleanup / completion分岐を整理する。旧Agent GUI全体の物理削除は#1599へ残す。

### Cycle 11: Full acceptance / cleanup

RED確認:

- ATUI-040、ATUI-041、ATUI-042の各scenarioがcontrolledなproduction invariant破壊を検出できることを確認し、その破壊を残さない。

GREEN:

- 全product acceptance、関連unit / integration test、通常quality gateを成功させる。

REFACTOR:

- acceptance専用production seam、dead code、重複DTO、不正なGateway state machine、不要なcompatibility pathを削除する。

## 13. Acceptance matrix

| Scenario | 必須検証 |
|---|---|
| ATUI-040 Auto | Submit→Stop、Stop→Submit、各signal重複、片側だけでは未完了、両方で一度だけ成功、後続一度だけ起動 |
| ATUI-041 Approval | 両順序、片側だけではWaitingApprovalにならない、両方で一度だけWaitingApproval、対象NodeだけApprove、Fanout兄弟非干渉 |
| ATUI-042 Fault / recovery | 片側signalのdurable待機、restart復元、explicit Retry、新Attempt分離、旧signal遅延、別session / binding拒否、invalid Artifactの全体拒否、完了後追加質問 |

## 14. Quality gate

実装完了前にCIと同じ入口で次を実行する。

```bash
cd src-tauri
cargo fmt --check
cargo clippy -- -D warnings
cargo test

cd ..
pnpm lint
pnpm test
pnpm build
pnpm test:integration
```

追加したproduct acceptanceの個別実行入口も記録し、fixture self-testと区別する。

## 15. 非目標

- deadline、timeout、`Stalled`、signal欠落の自動失敗判定
- Provider transcript本文の読み取り、複製、所有
- process exitまたはTerminal表示からのStop推定
- Releash管理外background processの停止保証
- 旧AgentSessionデータのmigration、backfill、compatibility reader
- Provider利用可否仕様の再設計
- 旧Agent GUI全体の物理削除
- Provider session resume成功の保証

## 16. 完了条件

次のすべてを満たした場合だけIssue #1598の実装完了とする。

- 本Specの外部仕様とlifecycleがDomain / Usecaseの単一authorityで実装されている。
- SubmitとStopが同一Attemptだけで合流し、片側signalは期限なくdurableに待機する。
- Auto、Approval、Fanout、Retry、Pause / Resume、restartが合意仕様どおり動作する。
- ArtifactなしSubmitとvalidated optional Artifactが動作し、旧Attempt Artifactが混入しない。
- Node / Workflow完了後もAgentSessionとPTYが維持され、自動入力されない。
- Workflow全体のWaitingApproval、Paused、Failedを状態遷移authorityとして使用しない。
- ATUI-040、ATUI-041、ATUI-042がproduction境界でGreenである。
- すべてのbehaviorについてRed-Green-Refactorの各段階を実行している。
- 全quality gateがGreenである。
- #1599の旧Agent GUI一括削除以外に、#1598新経路と競合するdead pathを残していない。
