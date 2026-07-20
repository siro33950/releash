# Design

本書は、[agent-chat-ideal-vocabulary.md](agent-chat-ideal-vocabulary.md) §9.5 / §10 が定義する論理契約を、将来のSQLite authorityへ実装するためのD3物理設計正本である。[Issue #1499 requirements](../../docs/specs/issues-1499/requirements.md) R-013 / R-018と[behavior](../../docs/specs/issues-1499/behavior.md) B-050 / B-098を満たし、F3 #1385、F8 #1491、F9 #1494、F10 #1497が共有する境界を確定する。

本書は設計判断、不変条件、失敗時の公開結果、移行条件、定量上限を定義する。公開DTO、状態遷移、codecのfield順、KATはVocabulary、[agent-chat-ideal-lifecycle.md](agent-chat-ideal-lifecycle.md)、[close-quit-decision-table.md](close-quit-decision-table.md)、[Issue #1499 design](../../docs/specs/issues-1499/design.md)を正本とし、本書では再定義しない。SQL DDL、trigger全文、Rust型全文、registryの実装件数、fixture全列挙はF3の実装・migration・test artifactに置く。

## The actual design

### Architecture

#### 目的と責任境界

D3は、Phase 0 file storeからSQLite transactional event storeへauthorityを一度だけ移す際の、実装可能な物理契約を定義する。責任は次のとおり分離する。

| 対象 | Owner | 本書で確定すること |
| --- | --- | --- |
| Phase 0 file-store closure、authority pointer、fault injection、Tauri / WebSocket parity | #1499 | F3へ渡すsourceとclosure不変条件 |
| SQLite transaction、schema family、stream head、global sequence、envelope、raw preservation、one-shot import / cutover、managed backup / restore | F3 #1385 | Decisions 1〜12 |
| bounded current / status / message / tool query | F8 #1491 | snapshot、cursor、direct index、query failure |
| history-independent commitとperformance gate | F9 #1494 | structural budgetとabsolute budget |
| queue pause projection / watermark / bounded query | F10 #1497 | projection generationとcheckpoint契約 |
| performance / migrationを含む横断gate | F3 / F8 / F9 / F10 | Decision 13 |

#1499の完了条件は、D3 design gateとPhase 0 closureの実装、fault injection、surface parityまでである。SQLite storeの実装、Phase 0からF3へのauthority cutoverの実装または実行、managed backup / restoreのruntime実装はF3 #1385が所有する。

#1499のTauri / WebSocketへ、managed backup / restore、privacy purge、app-data authority rotation、resource-isolated input、Gone、Purged、Retiredなどの将来専用command、query、public variantを追加しない。

#### Authority topology

production write authorityは常に次のどちらか一つである。

1. Phase 0 authority
2. SQLite authority

小さなauthority pointerがauthority kind、physical generation identity、immutable generation manifestのdigest、initial cutover identity、revisionを保持する。composition rootはこのpointerを検証して一つのstoreだけをinjectする。pointer、manifest、DB lineageのchainを検証できない場合はwriterを選ばずmutation admissionを閉じる。

Phase 0は汎用SQLite transactionをfile writeで再現しない。#1499対象scopeに限り、redo manifest、transaction inventory、immutable receipt、operation status、obligation、shutdown root / page / archiveをcrash-atomicに閉じる。F3 import sourceは、Phase 0 authority pointerが指すactivated generationだけである。legacy tree、bootstrap staging、未切替generationを追加sourceまたはfallbackとして読まない。

#### Backend ownership

application logicはRustが所有する。domain / usecaseはVocabulary §9.5のLocalEventTransactionRepository、context-specific read repository、LocalWatchRepositoryを参照し、SQLite、SQL、filesystem layout、serde、schema versionを参照しない。LocalEventTransactionStoreはinfrastructure gateway背後の実装であり、authority選択、SQLite worker、blob、migration、backupを所有する。Tauri / WebSocket controllerは同じusecaseを呼び、transport固有のdomain判断を持たない。

single writerがcanonical commit orderを所有する。readerとprojectorはsealed commitだけを読む。external provider / workflow / OS effectは、対応するdurable obligationがEffectReservedとしてcommitされた後のdispatch guardからだけ開始する。

### Interface

#### Internal ports

公開routeを増やさず、Rust内部portを次の責務に分ける。名前は実装時に変更できるが、責務の混在は認めない。

| Port operation | 責務 | 成功境界 |
| --- | --- | --- |
| commit batch | 複数stream、head CAS、event、receipt、critical stateを一つのlogical batchで保存する | SQL COMMIT後 |
| resolve transaction | transaction / operation identityからOutcomeUnknownを解決する | sealed commitまたは未commit証明の取得後 |
| get by identity | operation、terminal、obligation、shutdown、projectionをdirect indexで取得する | 一つのsnapshot内で完全な結果を得た後 |
| open snapshot | required projection sourceの共通watermarkへreadを固定する | leaseとwatermarkの同時確定後 |
| read page | cursorに束縛したsnapshot / filter / orderでbounded pageを返す | count / byte上限内の完全page取得後 |
| open watch | snapshot、replay fence、subscriptionをgapなしで固定する | fenceを含むwatch handle確定後 |
| rebuild projection | shadow generationをsealed commitから構築する | parity検証後のActive pointer CAS後 |
| enumerate obligations | pending-only indexから回復対象を列挙する | 固定snapshotのbounded page取得後 |
| run migration | immutable source manifestをstaging generationへ冪等importする | Ready検証後。authority切替は別操作 |
| cut over authority | verified staging generationへpointerを一度だけCASする | pointer / manifest / DB chainの再open確認後 |
| create backup | online backupと参照objectをcomplete generationとしてpublishする | complete manifestのlast publish後 |
| restore backup | fresh physical generationを検証しauthorityをCASする | new authority chainの再open確認後 |

#### Error contract

storage内部のtransaction classificationとfailureはdomain errorへ文字列化せず、少なくとも次を区別する。この表のclassは新しいpublic variantではない。

| Class | 意味 | 公開上の扱い |
| --- | --- | --- |
| BeforeCommit | canonical writerを開始していない、またはrollbackを確認済み | state / identity / external effect増分0 |
| HeadConflict | participantのexpected head不一致 | current headを用いた明示retryが必要 |
| PayloadConflict | 同じcaller identityを異なるexact payloadへ再利用 | 既存operationを変更しない |
| OutcomeUnknown | writer開始後にcommit結果を証明できない | 同じtransaction / operation identityでのみ解決 |
| CapacityExceeded | countまたはbyte permit不足 | enqueue / transaction前に拒否 |
| QueryBusy | 一貫したsnapshotを固定できない | partial resultを返さない |
| DeadlineExceeded | 公開deadline内に完全結果を作れない | partial resultを返さない |
| CursorMismatch | cursorのsnapshot / filter / owner binding不一致 | stateを変更せず先頭から再取得 |
| CursorExpired | lease、process、retentionの失効 | stateを変更せず先頭から再取得 |
| ProjectionBehind | required projectionがrequested watermarkへ未到達 | full replayへfallbackしない |
| MigrationBlocked | lossless import、parity、authority一意性を証明できない | pointerを切り替えない |
| SchemaTooNew | readerが理解できないschema | mutationを開かない |
| Corrupt | integrity、required reference、identity一意性の破損 | fail closed。推測結果へfallbackしない |
| AuthorityViolation | non-authoritative storeへのwrite要求 | write 0件、admissionを閉じる |

timeout、worker panic、receiver dropを一律BeforeCommitへ変換しない。writer開始済みならOutcomeUnknownのまま、同じidentityのfresh lookupまたは旧writer不在の証明で解決する。

gatewayはこの内部classを各endpoint正本へtotal mappingする。SendのBeforeCommitはRejectedBeforeCommit、Quit / SessionLifecycleを含むacceptance commandでは各正本のRejectedBeforeAcceptanceまたは個別rejectionへ写し、endpoint間でpublic tagを共通化しない。current shutdown queryは、shutdown不在だけをCurrent(None)、冗長authorityだけのsemantic mismatchをShutdownAuthorityMismatch付きReconciliationRequired、storage / decode / integrity / required-reference / identity一意性failureをInternal、同じplan identityへ束縛した保存結果不明だけをOutcomeUnknownへ写す。相互変換、current planへのfallback、部分resultは行わない。

#### Surface compatibility

Tauri / WebSocketの共有routeは同じbackend-owned resultをsemanticに同じtag / fieldで返す。SessionLifecycleはTauri専用であり、WebSocket routeを追加しない。semantic integerは既存正本どおり0または先頭ゼロのないASCII decimal stringで表し、最大値は9,223,372,036,854,775,807とする。page limit、byte limit、signed exit codeの既存JSON integer境界は変更しない。

D3は既存public commandの意味を変更しない。特にSend、Stop、Quit、SessionLifecycleのbinding / import parity、principal分離、immutable receipt、mutable status、safe action、shutdown Available / Compactedを維持する。4 command domainのbinding parityは各transportにrouteが存在することを意味しない。

### Data Model

#### Conceptual records

DDLではなく、実装が保持すべきrecord familyとownershipを定義する。

| Family | 主なidentity / key | 可変性 | 必須lookup / relation |
| --- | --- | --- | --- |
| authority pointer / generation manifest | physical generation、pointer revision | pointerはCAS、manifestはimmutable | pointerからmanifest、manifestからlogical lineage |
| store state | logical lineage singleton | allocator / active pointersだけCAS | next global sequence、app-data generation、projection authority |
| commits / commit seals | global commit sequence | commitはappend、sealはlast insert後immutable | global order、idempotency、sealed visibility |
| streams / batch participants | context、kind、stream ID | headだけCAS、participantはimmutable | current head、commit内ordinal |
| events | stream identity、stream sequence | immutable append | stream range、global commit、event type |
| caller bindings | app-data generation、principal、operation kind、caller identity | immutable | Send operationまたはbackend operationへのexact binding |
| backend operations / receipts / statuses | app-data generation、operation kind、backend operation ID | receiptはimmutable、statusはrevision CAS | caller bindingからのdirect解決、principal authorization、exact replay |
| terminal / obligation / result / claim / fence | stable operationまたはeffect identity | pendingからclosedへone-way | pending-only inventory、terminal closure |
| shutdown plan / pages / archive | plan identity、epoch、target key | planはmonotonic、page / archiveはimmutable | AvailableまたはCompactedのexact query |
| event envelope / blob / raw preservation | event identityまたはopaque blob identity | immutable | payload integrity、owner scope、unknown raw |
| projection source / rows / checkpoint | source generation、entity identity | shadow build後pointer CAS | current lookup、bounded list、watermark |
| snapshot / watch / backup lease | random lease identity | bounded TTL | GC pin、cursor binding、watch fence |
| migration run / source checkpoint | stable progress ID、source、record、substep | monotonic checkpoint、active bootstrap pointer | crash resume、source parity、bootstrap-safe quit locator |
| backup manifest | backup identity | complete publish後immutable | DB barrier、blob set、lineage、restore validation |

#### Cross-record invariants

1. authority pointerが選ぶproduction writerはexactly oneであり、opposite storeへのwriteは0件である。
2. 一つのlogical batchは一つのglobal commit sequenceを持ち、全participant、event、head、receipt、critical state、sealを一つのSQL transactionで確定する。
3. commit sealはcanonical mutationの最後に作る。schemaはsealなしCOMMITとseal後のcanonical mutationを拒否し、sealされていないcommitをreader、projector、watchへ公開しない。
4. stream sequenceとglobal commit sequenceは1始まりのsigned 64-bitで、gap、rewind、wrapを許さない。
5. Sendはcaller operation ID自体をoperation identityとする。Stop、Quit、SessionLifecycleはcurrent app-data generation、authenticated principal、command domain、caller request IDのunique bindingからbackend発行opaque operation IDへ写す。同じcaller bindingは保存済みresultを再生し、異なるpayloadはPayloadConflictである。別principalによる既存operation ID照会または無権限commandはNotFoundだが、authorizedな別principalから同じsessionへ新しいSessionLifecycle commandが来て既存operationが未解決なら、そのidentityを開示せずPendingOperationとする。
6. Accepted receiptはimmutableであり、latest statusだけをrevision CASで進める。statusからreceiptを再構築しない。
7. obligationはpendingまたはclosed resultのどちらか一方である。external effect、terminal、permission、publication、shutdown targetのkind-specific side stateは、対応するobligation transitionと同じcommitで確定する。
8. terminal resultはturnごとに一件であり、競合するcompletion、Stop、close、Fatalの後着結果はwinnerを変更しない。
9. projection rowはsealed inputだけを参照し、checkpointより先のrowをqueryへ公開しない。
10. known eventのunknown additive fieldは受理したexact envelope bytesをevent rowで保持する。unknown event、quarantine、oversized rawはsource tuple、owner scope、exact raw bytes、integrity情報をopaque raw objectで保持し、再serialize、dedupe、既知eventへの推測変換を行わない。
11. private exact payloadはsemantic event、public hash、log、UIへ含めず、owner-private opaque blobで保持する。retained equalityを許すcontentだけをcontent-addressedにできる。
12. shutdown detailは公開上AvailableからCompactedへの一方向だけであり、途中の部分detach、empty Available、NotFoundへの揺れを公開しない。
13. preexisting open / closed / archived session、確定transcript、既知eventの公開結果、terminal reason、session lifecycle、owner relation、permission、queue-linked inputはauthority移行前後で維持する。未完了active turn、queued input、permission、recoveryは元identityとknown observationを維持する。
14. external effectの開始または結果を証明できないstateは通常進行中へ推測せず、Paused、Failed、ReconciliationRequiredのいずれかと利用可能なsafe actionへ写す。
15. physical generation identityとlogical store lineage identityは別物である。restoreはfresh physical generationを作るがlogical lineage、initial cutover identity、app-data generationを変更しない。
16. bootstrap中はexactly oneのactive migration progress identityがread-only gateを所有する。automatic bootstrap中のquit operationはそのidentityをlocatorとして保存し、同じprogressとquit operationをrestart後にdirect取得できる。normal shutdown target / effectを作らず、13秒settleと15秒exit decisionを維持する。
17. restoreはbackup元bootのclaim / leaseをcurrent claimとして再利用しない。effect identityとpending obligationを維持して旧claimを失効させ、authoritative readbackを持つeffectだけを新claimへ進め、それ以外をReconciliationRequiredへ写す。

#### Physical table families

F3は少なくとも次のtable familyを持つ。table名、column名、trigger名の細部はF3 migrationで確定できるが、key、immutability、CAS、index、transaction境界は変更できない。

| Table family | Required key / index | Required rule |
| --- | --- | --- |
| store / commit ledger | singleton、global sequence unique、idempotency identity unique | allocatorはexact +1 CAS、sealなしCOMMITとseal後canonical writeを拒否 |
| stream ledger | stream PK、stream + sequence unique、global sequence index | headはexpected value CAS、MAX(history)をauthorityにしない |
| operation authority | caller bindingはgeneration + principal + operation kind + caller identity unique、direct keyはgeneration + operation kind + backend operation ID | receipt immutable、status revision CAS、caller bindingからdirect join、principal authorization、cross-principal非開示 |
| terminal / obligation | target identity unique、pending-only index、result identity unique | pending / result one-of、kind-specific closureを同commit |
| event payload | event identity、schema / type、blob ref | known upcast、unknown raw lossless、payload integrity |
| projection | source generation + entity key、watermark index | Building / Ready / Active / Failed、normal query admissionを開いたauthorityではActive exactly one |
| shutdown | plan / epoch、page ordinal、target key、archive locator | live planとimmutable archiveのclosed union |
| migration | stable progress ID、active bootstrap pointer、source / ordinal、source hash | source eventとcursorを同transaction、pointer CAS前failureでも同じprogressを維持 |
| backup / lease | backup ID、lease expiry、manifest state | destination-derived barrier、manifest last publish |

### Database

以下13件がR-013 / B-050のmechanical design gateである。各Decisionは採用案、却下案、理由、Failure behavior、Migration verification、具体上限を必ず持つ。

#### Decision 1: Storage engine and transaction boundary

##### 採用案

F3はrusqliteのbundled SQLite 3.45.0以上を使用する。system SQLiteへ暗黙fallbackしない。immutable physical generationごとに一つのDBを置き、writeはBEGIN IMMEDIATEによるsingle-writer transactionだけを使う。

一つのLocalAtomicBatchを一つのSQLite transactionへ写し、stream head、participant、event、operation receipt、terminal / obligationなど同時に観測されるstateを同じcommitへ参加させる。transaction外で同期済みimmutable blobをpublishし、transactionはblob identityだけを参照する。authority pointerはevent logとして使わない。

##### 却下案

session単位JSON / NDJSON、context別SQLite、participant別transactionと補償、RocksDB系store、in-memory authorityとasync flushを却下する。

##### 理由

一つのSQLite transactionだけが、AgentSessionとWorkflowの複数stream、head CAS、receipt、critical indexを同時に確定し、WAL recoveryとonline backupを同じdurability boundaryで成立させる。

##### Failure behavior

SQLite version、compile option、open、PRAGMA、schema、manifest chainを検証できない場合はmutation admissionを開かない。size超過はtransaction前に拒否する。statement failureはrollbackする。COMMIT結果不明は同じidempotency identityで照会し、別batchを開始しない。

##### Migration verification

1〜64 participant、上限 / 上限+1、blob publish前後、transaction開始、各mutation、seal、COMMIT、pointer publishの各境界へfaultを注入し、公開結果が変更前または完全な変更後だけであることを確認する。Fresh、InitialImport、Restoreのmanifest chainとphysical / logical identityの差をfixtureで固定する。

##### 具体上限

- SQLite: 3.45.0以上
- page size: 4 KiB
- participant: 1 batch当たり64以下
- event: participant当たり256以下、batch当たり1024以下
- inline payload: 8 MiB以下
- decoded batch: 16 MiB以下

#### Decision 2: Stream identity, global order, and indexes

##### 採用案

stream identityはcontext、kind、stream IDの組とする。stream sequenceとglobal commit sequenceは1始まりのsigned 64-bitである。global sequenceはbatchごとに一つ割り当て、participantはidentity byte順、participant内eventは入力順で安定化する。

stream headを明示rowとして保持し、direct identity query、pending obligation、operation、terminal、shutdown locatorに専用indexを置く。global allocatorのexact +1、participant / event ordinal、stream head、declared countをseal前に検査する。schema guardはsealなしCOMMITとseal後のcanonical mutationを拒否する。MAX(sequence)、COUNT(history)、directory scanをauthorityにしない。

##### 却下案

event履歴からのhead導出、timestamp順序、participant別global sequence、latest row scan、sealなしcommitを却下する。

##### 理由

明示headとdirect indexにより、履歴件数に依存せずCAS、identity lookup、recovery discoveryを実行できる。batch単位のglobal sequenceによりmulti-stream commitの一つの観測境界を表せる。

##### Failure behavior

unknown context / kind、duplicate participant、ordinal gap、sequence exhaustion、head / allocator / index parity破損はtransactionをrollbackする。既存DBの破損はCorruptとしてwriteを止め、履歴scanによる推測修復を通常pathで行わない。

##### Migration verification

1千eventと100万eventで、head CAS、identity query、pending discoveryのstatement数とvisited row上限が同じであることを検証する。participant順入替え、event ordinal欠損、head drift、global gap、seal欠損、seal insert後のcanonical writeをrejectする。

##### 具体上限

- stream / global sequence: 1〜9,223,372,036,854,775,807
- global sequence: logical batch当たりexactly 1
- participant: batch当たり64以下
- event: batch当たり1024以下
- history scan: critical pathで0件

#### Decision 3: CAS and idempotency

##### 採用案

全participantのexpected headを同じtransactionでCASする。caller binding indexはprincipal、app-data generation、operation kind、caller request identityをunique keyとする。backend operation direct indexはapp-data generation、operation kind、backend operation identityをkeyとし、principalはauthorization scopeとしてrecordに保持するがdirect key componentにはしない。Sendはcaller operation IDをbackend operation IDとして使い、Stop、Quit、SessionLifecycleはcaller request IDからbackend発行opaque operation IDへ写す。4 command domainはVocabularyのcanonical bindingとPhase 0から移した同一generationのbinding-key authorityを使う。

同じprincipal、operation kind、caller identity、payloadのretryはimmutable bindingから同じbackend operation、receipt、latest statusをexact replayする。同じcaller identityの異payloadはPayloadConflictである。別principalによる既存operation ID照会または無権限commandはNotFoundとする。authorizedな別principalから同じsessionへ新しいSessionLifecycle commandが来て既存operationが未解決なら、そのidentityを開示せずPendingOperationとする。accepted receiptとinitial statusは同じtransactionで作り、receiptを後続statusで上書きしない。

##### 却下案

payloadだけのdedupe、process mutexだけの排他、retry時のidentity再採番、receiptの別write、batch hashをcaller bindingとして使うこと、COMMIT error後のblind retryを却下する。

##### 理由

unique identity、principal binding、request binding、head CAS、immutable receiptを同じtransactionへ置くことで、response loss、restart、Tauri / WebSocket競合を一つのwinnerへ収束できる。

##### Failure behavior

head mismatchはHeadConflict、same caller binding / different payloadはPayloadConflict、cross-principal operation lookupと無権限commandはNotFoundとする。authorizedな別principalによるSessionLifecycle競合はPendingOperationでありNotFoundへ変換しない。writer開始後のtimeout、panic、receiver loss、COMMIT return不明はOutcomeUnknownであり、same caller slotまたはbackend operation identityのlookup以外から未commitまたはsuccessを推測しない。Accepted後のeffect結果不明はreceiptを維持したReconciliationRequiredへ進める。

##### Migration verification

同じpayload / 異payload / 別principal / operation kind差 / outer transport request ID差、parallel Tauri / WebSocket、SessionLifecycleのsame-session join / PendingOperation、head race、COMMIT直前 / 直後のresponse loss、restartを検証する。Phase 0の4 command domainについてcaller binding key、backend direct key、operation identity、receipt、statusをbyte-equivalentに移し、provider effect countが0または1へ収束することを確認する。

##### 具体上限

- caller operation / request identity: 1〜128 bytes
- caller binding unique key: principal + app-data generation + operation kind + caller identity
- backend direct key: app-data generation + operation kind + backend operation identity
- accepted receipt: operationごとにexactly 1
- active binding: app-data generationごとにexactly 1 authority key
- participant head CAS: batch内participantごとにexactly 1
- blind retry / duplicate provider effect: 0件

#### Decision 4: Durability and crash atomicity

##### 採用案

write connectionはWALとsynchronous=FULLを使い、SQL COMMIT成功後だけsuccessを返す。SQLite header、manifest chain、store singleton、PRAGMA actual valueを通常bootの定数検査とする。前回unclean、I/O異常、明示maintenanceではquick_checkを行い、migration、cutover、restoreではintegrity_checkとforeign_key_checkを必須にする。

通常checkpointはPASSIVEとし、graceful shutdownではadmission close、writer / projector drain、lease解放後にTRUNCATE checkpointを試み、最後にclean markerをcommitする。checkpointをapplication commit判定へ使わない。

##### 却下案

synchronous=NORMAL / OFF、WAL fileの存在によるcommit判定、commit前live公開、filesystem renameによるSQL補償、毎bootのfull history integrity scanを却下する。

##### 理由

WAL / FULLとsingle transactionがstatement failure、process crash、power interruptionに対するdurable boundaryになる。clean markerにより重い検査を必要なbootへ限定し、通常startupをhistory-independentに保てる。

##### Failure behavior

COMMIT前にrollbackを確認できれば変更前を返す。COMMIT後のresponse failureはidempotency lookupで回復する。checkpoint failureは既存commitを取り消さない。write継続の安全性を証明できないI/O failureではReadOnlyへ移り、integrity failureはCorruptとしてmutationを止める。TRUNCATE失敗時はclean markerを立てない。

##### Migration verification

statement、COMMIT、connection drop、PASSIVE / TRUNCATE checkpoint、clean marker、process reopenの各境界でfaultを入れる。participant、head、receipt、terminal、obligationが0件または全件であること、unclean bootだけがrequired checkを通ることを確認する。

##### 具体上限

- page size: 4 KiB
- automatic PASSIVE checkpoint: 1000 WAL pagesまたは5秒の先到達
- open時writer authority: 1 connection
- success publication: SQL COMMITより前は0件
- migration / restore validation: integrity checkとFK checkを各1 complete pass

#### Decision 5: Worker model and typed errors

##### 採用案

blocking SQLite I/Oはdedicated OS workerが所有し、tokio taskで同期I/Oを実行しない。writerは一つ、readerは最大4つとする。writer queueはnormal、critical terminal、shutdown planのbounded laneへ分け、request countとresident bytesの両方でadmissionする。critical / shutdownは上位のStop 10秒、shutdown durability 13秒、process decision 15秒を延長しない。

errorはInterfaceのtyped classで返し、raw SQL、path、secret、provider payload、unbounded error textをpublic resultへ含めない。

##### 却下案

unbounded channel、connection mutexをasync taskで共有すること、request countだけのcapacity管理、unbounded BUSY retry、deadline後のbackground commitをBeforeCommit扱いすること、string errorを却下する。

##### 理由

single writerでcommit orderを一意にし、byte permitとdeadlineをworker入口から管理することで、memory上限とStop / quitの公開deadlineを同時に守れる。typed errorにより未受理と結果不明を混同しない。

##### Failure behavior

slotまたはbyte permit不足はtransaction前にCapacityExceededとする。writer開始前のbusy expiryはBeforeCommit、開始後の結果不明はOutcomeUnknownである。readerがsnapshotを固定できなければQueryBusy、2秒以内に完全pageを作れなければDeadlineExceededとし、partial resultを返さない。worker failure時は新規admissionを閉じる。

##### Migration verification

各laneのcount / byte上限と上限+1、EDF順序、worker panic、receiver drop、busy timeout、deadline直前 / 直後、restart後のOutcomeUnknown lookupを検証する。tokio executor thread上でblocking SQLite callが0件であることをinstrumentationで確認する。

##### 具体上限

- writer: 1
- reader: 最大4
- normal lane: 256 requests / 64 MiB
- critical lane: 32 requests / 32 MiB
- shutdown lane: 32 requests / 32 MiB
- reader queue: workerごとに64 requests / 8 MiB
- normal commit default: 2秒
- read query deadline: 2秒
- Stop public deadline: 10秒
- shutdown durability cutoff: 13秒
- exit / restart / abort decision: 15秒

#### Decision 6: Versioned envelope and raw preservation

##### 採用案

domain eventはVocabulary §10のclosed typed enumを正本とし、gatewayがschema version、event type、canonical payloadまたはblob reference、integrity、owner scopeを持つversioned persistence envelopeへ変換する。known old versionはpureなlazy upcasterでcurrent domain eventへ変換し、canonical source rowを書き換えない。

known eventのunknown additive fieldは受理したexact envelope bytesを同じevent authorityで保持する。unknown event、quarantine、oversized rawはsource identity、owner scope、exact raw bytes、length、integrityをowner-private opaque blobとして保持する。safeにskipできるoptional eventだけをskipし、required sourceはFailedまたはSchemaTooNewで止める。malformed known eventはCorruptとする。private exact payloadをsemantic event、public hash、log、UIへ複製しない。

##### 却下案

serde_json Valueをdomain authorityにすること、read時のin-place rewrite、unknown fieldのdrop、private payloadのcontent-addressed dedupe、raw bytesの再serialize / redaction置換を却下する。

##### 理由

versioned envelopeとlossless raw preservationにより、old readerが理解できないdataを破壊せず、known eventだけを型安全に進化できる。private exact payloadの分離によりintegrity確認とequality disclosureを混同しない。

##### Failure behavior

known payloadのdecode / integrity failureはCorrupt、required future versionはSchemaTooNewでadmissionを閉じる。raw object、owner binding、ACL、hashを検証できない場合はeventを推測再構築せずMigrationBlockedまたはCorruptとする。unknown optional eventをskipした場合もraw recordは保持する。

##### Migration verification

known old versionのupcast、unknown event / field、malformed known event、private payload、same bytesを持つ別owner、raw copy途中のcrashを検証する。import前後でraw bytes、length、hash、source tuple、owner scopeが一致し、content dedupeが0件であることを確認する。

##### 具体上限

- inline event payload: 8 MiB
- decoded batch payload: 16 MiB
- raw copy chunk: 1 MiB
- raw import step: 16 MiB
- public transport request / response: 既存契約どおり16 MiB
- unknown rawの再serialize: 0回

#### Decision 7: Projection checkpoint and rebuild

##### 採用案

projection source generationはBuilding、Ready、Active、Failedのclosed stateを持つ。normal query / mutation admissionを開いたauthoritative generationではsourceごとにActiveはexactly oneである。initial importのstaging generationはBuilding / Ready中にActive 0件を許し、pointer CAS前にquery authorityにならない。checkpointはsealed global commit sequenceの連続watermarkであり、projection rowとcheckpointを同じprojector transactionで進める。

rebuildはshadow Building generationへ実行し、source coverage、row invariant、public oracle parityを確認してReadyにした後、Active pointerをCASする。旧Activeは切替完了までquery authorityである。queryはActive checkpointより先のrowを返さず、lag時にevent full replayへfallbackしない。

##### 却下案

Active tableのin-place rebuild、checkpoint先行更新、複数Active、queryごとのfull replay、projection error時のsilent old-row利用を却下する。

##### 理由

shadow rebuildとatomic pointer切替により、rebuild途中とcurrent queryを分離し、crash後も旧Activeまたは完全な新Activeだけを公開できる。

##### Failure behavior

projector failureはcheckpointを進めずBuilding / Activeを維持する。required watermarkへ届かなければProjectionBehind、snapshot deadlineを超えればDeadlineExceededとし、partial projectionやevent replay fallbackを返さない。Active corruptionはCorruptとしてfail closedし、任意の古いgenerationへfallbackしない。

##### Migration verification

projector chunk前後、row write後 / checkpoint前、Ready前後、Active pointer CAS前後へfaultを入れる。overlap、gap、unsealed input、checkpoint先行をrejectし、旧Activeまたは新Activeのどちらか一方だけが見えることを確認する。

##### 具体上限

- Active generation: normal admissionを開いたauthorityではsourceごとにexactly 1、staging Building / Readyでは0
- projector chunk: 64 commits、decoded 4 MiB、10 msの先到達
- projection wait: 2秒
- unsealed commitのprojection: 0件
- query時full replay: 0件

#### Decision 8: Bounded query, watch, replay, obligation inventory, and GC

##### 採用案

snapshotはrequired projection sourcesのcommon readable watermarkへpinし、opaque cursorをsnapshot、filter、owner、orderへ束縛する。identity queryはdirect index、listはstable cursorとcount / byte capを使う。get後にsubscribeする二段処理ではなく、snapshot、replay fence、subscriptionを一つのwatch-open境界で確定する。

startup recoveryはpending-only obligation indexをauthorityとし、全Session、全event、directoryをscanしない。external effectはobligationをEffectReservedとしてcommitし、dispatch直前guardが同じidentity、claim、fence、revisionを確認してから開始する。terminal/result、operation status、owner projection、claim/fence、kind-specific side stateを同じtransactionで閉じる。

GCはsnapshot / watch / backup leaseとcanonical referenceの到達性を確認し、bounded chunkでimmutable orphanだけを回収する。v1ではcanonical event、Accepted receipt、terminal result、shutdown archiveをTTLだけで削除しない。

##### 却下案

unbounded list、offset pagination、get-then-subscribe、watch buffer overflowのsilent drop、startup全履歴scan、generic result-only obligation completion、TTLだけのreceipt / terminal GCを却下する。

##### 理由

snapshot-bound cursorとdirect pending indexにより、履歴量と同時更新からresultを分離できる。obligationとdispatch guardによりrestart後のblind external effectを防ぐ。lease-aware GCによりreader / backupが参照するobjectを削除しない。

##### Failure behavior

snapshotを固定できなければQueryBusy、deadline超過はDeadlineExceeded、binding違反はCursorMismatch、lease失効はCursorExpired、watch lagは明示resync requiredとする。partial count / page / replayを返さない。obligationの結果を確認できなければReconciliationRequiredを維持し、自動effectを開始しない。

##### Migration verification

同時commit中のsnapshot、cursor改変 / filter再利用 / expiry、watch open境界、buffer overflow、pending 0 / 上限 / 上限+1、claim競合、effect直前crash、effect直後crash、GCと各leaseの競合を検証する。shutdownは4096 / 4097 targets、128 / 129 page entries、target authority 65,536 / 65,537 bytes、page canonical refsと対応authority canonical bytesの合計1 MiB / 1 MiB + 1を検証し、超過をBeforeCommitで拒否する。10件と100万件のhistoryでpending first pageのwork上限が同じであることを確認する。

##### 具体上限

- snapshot lease TTL: 30秒
- required projection sources: snapshot当たり64
- snapshot leases: process当たり512
- generic page: default 100件 / 1 MiB、最大500件 / 4 MiB
- pending recovery page: 200件 / decoded 4 MiB
- watch対象stream: 64
- watch buffer: 256 commits / 4 MiB
- watch replay: 256 commits / 1 MiB
- watches: process当たり128 / 合計64 MiB
- watch memory: 1 watch当たり4 MiB
- GC pass: 1000 recordsまたは50 msの先到達
- shutdown plan: 4096 targets、32 pages
- shutdown page: 128 targets以下
- target authority: 1〜65,536 bytes
- page size: page canonical refsと対応するstandalone target authority canonical bytesの合計1 MiB以下

#### Decision 9: Legacy import, authority cutover, and rollback

##### 採用案

Phase 0からF3への移行はuser操作を要求しないautomatic bootstrapであり、exclusive authority lock下のone-shot migrationとする。latest terminal shutdownがAvailableかつlatest-retiring pointerがNoneなら、migration reservation closureはそのlatest-retiring pointerだけをNoneから対象planのSomeへCASし、他のcanonical stateを変更しない。既にretiring Someなら同じplanの未完了compactionを先に再開する。このpointer-only reservationとexclusive lockが新しいshutdown root-init、通常 / background compactor、normal mutation、effect admissionをfreezeするので、別のgeneric reservation recordは作らない。

migration coordinatorだけがreserved latest-retiring pointerの下で既存のArchiveSwitch、DetailDetach、FinalizeDetachを順に実行する。保存結果不明のPhase 0 physical transaction / materializationを同じtransaction identityで解決してterminal shutdown detailを全てCompactedへ進め、最後のdedicated closureでlatest-retiring pointer、residual detail / page、pending materializationを0にする。その後にactivated source manifestを固定し、internal Phase 0 cutover snapshot、staging import、projection rebuild、identity / result / raw / count parity、authority pointer CAS、SQLite authority再openの順で進める。

shutdown latest-retiring pointerはcompaction専用であり、migration全体のbootstrap gateではない。stable migration progress recordとactive bootstrap pointerがInspectingSource、Importing、Verifying、ActivatingまたはFailedのread-only stateとcursorを所有する。process crashやpointer CAS前failureでもこのprogress identityを変えず、normal mutationを再開せず、same source manifest / cursor / staging identityでresumeする。

保存済みOutcomeUnknown、pending obligation、未完了active turn、permission、queue-linked input、recovery actionなどのsemantic pendingをterminal化することは要求しない。identity、known observation、safe actionをlosslessに移し同じpublic queryで再現できることを要求する。これらと、cutover前にCompactedへ確定したshutdown identity / summaryを失う、再採番する、通常進行中へ推測する場合はMigrationBlockedとする。

internal Phase 0 cutover snapshotはauthority pointer bytes / revision、activated generation manifest / root / pointer、reachable immutable object、owner-private blob、AgentOperationBindingKeyV1 record、current Active recovery-action authority record、ACL metadataをordered manifestとobject hashで固定する。objectとdirectoryをsyncし、complete manifestを最後にexclusive publishした後だけ成功である。isolated temporary rootへのrestore smokeでpointer / root chain、hash、ACL、4 command binding verifier、public parityを確認する。これはDecision 12のmanaged backupでもpublic APIでもない。

rollbackはauthority pointer CAS前のfailureでproduction data authorityをPhase 0のまま維持することだけを指す。Phase 0のshutdown detailは既にCompactedへ進み得るため、data bytesが完全に未変更という意味ではない。migration progressとactive bootstrap pointerを保持し、read-only / BootstrapInProgressまたはFailedのまま同じprogress identityで再開する。stagingを破棄する場合もsame runのcheckpointでnon-authority staging resetを確定し、別progress identityを採番せずnormal admissionを再開しない。

pointer CAS後はlive SQLite commitの有無にかかわらずPhase 0へ戻さず、SQLite authorityだけを使う。new authorityのpost-CAS validation完了後、一つのfinal activation closureがmigration runのimmutable completed provenance、active bootstrap pointer clear、durable normal read / mutation admission Openを同時に確定する。このclosureのreadback後だけruntime admissionを開き、bootstrap queryをNoneにする。commit前またはOutcomeUnknown中は同じprogress IDのActivatingとadmission Closedを維持する。migration runをpending obligationとして移さない。bootstrap-safe quit operationはcompleted provenanceの同じprogress IDをlocatorとして引き続き取得できる。cutover snapshotは監査 / forensic artifactとして保持できるが、production Phase 0 authorityへのrestoreまたはautomatic rollbackには使わない。

##### 却下案

lazy per-session migration、複数sourceのmerge、bootstrap stagingの補完source化、cutover中のproduction mutation、semantic pendingの強制terminal化、parity不足のbest-effort import、pointer CAS後のlegacy fallbackを却下する。

##### 理由

固定sourceとsingle pointer CASにより、import中のdata drift、二重authority、identity再採番を防ぐ。unresolved stateも含むlossless parityにより、cutoverをrecoveryの隠れた成功 / 失敗判定にしない。

##### Failure behavior

source drift、migration reservationのpointer-only CAS不一致、physical transaction / materialization結果不明、shutdown Available残存、retiring / residual / pending materialization残存、cutover snapshot / restore smoke failure、unknown required record、identity collision、raw / ACL不一致、operation / shutdown summary欠損、projection parity failureはMigrationBlockedでありpointerを切り替えない。incomplete cutover snapshotはcomplete manifestを持たずauthorityにもbackupにもならない。pointer CAS前failureはsame migration progressを維持してPhase 0 read-only authorityへ留まり、normal admissionを開かない。pointer結果不明はexpected revision、physical generation、manifest digest、cutover identityを再読込してActivatingのまま解決する。切替後のfailureはlegacyへfallbackしない。

##### Migration verification

migration reservationのlatest-retiring pointer None→Some前後と同closureの他state変更0件、preexisting retiring resume、ArchiveSwitch、DetailDetach、FinalizeDetach、physical transaction / materialization resolution、Available→Compacted、retiring / residual / pending clear、Phase 0 pointer、activated generation、cutover snapshot object / manifest sync、restore smoke、source manifest、staging Ready、pointer CAS、post-CAS validation、final activation closureの各境界でprocessを停止する。final activationはcompleted provenance、active bootstrap pointer clear、durable admission Openのall-or-noneとし、pointer clearだけのstate、bootstrap None + admission Closedを0件にする。CAS前failure / staging reset / restartはsame migration progress identityとcursorへ戻りBootstrapInProgressを維持する。確定transcript、open / closed / archived session、既知eventの公開結果、terminal reason、session lifecycle、owner relation、permission、queue、未完了active turnのidentity / known observation、4 command domain binding、OutcomeUnknown、shutdown Compacted identity / counts / deadline / failureをcutover前後の同じpublic oracleで比較する。pointer CAS後のPhase 0 rollbackとmigration run由来pending obligationが各0件であることも確認する。

##### 具体上限

- import source authority generation: exactly 1
- production write authority: exactly 1
- authority pointer CAS: cutover当たりexactly 1
- production dual-write: 0
- shutdown migration: 最大4096 targets / 32 pages
- terminal shutdown detail Available at source-manifest publish: 0
- latest-retiring / residual detail or page / pending materialization: 各0
- Phase 0 cutover snapshot: activated generation当たりexactly 1 complete manifest
- cutover snapshot object copy: 1 MiB chunk、1 step 16 MiB
- active migration progress: exactly 1
- pointer CAS前failure後のnew progress identity / normal mutation: 各0
- losslessに移せないunresolved identity / required detail: 許容0件

#### Decision 10: Dual-write policy

##### 採用案

production dual-writeを全期間禁止する。composition rootはauthority pointerが選んだstoreを一つだけinjectする。cutover前はPhase 0だけ、cutover後はSQLiteだけがwriteできる。opposite storeはcutover前のmigration sourceまたはstaging abort確認のためread-onlyにできるが、通常queryのfallbackとして使わない。

##### 却下案

canary dual-write、best-effort mirror、primary success後のasync secondary write、read repair、pointer不明時の両store writeを却下する。

##### 理由

dual-writeはpartial success、order divergence、idempotency divergenceを新しく作り、一つのauthorityを証明できなくする。one-shot cutoverとread-only sourceの方がfailure境界を有限にできる。

##### Failure behavior

non-authoritative storeへのwrite要求はAuthorityViolationとし、両storeのwriteを0件にする。pointerを検証できない場合はwriterを選ばずadmissionを閉じる。parity不足ではcutoverを開始せず、Phase 0 authorityを維持する。

##### Migration verification

composition root、background worker、recovery、projector、backup、test harnessの各write pathをtraceし、authority pointerと異なるstoreへのproduction writeが0件であることを検証する。release binaryにdual-write branch / mirror configurationがないことを確認する。

##### 具体上限

- production write authority: 1
- logical mutation当たりauthoritative commit: 1
- secondary production write: 0
- automatic fallback writer: 0
- dual-write configuration: 0

#### Decision 11: Crash-resumable migration

##### 採用案

migration runはimmutable ordered source manifestと、source、record ordinal、raw substepを持つmonotonic cursorを使う。canonical event / recordとcursorを同じSQLite transactionで確定する。commit前crashではcursorを進めず、commit後response lossはdeterministic import identityのlookupで解決する。

raw recordが通常chunkへ収まらない場合はsame record identityのcopy substepとして継続し、全bytes、hash、length、owner scope、ACLを確認してからrecord cursorを一つ進める。partial generationはauthority pointer、Active projection、public queryへ出さない。source bytes、order、hash、ACLが変化した場合はresumeしない。

##### 却下案

source完了時だけのcheckpoint、restart時の全削除 / 全再import、mtimeだけのsource同一性、filesystem列挙順、payload equality dedupe、raw再serialize、partial generation公開を却下する。

##### 理由

recordとcursorを同じtransactionへ置くことでcrash後のskipとduplicateを同時に防ぐ。ordered manifestとsubstep checkpointによりlarge rawもbounded memoryで同じidentityへ再開できる。

##### Failure behavior

commit前crashはcursor不進行、commit後response lossはsame identity lookupへ収束する。source、prefix、raw staging length / hash、owner binding、ACL driftはMigrationBlockedである。Failed staging generationはauthorityから不可視のまま、lease解除後のbounded cleanup候補にする。

##### Migration verification

record、byte、time各chunk境界、raw chunk publish / sync / checkpoint / finalize、cursor commit直前 / 直後へfaultを入れ、繰返しrestartする。source order変更、hash / ACL drift、上限 / 上限+1、unknown raw、same bytes / different ownerを検証し、duplicate / skip / dedupeが0件であることを確認する。

##### 具体上限

- normal import chunk: 256 records、decoded 1 MiB、50 msの先到達
- raw copy chunk: 1 MiB
- one import step: 16 MiB
- source order: manifestで固定したexactly 1 order
- public visibility of partial generation: 0件

#### Decision 12: Managed backup and restore

##### 採用案

managed backup / restoreはfuture F3の物理契約であり、#1499 public runtimeへrouteを追加しない。backupはcurrent SQLite authorityへBackupLeaseを取り、SQLite online backup APIでstaging DBを作る。barrierと参照blob / private object / owner-only authority record集合はdestination DBから導出し、全object検証後にcomplete manifestを最後にpublishする。

owner-only authority recordには、current app-data generationへ束縛したexactly oneのAgentOperationBindingKeyV1と、current Active recovery-action authority recordを含める。AgentOperationBindingKeyV1はexact 32-byte key、generation、owner ACL、canonical record SHA-256 verifierを保持し、backup / restoreの双方でdecode、re-encode、verifier一致を確認する。key、generation、verifierを再生成、redact、manifest / logへ複製しない。

backupは、current authorityがsealed snapshotと全参照objectを読めるなら、pending obligationやAvailable shutdown detailが存在しても許可できる。それらを完全にbackupへ含めるためである。これに対しPhase 0 cutoverは、migration-exclusive reservation下でterminal Available 0、retiring / residual / pending materialization 0を確認したCompacted sourceと、全identity / known observationのlossless import parityを要求する。この二つのadmission条件を混同しない。

restoreはofflineでfresh physical generationへcopyし、integrity、FK、manifest、blob、owner-only authority、ACL、public oracleを検証した後にauthority pointerを一度だけCASする。logical lineage、initial cutover identity、app-data generation、operation / terminal / shutdown identityを再採番しない。active generationを上書きしない。

backup元bootのclaim / leaseはrestore先でcurrent claimとして使わない。pointer CAS前のoffline normalization transactionで旧boot claimを失効させ、effect identity、pending obligation、saved observationは維持する。authoritative readbackによりsafe retryを証明できるeffectだけをnew bootで再claim可能にし、それ以外はReconciliationRequiredへ投影する。restore後のrecovery inventoryとpublic projectionを検証してからnormal mutation / effect admissionを開き、blind effectを開始しない。

##### 却下案

main DB fileだけのOS copy、open WAL DBのrename、live sourceからのbarrier推測、manifest先行publish、blob / private object除外、active generation上書き、restore時のlineage再採番を却下する。

##### 理由

destination-derived barrierとmanifest-last publishにより、並行commitがあってもDBとimmutable objectの同じsnapshot境界を確定できる。fresh generation restoreとsingle pointer CASにより、失敗時にcurrent authorityを維持できる。

##### Failure behavior

backup途中のfailureはcomplete manifestを作らず、stagingをlease解除後のbounded cleanup対象にする。blob、owner-only authority、ACL、barrier不一致、AgentOperationBindingKeyV1のmissing / duplicate / generation / length / verifier不一致はbackup failureである。restore validation、claim normalization、recovery parity、pointer CAS failureはcurrent authorityを変更しない。pointer outcome unknownはauthority revisionとphysical generation / manifest identityを再読込して解決する。

##### Migration verification

online backup各stepと並行commit、destination barrier、referenced blob / private object、lease中GC、manifest publish、directory publish、restore copy、validation、claim normalization、recovery gate、pointer CASの各境界へfaultを入れる。AgentOperationBindingKeyV1のexactly-one、32-byte key、generation、owner ACL、decode→re-encode verifierと4 command domain bindingを検証し、再生成0件を確認する。backup / restore後もoperation receipt / status、terminal result、pending recovery、shutdown Available / Compacted、counts、deadline、failureを同じpublic queryからexact replayでき、old claimによるeffect開始0件であることを確認する。

##### 具体上限

- online backup step: 256 pagesまたは50 msの先到達
- writer stall per step: p99 50 ms以下
- complete automatic backup retention: non-current直近3世代
- restore target: fresh physical generation exactly 1
- authority pointer CAS: restore当たりexactly 1
- AgentOperationBindingKeyV1: current app-data generation当たりexactly 1、key 32 bytes
- restored old-boot live claim / lease: 0
- #1499で追加するmanaged backup / restore public route: 0

#### Decision 13: Performance budgets and test gate

##### 採用案

wall-clockだけでなく、prepared statementごとのstatement count、visited row、returned / decoded bytes、query planを測る。critical pathはdirect index seekとcandidate件数比例に限定し、legacy read、directory scan、MAX / COUNTによるhead導出、full event replayを0とする。

structural budgetは次に固定する。

| Path | Structural budget |
| --- | --- |
| idempotency / caller binding lookup | unique seek 1、visited row 1以下 |
| backend operation / terminal / obligation ID lookup | identityごとにunique seek 1、visited row 1以下 |
| participant head CAS | participant数以下のPK seek / update、history row visit 0 |
| event insert | candidate event数と同数、existing event scan 0 |
| current projection by ID | Active source seek 1 + entity row seek 1、full replay 0 |
| list / pending page | limit + 1 rows以下、decoded byte cap以下 |
| watch replay | 256 commits / 1 MiB以下 |
| normal startup | event / projection / directory scan 0、history sizeに依存しない定数statement |
| recovery startup | pending-only index 200 records / 4 MiB page、historical event / 全Session scan 0 |
| shutdown page | page index最大32、targetごとにdirect authority / obligation lookup、history scan 0 |

reference datasetは1千eventと100万eventを比較する。CIは少なくとも10万eventでstructural budgetをgateし、release / nightlyは100万eventでabsolute latency、memory、writer stallをgateする。fixed seed、release build、local SSD、WAL / FULL、warm cache、warm-up 50、1000 samples以上を使い、enqueueからpublic resultまでを測る。

##### 却下案

平均値だけ、小さな空DBだけ、EXPLAINだけでactual visited rowを測らない評価、queue waitを除外したlatency、runner / compile option不足をpass扱いすることを却下する。

##### 理由

structural budgetでhistory independenceを決定的に検査し、absolute latencyとmemoryで利用者体験を補完する。両方を満たさなければbounded designを実装したとは判定できない。

##### Failure behavior

structural budget、atomicity、memory capの一回の超過でgate failureとする。latencyは同一runnerで2回連続超過した場合にfailureとし、scanstatus、query plan、queue / DB内訳をartifactへ保存する。required runner、dataset、compile option不足はpassでなく未測定としてclosureをblockする。

##### Migration verification

multi-stream atomicity、CAS、idempotency、OutcomeUnknown、raw preservation、projection rebuild、snapshot / watch、obligation recovery、cutover、dual-write不在、backup / restoreを、各budgetと一対一に対応させる。F1 / F1b public goldenとTauri / WebSocket parityも同じrelease gateで維持する。

##### 具体上限

| Operation | Budget |
| --- | ---: |
| single-stream / 8-event commit | p95 25 ms、p99 75 ms |
| 4-stream / 32-event commit | p95 50 ms、p99 150 ms |
| bounded current / ID query | p95 20 ms、p99 50 ms |
| watch commit to fence | p95 100 ms、p99 500 ms |
| pending recovery first 200 | p95 50 ms |
| import throughput | 2000 events/s以上 |
| online backup writer stall per step | p99 50 ms以下 |
| 1千event対100万eventのp95比 | 1.25以下 |

- warm-up: 50
- measured samples: 1000以上
- CI structural dataset: 10万events
- release / nightly dataset: 100万events
- critical history scan: 0 rows
- one watch / all watches memory: 4 MiB / 64 MiB
- writer lanes: Decision 5の64 / 32 / 32 MiBを超えない

### UI/UX

該当なし。D3はbackendのphysical store designであり、新しいUI、Tauri command、WebSocket routeを追加しない。

既存UIが観測するreceipt、status、terminal、pending recovery、safe action、shutdown Available / Compacted、typed failureはauthority移行前後でsemanticに同一である。storage / migration / projection failureを通常Idle、success、empty resultへfallbackしない。将来managed backup / restore UIはF3以後の別Issueで公開契約を定義する。

### Algorithm

#### Canonical commit

1. authority pointer、generation manifest、DB lineageを検証し、選択されたsingle writerへrequestを送る。
2. count / byte / identity / schemaをtransaction前に検証し、必要なimmutable blobをstagingからexclusive publish / syncする。
3. BEGIN IMMEDIATEを開始し、operation / transaction identityをdirect lookupする。
4. same bindingのsealed resultがあれば保存済みreceipt / resultを返す。different bindingはPayloadConflictでrollbackする。
5. 全participant headをexpected valueで読み、stable orderでCAS対象を固定する。
6. next global sequenceをexact +1で予約する。
7. participant、event、stream head、receipt、status、terminal / obligationなどbatchのcanonical mutationを行う。
8. participant / event count、ordinal、head、allocator、required index parityを確認する。
9. commit sealを最後にinsertする。schema guardがseal後のcanonical writeを全て拒否することを確認し、SQL COMMITする。
10. COMMIT成功後だけsuccessを返す。結果不明ならsame identity lookupへ移り、external effectを開始しない。

#### External-effect dispatch

1. usecaseがeffect identity、target revision、exact payload reference、safe recovery policyを決める。
2. corresponding obligation、claim、fenceをEffectReservedとしてcanonical commitする。
3. dispatch guardがsame authority、same obligation、same claim、same target revision、pending stateをfresh readで確認する。
4. guard内で新しいstorage / provider / session lock待ちを行わず、effectを一回だけ開始する。
5. resultまたは信頼できるobservationをkind-specific side stateと同じcommitで閉じる。
6. outcomeを証明できない場合はReconciliationRequiredを維持し、同じeffect identity以外でretryしない。

#### Snapshot and watch

1. required projection source集合を最大64件に正規化する。
2. sealed commitのcommon readable watermarkを固定し、30秒leaseを取る。
3. cursorをsnapshot、filter、owner、orderへ束縛する。
4. watchの場合は同じ境界でreplay start、fence、subscriptionを作る。
5. buffer lag、lease expiry、projection lagではpartial deltaを捨て、typed resultでsnapshotからの再開を要求する。

#### Phase 0 to F3 migration

1. stable migration progress identityとactive bootstrap pointerを取得または再開し、exclusive authority lockを取って新規mutationとeffect admissionを閉じる。
2. latest terminal shutdownがAvailableかつlatest-retiring Noneなら、pointerだけを対象planのSomeへCASする。preexisting retiring Someなら同じplanをresumeする。exclusive lockとreserved pointerでshutdown root-init、通常 / background compactor、normal mutation、effect admissionをfreezeし、別のreservation recordを作らない。
3. migration coordinatorだけがreserved pointer下でArchiveSwitch、DetailDetach、FinalizeDetachを完了する。physical transaction / materializationのOutcomeUnknownをsame identityで解決し、shutdown Availableを全てCompactedへ進め、最後のdedicated closureでretiring / residual / pending materializationを0にする。semantic pending operation / obligationはterminal化しない。
4. authority pointerが指すactivated generationだけからimmutable source manifestを作る。
5. internal Phase 0 cutover snapshotをcomplete-manifest-lastで作り、isolated rootへのrestore smokeを行う。
6. fresh staging physical generationへDecision 11のcursorでimportする。
7. 確定transcript、既知event、identity、receipt / status、terminal reason、session lifecycle、owner relation、permission、queue、未完了active turn / obligation / recoveryのknown observation、shutdown Compacted、projectionのparityを検証する。
8. stagingをReadyにし、expected authority revisionでpointerを一度だけCASする。
9. pointer、manifest、DB chainをfresh process相当で再openする。
10. CAS後はSQLiteだけをauthorityとし、Phase 0をread-only provenanceとして扱う。post-CAS validation後のfinal activation closureでmigration run completed provenance、active bootstrap pointer clear、durable normal admission Openを同時に確定し、readback後にruntime admissionを開く。automatic rollbackは行わない。

#### Managed backup and restore

Backup:

1. current SQLite authorityとBackupLeaseを固定する。
2. online backup APIを256 pagesまたは50 msでyieldしながらstaging DBへcopyする。
3. destination DBからsealed barrierと参照object集合を導出する。
4. blob、private object、AgentOperationBindingKeyV1、current Active recovery-action authority recordをcopyし、integrity、decode→re-encode verifier、ACLを検証する。
5. complete manifestを最後にexclusive publishする。
6. leaseを解放し、complete generationだけをbackupとして列挙する。

Restore:

1. backup manifest、DB、object、lineage、public oracleをoffline検証する。
2. fresh physical generationへcopyし、logical lineageとapp-data identityを維持する。
3. integrity check、FK check、projection / operation / shutdown parityを検証する。
4. offline normalization transactionでbackup元bootのclaim / leaseを失効させ、effect identityとpending obligationを維持する。
5. expected authority revisionでpointerを一度だけCASする。
6. new authorityを再openし、pending recoveryを再構築する。safe readbackを証明できないeffectをReconciliationRequiredへ写してからnormal admissionを開く。
7. 失敗時はcurrent authorityを維持する。

### Infra

#### SQLite runtime

- bundled SQLite 3.45.0以上を固定し、startupでactual versionとrequired compile optionを検証する。
- write connectionはWAL、synchronous=FULL、foreign_keys=ON、trusted_schema=OFF、defensive modeを使う。
- fresh DBは4 KiB pageを固定する。
- macOSではfullfsync / checkpoint fullfsyncの利用可能性とactual settingを検証する。
- writer一つ、reader最大4つをdedicated blocking workerで動かす。
- performance buildはactual visited rowを取得できるscanstatus相当のinstrumentationを有効にする。

#### Filesystem layout

physical generation directoryはowner-onlyとし、DB、immutable retained blob、owner-private opaque blob、AgentOperationBindingKeyV1とcurrent Active recovery-action authority record、generation manifestを分離する。POSIXではdirectory 0700、file 0600相当を作成時とopen時に検証し、他platformではcurrent userと必要なsystem principalだけへ制限する。

authority pointerとgeneration manifestはtemp write、file sync、exclusive publishまたはexpected-revision atomic replace、parent directory syncで更新する。incomplete staging directoryはauthorityではない。blob GC、migration cleanup、backup cleanupはactive transaction、snapshot、watch、backup leaseを侵害しないbounded background workとして実行する。

#### Observability

次をsafe metadataとして記録する。

- correlation identity
- authority kindとphysical generation identity
- transaction / operation identityの非secret表現
- lane、queue wait、worker time、SQL time、checkpoint time
- statement count、visited row、returned / decoded bytes
- projection watermarkとlag
- migration run / source / record / substep
- backup / restore phase
- typed error class

raw SQL、filesystem path、secret、binding key、private payload、provider payload、owner-private blob bytesをlogまたはmetric labelへ出さない。

## Alternatives Considered

各Decisionの却下案が個別の正本である。横断的な代替案と却下理由をまとめる。

| Alternative | 却下理由 |
| --- | --- |
| Phase 0 file storeを汎用storeとして拡張し続ける | multi-stream transaction、global order、bounded index、online backupを一つのdurability boundaryで保証できない |
| context / sessionごとの独立DB | cross-stream atomicityと一意のglobal commitを失う |
| dual-writeによる段階cutover | partial successとorder / idempotency divergenceを作る |
| event historyからcurrent stateを毎回foldする | history-independent query / commit budgetを満たさない |
| in-place projection rebuild | rebuild途中のrowをcurrent queryから隔離できない |
| unknown eventをdropまたはcurrent型へbest-effort変換する | lossless migrationとfuture compatibilityを失う |
| OS file copyによるbackup | WALと参照blobの同一snapshotを証明できない |
| latencyだけを測るperformance gate | accidental scanをsmall fixtureやfast runnerが隠す |

## Cross-cutting concerns

#### Crash consistency

canonical stateはsealを最後に持つ一つのSQLite transactionだけで公開する。filesystem objectはtransaction前にimmutable publish / syncし、DB参照が到達性を所有する。OutcomeUnknownを別identityのretryへ変換しない。

#### Security and privacy

principal分離、owner scope、ACL、private opaque identityをauthority移行前後で維持する。cross-principal operation lookupはNotFoundで存在を開示しない。secret、private payload、raw SQL、pathをpublic failure / logへ出さない。content-addressed storageはequality disclosureを許したretained contentだけに使う。

D3は将来privacy purge / authority rotationのpublic behaviorを定義しない。F3以後に追加する場合も、本書のcommit、obligation、backup、restore、GC不変条件を破らない別Requirements / Behavior / Designが必要である。

#### Compatibility

R-018に従い、変更前のopen / closed / archived session、確定transcript、既知eventの公開結果、terminal reason、session lifecycle、owner relation、permission、queue-linked inputを明示migrationなしに読める。未完了active turn、queued input、permission、recoveryは元identityとknown observationを維持する。bootstrap中はlegacyをread-onlyで表示し、新規mutationはBootstrapInProgressとする。

bootstrap中quitはnormal shutdown target / provider / workflow effectを作らない。quit operation direct recordをcurrent migration progress identityへ束縛し、13秒settle、15秒exit decisionの後も同じmigration progressとquit operationをrestart後にdirect取得する。implicit process exitをmigration成功、terminal、ConfirmedNoEffectへ推測変換しない。

F1 / F1b public golden、共有routeのTauri / WebSocket semantic parity、lossless decimal integer、SessionLifecycleを含む4 command domainのbinding / import parityを維持する。SessionLifecycleはTauri専用でありWebSocket routeを追加しない。

#### Bounded resources

countだけでなくdecoded bytes、resident bytes、deadline、visited rowを上限化する。capacity超過、query deadline、watch lag、cursor expiryでpartial resultを返さない。queue、snapshot、watch、migration、backup、GCの各上限はDecision 5、8、11、12、13を正本とする。

#### Backup versus cutover

managed backupは、current SQLite snapshotと参照objectを完全に保存できればpending stateとAvailable shutdown detailを含んだまま許可できる。Phase 0 cutoverは、migration-exclusive reservation下でterminal Available 0、retiring / residual / pending materialization 0を確認したCompacted sourceと、全public / recovery identityのlossless parityが必要である。backup successをcutover readinessとみなさず、cutover readinessをbackupからpending operation / obligationを除く理由にしない。

#### Traceability

| Requirement / Behavior | Design decision |
| --- | --- |
| R-013 / B-050 | Decisions 1〜13の6項目と具体上限 |
| R-018 | Interface compatibility、Data Model invariants 13〜15、Decision 6 / 9 / 11 |
| B-098 cutover | Decision 9、Phase 0 to F3 migration algorithm |
| B-098 backup / restore | Decision 12、Managed backup and restore algorithm |
| history-independent operation | Decision 2 / 7 / 8 / 13 |
| effect safety / recovery | Data Model invariant 7 / 14、External-effect dispatch |
| surface parity | Interface Surface compatibility、Cross-cutting Compatibility |

## Risks

| Risk | 影響 | Mitigation / gate |
| --- | --- | --- |
| single writer contention | Stop / shutdown deadline超過 | bounded lane、byte permit、EDF、Decision 13 latency gate |
| SQLite planner drift | direct queryがhistory scanへ退行 | statement count / visited row / query-plan golden |
| filesystem sync差 | COMMIT外objectまたはpointerのdurability差 | bundled settings、actual PRAGMA / fsync verification、fault injection |
| migration source drift | duplicate、skip、identity差替え | exclusive authority lock、immutable source manifest、same-transaction cursor |
| physical / logical identity混同 | restore後にlineageまたはoperation identityが変わる | separate manifest fields、restore parity、pointer chain validation |
| unresolved operation / shutdown detail欠損 | cutover後のsafe recovery不能 | MigrationBlocked、B-098 public oracle parity |
| projection rebuild failure | stale / partial current state | shadow generation、authoritative normal admission時Active exactly one、no replay fallback |
| watch / snapshot retention圧迫 | memory増大、GC停止 | TTL、process cap、lag / expiry、lease-aware GC |
| backupの参照object欠損 | restore後のpartial state | destination-derived object set、manifest-last publish、restore validation |
| future featureの先取り | #1499 surfaceとF3 schemaの不要な肥大化 | managed backup以外のfuture public behaviorをnon-goalとし、別Issueで設計 |
| DDL細部の実装差 | invariantをschemaが強制しない | F3 migration reviewでtable family、key、CAS、index、seal、fault gateを照合 |

F3実装時にtable / column / trigger名、statement分割、query syntaxを選べる。ただし、本書のauthority一意性、transaction境界、key / CAS / index、不変条件、公開failure、migration / backup behavior、具体上限を変更する選択はD3の再reviewを必要とする。
