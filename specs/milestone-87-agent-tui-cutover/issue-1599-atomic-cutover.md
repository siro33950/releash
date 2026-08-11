# Issue #1599 Agent TUI atomic cutover implementation specification

## 1. 目的

Milestone 87で成立したProvider CLI TUI、Durable Terminal Surface、Provider lifecycle、Provider availability、AgentSession lifecycle、Workflow control planeを唯一のproduction経路へ切り替える。

旧Agent GUI、旧Session / Turn / Message / MessagePart model、旧Agent runtime、旧Provider adapter、旧conversation read model、旧operation surfaceを物理削除し、新旧のstate authority、runtime、Config、UI、API、Docsが共存しない状態にする。

本Issueはdead codeを到達不能にするだけのcleanupではない。途中状態を製品releaseせず、integration branch上で一つのAgentSession契約へatomic cutoverする最終Issueである。

## 2. 正本と優先順位

本Specは次の正本を実装可能な単位へ落としたものである。

1. GitHub Milestone 87「Agent TUI移行・一括切替」
2. GitHub Issue #1599本文と全コメント
3. `specs/milestone-87-agent-tui-cutover/acceptance-contract.md`
4. GitHub Issue #1596および`issue-1596-provider-lifecycle.md`
5. GitHub Issue #1597および`issue-1597-agent-session-vertical-slice.md`
6. GitHub Issue #1598および`issue-1598-workflow-control-plane.md`
7. GitHub Issue #1603および`issue-1603-provider-availability.md`
8. repository rootの`AGENTS.md`
9. `src-tauri/AGENTS.md`
10. `.claude/rules/rust-first-logic.md`
11. `docs/architecture/`のlayer規約
12. 本Spec

旧Agent Chatを前提とするGlossary、Milestone 84のAgent Chat文書、過去Issue spec、現行実装が上位の正本と矛盾する場合は、本SpecとMilestone 87の正本を優先する。

## 3. 現在の不一致

integration branchには新しいAgentSession production pathが成立している一方、次の旧経路が残っている。

- Workflow Session Nodeは新しいAgentSession TUIを表示するが、Workspace treeは旧direct Session projectionも列挙する。
- Standalone AgentSessionは新TUIを使用するが、旧closed Sessionと旧Session historyもUIへ表示される。
- `MainLayout`が旧`AgentChatProvider`を常時mountし、旧Session hydration、stream購読、activity projectionを維持する。
- `AgentChatPanel`、`useAgentChat`、`useAgentSdkListeners`、`useSessionStore`、旧Message reducerがproduction bundleに残る。
- 旧Agent Tauri command、Local API、protocol DTO、presenterが登録されたままである。
- Claude stream-json / Codex app-serverをReleash独自Message modelへ変換する旧Provider adapterとruntimeが残る。
- 旧SessionStore、Message projection、tool output blob、attachment blob、operation recoveryが新AgentSession lifecycleと同じprocessに存在する。
- Workflow approval chatとReview Thread handoffが旧`send_agent_message`へ依存する。
- application shutdown coordinatorが旧Agent runtime Sessionを個別shutdown targetとして列挙する。
- Configに旧backend defaultと旧model listが残る。
- canonical docsがSession / Turn / Message / PermissionRequestをReleash-owned modelとして記載する。
- 新経路を区別するために導入した`ProviderAgentSession`という暫定名が、canonicalな`AgentSession`と並存する。

## 4. 達成すべき外部仕様

### 4.1 AgentSession surface

- StandaloneとWorkflow NodeのAgentSessionは同じAgentSession TUI surfaceを使用する。
- Provider CLI TUIがconversation、thinking、tool表示、permission、model、sandbox、対話入力のsurfaceである。
- ReleashはProvider transcript本文を読まず、独自Message / MessagePart projectionを生成しない。
- Workflow Session Nodeを開くと、そのNodeが参照するAgentSessionのTerminal Surfaceへattachする。
- Standalone AgentSessionを開くと、同じAgentSessionのTerminal Surfaceへattachする。
- 完了Node、WaitingApproval Node、Abort済みWorkflowのAgentSessionとPTYは自動終了しない。
- ユーザーはNode完了後も同じTUIへ追加質問できる。

### 4.2 AgentSession lifecycle

