# Design

Primary Spec: [requirements.md](requirements.md) / [behavior.md](behavior.md)

## The actual design

### Architecture

Rust backend が、command acceptance、operation identity、domain transition、persistence admission、recovery、shutdown を所有する。frontend は入力 attempt と backend が返す read model を保持し、domain decision を再実装しない。controller は operation usecase だけを呼び、usecase は一つの canonical persistence gateway と、provider / workflow / process exit を抽象化した effect port を使う。

この分割は [DOMAIN.md](../../architecture/DOMAIN.md)、[USECASE.md](../../architecture/USECASE.md)、[GATEWAY.md](../../architecture/GATEWAY.md)、[CONTROLLER.md](../../architecture/CONTROLLER.md) と repository の Rust-first 規約に従う。

責務境界は次のとおり。

| Concern | Owner |
| --- | --- |
| message、turn、permission、queue、operation、terminal の意味 | domain |
| send、Stop、recovery、Session lifecycle、shutdown、startup outcome の調停 | usecase |
| SQLite の create / open、schema evolution、atomic persistence、query | gateway |
| provider、runtime、workflow、process exit への作用 | infrastructure port |
| Tauri / WebSocket の意味的に同じ入出力 | controller / presenter |
| 表示と caller attempt の保持 | frontend |

恒久 SQLite store は固定 path に一つだけ存在し、normal runtime の唯一の persistence authority である。旧 file-store は production composition の依存、入力、fallback、cleanup 対象に含めない。この保証は startup だけでなく、background maintenance、retention、cleanup、shutdown にも適用する。

startup composition は二段に分ける。pre-admission では `ApplicationStartupAuthority`、process-local exit port、固定 store の opener だけを構築する。store が `Ready` を返した後にだけ Session、Workflow、PTY、provider、normal application state、WebSocket server、startup recovery、maintenance を構築する。Tauriのinvoke handlerが静的にroute名を登録していても、top-level dispatcherは`ApplicationStartupOutcome`を最初に検査し、`Failed`ではstartup用二command以外をstate解決前に`ApplicationUnavailable`として拒否する。したがってstore open failureからnormal runtimeの一部だけが生きる経路はない。

`ApplicationStartupAuthority` はusecase-ownedである。gatewayはpath / SQLite failureをprivateなclosed errorへ分類し、usecaseがpublicな`ApplicationStartupOutcome`と利用可能actionを決め、presenterがallow-listed wire representationへ写像する。controllerとfrontendはraw errorを解析しない。

shutdown は一つの Rust-owned durable aggregate を正本とする。保存、整合性検証、作用開始可否、current read、history read、target pagination は同じ SQLite の `shutdown_plans` / `shutdown_targets` と committed revision を共有する。summary と detail は同じ aggregate の view であり、不一致時は fail closed にする。page file、page reference、root hash、root page、別 archive blob、current recovery collection は shutdown authority にしない。

### Interface

公開 interface は実装型ではなく、次の意味を保証する。

- deadline、capacity、page、wire integer の exact value は [requirements.md](requirements.md) の public boundary table を正本とし、本 Design では再定義しない。
- command は未受理、受理済み、結果確認必要、完了を区別する。
- 同じ authorized caller と request identity の同じ intent は同じ result を返す。
- 同じ request identity の異なる intent は effect を開始せず conflict になる。
- 受理済み operation は response loss と restart 後も direct lookup できる。
- collection access は一貫した revision の結果だけを返し、完全性を保証できない場合は partial result を返さない。
- Tauri と WebSocket は transport 表現が異なっても同じ semantic result を返す。
- public integer は lossless に往復し、不正表現を domain command へ渡さない。

Safe failure は利用者が行動を選べる分類、公開可能な説明、相関情報だけを持つ。private payload、path、SQL、internal proof は公開しない。

recovery action と application shutdown status の公開意味は [agent-chat-ideal-vocabulary.md](../../../specs/milestone-84-agent-chat-stabilization/agent-chat-ideal-vocabulary.md) を共通語彙とし、transport や内部 state machine が別の意味を追加しない。

normal admission 前の Rust interface は次の closed contract とする。これは legacy-data migration の state、progress、query、gate、または別名ではなく、一回の store create / open attempt の process-local result である。

