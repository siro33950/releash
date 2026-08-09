# Issue #1597 AgentSession vertical slice implementation specification

## Status

GitHub Issue #1597「provider CLIを使うAgentSessionの縦切りを成立させる」の実装Goal。

本Specに記載した外部仕様、責任境界、不変条件、Red-Green-Refactor cycle、acceptance、削除、品質ゲートをすべて満たした場合だけIssue #1597を完了とする。一部の部品、fixture、mock IPC、旧Message runtime上の代替動作だけが成功しても完了としない。

## Sources of truth

優先順は次のとおりとする。

1. GitHub Issue #1597本文
2. GitHub Issue #1597の「AgentSession lifecycle 採用仕様」コメント
3. GitHub Issue #1597の「Provider起動・Hook・Workflow接続境界 採用仕様」コメント
4. `specs/milestone-87-agent-tui-cutover/acceptance-contract.md`
5. GitHub Milestone 87
6. GitHub Issues #1594、#1595、#1596、#1598、#1599、#1603の責任境界
7. repository rootの`AGENTS.md`
8. `src-tauri/AGENTS.md`
9. `.claude/rules/rust-first-logic.md`
10. `docs/architecture/README.md`
11. `docs/architecture/DOMAIN.md`
12. `docs/architecture/USECASE.md`
13. `docs/architecture/GATEWAY.md`
14. `docs/architecture/INFRASTRUCTURE.md`
15. `docs/architecture/CONTROLLER.md`
16. `docs/architecture/TEST.md`
17. Provider CLIのsupported installed versionと公式仕様

Milestone 87以前のMessage中心のGlossary、旧AgentSession lifecycle、#1596のWorkflow attemptをProviderLifecycle scopeへ含める記述が本Specと矛盾する場合は、本Specと上記1から5を優先する。古い記述を新しいproduction pathへ持ち込まない。

## Required development method

すべてのobservable behavior changeを、独立した完全なRed-Green-Refactorで実施する。

### RED

- 受け入れ済み外部仕様または不変条件を表すテストをproduction codeより先に追加する。
- 最小の対象テストを実行し、未実装または誤実装という意図した理由で失敗することを確認する。
- runnableなblack-box testを書ける場合、未完成testによるcompile failureだけをREDの証拠にしない。
- 現行テストが誤通過する場合はcontrolled mutationまたは反例fixtureで検出力を確認し、mutationを残さない。

### GREEN

- REDを満たす最小のproduction implementationを追加する。
- focused testと直接関連するtest moduleまたはintegration targetを実行する。
- テストの期待値を誤ったimplementationへ合わせて変更しない。

### REFACTOR

- 重複、旧責任境界、dead branch、無効なdependency、二重のstate authorityを削除する。
- Domainの不変条件をusecase、gateway、controller、frontendへ複製しない。
- focused testと関連testを再実行し、Greenを維持する。

一つのcycleがREFACTORまでGreenになる前に、次のobservable behaviorのproduction implementationへ進まない。各cycleで実行したRED、GREEN、REFACTORのcommandと結果を作業報告に残す。

## Issue responsibility

Issue #1597は次を所有する。

- #1595のDurable Terminal Surface、#1596のProvider lifecycle、#1603のProvider availabilityを一つのAgentSession production pathへ接続する。
- Standalone AgentSessionとWorkflow Node AgentSessionの生成、永続化、表示、入力、復帰、終了を成立させる。
- Provider CLIをAgentSession Terminal Surfaceのroot processとして起動する。
- AgentSessionのOpen、Paused、Archived、Delete、GC lifecycleを成立させる。
- Provider session IDとopaque transcript referenceをAgentSessionへ関連付ける。
- Provider transcript historyから同一Worktreeの復帰候補を列挙し、新しいAgentSessionとしてresumeする。
- Hook healthをProviderごとに観測し、アプリ単位の警告へ集約する。
- Workflow初期指示をAgentSessionごとに最大一回だけ送信する。
- ATUI-030をfixtureだけでなくproduction pathへ接続する。
- #1596実装に残るWorkflow execution、NodeExecution、attempt所有とone-shot Stopを、採用済みのAgentSession所有境界へ修正する。