- #1597で確定したOpen、Paused、Archived、Restore、Resume、Delete、GC、history resumeを維持する。
- Standalone AgentSessionだけがArchive / Restore / Deleteを提供する。
- Workflow Node AgentSessionに個別Archive / Deleteを追加しない。
- live PTY不在時の自動resume、失敗時Paused、明示Resumeを維持する。
- provider session IDなしでArchiveできない場合の確認付きDeleteを維持する。
- provider transcript historyから同一Worktreeの未管理sessionを新しいAgentSessionとして復帰できる。

### 4.3 Workflow control plane

- SubmitとProvider Stopは独立signalのまま維持する。
- Auto / Approvalは同一AttemptのSubmitとStopが揃った場合だけ進行する。
- ArtifactはSubmitへ任意添付でき、Artifactなしでも完了意思を表明できる。
- WaitingApproval、Approve、Retry、Attempt分離、遅延signal拒否を維持する。
- Workflow初期指示はTerminal Surface inputとして対象AgentSessionへ最大一回だけ送る。
- 旧Agent Turn completion、旧Message送信、旧approval chat commandからWorkflowを進行させない。

### 4.4 Provider availability / lifecycle

- Provider選択、Workflow検証、process起動は#1603の同じbackend-owned registryを参照する。
- Provider lifecycleはroot AgentSessionのsession identity、opaque transcript reference、Stopを観測する。
- Hook未設定または機能不全でもAgentSession操作を阻害せず、アプリ単位の警告を維持する。
- #1596で削除済みの旧Claude専用Hook、hook port、旧Hook Config、旧Hook commandを再導入しない。
- ユーザー所有のClaude / Codex設定、Hook設定、Provider transcriptを削除または上書きしない。

### 4.5 旧データ

- 旧AgentSession、Message、MessagePart、tool output、attachmentを読み取らない。
- migration、backfill、互換decoder、互換readerを実装しない。
- 旧データの物理削除を行わない。
- 旧projection rowや旧ファイルが残っていても、新AgentSession一覧、Workspace tree、Workflow、起動、shutdownを失敗させない。

## 5. canonical state ownership

cutover後のstate authorityは次の一つずつとする。

| 状態 | 正本 |
|---|---|
| AgentSession identity、Provider、origin、lifecycle、provider session ID、opaque transcript reference、Terminal ownership | Rust AgentSession aggregateとdurable lifecycle projection |
| conversation、thinking、tool、permission、model、Provider UI | Provider CLI / Provider transcript |
| live PTY、screen、cursor、style、bounded scrollback、checkpoint | Durable Terminal Surface |
| WorkflowExecution、NodeExecution、Attempt、Submit、Stop適用、Artifact、Approval | Workflow domain / control plane |
| Provider executableと利用可否 | Provider availability registry |
| root Provider session identity、transcript reference、Stop観測 | Provider lifecycle |
| editor、layout、selected surface | frontend UI state mirror |

Frontend、Tauri controller、Local API、Gatewayは上記状態を再定義しない。

## 6. canonical naming cutover

旧Sessionとの共存を表すための暫定的な`ProviderAgentSession`名称を、cutover後の正規語`AgentSession`へ統一する。

- Domain正規語は`AgentSession`とする。
- frontend component、hook、type、eventは`AgentSession`を使用する。
- usecase、repository、query、protocol DTOは`AgentSession`を使用する。
- Tauri commandは`create_agent_session`、`list_agent_sessions`、`get_agent_session`、`open_agent_session`、`resume_agent_session`、`archive_agent_session`、`restore_agent_session`、`delete_agent_session`等のcanonical名へ切り替える。
- Provider固有の外部境界である`ProviderLifecycle`、`ProviderAvailability`、`ProviderKind`、`ProviderSessionId`はProvider名称を維持する。
- 旧command名や暫定command名のaliasを残さない。
- 旧protocol versionまたはcompatibility facadeを追加しない。

## 7. Frontend cutover

### 7.1 維持するUI

- AgentSession TUI panel / route
- Durable Terminal Surface attach / input / resize / theme
- WorkspaceごとのAgentSession一覧
- NewSessionのProvider picker
- Archive / Restore / Delete / Resume / history resume
- Workflow tree、Node header、Submit / Stop待機表示、Approve / Retry
- Provider availability Settings
- Provider Hook health warning
- application-level shutdown feedback

### 7.2 削除するUI / state