```text
ApplicationStartupOutcome =
  Ready
  | Failed {
      kind: StartupFailureKind,
      safe_description: String,
      correlation_id: String,
      retry_on_next_launch: bool,
      actions: [Quit],
    }

StartupFailureKind =
  StoreInUse
  | StorageUnavailable
  | UnsupportedRuntime
  | UnsupportedStoreVersion
  | InitializationStateInvalid
  | StoreValidationFailed
  | SchemaEvolutionFailed
```

`safe_description` は kind から作る allow-listed 文言であり、下位 error の文字列を使わない。`correlation_id` は一回の失敗ごとにUUID v4として生成する非秘密の診断相関子で、installation identityではない。`retry_on_next_launch`は同一process内のloopを許可する値ではない。`actions`は`Failed`で常に`Quit`一件、`Ready`では空である。

`retry_on_next_launch` の写像は次に固定する。

| Kind | Value | Reason |
| --- | --- | --- |
| `StoreInUse` | `true` | owner process の終了後は同じ fixed store を再評価できる |
| `StorageUnavailable` | `true` | 次回 attempt で filesystem condition を再評価できる |
| `UnsupportedRuntime` | `false` | 対応 build が必要で、同じ build 内の再試行では変わらない |
| `UnsupportedStoreVersion` | `false` | 対応 build が必要で、既存 store を変更しない |
| `InitializationStateInvalid` | `false` | 自動 create / repair を安全と証明できない |
| `StoreValidationFailed` | `false` | 初期化済み store の自動 repair を行わない |
| `SchemaEvolutionFailed` | `true` | transaction 後の旧 / 新 schema を次回 startup で再分類できる |

`ApplicationStartupAuthority` は最初の outcome と correlation を process lifetime 中保持し、query のたびに store attempt を再実行しない。pre-admission Tauri surface は read-only な `get_application_startup_outcome` と、`Failed` のときだけ有効な `quit_after_startup_failure` に閉じる。後者は process-local single-flight に join し、SQLite、normal shutdown coordinator、Session / Workflow state を参照または作成しない。他のTauri commandは`ApplicationUnavailable`を返し、domain-specific failureや空read modelへfallbackしない。normal WebSocket serverは`Ready`前に起動しないため、startup failure用WebSocket routeは作らない。

presenterがfrontendへ渡すclosed wire shapeは次のとおりである。fieldの追加はpublic representation versionの変更として扱う。

```text
{ "type": "ready" }

{ "type": "failed",
  "kind": "store_in_use"
        | "storage_unavailable"
        | "unsupported_runtime"
        | "unsupported_store_version"
        | "initialization_state_invalid"
        | "store_validation_failed"
        | "schema_evolution_failed",
  "safeDescription": <allow-listed string>,
  "correlationId": <same process outcome value>,
  "retryOnNextLaunch": <fixed mapping>,
  "actions": ["quit"] }
```

`quit_after_startup_failure`は`{ "type": "accepted", "correlationId": <同一process outcomeの値> }`を返すか、native exitが先に完了する。重複callも同じ結果へjoinし、process-local progress DTOを作らない。`Ready`でのcallはeffect 0件の`ApplicationUnavailable`である。

### Data Model

`MessagePart`、domain event、operation、terminal、recovery obligation、shutdown aggregate は domain vocabulary である。SQLite representation と public representation はこれらを各境界へ写像するが、domain の意味を変更しない。

operation binding は canonical SQLite store 内で、authorized principal、command kind、caller request identity、semantic intent を一意に対応させる。別の永続 generation や切替 authority を operation scope にしない。

receipt は受理時点の不変事実、status はその後の進行状態である。status failure は receipt を未受理へ戻さない。terminal と recovery result は同じ owner / operation の canonical outcome を参照する。

未完了の外部作用は operation に結び付く recovery obligation として追跡する。terminal は turn の outcome と競合 operation の解決を同時に確定し、shutdown はその flight に属する target obligation と summary を一つの aggregate として扱う。一つの利用者操作で同時に変わるこれらの state は同じ atomic persistence boundary に参加する。

永続 identity は `installation_id` 一つにする。owner は local event store gateway であり、初回初期化 transaction 内で UUID v4 として一度だけ生成し、singleton `store_metadata` に保存する。restart と supported schema evolution は同じ値を読む。ready store で欠損、形式不正、変更を検出した場合は生成し直さず `StoreValidationFailed` にする。