Issue #1597は次を所有しない。

- Provider CLI利用可否の初期化、設定表示、再判定。#1603が所有する。
- Submit、Stop、ApprovalをNode Attemptへ適用するWorkflow遷移。#1598が所有する。
- 旧Agent GUI、旧Message projection、旧Agent runtime全体の最終物理削除。#1599が所有する。
- Provider transcript本文の検索、編集、Releash storeへの複製。
- App process restartまたはPC restartを跨いだ同一PTYまたはProvider processの継続。
- Provider resumeの成功またはresume後のProvider内部挙動の保証。
- 旧AgentSessionデータのmigration、backfill、互換reader。
- Releash管理外へescapeしたbackground processの停止保証。

## State authority and architecture

### AgentSession

新しいTUI production pathのAgentSession aggregateを、AgentSession lifecycleの唯一のstate authorityとする。少なくとも次を所有する。

- Releash AgentSession identity。
- 所属Workspaceと参照Worktree。
- 選択済みProvider。作成後は変更しない。
- StandaloneまたはWorkflow Node所有というorigin。
- Workflow Node所有の場合の固定された所有関係。ProviderLifecycleへWorkflow execution、NodeExecution、attemptを複製しない。
- optionalなprovider session ID。
- optionalなopaque transcript reference。
- Open、Paused、Archived lifecycle。
- Terminal Surface owner identityとの一対一対応。
- Workflow初期指示のdispatchが未要求か、既に一度要求済みかというat-most-once invariant。
- Archive、Restore、Resume、Delete、GC、Provider process exit、SessionStart associationの受理条件。
- `provider + provider session ID`を未Delete AgentSession間で重複所有しない不変条件。

状態遷移の可否はAgentSession aggregateまたはAgentSession domain serviceが判断する。usecase、gateway、controller、frontendに状態機械を置かない。

旧Message runtimeが使用するlegacy Session表現は新しいAgentSession IDのstate authorityにしない。#1599で削除されるまでrepository内に存在する場合も、新しいproduction pathと同一IDまたは同一操作を二重所有させず、どの経路が#1599で削除されるかを明示する。

### Terminal Surface

- AgentSessionはWorkspace内で一つのTerminal Surfaceを所有する。
- identityは`WorkspaceIdentity + AgentSessionId`であり、frontendのmount、tab、PTY generation、temporary keyをdurable identityにしない。
- Terminal Surfaceはlive PTY、terminal checkpoint、bounded scrollback、process exitを所有する。
- AgentSessionはTerminal Surfaceの画面状態を複製しない。
- Terminal Surface process exitはbackendからAgentSession usecaseへ通知され、frontend接続の有無に依存せずlifecycleへ反映される。
- ArchiveではProvider processとPTYを停止する。DeleteまたはGCではAgentSession metadata、Terminal checkpoint、provider resume referenceを削除する。
- App process restart後のcheckpoint restoreと、新しいProvider resume processのspawnを区別する。同一process継続を成功扱いしない。

### ProviderLifecycle

ProviderLifecycleはProvider launch bindingの認証とsignal検証・正規化だけを所有する。

- scopeはProvider、Releash AgentSession、launch binding、launch capabilityから成る。
- Workflow execution ID、NodeExecution ID、attemptをscope、Hook環境変数、durable ProviderLifecycle eventへ含めない。
- root SessionStartからprovider session IDとopaque transcript referenceを正しいAgentSessionへ渡す。
- Provider Stopを現在のTurnが停止した観測として渡す。
- Stop観測によってProviderLifecycle bindingを終端化しない。同じAgentSessionの後続Turnで別のStopを観測できる。
- duplicate Stopが到着しても#1598のWorkflow Attemptを二重遷移させない。ProviderLifecycleにAgentSession全期間のone-shot `stopped` stateを持たせない。
- Provider process exitをProvider Stopへ変換しない。
- ProviderLifecycleはWorkflow state、AgentSession lifecycle、Terminal Surface lifecycleを直接変更しない。
- Hook unavailableは診断・health observationであり、bindingを不可逆な終端状態にしない。後から届いた正常なroot SessionStartを受理できる。
- root agentとsubagentの判定はProvider固有payloadを内側のsignalへ変換するgatewayで行い、subagent signalをroot AgentSessionへ入れない。