- `AgentChatPanel`全体
- `AgentChatContext`
- `useAgentChat`
- `useAgentSdkListeners`
- legacy AgentChat reducerとstream projection
- legacy `useSessionStore`
- SessionFeedback / Notice UIの旧Message runtime経路
- model selector、permission selector、plan mode、Message composer、tool/activity renderer
- 旧Session / closed Session / legacy Session history表示
- 旧Session用center selectionと`content.kind === "session"`
- legacy Agent operation supervisionのSession scope
- 旧Agent commandにだけ使うfrontend type / utility / event
- 旧Agent operation identityを保持するlocalStorage

### 7.3 Workspace tree

- Workspace treeはWorkflowとWorkflow Nodeだけをbackend tree projectionから読む。
- Standalone AgentSessionはbounded AgentSession queryから別に列挙する現行方式を維持する。
- 旧`session_projection.public_summary`からdirect SessionをWorkspace treeへ混ぜない。
- closed legacy SessionをSessionHistoryへ表示しない。
- Provider historyとArchived AgentSessionだけをSessionHistoryへ表示する。

## 8. Review Thread handoff

旧`send_agent_message`による自動送信は廃止する。Provider TUIの状態を推測してPTYへ文字列とEnterを注入しない。

採用する外部フローは次とする。

```text
Review Thread
  -> Rust usecaseがhandoff instructionを生成
  -> frontendが生成済みtextをclipboardへコピー
  -> ユーザーが対象Provider TUIへ貼り付けて送信
```

- Diff commentの操作は「Agentへ送信」ではなく「Agent向け指示をコピー」として表示する。
- active legacy Sessionの存在を操作可否に使用しない。
- handoff instructionの内容生成は引き続きRustが所有する。
- clipboard write成功・失敗をUIで観測できる。
- clipboard操作はconversation stateまたはAgentSession stateを変更しない。

## 9. Backend cutover

### 9.1 削除するproduction経路

- legacy Agent backend registryとdefault backend解決
- Claude stream-json backendとCodex app-server backend
- wireからMessage / MessagePartへ変換するadapter
- legacy Agent runtime、queue、turn、permission、streaming、watchdog、recovery
- legacy SessionStore、message store、blob store、projection commit
- legacy Agent operation command、recovery command、notice / feedback command
- legacy Agent Tauri command registration
- legacy Agent Local API route
- legacy Agent protocol DTO / presenter / event notifier
- legacy Agent-specific test fixture、golden、acceptance
- legacy Workflow approval chat commandとruntime adapter
- legacy direct Session creation / restore / archive / fork / model switching

### 9.2 維持するproduction経路

- AgentSession aggregate、lifecycle usecase、query、repository
- structured Provider launch
- AgentSession Terminal gateway
- Provider history query
- Provider availability
- Provider lifecycle association
- workflowからAgentSessionを作成するlaunch boundary
- Terminal Surface
- Workflow control plane
- Local APIのHook専用commandとWorkflow command

### 9.3 Layer boundary

- AgentSession lifecycle判断はDomain aggregateが所有する。
- Usecaseはrepository、Provider launch、Terminal Surface、Provider lifecycleを調停する。
- Gatewayはpersistence / provider history / process launchとの変換を行う。
- InfrastructureはPTY、process、filesystem、Provider history raw dataを外部世界の形で扱う。
- ControllerはTauri / Local API inputをUsecaseへ変換するだけとする。
- frontendにlifecycle、availability、resume、GC、Workflow遷移判断を置かない。

## 10. Persistence cutover

- legacy `SessionProjectionRecord::AgentSession`とMessage projectionを削除する。
- canonical AgentSession lifecycle projectionだけをAgentSession queryの対象にする。
- projection storage keyはcanonical AgentSession namespaceへ統一する。
- generic queryが旧projectionをdecodeしないよう、AgentSession queryをcanonical namespaceで限定する。
- existing legacy rowsは無視し、decode failureやWorkspace tree failureを起こさない。
- old projection migration、row rewrite、table cleanupを行わない。
- Provider session ownership、Provider lifecycle、Hook health、Workflow projection、Terminal checkpointを維持する。
- generic local event transaction、idempotency、Workflow recovery、application shutdownに必要なrecordは維持する。
- legacy Agent runtimeだけが使用するoperation / obligation / projection variantは参照元削除後に物理削除する。
- generic application / Workflow recovery型がlegacy AgentSession moduleに置かれている場合は、意味を変更せず正しいdomain / usecase ownerへ移す。