operation binding、logical commit の idempotency、cursor HMAC、recovery action、obligation、shutdown correlation は、同じ SQLite file に属することを `installation_id` で domain separation する。単一 database 内の primary key に `installation_id` を反復保存する必要はないが、署名・hash の canonical input には含める。`store_id`、`generation_id`、`app_data_generation` は持たない。

`cursor_hmac_key` と `operation_binding_hmac_key` は cryptographically secure な 32 bytes とし、`installation_id` と同じ初回 transaction で一度だけ保存する。restart、schema evolution、quit、recovery で再生成しない。欠損・長さ不正は validation failure である。

`process_instance_id` は process 起動ごとに生成する別の fence token である。effect reservation やshutdown planに保存してよいが、restart後は「このprocessは現ownerではない」と証明するhistorical valueとしてだけ読む。新processは新しい値を使い、operation binding、HMAC、idempotency、obligation / action identity、public correlationのscopeには使わない。

### Database

#### Fixed layout

gateway が app-data から導出して production で触れる local event store path は次の三つだけである。

| Path | Responsibility |
| --- | --- |
| `<app-data>/local-event-store.sqlite3` | 唯一の恒久 read / write authority |
| `<app-data>/local-event-store.lock` | process 間の exclusive writer mutex。durable state を保存しない |
| `<app-data>/local-event-store.initial-create` | 初回 create を normal admission まで完了していないことだけを証明する versioned evidence |

SQLite 自身の `-wal` / `-shm` sidecar は fixed database の一部として扱う。database generation directory、authority pointer、staging database、legacy source inventory は作らない。`StoreLayout` は上の固定名を join するだけで、app-data または legacy root を列挙しない。

#### Initial-create protocol

既存 file が 0 byte であるという事実だけでは、初回 create の中断と初期化済み database の切詰めを区別できない。そのため、空 file を無条件で再初期化しない。初回作成は次の durability order を使う。

1. `local-event-store.lock` を non-blocking に exclusive lock する。
2. fixed database が absent で initial-create evidence も absent の場合だけ、固定 magic、format version、checksum を持つ `local-event-store.initial-create` を create-new し、file と app-data directory を sync する。evidence は identity、source path、schema progress、store locator、operation state を持たない。
3. fixed database を直接 create / openし、`BEGIN IMMEDIATE` 内で全 schema、singleton metadata、`installation_id`、二つの HMAC key、`PRAGMA application_id = 0x524C5348`、schema version を一つの初期化成果として確定する。
4. commit 後に database / WAL を checkpoint・syncし、readback validation を行う。
5. validation 成功後に initial-create evidence を削除して app-data directory を syncする。ここまで normal admission を開かない。

databaseがabsentなら、partial / invalid evidenceはdatabase作成前に中断したapp-owned artifactとして削除・directory syncし、手順2から再作成できる。有効なevidenceが残る間はnormal commandが一度もadmissionされていないため、fixed databaseがabsent、0 byte、またはapplication tableのない検証可能なSQLiteなら、その初回作成残骸だけを除去して同じpathで手順3から再試行できる。databaseが既にready metadataまで検証できる場合はdatabaseを変更せずevidenceだけを完了処理する。evidenceがabsent / invalidの既存空file、別用途SQLite、非SQLite、またはevidenceがあっても非空で検証不能なfileは初回残骸と推測せずfail closedにする。

initial-create evidence は成功後に存在せず、runtime authority、installation identity、schema version、進捗、public startup state ではない。通常起動、schema evolution、operation、shutdown はこれを参照しない。

#### Open and schema evolution

既存 store は read-only classification で SQLite header、`application_id`、singleton metadata、schema version、HMAC key の形を確認してから write-capable connection を渡す。ready store の validation failure は file の byte を置換、削除、truncate、再初期化しない。SQLite の通常 crash recovery と supported schema evolution 以外の修復を自動実行しない。

startup error の写像は次に固定する。

| Condition | Outcome |
| --- | --- |
| exclusive writer lock が別 process の ownership / contention により取得できない | `StoreInUse` |
| bundled SQLite runtime がminimum version / required featureを満たさない | `UnsupportedRuntime` |
| fixed path / lock / evidence / SQLiteへのI/O、permission、capacity error | `StorageUnavailable` |
| ready storeを検証できず、absent / empty / foreign / non-SQLite fileを未完了初回作成と証明できない | `InitializationStateInvalid` |
| recognized application storeのschema versionがnewer、gap、またはcompiled-in pathの対象外 | `UnsupportedStoreVersion` |
| recognized supported schemaのsingleton metadata、installation identity、HMAC key、schema / integrity invariantが不正 | `StoreValidationFailed` |
| supported schema stepのtransaction、commit readback、post-step invariantを確認できない | `SchemaEvolutionFailed` |