### Persistence and read models

- AgentSession lifecycleとprovider associationはdurableに保存し、App process restart後に再構築できる。
- 既存SQLite local event storeまたは既存のcanonical durable storeを使用し、parallel JSON state authorityまたは別DBを追加しない。
- RepositoryはEntityを生成・保存する。Command側はEntityを操作し、Command DTOを新設しない。
- 一覧、設定、履歴候補、画面表示の形はread requirement起点のQueryService responseとし、EntityからDTOへ詰め替えない。
- QueryService実装はdata sourceからbounded read modelを直接構築する。
- provider transcript本文、conversation、thinking、tool output、permission表示、raw Hook payload、raw capability、Local API bearer tokenをAgentSession storeへ保存しない。
- DeleteとGCは最小限のtombstoneだけを残し、一覧、resume ownership、live AgentSessionとして復元しない。

### Layer boundaries

- Domainは外部dependency、serde、tokio、Tauri、SQL、Provider JSON、PTY libraryを知らない。
- UsecaseはDomainの判断を呼ぶ順序と複数集約の調停だけを所有する。process、filesystem、SQLite、transportの具体型を持たない。
- InfrastructureはPTY process、structured executable/argv/env、raw filesystem、raw Provider log、raw Hook stdinをその形のまま扱い、Domain型へ変換しない。
- GatewayはProvider CLI launch、Provider payload、Provider history、persistence表現を内側の言語へ変換する。独自のlifecycle state machineを持たない。
- Controllerはprotocol inputを変換してUsecaseを呼ぶだけとし、RepositoryまたはQueryServiceを直接呼ばない。
- FrontendはProvider選択入力、xterm mount、button操作、error/warning表示、invokeだけを担当し、lifecycle判断、GC判断、Provider検出、history filtering、重複判定を持たない。

## External behavior

### Provider selection and creation

- Standalone New Sessionは#1603が利用可能と判定したProviderを一つ選択するまでAgentSessionとPTYを作成しない。
- 暗黙のdefault Providerを使用しない。
- frontendにClaude、Codex等のProvider一覧を固定しない。
- New Sessionで選択するのはProviderだけとし、model、permission、sandboxはProvider CLI TUIに委ねる。
- 選択済みProviderはAgentSession作成後に変更できない。
- Workflow NodeはWorkflow設定からProviderを確定し、#1603の同じavailabilityを参照する。利用不可ならAgentSessionまたはPTYを作る前に実行エラーとする。
- Provider history候補からの復帰では候補のProviderを使い、別のProvider選択を要求しない。

### Provider process and Terminal Surface

- Provider CLIをshellへの文字列入力ではなく、structured executable、argv、envとしてAgentSession PTYのroot processにする。
- Provider CLI終了後にinteractive shellを残さない。
- ProviderLifecycle launch binding、Hook command、per-launch file、environmentを同じProvider launchへ適用する。
- WorktreeをProvider processのworking directoryにする。
- Provider CLI TUIの画面、入力、permission、追加質問は既存Terminal Surface attach/write/resize経路を使う。
- xterm unmount/remount、tab・workspace切替、renderer reloadで同じlive PTYへgap、duplicate、順序逆転なく再接続する。
- Provider process exit後も最終checkpointを観測でき、AgentSession lifecycle operationが必要な保持・削除を明示的に行う。

### Open and Paused