## 11. Application shutdown cutover

- application shutdown coordinator自体は維持する。
- WorkflowExecution shutdown targetを維持する。
- Provider exit observer、Terminal Surface shutdown、checkpoint persistence、Local API shutdownを維持する。
- legacy Agent runtime Sessionを個別shutdown targetとして列挙しない。
- AgentSessionのmanaged PTYはTerminal Surface shutdownが一括して停止・checkpointする。
- AgentSession lifecycleをApp終了時にArchivedまたはDeletedへ変更しない。
- 次回起動後はlive PTY不在として#1597のopen / resume / GC規則を適用する。
- application shutdown protocolとUIをlegacy Agent protocolから分離する。
- app quit / restartのsingle-flight、durable activation、outcome unknown、retry可能なWorkflow targetという既存保証を落とさない。

## 12. Config cutover

- `agents.claude.cli_path`と`agents.codex.cli_path`を維持する。
- `agents.default`をmodel、domain repository、wiring、test、serializationから削除する。
- `agents.claude.models`と`agents.codex.models`を削除する。
- implicit default Providerを追加しない。
- model、permission、plan、sandboxのReleash Configを追加しない。
- 旧Config migrationを実装しない。
- 旧keyを含むConfigは未知fieldとして無視できるが、新しい保存結果へ旧keyを再出力しない。
- Provider availabilityのupdate / reset / refreshとresolved executableを維持する。

## 13. Hook cutover boundary

- 旧Hook実装の物理削除は#1596完了済みであり、#1599で別実装を作らない。
- legacy Hook command、hook port、frontend Hook settings、Claude専用Hook gatewayが登録されていないことを検査する。
- current Hook-only CLI、authenticated Local API、per-launch capability、Provider Hook healthを維持する。
- Provider設定ファイルへの操作は#1596 / #1603の現行実装だけを使用する。

## 14. Canonical Docs cutover

次の文書を現行AgentSession契約へ更新する。

- `docs/architecture/README.md`
- `docs/architecture/GLOSSARY.md`
- `docs/architecture/INFRASTRUCTURE.md`
- `docs/architecture/TEST.md`
- 必要な`DOMAIN.md`、`USECASE.md`、`GATEWAY.md`、`CONTROLLER.md`
- `docs/domain-model/current-state.md`
- `docs/workflow-engine-model-boundary.md`
- `docs/workflow-engine-evolution-plan.md`
- `docs/workflow-yaml-syntax.md`
- `docs/spec/README.md`

Docsは少なくとも次を明示する。

- canonical語はAgentSessionである。
- ReleashはTurn、Message、MessagePart、PermissionRequestを所有しない。
- Provider CLI / transcriptがconversationの正本である。
- AgentSessionはlifecycleとTerminal ownershipを所有する。
- Terminal SurfaceはWorkspaceまたはAgentSessionに所有される。
- Workflow NodeExecutionはAgentSessionを参照するが所有しない。
- Workflow completionとAgentSession lifecycleを分離する。
- Submit / Stop / Approval / ArtifactはWorkflowが所有する。
- Provider lifecycleとProvider availabilityの境界。
- 旧Agent GUI specは現行正本ではない。

`docs/agent-model-selector-direction.md`は旧GUI専用のため削除する。Milestone 84 Agent Chat文書は歴史的資料として明示し、canonical indexから参照しない。現在も必要な永続化・競合・recovery invariantはcanonical architecture / lifecycle文書へ移す。

## 15. Dependency / package cleanup

- legacy Agent GUIだけが使用するfrontend dependencyを削除する。
- legacy Agent provider runtimeだけが使用するRust moduleと未使用dependencyを削除する。
- test-only importでdead production moduleを生存させない。
- `#[allow(dead_code)]`、compatibility facade、空のmoduleを残して削除を回避しない。
- generated lockfileをdependency削除へ同期する。

## 16. ATUI-050 product acceptance

ATUI-050はrepository scanだけでなくproduction境界を通して次を検証する。