fully validなready storeがある場合、initial-create evidenceはdatabase authorityを変更しないstale artifactとしてReady前に削除・directory syncする。そのcleanupに失敗した場合は`StorageUnavailable`であり、normal admissionを開かない。fixed databaseが存在してreadyでない場合、invalid evidenceをrepair / recreateの根拠にしない。database absent時のinvalid evidence再作成は、databaseが存在しないことをlock取得後に再確認した場合だけ許可する。

supported schema evolution は、compiled-in の現在 version まで連続する各 step を一度ずつ `BEGIN IMMEDIATE` transaction で適用し、step の schema / data invariant、`PRAGMA user_version`、metadata の schema version を同じ commit で更新する。commit 後の readback が全て成功してから次 step または normal admission へ進む。installation identity と HMAC key は mutation 対象にしない。未対応 version、version gap、step failure、commit 結果不明は `UnsupportedStoreVersion` または `SchemaEvolutionFailed` として fail closed にする。

一回の startup attempt は、writer lock を待たず、SQLite `busy_timeout` を最大 2 秒とし、create / open / evolution / validation の内側で retry loop を持たない。ここで bounded とは、対象 path、lock attempt、SQLite busy wait、version step の実行回数が有限であることを指す。OS filesystem call 自体の wall-clock 完了を推測する意味ではない。fault test は各 I/O / transaction 境界で呼出回数と outcome を観測する。

current target schema と codec / domain enum は `store_id`、`generation_id`、migration operation kind、migration quit flag、`local_store_migrations`、`legacy_source_inventory`、`legacy_raw_records`、`legacy_raw_record_chunks`、`migration_quit_flights`、`shutdown_compact_archives` を持たない。supported SQLite schema evolutionでこれらの廃止済みfield / tableを除去する場合も、legacy pathを探索・読込せず、authority・progress・compatibility readとして保持しない。

#### Canonical shutdown rows

`shutdown_id` は accepted application quit の backend operation identity と同じ値であり、別のplan ID、epoch、generation identityを作らない。`shutdown_plans`はshutdown / operation identity、intent、accepted process instance、`T0`、`T0 + 13秒`のpreparation cutoff、`T0 + 15秒`のexit deadline、phase、aggregate revision、target count、state別count、safe failure、detail availabilityを一行で持つ。保存されるphaseは`Prepared | Activated | Quiescing | Completed | Failed | Cancelled | ReconciliationRequired`に閉じ、`Prepared`をvocabularyの利用者可視`Preparing`へ、それ以外を対応する利用者可視statusへ一方向に写像する。

`shutdown_targets`は`(shutdown_id, ordinal)`をprimary keyとし、stable target identity、`AgentSession | WorkflowExecution | WorkflowNode`のkind、`Prepared | EffectReserved | Completed | Failed | ReconciliationRequired`のtarget state、effect identity / observation、recovery action identity、safe detail、row revisionを順序付き一行で持つ。`shutdown_recovery_snapshots`は`(shutdown_id, ordinal)`をprimary keyとし、quit acceptance時点のpending recovery identity、owner、safe recordを同じaggregate revisionに固定する。

current shutdown locatorは`store_metadata.current_shutdown_id`から同じ`shutdown_plans` rowへのnullable foreign keyとする。quit acceptanceで設定し、`Completed | Failed | Cancelled`へのterminal transactionでclearする。`ReconciliationRequired`はblocking currentとして保持する。plan history rowはlocatorの有無にかかわらず残るため、current read、known-operation read、history readが別結果を合成しない。別fileやhashをauthorityにしない。

quit の acceptance transaction は operation binding / receipt、plan row、0〜4096 の ordered target rows、開始時 recovery snapshot rows、current locator を全て確定する。`target_count = 0`ではtarget rowが0件で`MIN / MAX`は`NULL`、`target_count > 0`ではordinalが0から連続し、`COUNT(*) = target_count`、`MIN = 0`、`MAX = target_count - 1`でなければeffect gateを閉じる。recovery snapshot countもplan rowと実row数を一致させる。保存後検証はこのrow setをread transactionで読むだけであり、serialized page、reference、root hash、root pageを生成・比較しない。