- live PTYがあるAgentSessionを開くと既存PTYへattachする。
- App process restart等でlive PTYがなくprovider session IDがあるAgentSessionを開くと、自動で同じprovider sessionのresumeを一度試行する。
- 自動resume成功でOpenにする。
- 自動resume失敗でPausedにし、errorとResume操作を表示する。
- Resumeは保存済みprovider session IDを使う同じ復帰工程を再試行する。成功でOpen、失敗でPausedを維持する。
- Releash実行中にProvider CLIが終了しprovider session IDがある場合はPausedにし、errorとResumeを表示する。
- Provider CLI exit reasonを推測して分岐せず、即時自動再起動しない。
- live PTYがなくprovider session IDもない場合はGC対象とする。
- tab、workspace、renderer、frontend切断だけでPaused、Archived、Deleteへ遷移しない。

### Archive, Restore, Delete

- Standalone AgentSessionのXはArchive requestである。
- provider session IDを保持するAgentSessionのArchiveはProvider processとPTYを停止し、Archivedにする。
- provider session IDがないAgentSessionのArchiveは成立させず、Releash AgentSessionをDeleteすることとProvider transcriptが残れば履歴から再接続できることを明示して確認を求める。
- 確認後の縮退DeleteはProvider processとPTYを停止してAgentSessionをDeleteする。
- 対応未確定の最新Provider log等をArchive用provider session IDとして推測しない。
- ArchiveまたはDelete完了時にReleash管理下のProvider processが動作していないことを保証する。
- Archived AgentSessionを開く操作はRestoreであり、保存済みprovider session IDを新しいPTYでresumeする。
- Restore成功でOpenにする。
- Restore失敗ではArchivedを維持し、errorと再試行可能なRestoreを表示する。Pausedへ変えない。
- 明示DeleteはArchived AgentSessionだけに提供する。
- DeleteはReleash所有AgentSession data、Terminal checkpoint、provider resume referenceを削除し、provider transcript本文は削除しない。
- Workflow Node AgentSessionには個別X、Archive、Deleteを提供しない。

### GC

- backendがlive PTY不在を確定しprovider session IDもないOpen AgentSessionだけをGCする。
- frontend disconnect、tab/workspace切替、renderer reload、生死不明ではGCしない。
- App process restart/crash、provider session ID確定前のPTY exit、terminal runtime exitなど、backend-ownedな事実から不在を確定する。
- GCはReleash ownership内のAgentSession data、Terminal checkpoint、provider resume referenceを削除し、最小tombstoneだけを残す。
- Provider transcriptとProvider logを削除しない。
- GCはArchivedだけに提供する明示Delete原則の例外である。

### Provider history resume

- Releash上でDelete済みまたは未所有のProvider sessionを、同一WorktreeのProvider logから列挙する。
- candidate metadataはProvider、provider session ID、最終更新時刻だけとする。
- conversation、thinking、tool、preview、transcript bodyをcandidate表示またはfilteringのために読み込まない。
- history queryはbounded pageまたはbounded limitで返し、全Provider transcriptのfull retentionを行わない。
- candidate選択で同じprovider session IDを持つ新しいReleash AgentSessionを作り、新しいPTYでresumeする。
- Delete済みReleash AgentSession entityは復元しない。
- `provider + provider session ID`は同時に一つの未Delete AgentSessionだけが所有できる。
- Releash管理中のprovider session IDをcandidateから除外する。
- Provider logがない、削除済み、または必要metadataを取得できない場合の復帰を保証しない。
- resume失敗時はprovider session IDを保持するためPausedとして再試行できる。成功を保証しない。

### Hook health