1. Standalone AgentSession作成からTUI attach、input、Archive / Restore / Deleteがcanonical commandで動作する。
2. Workflow Session Nodeが同じAgentSession TUI surfaceを使用し、旧Message runtimeを無効にしても開始・初期指示・追加質問が動作する。
3. Submit / Stop / Approval / Retryが旧turn completionなしで動作する。
4. 旧Session projection rowを配置してもAgentSession一覧とWorkspace treeが失敗せず、旧Sessionを表示しない。
5. application quitでWorkflow target、Terminal checkpoint、Provider observer、Local API shutdownが実行され、旧Agent runtime targetを要求しない。
6. Config serialize結果に`agents.default`と`models`がなく、Provider executable overrideが維持される。
7. Review Thread handoff instructionをclipboardへコピーでき、Agent commandを呼ばない。
8. legacy Tauri commandとlegacy Local API routeが利用できない。
9. repository scanで旧GUI、旧runtime、旧Provider adapter、旧Message projection、旧command registrationが物理削除済みである。
10. ATUI-010、011、012、020、021、025、030、040、041、042が引き続きGreenである。

## 17. Red-Green-Refactor実施順

すべてのobservable behavior changeを独立した完全なRed-Green-Refactorで実施する。一つのcycleをRefactorまでGreenにする前に次のproduction behaviorへ進まない。

### Cycle 0: ATUI-050 sensitivity

RED:

- canonical commandだけを要求するATUI-050 acceptanceを追加する。
- 旧Session row非表示、旧command拒否、Config旧field不在、shutdownの旧Agent target不在をproduction境界で失敗させる。
- failureがfixture不足やcompile用stubではなく現行の新旧共存によることを確認する。

GREEN:

- なし。外側のacceptance failureを記録する。

REFACTOR:

- ATUI-030やunit testがATUI-050を代替していないことを確認する。

### Cycle 1: Workspace / frontend surface一本化

RED:

- Workspace treeが旧direct Sessionを公開しないtestを追加する。
- StandaloneとWorkflow Nodeがcanonical AgentSession routeを使用するcomponent testを追加する。
- legacy closed Session historyが表示されないtestを追加する。

GREEN:

- Workspace tree queryからlegacy Session projectionを外す。
- Node contentとWorkspace listをAgentSession TUIへ一本化する。

REFACTOR:

- AgentChatContext、legacy selection、legacy Session UI state、関連testを削除する。
- focused frontend / Rust testを再実行する。

### Cycle 2: Review Thread clipboard handoff

RED:

- active Sessionなしでhandoff instructionを生成・copyできるtestを追加する。
- legacy Agent send commandを呼ぶ実装を失敗として固定する。
- clipboard failureがUIに表示されるtestを追加する。

GREEN:

- Rust instruction生成とfrontend clipboard操作を接続する。

REFACTOR:

- ReviewThreadHandoffからAgentChatContext依存を削除する。
- 旧送信文言、旧send pathを削除する。

### Cycle 3: canonical AgentSession command / type naming

RED:

- canonical AgentSession command、protocol、frontend typeだけを使用するtestを追加する。
- 旧command名と暫定provider-prefixed command名が登録されている現状を失敗させる。

GREEN:

- 新AgentSession経路をcanonical名称へ切り替える。

REFACTOR:

- compatibility alias、duplicate DTO、temporary facadeを削除する。

### Cycle 4: legacy command / API / GUI removal

RED:

- legacy command名が未登録であるrouter testを追加する。
- legacy Local API routeが利用不能であるtestを追加する。

GREEN:

- legacy controller、protocol、presenter、frontend callerを切断する。

REFACTOR:

- AgentChatPanel、hooks、reducer、legacy types、testsを物理削除する。
- command / API moduleを物理削除しrouterを簡潔化する。

### Cycle 5: legacy runtime / provider adapter removal

RED:

- Workflow start、AgentSession start、Provider permission、追加質問が旧runtimeなしでATUI acceptanceを通ることを確認する。
- application compositionがlegacy backend registryを要求する現状をfailureとして固定する。

GREEN:

- composition rootを新AgentSession、Terminal Surface、Provider lifecycle、Provider availabilityだけで構築する。

REFACTOR:

- legacy backend registry、runtime、Claude/Codex adapter、wire model、fixture、operation pathを物理削除する。
- unused dependencyとtest supportを削除する。

### Cycle 6: persistence projection cutover

RED:

- 旧projection rowが存在してもcanonical AgentSession queryとWorkspace treeが成功して旧rowを返さないtestを追加する。
- Message projectionを書込・読込できる現状をcutover違反として検出する。

GREEN:

- canonical namespace限定queryとAgentSession lifecycle projectionへ一本化する。

REFACTOR:

- legacy projection variant、codec、message/blob store、query、mutation、migrationを物理削除する。
- generic local event / Workflow / shutdown recordを正しいownerへ整理する。

### Cycle 7: application shutdown cutover

RED:

- shutdown inventoryがWorkflowだけをdurable targetとし、Terminal Surfaceをsubordinate shutdownするtestを追加する。
- legacy AgentSession targetを要求する現状を失敗させる。

GREEN:

- shutdown compositionをWorkflow、Terminal Surface、Provider observer、Local APIへ接続する。

REFACTOR:

- legacy Session lifecycle shutdown、Agent operation protocol依存、Session supervisionを削除する。
- application shutdown固有型を正しいmoduleへ移す。

### Cycle 8: Config cutover

RED:

- Config roundtripがProvider cli pathを保持し、default / modelsを出力しないtestを追加する。
- provider selectionがimplicit defaultを参照しないことを検証する。

GREEN:

- Config model、repository、wiringからlegacy fieldを削除する。

REFACTOR:

- legacy AgentConfigRepository method、backend registry wiring、test fixtureを削除する。

### Cycle 9: canonical Docs / dependency cleanup

RED:

- canonical doc indexと禁止旧語彙のcutover checkを追加する。
- repository scanで旧production module、command、Config fieldが残っているため失敗することを確認する。

GREEN:

- canonical docsを新state ownershipへ更新する。
- obsolete docを削除またはhistoricalへ明示的に降格する。

REFACTOR:

- dead package、empty module、obsolete test、stale importを削除する。

### Cycle 10: ATUI-050 Green / release gate

RED:

- Cycle 0の未達境界を列挙し、product path以外のmockだけで通っていないことを確認する。

GREEN:

- ATUI-050をproduction境界でGreenにする。

REFACTOR:

- 全repository scan、全acceptance、通常CI、package / release buildを実行する。
- 新旧共存のための分岐、alias、compatibility readerがないことを再確認する。

## 18. 品質gate

実装完了前に少なくとも次を実行し、すべて成功させる。

```bash
pnpm lint
pnpm test
pnpm build

cd src-tauri
cargo fmt --check
cargo clippy -- -D warnings
cargo test

cd ..
pnpm test:integration
```

加えて次を実行する。

- ATUI-050単独test
- terminal surface acceptance
- provider lifecycle acceptance
- AgentSession TUI acceptance
- Workflow control plane acceptance
- Provider availability acceptance
- package / release build
- repository scanによるlegacy production path不在確認

## 19. 非対象

- 旧AgentSessionデータのmigration、backfill、互換reader、物理削除
- Provider transcript本文の読み込み、コピー、index、検索
- Provider CLI内部のconversation / permission / model UI再実装
- App process restartを跨ぐ同一PTY / processの継続
- provider resume成功の保証
- remote / mobile向けTerminal transportの新設
- daemon化
- 新Providerの追加
- Provider CLI version compatibility判定
- Workflow lifecycleそのものの新機能
- Review Thread instructionのProvider TUIへの自動注入
- 旧Hook機構の再設計

## 20. 完了条件

次のすべてを満たした場合だけ#1599を完了とする。

- 本Specの外部仕様とstate ownershipを満たす。
- ATUI-050がproduction境界でGreenである。
- ATUI-010〜042が回帰していない。
- Agent操作がcanonical AgentSession TUI surfaceだけを使用する。
- 旧Agent GUI、旧Message projection、旧Agent runtime、旧Provider adapter、旧command / APIが物理削除されている。
- frontendとbackendに二重のAgentSession state authorityがない。
- Configにimplicit default Providerとmodel listがない。
- Review Thread handoffがclipboard copyとして動作し、legacy Agent sendに依存しない。
- application shutdownがWorkflow、Terminal Surface、Provider observer、Local APIを正しく停止し、legacy Agent runtimeを要求しない。
- 旧dataを読まず、旧dataの存在で新queryが壊れない。
- canonical docsが新AgentSession、Provider transcript、Terminal Surface、Workflow control planeの所有関係と一致する。
- obsolete docsが現行正本として参照されない。
- frontend、Rust、integration、package / release buildの品質gateがすべて成功する。
- integration branchからmainへの一括cutover PRを作成できる状態である。