acceptance transactionのplan phaseは`Prepared`である。保存後検証に成功したaccepted processだけが、plan revisionとcurrent locatorをguardに`Prepared -> Activated`を一transactionでcompare-and-setできる。activation commitをreadbackできない場合はeffectを開始せず、同じshutdownを`ReconciliationRequired`として解決する。previous processの`process_instance_id`を持つplanや、restartで発見した未予約targetからshutdown effectを自動開始しない。

`Activated`を確認したaccepted processは、各external shutdown effectの直前に同じtarget rowを`Prepared`から`EffectReserved`へcompare-and-setし、plan revisionとsummary countを同じtransactionで更新する。commit済みreservation、current process instance、current owner revisionをreadbackできた場合だけeffectを開始する。外部作用を要しないtargetはactivation後に`Prepared -> Completed`を一transactionで確定する。effect resultは同じtarget rowをterminalまたはreconciliationへ進め、最後のtargetとplan terminalを同じtransactionで確定する。`target_count = 0`はactivation readback後にeffect 0件のままplanを`Completed`へ進めてcurrent locatorを同じtransactionでclearする。current / history summary、target detail、effect gate、recovery actionは全てこのrow stateを読む。

target pagination は `(shutdown_id, plan_revision, next_ordinal, limit)` を installation-scoped cursor key で署名し、`ORDER BY ordinal` で rowを直接読む。continuation時に plan revisionが変われば partial pageを返さず revision conflictにする。full detailを保持するterminal shutdownが2件を超えた場合は、一つのtransactionで最古planをsummary-onlyへ変更してそのtarget / historical recovery rowsを削除する。summary rowを別archive blobへ複製しない。

#### Other mutations and reads

operation binding の database key は `(principal, command_kind, caller_request_identity)`、logical commit の idempotency key は `(operation_kind, idempotency_key)` とし、single fixed database が installation scope を与える。HMAC / digest の canonical bytes には `installation_id` を含める。mutation は一つの atomic persistence boundary で event と必要な state を確定する。read は同じ committed revision から結果を作り、異なる revision の断片を合成しない。identity lookup と bounded collection access は無関係な全履歴に依存しない。

### UI/UX

通常起動に成功した場合だけ normal workbench を表示する。startup に失敗した場合は S11 を排他的に表示し、Rust が返した failure kind に対応する allow-listed label、safe description、correlation、`retry_on_next_launch` の guidance、Quit だけを表示する。Session、transcript、normal mutation、migration progress、initial-create evidence、旧 file-store data、raw path / SQL / database error を表示しない。

composer は durable acceptance 後だけ対応する attempt を clear する。accepted operation、pending recovery、Session lifecycle、shutdown は backend read model を表示し、frontend timeout や文字列解析から retry、terminal、成功を合成しない。

### Algorithm

通常 command は、authorized caller と semantic intent を検証し、既存 operation があれば replay / conflict を決め、新規なら durable acceptance を確定する。全 external effect は共通の Rust-owned dispatch boundary を通り、実際の呼出し直前に canonical intent と現在の owner がまだ有効であることを再確認してから開始する。stale な intent は作用を開始しない。作用後の結果不明は同じ operation を reconciliation へ進める。

terminal 競合は一つの canonical outcome へ収束する。Stop、close、quit、failure、crash は queue を pause し、通常完了だけが許可された次 item を開始できる。

application composition は initial-create protocol、SQLite の create / open、必要な schema evolution、store validation、initial-create evidence の完了を終えてから normal command ingress を公開する。各試行は Startup attempt boundary の一回だけで Ready または safe failure へ着地し、旧 file-store の存在や履歴量を待たない。startup failure 中の cooperative quit は durable operation を作らず、request ingress から15秒以内に process-local exit を一度だけdispatchする。

application quit は最初のaccepted intentを一つのflightに固定する。`T0 + 13秒`をdurability / preparation cutoffとし、それ以後は新しいtarget reservationやshutdown effectを開始しない。cutoffまでにeffect開始前の安全なabortを確定できる場合だけworkbenchへ戻る。reservation済み、effect開始済み、または開始結果不明の場合はadmissionを再開せず、残り2秒で未完了targetをreconciliationとして確定し、`T0 + 15秒`までにexit / restartする。未完了結果は同じshutdown identityで次回起動へ残す。

### Infra

SQLite、filesystem、provider process、native exit は infrastructure concern であり、domain へ library type や physical layout を露出しない。blocking I/O は async executor を塞がず、deadline 後の late result は元 operation にだけ適用する。