- HookはAgentSession起動、操作、Archive、Delete、history resumeの必須条件ではない。
- Hook未設定でもProvider CLI TUIを起動・操作できる。
- Hook未設定、Provider hook configuration rejected、Local API delivery failure、発生が確定しているSessionStart欠落をProvider別healthとして記録する。
- Hook対象eventが発生していない状態を時間経過だけで故障と断定しない。
- Providerごとの最新launch healthをbackendが所有する。
- 未解消healthが一つでもあれば、画面にアプリ全体の警告を一つ表示する。
- 同じProviderの後続launchで正常なroot SessionStartを観測したら、そのProviderのwarningを解除する。
- Hook異常後の正常SessionStartを受理できる。
- Hook異常またはwarningはProvider process、AgentSession、PTYを終了せず、操作を阻害しない。

### Workflow initial instruction and completion

- Workflow NodeがAgentSessionを作成した場合はNode initial instructionを自動入力する。
- 同じAgentSessionへの自動入力requestは最大一回とする。
- dispatch requestをdurableに確定してからTerminal writeを行い、crashまたは結果不明時に同じAgentSessionへ再送しない。
- renderer reload、attach、App process restart後のresumeで再送しない。
- 未送信または結果不明から再実行する場合は#1598のWorkflow Retryとして新しいAttemptと新しいAgentSessionを作る。
- Provider Stop、Node completion、Workflow completionだけでAgentSessionまたはPTYを停止しない。
- 完了NodeのAgentSessionへユーザーが明示的に追加質問できる。
- 完了後にReleashが自動入力または自動resumeしない。

### Provider transcript boundary

- Provider CLIとProvider transcriptをconversation、thinking、tool表示、permissionの正本とする。
- Releashはこれらを旧Message、MessagePart、独自conversation projectionへ変換しない。
- Releashが保持するのはAgentSession lifecycle、Provider identity、optional provider session ID、optional opaque transcript reference、Terminal Surface identity、Workflow ownershipだけとする。

## Product entry points

新しいproduction pathは少なくとも次の入口を同じRust-owned stateへ接続する。

- Standalone New Session Provider selectionとcreate。
- Workflow Node AgentSession createとinitial instruction dispatch。
- AgentSession list/get/open。
- Terminal Surface attach/write/resize。
- Resume、Archive、Restore、Delete、GC。
- Provider history listとcandidate resume。
- Hook health read。
- ProviderLifecycle SessionStart、Stop、unavailable observation。
- Terminal Surface process exit observation。

TauriとLocal APIまたは将来のWebSocket surfaceは同じUsecaseとbackend-owned stateを使用し、frontend専用state authorityを作らない。

## Red-Green-Refactor implementation cycles

### Cycle 0: baseline and ATUI-030 sensitivity

RED:

- ATUI-030 product acceptanceを追加し、現行production pathではProvider selectionからTUI AgentSession lifecycleまで成立しないことを確認する。
- fixture self-testがproduct acceptanceを代替していないことを確認する。

GREEN:

- なし。baseline failureを記録する。

REFACTOR:

- acceptance helperがproduction seamを迂回しないよう整理する。

### Cycle 1: ProviderLifecycle ownership correction

RED:

- ProviderLifecycle scopeがWorkflow execution、NodeExecution、attemptを要求することを失敗として固定する。
- 一つ目のStop後に同じAgentSessionの後続Stopが拒否またはduplicate化される現行動作を失敗として固定する。
- unavailable後の正常SessionStartが拒否される現行動作を失敗として固定する。
- subagent payloadがroot signalへ変換される現行動作を失敗として固定する。

GREEN:

- scopeをAgentSession launch correlationへ縮小する。
- one-shot stopped stateを除去し、Stop observationを繰り返し受理可能にする。
- unavailableをrecoverable health observationへ分離する。
- Provider gatewayでroot/subagentを変換する。

REFACTOR:

- Workflow/Node/attempt環境変数、event field、codec field、test fixture dependencyを削除する。
- #1598が消費するStop observation以外のWorkflow判断をProviderLifecycleから削除する。

### Cycle 2: AgentSession Domain lifecycle

RED:

- Open、Paused、Archivedの正当な遷移と不正遷移。
- StandaloneとWorkflow Nodeで許可する操作差。
- provider session ID associationと重複ownership拒否。
- Archive ID不明時のconfirmation required。
- Restore failureがArchived、Resume failureがPausedを維持する差。
- initial instruction at-most-once admission。

GREEN:

- AgentSession aggregate、value objects、domain errors、domain eventsを実装する。

REFACTOR:

- lifecycle判断をusecase、legacy state helper、frontendから集約へ戻す。
- Entityとread modelを分離する。

### Cycle 3: durable AgentSession repository and recovery

RED:

- create、provider association、state transition、delete tombstone、restart replay。
- persistence failureでmemory stateだけ進まないこと。
- duplicate provider session IDのatomic rejection。
- transcript bodyとsecretが保存されないこと。

GREEN:

- canonical durable storeのRepositoryとcodecを実装する。
- AgentSession command usecaseとbounded QueryServiceを接続する。

REFACTOR:

- parallel file store、EntityからDTOへの詰め替え、full scan/read modelを除去する。

### Cycle 4: structured root process Terminal Surface

RED:

- shell startup inputではProvider CLIがroot processにならないこと。
- Provider exit後にshellが残ることを検出する。
- exited/cold-restored AgentSession Surfaceへresume processを再spawnできないこと。
- Archive stopとDelete checkpoint removalを区別できないこと。

GREEN:

- executable、argv、env、cwdをstructured spawn requestとしてTerminal infrastructureへ渡す。
- AgentSession Surfaceのruntime replacement、stop、checkpoint保持、Deleteを成立させる。
- process exitをbackend eventとしてAgentSessionへ接続する。

REFACTOR:

- shell command文字列連結、startup command inputによるProvider launch、重複process registryを削除する。

### Cycle 5: Provider launch and Hook health

RED:

- `ProviderLaunchSpec`がTerminal spawnへ接続されていないこと。
- Claude/CodexのNew/Resume argv、per-launch files、environment、working directoryが欠落すること。
- Hook missing/rejected/unavailableがapp warningへ反映されないこと。
- 後続SessionStartでwarningが解除されないこと。

GREEN:

- Provider-specific launch gatewayをAgentSession launch usecaseへ接続する。
- Claude/CodexのNew/Resumeを同じAgentSession launch contractへ変換する。
- per-launch file lifecycleとHook health aggregate/queryを実装する。

REFACTOR:

- Provider非依存mechanicsだけを共有し、Provider field parsingを共通化しすぎない。
- warning判断をfrontendとcontrollerから除去する。

### Cycle 6: Standalone AgentSession vertical slice

RED:

- Provider未選択でcreateできる現行動作。
- default backend/model/permissionを暗黙選択する現行動作。
- 新Sessionが旧Message UI/runtimeを必要とする現行動作。

GREEN:

- #1603のProvider availabilityを消費するcreate command/queryを接続する。
- Provider-only picker、AgentSession terminal panel、list/openを接続する。
- model/permission UIを新TUI pathへ表示しない。

REFACTOR:

- 新TUI pathからlegacy backend default、model resolver、permission state、Message projection dependencyを削除する。

### Cycle 7: Resume, Archive, Restore, Delete, GC

RED:

- live attach、自動resume、Paused manual Resume、Archive、Restore failure、Delete制約、ID不明Archive縮退、GC条件の各scenario。
- frontend disconnectまたはunknown PTY stateでGCされる誤動作。
- Archive/Delete完了後にmanaged Provider processが残る誤動作。

GREEN:

- AgentSession usecase、Terminal Surface operation、controller、minimal UIを接続する。

REFACTOR:

- lifecycle分岐をfrontendから削除し、同一operationを一つのUsecaseへ集約する。

### Cycle 8: Provider history resume

RED:

- Claude/Codexで同一Worktree候補をmetadataだけで取得できないこと。
- transcript bodyを読む実装、unbounded full scan、managed IDの再表示、duplicate ownershipを検出する。

GREEN:

- raw Provider log accessをInfrastructure、Provider metadata変換をGateway、bounded read modelをQueryServiceへ実装する。
- candidate resumeを新AgentSession createとProvider resumeへ接続する。

REFACTOR:

- Providerごとの重複filtering、frontend filtering、transcript body retentionを削除する。

### Cycle 9: Workflow Node integration

RED:

- Workflow Nodeがlegacy Message runtimeへSessionを作ること。
- initial instructionのduplicate、reload/restart時再送、unknown dispatch retryを検出する。
- Node completionまたはStopでAgentSession/PTYが終了することを検出する。

GREEN:

- Workflow Node createを新AgentSession usecaseへ接続する。
- persist-before-writeのat-most-once initial instruction dispatchを実装する。
- completion後の明示的追加質問を維持する。

REFACTOR:

- #1598のWorkflow transitionを取り込まず、AgentSession ownershipとStop observationだけを公開する。

### Cycle 10: ATUI-030 production acceptance

RED:

- 各production entry pointまたはProvider integrationを一時的に外すcontrolled mutationでATUI-030が失敗することを確認する。

GREEN:

- Claude/Codex、fixture fault、Tauri/backend pathを含む全acceptance matrixを成功させる。

REFACTOR:

- acceptance専用production seam、fixture-only success、旧Message fallbackを削除する。
- Issue、comments、contract、Spec、規約とproduction pathを再監査する。

## ATUI-030 product acceptance matrix

少なくとも次をproduction boundaryで自動検証する。

- Provider未選択ではAgentSessionもPTYも作られない。
- #1603が返す利用可能Providerだけが候補になる。
- default Provider、model、permissionをReleashが暗黙選択しない。
- Claude New Sessionを実PTY root processとして起動できる。
- Codex New Sessionを実PTY root processとして起動できる。
- Provider exit後にshellが残らない。
- TUI入力、permission応答、追加質問をTerminal Surfaceへ送れる。
- tab/workspace切替、unmount/remount、renderer reloadで同じlive PTYへ再接続できる。
- Provider session IDなしでもAgentSessionとPTYを利用できる。
- root SessionStartだけがprovider session IDを関連付ける。
- subagent SessionStartとStopがroot AgentSessionを変更しない。
- 一つのAgentSessionで複数TurnのStopを観測できる。
- Hook unavailable後の正常SessionStartを受理できる。
- Hook unavailableでProvider process、AgentSession、PTYを停止しない。
- Provider別healthを一つのapp warningとして表示し、後続成功で解除する。
- App restart後、live PTYなし・known provider IDのSessionを開くと自動resumeを試行する。
- 自動resume失敗はPaused、manual Resume成功はOpenになる。
- live Provider exitはknown IDならPaused、unknown IDならGC/Deleteになる。
- Archive known IDはProvider processを停止してArchivedになる。
- Archive unknown IDは確認なしに推測またはDeleteしない。
- 確認後のArchive縮退Deleteでmanaged processとAgentSessionを削除する。
- Restore成功はOpen、Restore失敗はArchivedを維持する。
- 明示DeleteはArchivedだけで可能で、checkpointとresume referenceを削除する。
- frontend disconnect、生死不明、tab切替ではGCしない。
- confirmed no PTYかつno provider IDだけをGCする。
- history candidateは同一Worktreeのprovider、session ID、updated timeだけをboundedに返す。
- transcript body、thinking、tool、previewをhistory queryで読まない。
- managed provider session IDをhistory candidateから除外する。
- candidate resumeはDelete済みentityを復元せず新AgentSessionを作る。
- 同一`provider + provider session ID`を二つの未Delete AgentSessionが所有できない。
- Workflow initial instructionを同一AgentSessionへ一回だけ送る。
- unknown dispatch、reload、resumeでinitial instructionを再送しない。
- Provider Stop、Node completion、Workflow completionでAgentSession/PTYを終了しない。
- 完了NodeのSessionへ明示的に追加質問できる。
- 旧Message/MessagePart projectionを無効にしても全scenarioが成立する。
- Provider transcript body、raw Hook payload、secretをReleash AgentSession storeへ保存しない。