production validation は path-recording filesystem port と sentinel を使い、startup、通常処理、background maintenance、retention、cleanup、shutdown、restart の各 lifecycle でB-070が列挙する旧file-store pathへのopen / stat / read_dir / read / write / rename / removeが0件であることを検証する。configuration migration、SQLite schema evolution、watch subscription initialization は別fixtureとallow-listで非退行を検証し、この禁止に含めない。

startup / schema acceptance oracle は次に固定する。

| Behavior | Fault / fixture | Required observation |
| --- | --- | --- |
| B-071 initial create | evidence create前、partial write後、file sync後、directory sync後、SQLite file create後、initialization commit前、commit reply loss後、database sync後、evidence unlink前後でprocessを停止 | 次回はCase A / B / Cのいずれかへ一意に分類し、Readyは一つのinstallation identity / key setだけを持つ。normal effectはReady前0件 |
| B-071 initialized failure | valid ready fixtureのheader、application ID、schema version、metadata row、installation identity、各HMAC key、required indexを一項目ずつ破損 | closed failure kindが期待値と一致し、database / WAL / SHMの存在、length、SHA-256をattempt前後で変更せず、normal stateを構築しない |
| B-071 safe surface | 各`StartupFailureKind`と重複Quitをfake gateway / fake clock / fake exit portへ入力 | allow-listed fieldだけを返し、全normal Tauri commandは`ApplicationUnavailable`、WebSocket listen 0回、exit dispatch 1回、deadline 15秒以内 |
| B-098 schema evolution | supported old schemaにoperation、terminal、obligation、shutdown、signed cursorを保存し、各version stepのtransaction開始前、commit前、commit reply loss後、readback前で停止 | 再起動後は検証可能な旧 / 新schemaだけになり、同じoperation replay、terminal、obligation、shutdown summary / detail、cursor semanticsへ収束し、installation identity / HMAC keyはbyte不変 |
| B-070 legacy non-reference | 各legacy rootにmalformed sentinelを置き、production startupからrestartまでpath-recording gatewayを通す | B-070が列挙するaccessは0件、sentinel byte / metadata / directory entryは不変。SQLite schema / configuration / watchの各正当な初期化testは成功 |

## Alternatives Considered

- legacy file-store を読み、SQLite へ import して切り替える案: 二重 authority と証拠不足の未完了作用を生むため不採用。
- file-store compatibility API を残す案: normal runtime が旧物理設計へ依存し続けるため不採用。
- event だけを保存し、起動時や query 時に全履歴を再計算する案: history-independent な操作保証を満たさないため不採用。
- frontend が timeout、retryability、terminal winner を決める案: Rust-owned authority と live / reload 等価性を破るため不採用。

## Cross-cutting concerns

- Crash consistency: committed SQLite state だけを受理・完了の根拠にする。
- Security: secret、private response、path、internal proof を public failure や telemetry へ出さない。
- Idempotency: same identity / same intent は replay、different intent は conflict。
- Compatibility: SQLite persistence と public representation を独立して version 管理する。
- Performance: direct lookup と bounded collection は無関係な履歴量から独立させる。
- Retention: cleanup は canonical state を変更せず、旧 file-store を対象にしない。

## Risks

- 初回作成残骸を破損済み store と誤認すると永久 block になる。初期化が完了した事実を安全に確認できるかどうかで outcome を分ける acceptance test を必須にする。
- 初期化済み store の検証 failure を空 store と誤認すると data loss になる。initial-create evidence が正常にdurable化され、かつnormal admission前であると証明できる残骸以外の自動置換、削除、再初期化を禁止する。
- initial-create evidence の削除前にnormal admissionすると、evidenceがdata消失を許す誤った証拠になる。database readback、sync、evidence削除、directory syncの完了後だけReadyを公開する。
- installation identity またはHMAC keyをschema evolution時に再生成すると、replay、cursor、obligation correlationがrestart前後で分裂する。ready storeではimmutableとして検証し、欠損時はfail closedにする。
- startup failure を normal shutdown と共用すると不要なdurable progressが生じる。startup failure は pre-admission の process-local exit に限定する。
- shutdown summary と detail を別 authority にすると結果が分裂する。plan / ordered target rowと同じaggregate revisionからのみ保存検証、effect reservation、公開readを行い、不一致をfail closedにする。