## Test placement and naming

- Domain、Usecase、Gatewayの新規behavior testは必須とする。
- 同一moduleのtestは実装と同じdirectoryの`*_test.rs`へ配置する。
- test functionは`test_{業務機能}_{条件と期待結果}`形式で日本語の業務機能と期待結果を表す。
- test moduleは`{implementation_name}_tests`形式とする。
- multi-layer product acceptanceは`src-tauri/tests/`へ配置する。
- frontend testは対象component/hookの隣に置き、表示・入力・invokeだけを検証する。Domain判断のmirror testを作らない。
- 外部processをunit testで直接実行せず、process mechanicsはintegration/acceptanceで検証する。

## Boundary with Issue #1603

#1597は、Provider executableをbackendで検証して得たavailabilityを、Standalone Provider selectionとWorkflow Provider validationの共通production入力として使用する。temporary hard-coded Provider list、legacy `AgentBackendRegistry` default、frontend executable detectionは作らない。

#1603はavailabilityの初期化時評価、設定画面、再評価、および設定方式の最終仕様を所有する。#1597では現在のproduction executable probeを使って縦切りを完了でき、#1603の詳細設計・実装を完了条件にはしない。

## Quality gates

各cycleのfocused RED/GREEN/REFACTORに加え、最終的にCIと同じ次のgateをすべて成功させる。

```bash
cd src-tauri
cargo fmt --check
cargo clippy --locked -- -D warnings
cargo test --locked --test agent_tui_harness
cargo test --locked --test terminal_surface_acceptance
cargo test --locked --test provider_lifecycle_acceptance_test
cargo test --locked --test provider_lifecycle_characterization_test -- --ignored --nocapture
cargo test --locked --test agent_session_tui_acceptance
cargo test --locked --test agent_session_tui_acceptance -- --ignored --nocapture
cargo test --locked
cargo deny --locked check
cargo build --locked

cd ..
pnpm lint
pnpm test
pnpm build
pnpm test:integration
qlty check --no-progress --all
```

実際のacceptance target名はREDで追加したproduction test名へ合わせるが、ATUI-030を単体test、fixture self-test、mock controller testだけで代替しない。`agent_session_tui_acceptance`の通常実行はfixture matrix、`--ignored`実行はinstalled Claude/Codexを実PTY root processとして起動するmanual/release gateであり、両方の成功を完了条件とする。

## Completion criteria

次をすべて満たした場合だけGoalをcompleteとする。

- Cycle 0から10まで、各observable behaviorのRED、GREEN、REFACTORを順番に完了した。
- Issue #1597本文と両採用コメントの全項目をproduction pathで満たした。
- ATUI-030 product acceptance matrixの全項目がGreenである。
- ClaudeとCodexの両方を実PTYのroot processとして検証した。
- #1603のproduction availability boundaryをNew SessionとWorkflow validationが使用している。
- AgentSession lifecycleのstate authorityがRust Domainに一つだけ存在する。
- ProviderLifecycleがWorkflow execution、NodeExecution、attemptを所有せず、StopをAgentSession全期間のone-shot stateにしていない。
- frontendがlifecycle、GC、history filtering、Provider検出、duplicate ownershipの判断を持たない。
- new TUI pathが旧Message、MessagePart、legacy backend default、legacy model/permission selectionへ依存しない。
- Provider transcript bodyとsecretを保存していない。
- App restart recovery、Paused、Archive、Restore、Delete、GC、history resumeがdurable stateと整合する。
- Node completion後もAgentSessionとPTYが残り、追加質問できる。
- 全quality gateが成功した。
- 最後にIssue #1597、両コメント、Milestone 87、acceptance contract、root/Rust/architecture/test規約を全文再読し、非準拠をすべて追加のRed-Green-Refactorで修正した。
