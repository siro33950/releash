## B-001: 通常 send の初回受理

GIVEN Idle の session s-1 と caller が保持する operation identity op-1 と入力 hello がある
WHEN Tauri から operation identity op-1 を指定して通常 send を行う
THEN Accepted は operation identity、session identity、opaque input_ref、StartedTurn dispositionを持つimmutable receiptとlatest statusを返す
AND session s-1 には human message と turn が各1件だけ現れ、provider start は最大1回である

## B-002: 応答喪失後の通常 send 再試行

GIVEN operation identity op-1 の通常 send が受理された直後に response を失っている
WHEN 同じ principal が同じ operation identity と同じ payload で再試行する
THEN 初回と同じ session、receipt、disposition、turn または queue item、latest status が返る
AND human message、assistant message、turn、queue item、provider effect は増えない

## B-003: Restart 後の通常 send 再試行

GIVEN operation identity op-1 の通常 send が受理された直後に process が終了している
WHEN process を再起動して同じ operation identity と同じ payload で再試行する
THEN 再起動前と同じ Accepted receipt と latest status が返る
AND human message、turn、queue item、provider start は各最大1件である

## B-004: Tauri と WebSocket の並行再試行

GIVEN Tauri と認証済み WebSocket が同じ installation authority に接続している
WHEN 両 surface から同じ operation identity と同じ payload を同時に送る
THEN 両 surface は同じ Accepted receipt、disposition、latest status へ収束する
AND human message、turn、queue item、provider start は各最大1件である

## B-005: 新規 session 作成を伴う通常 send の再試行

GIVEN 新規 session を対象にした operation identity op-new と入力 hello がある
WHEN 初回 response 喪失後に同じ operation identity と payload で再試行する
THEN 同じ session identity と同じ Accepted receipt が返る
AND session、human message、turn、provider start は各最大1件である

## B-006: Active turn 中の queued send

GIVEN session s-1 に active turn があり、operation identity op-q の通常 send を受理できる
WHEN op-q を送信し、response 喪失後に restart して同じ payload で再試行する
THEN 受理時から変わらない Queued disposition、同じ queue item、latest status が返る
AND provider start はその queue item が実行可能になるまで0件で、実行後も最大1件である

## B-007: 通常 send の保存結果不明

GIVEN operation identity op-1 の最初の通常 send で保存結果を確認できない
WHEN send result を受け取り、続けて operation identity op-1 を照会する
THEN send result と writer 未解決中の照会は同じ operation identity の OutcomeUnknown を返し、解決後はcanonical record不在のNotFoundまたは同じreceipt / statusを持つAcceptedのどちらかだけへ収束する
AND NotFound後は同じpayloadで再試行でき、別 operation identity の自動 send と重複 provider effect は発生しない

## B-008: Accepted 後の実行結果不明と恒久 failure

GIVEN operation identity op-1 には immutable Accepted receipt がある
WHEN provider effect の結果不明または恒久 failure を発生させて operation を再取得する
THEN top-level Accepted と元の receipt は維持され、latest status だけが ReconciliationRequired または Failed へ変わる
AND composer に入力は復元されず、別 operation の自動 send は発生しない

## B-009: Operation identity の入力制約

GIVEN 独立したIdle sessionごとに1 byteと128 bytesの許可文字だけからなるoperation identity、および空、129 bytes、non-ASCII、または`[A-Za-z0-9._:-]`以外のASCIIを含むoperation identityがある
WHEN TauriとWebSocketの各surfaceから各identityを指定して通常sendを要求する
THEN 1 byteと128 bytesのidentityはAcceptedとなり、不正なidentityは両surfaceでInvalidRequestとなる
AND 不正なidentityではsession、message、turn、queue item、provider effectは変更されず、各valid identityのoperationは1件だけである

## B-010: Operation payload conflict

GIVEN operation identity op-1 が content、image、mention、editor context、session target、worktree、実行 configuration を含む payload P に束縛されている
WHEN P の各項目を一つだけ変更した payload P2 と同じ operation identity で再試行する
THEN Tauri と WebSocket は同じ PayloadConflict と operation identity を返す
AND 元の receipt、status、session、message、turn、queue item、provider state は変わらない

## B-011: 受理後 state 変化を conflict にしない

GIVEN operation identity op-1 の payload が受理され、その後 session state と server が決めた disposition が変化している
WHEN 同じ operation identity と受理時と同じ caller payload で再試行する
THEN PayloadConflict ではなく同じ immutable receipt と現在の latest status が返る
AND message、turn、queue item、provider effect は増えない

## B-012: Composer の Accepted clear 境界

GIVEN composer に hello があり、送信待機中に追加入力 world を行う
WHEN hello に対応する immutable Accepted receipt を受け取る
THEN hello の snapshot だけが clear され、world は composer に残る
AND Accepted 後の status 更新または通知 failure で hello は復元されない

## B-013: Composer の未受理保持

GIVEN composer に hello があり、通常 send を要求する
WHEN RejectedBeforeCommit または top-level OutcomeUnknown を受け取る
THEN hello の snapshot は composer に残る
AND frontend は別 operation identity で自動再送しない

## B-014: 通常 send の受理前 failure

GIVEN operation identity op-1 の通常 send で受理情報を保存できない
WHEN send を要求する
THEN RejectedBeforeCommit と safe failure が返る
AND message、turn、queue item、provider I/O は0件である

## B-015: Permission exact payload の restart 回復

GIVEN answers、updated input、deny message を含む permission response が受理済みで provider 未送信である
WHEN process を再起動して pending recovery を取得し、同じ effect を安全に再開する
THEN 元と同じ permission response が最大1回だけ provider へ送られ、同じ identity の結果が表示される
AND redacted summary から別 payload は生成されない

## B-016: Permission exact payload の欠損

GIVEN 未完了 permission response の exact payload、size、または owner access を確認できない
WHEN restart 後に pending recovery を取得する
THEN 同じ permission identity は Failed と safe actions を表示する
AND provider response は0件である

## B-017: Provider establish と send の依存順

GIVEN provider session create または resume が必要な Accepted send がある
WHEN create または resume を未解決、failure、success の順に変化させる
THEN success 確認前は receipt dispositionを維持して status を AwaitingProviderStart と表示し、確認後だけ provider start へ進む
AND 未解決または failure の間は turn と queued input の provider start は0件である

## B-018: Readback できる外部作用直後の crash

GIVEN stable effect identity を持つ provider 操作が外部で完了している
WHEN result 保存前に process を終了し、restart 後に回復する
THEN authoritative readback により同じ operation identity が完全な結果へ確定する
AND provider effect は再実行されない

## B-019: Readback できない外部作用直後の crash

GIVEN provider effect は開始されたが結果を一意に読み戻せない
WHEN result 保存前に process を終了し、restart 後に pending recovery を取得する
THEN 同じ operation identity、既知 observation、ReconciliationRequired、利用可能な safe actions が表示される
AND provider effect は自動再実行されない

## B-020: Streaming part の保存 failure

GIVEN active turn が provider から一つの streaming part を受信する
WHEN その part の保存を失敗させて live と reload を読む
THEN 両方に未保存 part は表示されない
AND 次の完全に保存済み結果との間に部分 message は現れない

## B-021: Invalid effect intent

GIVEN retry または readback capability と stable effect identity の組合せが不正な未完了操作がある
WHEN pending recovery の実行を要求する
THEN InvalidEffectIntent と同じ未完了 identity が返る
AND provider、workflow、OS effect と未完了一覧 entry は増えない

## B-022: Terminal 確定中 crash の原子性

GIVEN active turn に final parts、assistant message、terminal result、session state、permission、queue state の変更が必要である
WHEN 保存処理の各境界で process を終了して live と reload を読む
THEN 変更前または全項目が揃った変更後だけが表示される
AND 部分 terminal、未保存 success、通常 Idle を装う未完了状態は表示されない

## B-023: Terminal 確定後の通知 failure

GIVEN turn の完全な terminal result が保存済みである
WHEN UI 通知または配信を失敗させて再取得する
THEN 同じ final parts、assistant message、terminal reason、session state、permission、queue state が返る
AND terminal は取り消されず重複もしない

## B-024: Normal completion 後の queue 継続

GIVEN pause されていない queue と active turn がある
WHEN turn が正常完了する
THEN terminal の全項目と整合した queue state が表示され、既存の queue 進行条件が維持される
AND terminal result は1件だけである

## B-025: Stop または close terminal の queue pause

GIVEN queue を持つ active session がある
WHEN Accepted Stop、normal session close、open-session archive、または graceful quit が terminal を確定する
THEN terminal の全項目と同時に queue は内容を保持した Paused と表示される
AND 既存 pause は解除されない

## B-026: 競合する terminal

GIVEN 同じ started turn に Stop、watchdog、session close、Fatal、provider completion が競合する
WHEN すべての結果を任意順で到着させる
THEN 最初に確定した terminal reason と parts だけが live と reload に表示される
AND terminal、assistant message、Stop resolution はそれぞれ最大1件である

## B-027: 過去 turn の遅延 event

GIVEN turn t-1 が terminal となり後続 turn t-2 が開始済みである
WHEN t-1 の streaming または terminal event を遅延到着させる
THEN t-1 の確定済み結果と t-2 の parts、state、terminal は変わらない
AND 新しい message または terminal は増えない

## B-028: Stop の10秒 deadline

GIVEN storage は利用可能で、Accepted Stop の provider interrupt と session 処理が永久停止する
WHEN Accepted 時刻から10秒まで live 表示と reload 結果を観測する
THEN 10秒以内に両方が Interrupted(Timeout) へ確定する
AND terminal result は1件だけである

## B-029: Stop 後の stale result

GIVEN Accepted Stop が Interrupted(Timeout) へ確定している
WHEN 元の provider interrupt または completion を遅延到着させる
THEN terminal reason、parts、session state、queue state は変わらない
AND assistant message と terminal は増えない

## B-030: Stop request identity の payload conflict

GIVEN Stop request identity stop-1 が exact payload `session s-1 / 未解決turn t-1 / expected session revision 1` とbackend Stop identityへ束縛されている
WHEN 別request identity stop-2で同じtargetと別expected revisionを提示した後、stop-1を別session、別turn、または別expected revisionへ再利用する
THEN stop-2は既存のbackend Stop identityとresultへ合流して初回revision guardを変更せず、stop-1の3 fieldいずれかの変更はPayloadConflictを返す
AND 応答喪失、restart、同時join後も同じbackend Stop identityとresultを返し、合流または競合による追加のprovider interrupt、terminal変更は0件である

## B-031: Stop capacity の境界

GIVEN storage は利用可能で、異なる32 target の Accepted Stop が未解決である
WHEN 33件目の別 target に Stop を要求する
THEN 先の32件は10秒保証を維持し、33件目は StopCapacityExceeded を返す
AND 33件目の provider interrupt と terminal は0件である

## B-032: Stop 受理情報の保存 failure

GIVEN active turn に Stop を要求できる
WHEN target turn と queue pause をrestart後も識別する情報の保存を失敗させる
THEN Stop は RejectedBeforeAcceptance と safe failure を返す
AND provider interrupt、terminal、queue 変更は0件である

## B-033: Accepted Stop 後の terminal 保存 failure

GIVEN target turn と queue pause を識別できる Accepted Stop がある
WHEN 10秒まで terminal 保存を失敗させる
THEN session は同じ target turn と Stop identity の ReconciliationRequired を表示する
AND 通常 Idle と queue drain へ進まず Stop capacity を保持する

## B-034: Stop recovery の一回性

GIVEN Accepted Stop が terminal 保存 failure により ReconciliationRequired である
WHEN storage 復旧後に restart recovery と manual retry を競合させる
THEN terminal と Stop resolution は各1件だけ確定する
AND 完全解決後は解放された capacity を別 target の Stop が利用できる

## B-035: Startup recovery discovery

GIVEN send、queued execution、permission、provider create / resume、streaming / terminal、session close、backend recovery、workflow shutdown、publication の未完了操作が各1件ある
WHEN 個別 session を開かず current pending recovery の最初の page を取得する
THEN 全 category が元 identity、ownerまたはpartition、既知 status、available actions とともに列挙される
AND 通常 Recovering、Idle、完了へ推測変換されない

## B-036: Recovery crash boundary

GIVEN 未完了 recovery が1件ある
WHEN recovery 開始直後、external effect直後、completion直後、message publication直前の各時点で process を終了して再起動する
THEN 同じ recovery identity が変更前または完全な変更後として再表示される
AND external effect と公開 message は各最大1件である

## B-037: Recovery owner partition

GIVEN normal、workflow-owned、closed、archived、owner不明の未完了 recovery がある
WHEN owner filter と ClosedSession、ArchivedSession、UnownedRuntime partition を指定して取得する
THEN normal は session list、workflow-owned は owning run / node、closed は closed history、各partitionは該当 entryだけを返す
AND closed session は自動 reopen または provider resume されない

## B-038: Recovery page snapshot と cursor

GIVEN current pending recovery が201件あり、最初のpage取得後に一覧が更新される
WHEN 同じcursorで末尾まで読み、別filterへの再利用、改変、restart後の再利用を行う
THEN valid cursorは最初に固定したsnapshotだけを200件かつencoded 4 MiB以下で返す
AND 別filterまたは改変はCursorMismatch、restartまたは失効はCursorExpiredを返しpartial pageを返さない

## B-039: 未解決 shutdown による new quit の拒否

GIVEN same-boot または previous-boot の未解決 shutdown がある
WHEN new quit を要求する
THEN PreviousShutdownReconciliationRequired と既存 identity、available actions が返る
AND 新しい shutdown identity、terminal、workflow shutdown、child termination、OS effect は0件である

## B-040: Recovery 中の mutation 抑止

GIVEN sessionまたはworkflowに未解決 recovery がある
WHEN new turn、queue drain、workflow resume を要求する
THEN 対象の未解決 identity と safe failure が表示される
AND provider start、queue drain、workflow resume effect は0件である

## B-041: Meta を読めない failure feedback

GIVEN request が session s-1 を明示し、session data と meta の双方を読めない
WHEN Tauri または WebSocket からその操作を行う
THEN s-1 の feedback query は operation kind、available actions、canonical `failure: SafeOperationFailure`を返し、`failure.correlation_id`は同じfailureの表示とlogを結ぶ一意なidentityである
AND kind、retryable、label、detailをfeedback直下へ複製せず、filesystem path、secret、raw SQL、provider payload、raw error は返さない

## B-042: Failure feedback の paging と独立性

GIVEN session s-1 に異なる33件の未解決 feedback がある
WHEN pageを最後まで取得し、別sessionの成功と別failureの追加を行う
THEN 各pageは32件以下で、33件すべてを重複なく返す
AND 既存 feedback は別sessionの成功、別operationの成功、古い成功、別failure追加では消えない

## B-043: Failure identity による clear

GIVEN session s-1 に未解決 feedback f-1 と f-2 がある
WHEN f-1 をdismissするかf-1を解決した成功を確定する
THEN f-1だけが消え、f-2は残る
AND 未解決件数は1件だけ減る

## B-044: Failure feedback capacity

GIVEN process全体に512件の未解決 feedback がある
WHEN 新しい failure を追加し得る readまたはmutationと、既存feedbackの取得・dismiss・resolution retryを行う
THEN 新しい操作だけが FeedbackCapacityExceeded となり、既存3操作は利用できる
AND 新しい external effect、feedback、既存identityの削除は0件である

## B-045: Feedback revision conflict

GIVEN feedback f-1 のcurrent revisionが2である
WHEN expected revision 1でdismissまたはresolution retryする
THEN RevisionConflictとcurrent revision 2が返る
AND entry、未解決件数、capacity slot、external effectは変わらない

## B-046: Feedback 表示上限

GIVEN labelが160 UTF-8 bytesを超え、detailが2048 bytesを超えるsafe failureがある
WHEN feedbackを取得する
THEN labelとdetailは各上限以下でtruncation markerを含む
AND path、secret、raw SQL、provider payload、unbounded raw errorは含まれない

## B-047: Production runtime event golden

GIVEN ClaudeとCodexの代表wire fixtureと期待するpublic event、live / reload read model、terminal resultがある
WHEN fixtureをpublic production session interfaceへ入力する
THEN expected public event、live / reload read model、terminal resultと一致する
AND fixture replayはlive runtime eventと同じpublic production session interfaceを通る

## B-048: Wire互換とprojection互換の独立検出

GIVEN B-047のgolden suiteが成功している
WHEN provider入力からpublic eventへの変換だけ、またはpublic eventからsession resultへの変換だけを別々に破壊する
THEN 前者ではwire互換testだけ、後者ではprojection互換testだけが対応して失敗する
AND 既存F1 goldenも引き続き成功する

## B-049: Hermetic F1b

GIVEN networkと実provider processを利用できないCI環境がある
WHEN F1b golden suiteを実行する
THEN ClaudeとCodexの全fixtureが既存CI内で完了する
AND CLI、network、実provider processは起動されない

## B-050: 恒久SQLite transactionのatomicity

GIVEN agent session event、workflow event、operation binding、obligation、terminal、queue pause、shutdown planを同時に変更するbatchと、各streamのexpected head、idempotency keyがある
WHEN bundled SQLite storeへbatchをcommitし、transaction開始前、各participant write後、commit応答前後でstorage errorまたはprocess crashを発生させ、同じidentityで再試行する
THEN public queryは変更前または全participant確定後だけを返し、成功したbatchには連続したglobal sequenceとstream sequenceが一度だけ割り当てられ、同じidempotency key / payloadの再試行は同じcommit resultへ戻る
AND expected head不一致はtyped conflict、queue admission前の上限超過はtyped capacity failure、commit結果確認不能は同じtransaction identityのOutcomeUnknownとなり、部分event、partial projection、legacy dual write、全履歴scanは発生しない

## B-051: Close / quit decision table

GIVEN close / quit正本がある
WHEN chat tab、panel、normal session close、open / closed archive、backend switch、workflow node tab、workspace、window close、Cmd-Q、menu、Dock、tray、OS logout / shutdown、programmatic exit / restart、hard killの各行を読む
THEN 各行にscope、admission、active turn、parts、permission、queue、runtime、deadline、persist failure、UI result、testが記載される
AND 行間で同じsurfaceに矛盾する意味論がない

## B-052: View close の意味論

GIVEN active turnとqueueを持つsessionのchat tab、panel、workflow node tab、workspace view、windowが開いている
WHEN 各viewだけを閉じる
THEN 対象viewだけが閉じ、session、turn、parts、permission、queue、runtimeは変わらない
AND terminal、provider interrupt、session closeは0件である

## B-053: Active normal session close と open archive

GIVEN active turnとqueueを持つopen normal sessionがある
WHEN session closeまたはopen-session archiveを要求する
THEN SessionClosed terminalとClosedまたはArchived stateが整合して表示され、queueは内容を保持したPausedになる
AND terminalとruntime closeは各最大1件である

## B-054: Idle close と archive

GIVEN queueを持つIdle open sessionと既にClosedのsessionがある
WHEN open sessionをcloseまたはarchiveし、Closed sessionをarchiveする
THEN open sessionはqueueを保持したPausedのClosedまたはArchivedとなり、Closed sessionはqueueを変えずArchivedになる
AND synthetic terminalは追加されない

## B-055: Backend switch

GIVEN sessionがIdleでpending permission、recovery、provider処理を持たない
WHEN backend switchを要求する
THEN queueを保持したPausedと新しいbackendが表示される
AND active turnまたはいずれかのpending作業があるcaseは受理されずbackendとqueueを変更しない

## B-056: Close系commandの10秒結果

GIVEN activeまたはIdle normal session close、activeまたはIdle open archive、closed archive、Idle backend switchの各対象でruntime処理が永久停止する
WHEN requestから10秒まで結果を観測する
THEN 各commandは完了または結果未確認を示すtyped resultへ10秒以内に確定する
AND 遅延結果はterminalと外部作用を重複させずsessionをreopenせず別backendを開始しない

## B-057: Graceful quit surface のsingle flight

GIVEN Cmd-Q、application menu、Dock、tray、native exit、cooperative OS logout / shutdownから同時quitできる
WHEN 各surfaceから同じintentでquitを要求する
THEN 全surfaceのcaller bindingは同じbackend発行opaque ApplicationQuitOperationId、最初に固定したExitまたはRestartとexit code、同じ進行resultへ合流する
AND shutdown plan、target terminal、workflow shutdown、child termination、process exitは各最大1件である

## B-058: Quit request identity のpayload conflict

GIVEN quit request identity quit-1がExitかつcode 0へ束縛されている
WHEN quit-1をRestartまたは別exit codeへ再利用する
THEN PayloadConflictが返る
AND Tauri / WebSocketの両方で最初のoperationとintentが変わらず、新しいshutdown identity、admission変更、shutdown effectは0件である

## B-059: Shutdown admission

GIVEN 最初のquitより前にAcceptedとなった通常操作と、quit後に要求するagent、workflow、local API、Tauri mutationがある
WHEN quitを受理してshutdown中に各結果を取得する
THEN quit前のAccepted操作は完了またはrestart後も取得できる未完了結果へ進み、quit後のmutationはShutdownInProgressを返す
AND quit後に新しいagent、workflow、local API mutationは受理されない

## B-060: Shutdown target 上限

GIVEN openなactive / Idle sessionとrunning workflowを合計4096件または4097件持ち、closed / archived recoveryとowner不明runtimeもある
WHEN quitを要求する
THEN 4096件は一つのplanとして受理され、4097件はCapacityExceededでeffect 0件のabortとなる
AND closed / archived recoveryとowner不明runtimeはtarget数に含まれずexit summaryに残る

## B-061: Previous shutdown とcompaction gate

GIVEN 未解決previous shutdown、cleanup待ちのterminal shutdown、cleanup進行中のterminal shutdown、detailがCompactedのterminal shutdownがある
WHEN new quitを要求する
THEN 未解決caseはPreviousShutdownReconciliationRequired、cleanupが別planを保持中のcaseはPreviousShutdownCompactionPending、cleanup待ちまたはCompactedで受入条件を満たすcaseは一つの新しいflightを返す
AND authority不整合を含む拒否caseでは新しいplan、target、effectは0件であり、restartや同時要求後も同じ判定へ収束する

## B-062: Shutdown projection と plan page の公開境界

GIVEN same-bootのPreparing / Prepared / Activated / Quiescing / Completed / Failed / Cancelled / ReconciliationRequired plan、details Available / Compactedのterminal plan、4096 target、limit 1 / 128 / 129、encoded 1 MiB以下 / 超過のtargetとpage、unknown plan、valid / 改変 / 失効cursorがある
WHEN TauriとWebSocketからcurrent shutdownとexact plan pageを取得する
THEN same-boot currentは同じplan identityとexact phaseを返し、Available planは最大128件かつ1 MiB以下の同じsnapshot pageを返し、Compacted planは同じidentity、intent、terminal phase、counts、deadline、failureを維持してentries空、next cursorなしを返す
AND unknown planはNotFound、limit 129はInvalidRequest、改変 / 失効cursorはCursorMismatch / CursorExpired、encoded超過はResponseTooLargeとなり、partial target / pageを返さずstateとexternal effectを変更しない

## B-063: Shutdown activation 前 failure のabort

GIVEN quitの全targetをrestart後も識別できる状態へ準備中である
WHEN 一targetの保存を失敗させ、shutdown effect未開始を確認できる
THEN 15秒以内に`AbortedBeforeActivation(details=Available)` summaryとsafe failureが返り、成功済みtarget detailと固定済みrecovery snapshot detailを同じplanから取得でき、通常mutation受付が再開する
AND summaryのtarget countは取得可能な成功済みtarget detail件数と一致し、session close、workflow shutdown、child termination、provider effectは0件である

## B-064: Shutdown activation 後のbounded exit

GIVEN 全target準備後にshutdownが開始され、一部targetが永久停止する
WHEN 最初のquitから15秒まで観測してprocessを再起動する
THEN processはabortせず15秒以内にExitedWithRecoveryでexitし、restart後に同じidentityの未完了targetが表示される
AND 完了済みtargetのterminal、workflow shutdown、child terminationは再実行されない

## B-065: Durable plan activation の結果不明

GIVEN plan identityとPrepared resultはdurableに取得できるが、activation writerの結果を確認できないshutdownがある
WHEN quitの15秒deadlineへ到達し、current shutdownとknown quit operationを取得する
THEN current shutdownは同じplan identityのReconciliationRequiredを返し、known quit operationは元のAccepted operationと同じplanを返す
AND 未開始と推測してabortまたは別shutdown commandを開始せず、明示shutdown command 0件のままExitedWithRecoveryとしてexitし、restart後も同じplan identityを返す

## B-066: Process exit に伴う暗黙作用

GIVEN activated shutdownに未開始targetと既存childが残っている
WHEN process exitによりpipe close、job object、parent-death signalが起こり得る状態でrestartする
THEN shutdown planまたはpending recovery queryはtarget public stateをReconciliationRequiredとして返し、同じplan / epoch / effect identityの`SafeEffectObservation::ExitCoupledOutcomeUnknown`を併記する
AND ExitCoupledOutcomeUnknownをpublic state variantにせず、未開始または成功へ推測せず、同じ外部作用を自動再実行しない

## B-067: Shutdown 遅延結果のfence

GIVEN abort済みまたは新しいflightが開始済みのshutdownに旧flightの遅延結果がある
WHEN 遅延結果を到着させる
THEN 新しいsession、workflow、shutdown result、admission stateは変わらない
AND terminal、workflow shutdown、child termination、process exitは増えない

## B-068: 履歴件数に依存しないbounded operation

GIVEN 未完了件数を固定し、無関係なsessionまたはevent履歴が10件と1000000件のfixtureがある
WHEN startup recovery first page、同じturnのterminal、同じpayloadのmutation、identity queryを各1000 sample実行する
THEN identity、result、page件数、byte上限、effect件数が一致し、大規模fixtureのp95は小規模の1.25倍以下である
AND pending recovery first 200件はp95 50 ms以下、identity queryはp95 20 msかつp99 50 ms以下である

## B-069: Shutdown snapshot query のbounded failure

GIVEN shutdown snapshot取得中に同時commitが競合するcaseと、公開2秒上限を超えるcaseがある
WHEN 各caseでsnapshot queryを行う
THEN 前者はQueryBusy、後者はDeadlineExceededを返す
AND partial count、entry、pageを返さずstateとexternal effectを変更しない

## B-070: 既存dataの互換読込

GIVEN 変更前のopen / closed / archived session、event、terminal、permission、queue-linked inputと未完了作業がある
WHEN 明示migration操作なしで自動migrationを完了し、SQLite authorityからliveとreloadを読む
THEN 確定済み結果は変更前と一致し、未完了作業は元identityと既知observationを保ったPaused、Failed、またはReconciliationRequiredとして表示される
AND 証明できないprovider effectは自動開始されず、legacy dataは自動書換え、dual write、record単位fallbackの対象にならない

## B-071: Upgrade 中断と再開

GIVEN legacy inventoryからstaging SQLiteへのmigration途中dataと、migration中に受理済みのquit operationがある
WHEN source batch commit、parity verification、authority pointer切替の各中断点でprocessを終了して再起動し、同じcaller identityとoperation IDから進捗とquit resultを取得する
THEN 完了確認済みのdataを重複させず同じmigration identityとcheckpointから再開し、既存message、turn、queue、terminal、permissionを重複させず、quit queryはnormal shutdown planへfallbackせず同じMigration projectionを返す
AND 完了前のmutationはMigrationInProgressとなり、quitはagentまたはworkflow終了commandを開始せず15秒以内にprocessを終了し、次bootで同じflightをExitedへ確定する。cutover後のauthorityはSQLiteだけでありlegacyへ戻らない

## B-072: Tauri とWebSocketのsurface一致

GIVEN 同じoperation、session、pending recovery、shutdown、feedback stateがある
WHEN Tauriと認証済みWebSocketから対応するsend、identity query、recovery page / action、shutdown query / quit、feedback query / controlを行う
THEN 両surfaceのresult、receipt、status、failure、action、page内容はsemanticに一致する
AND transport固有の判断による追加stateまたはeffectは発生しない

## B-073: WebSocket認証とresource上限

GIVEN loopback local APIに16接続、1接続32 in-flight、rate 60 requests/s burst 120、request / response 16 MiB、outbound 32 responses / 16 MiBの各境界fixtureがある
WHEN Bearerなし、17本目、各上限内、各上限超過を送る
THEN BearerなしはHTTP 401、17本目はHTTP 503 CapacityExceeded、in-flight超過はCapacityExceeded、rate超過はRateLimited、response超過はResponseTooLarge、frame超過は1009、outbound超過は1013となる
AND 上限内requestはconnectionを維持して処理される

## B-074: WebSocket request identity と切断

GIVEN 同じconnectionで同じrequest IDの2 requestと、受理済みoperationがある
WHEN 2 requestを並行送信してconnectionを切断し再接続する
THEN 片方だけがoperationとして受理され、他方はRequestIdConflictを返し、再接続後はoperation identityで同じreceiptとstatusを取得できる
AND connection切断はoperation結果を変更しない

## B-075: 公開整数fieldのlossless境界

GIVEN 0、1、9223372036854775807、負数、先頭ゼロ、正符号、指数表記、空白、9223372036854775808、JSON numberを各semantic integer fieldへ入力する
WHEN TauriとWebSocketでrequest / responseを往復する
THEN 定義上0を許すfieldは0を、1始まりfieldは1以上をcanonical decimal stringでlosslessに返し、不正表現はInvalidRequestとなる
AND 最大値から次値を必要とするmutationはCapacityExceededとなりstateとeffectを変更しない

## B-076: Current application shutdown のboot境界とerror境界

GIVEN shutdownなし、same-bootの各phase、previous-boot nonterminal、previous-boot terminal、plan identityをanchorする最初のwriterの結果不明、冗長authorityだけのsemantic mismatch、storage / decode / integrity / required-reference failure、複数または一意にanchorできないidentityの各fixtureがある
WHEN TauriとWebSocketからcurrent application shutdownを取得し、previous-boot terminalだけはexact history queryも行う
THEN shutdownなしはCurrent(None)、same-bootは同じidentityのexact phase、previous-boot nonterminalは同じidentityのReconciliationRequired、previous-boot terminalはCurrent(None)かつhistory queryで同じterminal plan、最初のwriter結果不明はOutcomeUnknownを返す
AND 冗長authorityだけのmismatchはShutdownAuthorityMismatch付きReconciliationRequired、storage / decode / integrity / required-reference / identity一意性failureはInternalとなり、いずれもCurrent(None)、別plan、migration-safe quitへfallbackせずstateとexternal effectを変更しない

## B-077: Phase 0完了済み契約の非退行

GIVEN 次のtrace matrixに記載したD1 design contractとF1 / L1 / L2 / L4 / L6 / L7 / L8 / L10 / S10a / P2 / X1の最小fixtureがある
WHEN #1499適用後に各checkまたはtestを同じ入力で実行する
THEN 各rowのexpected resultが維持される
AND #1499のoperation、terminal、recovery、shutdown stateは既存message、terminal、queue item、notice、external effectを重複させない

| Gate | 正本 path / exact anchor | Check / test name | 最小入力 | Expected result |
| --- | --- | --- | --- | --- |
| D1 #1445 | `docs/specs/issues-1445/behavior.md` — `Feature: Agent 実行設定の新 domain 確定（configuration / Goal / Reasoning effort / launch / permission）`; design-only根拠は`docs/specs/issues-1445/design.md`冒頭 | 新規doc contract check `issue_1499_d1_contract_is_not_redefined`。既存runtime testはD1の明示Non-goal | #1499の公開型・decision tableをD1のconfiguration / Goal / capability境界と照合する | #1499がAgentMode、Goal authority、reasoning effort、provider capability、frontend action enablementを再定義しない |
| F1 #1383 | `docs/specs/feat-issues-1383/behavior.md` — `Rule: replay golden（convert 層）は現状の変換出力を固定する`、`Rule: 統合 golden（read model 層）は projector までの現状挙動を固定する` | `src-tauri/src/infrastructure/agent_session/fixtures/mod.rs::{claude_fixtures_match_convert_golden,codex_fixtures_match_convert_golden}`、`src-tauri/src/test_support/agent_session_wire_replay.rs::{claude_fixture_matches_read_model_golden,codex_fixture_matches_read_model_golden}` | `fixtures/{claude,codex}/normal_turn/wire.jsonl` | convert outputとread modelが各`convert.golden` / `read_model.golden`に一致し、fixtureごとのterminalが1件 |
| L1 #1402 | `docs/specs/issues-1402/behavior.md` — `Rule: 停止操作はどの phase でも常に受理される`、`Rule: Codex の turn_id 未取得ウィンドウでの停止予約`、`Rule: 停止後に pending queue を自動実行しない` | `runtime/usecase.rs::production_interrupt_watchdog_finalizes_at_the_ten_second_boundary`、`codex/session.rs::read_loop_writes_the_reserved_interrupt_exactly_once_after_turn_started`、`runtime/usecase.rs::queue_pause_and_explicit_resume_survive_runtime_state_restart` | unresponsive backend、turn ID通知前Stop、pending queue 1件、fake clock 10秒、restart | Timeout terminalが10秒、reserved interrupt 1回、queueはPausedで明示resumeまで開始0件 |
| L2 #1403 | `docs/specs/issues-1403/behavior.md` — `Rule: 実行中 turn への送信は成功以外の結果を持たない`、`Rule: queue に積むメッセージは欠落なく永続化される`、`Rule: 入力欄は送信成功時にのみクリアされる` | `runtime/usecase.rs::test_stale_watchdog_無進捗turnをstall_signalに留めruntimeを閉じない`、`MessageInput.test.tsx::clears input only after sending succeeds`、`MessageInput.test.tsx::serializes submissions and preserves edited input and attachments added in flight` | steering非対応stall turnへ本文、image、mention、editor_contextをsendし、send pending中にdraftを追加 | exact payloadのqueue item 1件、raw steering errorなし、Accepted分だけclear、追加入力は残る |
| L4 #1405 | `docs/specs/issues-1405/behavior.md` — `Rule: 進行中 turn を持つ終了経路は turn を必ず終端させる`、`Rule: アプリ終了→再起動でも終端が保たれる` | `runtime/usecase.rs::close_session_finalizes_streaming_turn_and_persists_terminal_projection`、`close_session_without_active_turn_does_not_create_interruption`、`close_session_appends_terminal_batch_atomically_and_can_retry`、`ChatSessionView.test.ts::shows the durable SessionClosed interruption on the reopened agent turn` | streaming partsとpending permissionを持つactive session、およびIdle sessionをcloseしてreload | activeはparts保持＋SessionClosed terminal＋permission settlement、Idleはsynthetic terminal 0、reloadも同じ |
| L6 #1407 | `docs/specs/issues-1407/behavior.md` — `Rule: Codex の backend thread 消失後もセッションは恒久死しない`、`Rule: backend セッション再確立時にユーザーへ通知する` | `runtime/usecase.rs::test_codex_resume失敗はfresh_sessionで復活しdead_threadを再利用しない`、`completed_recovery_notice_is_restored_once_before_the_next_turn_after_restart` | dead Codex thread IDでresume failure、次send、notice publication前restart | fresh backend sessionでturn継続、dead ID再利用0、recovery noticeは同一内容で1回 |
| L7 #1408 | `docs/specs/issues-1408/behavior.md` — `Rule: 非 JSON 行はセッションを終了させず skip される`、`Rule: 1 行サイズ上限を超える行は保持されず読み捨てられる`、`Rule: 非 JSON 行・巨大行を混ぜた fixture でセッション継続が固定される` | `claude/session.rs::test_claude_read_loopは_mixed_stdout_fixture後も処理を継続する`、`codex/session.rs::test_codex_read_loopは_mixed_stdout_fixture後も処理を継続する` | `src-tauri/tests/fixtures/agent_session/mixed_stdout_{claude,codex}.jsonl` | non-JSON / oversizeをskip / dropし、その後のvalid eventを処理してsessionを継続 |
| L8 #1409 | `docs/specs/issues-1409/behavior.md` — `永続化失敗を無言に握りつぶさない`、`破損した event log を append 側で自己修復する`、`最終永続化失敗時に persist 済み本文を失わない` | `event_store.rs::append_session_event_recovers_unclosed_log_then_appends`、`runtime/usecase.rs::reopen_runtime_persist_failure_retries_reports_and_returns_error`、`final_parts_append_failure_keeps_body_not_tool_only` | closing `]`欠損event log、persist retry全失敗、Text＋tool partsのFinalParts失敗 | log修復後append成功、継続failureはerror＋session notice、reload後も本文保持 |
| L10 #1411 | `docs/specs/issues-1411/behavior.md` — `Rule: 解放済みの未参照 lock エントリは無期限に蓄積しない`、`Rule: 規約違反の lock 再入がテストビルドで検出される`、`Rule: 外部から観測可能な振る舞いは変わらない` | `runtime/usecase.rs::repeated_session_runtime_locks_do_not_accumulate_registry_entries`、`session_runtime_lock_reentry_is_detected_in_tests`、`sequential_session_runtime_lock_acquires_are_not_reentry` | 多数sessionのacquire / drop、同一flowの二重acquire、解放後の別session acquire | registryは保持中相当へ収束し、reentryだけを検出し、逐次通常操作は成功 |
| S10a #1398 | `docs/specs/feat-issues-1398/behavior.md` — `Feature: turn 実行中の crash が live で chat panel に着地する`、`Feature: Idle 中の Fatal がその理由付きで記録され live に着地する`、`Feature: live と reload 後の表示が一致する` | `runtime/usecase.rs::crash_emits_projected_error_snapshot_before_state_change_and_matches_reload`、`idle_fatal_is_durable_live_and_survives_later_projection`、`ChatSessionView.test.ts::renders the live or reloaded error part as an Error block` | streaming中Crash、Idle中Fatal、その後reload | live Error block、既存parts保持、Error reason durable、live / reload一致 |
| P2 #1414 | `docs/specs/feat-issues-1414/behavior.md` — `Rule: 表示スコープは発生元 session のパネルに限定される`、`Rule: 他 session の活動では banner が消えない`、`Rule: banner のクリア契機は 2 つに限定される` | `BoundSessionChat.test.tsx::shows each session error only in its source pane`、`keeps session A error visible when session B updates and dismisses only A`、`notice.rs::test_session_notice_update_matching_success_recovers_only_same_operation` | session A failure、session B update / success、A dismissまたはA同operation success | Aだけにbanner、B活動で消えず、Aの明示dismissまたはmatching successだけでclear |
| X1 #1417 | `docs/specs/issues-1417/behavior.md` — `Rule: prompt が非空、または画像がないときだけ text block を含める` | `claude/wire.rs::test_claude_user_message画像のみなら空text_blockを含めない`、`test_claude_user_message本文も画像もなければ空text_blockを含める` | empty prompt＋image 1件、empty prompt＋image 0件 | image-onlyはtext block 0、画像なしempty inputは既存互換のempty text block 1 |

## B-078: Stop winner の解決結果

GIVEN started turnにAccepted Stopがある
WHEN Stopがterminalを先に確定する
THEN terminal resultと同時にStop resolutionはTerminalかつSucceededとして表示される
AND final parts、assistant message、session state、permission、queue stateは同じ整合した結果を示す

## B-079: Stop superseded の解決結果

GIVEN started turnにAccepted Stopがありnormal completion、Fatal、またはcloseが競合する
WHEN Stop以外のterminalが先に確定する
THEN terminal resultと同時にStop resolutionはTerminalかつSupersededとして表示される
AND terminalとStop resolutionはrestartまたは再試行後も各1件である

## B-080: Terminal 保存 failure とStop capacity

GIVEN Accepted Stopを含むterminal closureで保存failureが起きる
WHEN live、reload、restart後のStop operationを取得する
THEN terminal各項目とStop resolutionは部分表示されず、同じ未解決identityが表示される
AND Stop capacityは保持され、完全解決後だけ解放される

## B-081: Pending recovery action のclosed kind

GIVEN current pending recoveryがReadAgain、RetrySameEffect、UseObservedResult、CancelIfSafe、KeepForManualResolutionの利用可能actionを提示する
WHEN 各actionを提示されたidentityとcurrent revisionで実行する
THEN ReadAgainはreadback、RetrySameEffectは同じeffect identity、UseObservedResultとCancelIfSafeは再検証可能な根拠だけを使い、KeepForManualResolutionはUnchangedを返す
AND CancelIfSafeを提示しないtargetへの直接要求はActionUnavailableでeffect 0件となる

## B-082: Recovery action のresponse喪失とrestart

GIVEN recovery action action-1が実行されresult保存後にresponseを失う
WHEN restart後に同じactionを再実行しidentityだけで取得する
THEN 保存済みと同じoutcome、classification、resource revision、canonical result hash、resource viewが返る
AND external effectは増えない

## B-083: Recovery action のinvalid identity とstale view

GIVEN 未発行、改変、current viewで利用不能、stale revision、handoff前にtarget revision変更となるactionがある
WHEN 各actionをTauriとWebSocketから実行する
THEN 順にNotFound、NotFound、ActionUnavailable、RevisionConflict、TargetRevisionChangedが返る
AND 公開stateとexternal effectは0件の変更である

## B-084: Recovery action classification の組合せ

GIVEN provider observationが開始のみ、成功、未開始確認、ambiguousの各caseと取消可能・不能targetがある
WHEN 提示されたrecovery actionを実行してresultを再取得する
THEN Pending+Pending、Pending+ConfirmedNoEffect、Pending+Ambiguous、Terminal+Succeeded、Terminal+CancelledBeforeEffect、Unchanged+Unchangedのいずれかだけが返る
AND 開始だけをSucceededへ、ambiguousをConfirmedNoEffectへ、取消不能targetをCancelledBeforeEffectへ読み替えない

## B-085: Shutdown target action とplan terminal

GIVEN shutdown planに複数の未解決targetと各targetのsafe actionがある
WHEN 各targetをsame action identityで解決する
THEN owner側resultとtarget resultが一つの結果として確定し、全target解決後はplanがterminalになる
AND 次のquitは未解決planを理由に拒否されない

## B-086: Recovery action の保存結果不明

GIVEN action-1の実行結果を保存したか確認できない
WHEN command resultを受け取り、action identityで再取得する
THEN command resultはActionOutcomeUnknownとaction-1を返し、再取得は同じattemptのInProgress、OutcomeUnknown、ReconciliationRequired、Completedのいずれかを返す
AND 別actionまたは別effectは作られない

## B-087: Stop と quit の request identity 境界

GIVEN 独立した受理可能fixtureごとに1 byteと128 bytesの許可文字だけからなるStop / quit request identity、および空、129 bytes、non-ASCII、または`[A-Za-z0-9._:-]`以外のASCIIを含む各request identityがある
WHEN TauriとWebSocketから各identityを指定してStopとquitを要求する
THEN 1 byteと128 bytesのidentityは各commandでAcceptedとなり、不正なidentityは両surfaceでInvalidRequestとなる
AND 不正なidentityではStop受理、provider interrupt、shutdown identity、admission変更、shutdown effectは0件である

## B-088: Known quit operation の読取境界

GIVEN Availableなlive normal shutdown、Compactedなlive normal shutdown、compact shell削除後のarchive-only normal shutdown、liveとarchiveが一致または不一致のnormal shutdown、migration-safe flight、未発行operation、known operation固有の読取authorityまたは参照先が欠損・decode不能・integrity不一致のcase、acceptance保存結果不明のcase、Accepted後の参照先transaction結果不明のcaseが各1件ある
WHEN operation identityを指定してTauriとWebSocketからquit operationを取得する
THEN 正常なlive / archive-only / live+archive一致は同じShutdown projection、migrationはMigration projection、未発行identityはNotFound、live+archive不一致を含むauthority破損はInternal、acceptance保存結果不明はtop-level OutcomeUnknown、Accepted後の参照先transaction結果不明はAccepted内のOutcomeUnknownを両surfaceで返す
AND compact shellの存否でterminal result bytesを変えず、authority破損、結果不明、MigrationをCurrent(None)、normal shutdown、別operationのprojectionへfallbackしない

## B-089: Plan固定 recovery snapshot の照会境界

GIVEN shutdown plan p-1 / epoch 7が固定したrecovery snapshot s-1にClosedSession 1件、ArchivedSession 0件、UnownedRuntime 1件がある
WHEN 同じplan / epoch / snapshotで3 partitionを取得し、別plan、別snapshot、別partitionのcursor、改変cursor、失効cursor、details compacted、unknown partition tagでも取得する
THEN validな3 partitionは固定snapshotだけを返し、ArchivedSessionはentries 0件のempty pageとなり、順にSnapshotMismatch、SnapshotMismatch、CursorMismatch、CursorMismatch、CursorExpired、DetailsCompacted、InvalidRequestを返す
AND error caseでpartial pageを返さず、emptyまたはerrorをcurrent recovery inventoryへfallbackしない

## B-090: Current recovery の shutdown plan association filter

GIVEN current pending recoveryにplan p-1 / epoch 7へ関連付く201件、p-1 / epoch 8へ1件、p-2 / epoch 7へ1件、shutdown associationなし1件がある
WHEN p-1 / epoch 7のshutdown plan association filterでcursorを使って末尾まで取得する
THEN p-1 / epoch 7へ関連付く201件だけを200件かつencoded 4 MiB以下のpageで重複なく返す
AND 別plan、別epoch、associationなしのentryを混ぜず、plan固定snapshotまたはfilterなしcurrent inventoryへfallbackしない

## B-091: 別request identityのquit intent join

GIVEN request quit-1のExit / code 0がcurrent shutdown flightとしてAcceptedである
WHEN 別のvalid request quit-2からRestart / code 42を要求する
THEN quit-2はquit-1と同じbackend発行operation identity、Exit / code 0、plan、deadline、進行resultへ合流する
AND PayloadConflict、新しいplan、Restart permit、追加shutdown effectは発生せず、quit-1のintentも変更されない

## B-092: RetryQuit の提示条件

GIVEN same-bootでactivation前Failed、shutdown effect 0件、durable terminal fence確定、mutation admission Open、store Healthyを同じsnapshotで満たすplanと、各条件を一つずつ満たさないplan、fresh bootのplan、OutcomeUnknownのplanがある
WHEN TauriとWebSocketからcurrent shutdown projectionを取得する
THEN 全条件を同時に満たすplanだけがavailable actionsにRetryQuitを含み、他のplanはRetryQuitを含まない
AND queryによってadmission、plan、terminal、shutdown effectは変更されない

## B-093: Completed recovery action の完全replay

GIVEN recovery action action-1がCompletedとなりoutcome、classification、resource revision、canonical result hash、safe resource viewが保存されている
WHEN 時間経過とrestart後にcurrent resource revisionを進め、shutdown detailsをcompactし、current resourceの通常queryをStorageUnavailableにした状態でaction-1をidentityだけから再取得する
THEN 各取得はCompleted時に保存したoutcome、classification、resource revision、canonical result hash、safe resource viewとexactly同じresultを返す
AND current resourceからresultを再構築せず、新しいactionまたはexternal effectを作らない

## B-094: Feedback resolution retry の再失敗

GIVEN process全体の未解決feedbackが512件あり、feedback f-1のrevisionが2でresolution retry actionが利用可能である
WHEN expected revision 2と提示済みaction identityでretryし、そのresolutionが再びfailureとなる
THEN f-1と同じfeedback identityが更新後failure、利用可能action、revision 3を持って返り、未解決件数は512件のままである
AND 新しいfeedback identityとcapacity slotを作らず、他のfeedbackを変更しない

## B-095: Session close のcrash境界

GIVEN active turnを持つnormal sessionとIdle normal sessionに対するclose要求がある
WHEN close acceptance保存前、保存確定直後、runtime close effect直後、result保存直前の各境界でprocessを終了して再起動する
THEN 保存前caseは元のOpen stateとruntime close effect 0件を返し、保存後caseは同じclose identityの未完了作業または完全なClosed resultとして回収される
AND active caseのSessionClosed terminal、Idle caseのsynthetic terminal 0件、queue pause、runtime close effect最大1件という結果はrestartとretry後も変わらない

## B-096: Shutdown details の Available から Compacted への切替

GIVEN Completed / Failed / Cancelledのterminal shutdown planがdetails Availableで、同じplanからtarget detailとplan固定recovery detailを取得できる
WHEN background compactionの任意時点でprocessを終了してrestartし、TauriとWebSocketから同じplan ID / epochを繰り返し取得する
THEN 各取得結果は完全なAvailableまたは完全なCompactedのどちらかであり、途中のNotFound、empty Available、current inventoryへのfallback、CompactedからAvailableへの逆行を返さない
AND Compacted後もplan identity、intent、terminal phase、target / completed / unresolved / preexisting recovery counts、cutoff、deadline、outcome、safe failureはAvailable時と一致し、pageはentries空、next cursorなし、exact target / recovery detail queryはDetailsCompactedを両surfaceで返す

## B-097: Previous shutdown compaction 中の new quit

GIVEN terminal previous shutdownのdetailsがまだAvailableで、そのdetailを保持したまま新しいfull-detail flightを公開できない状態にある
WHEN new quitを要求し、同時にold planをexact history queryする
THEN new quitはRejectedBeforeAcceptanceのPreviousShutdownCompactionPendingとold planのblocking shutdown projectionを返し、新しいshutdown operation、admission変更、external effectを0件にする
AND old planはAvailableまたはCompactedとして継続取得でき、Compactedへ確定した後のnew quitだけが通常の受理条件へ進み、old planは同じidentityのCompacted historyとして残る

## B-098: LegacyからSQLiteへのone-shot cutover

GIVEN 固定したlegacy source inventory、staging SQLite、migration checkpoint、authority pointerと、未解決operation / shutdown detailを含むfixtureがある
WHEN bounded import、projection parity、known-result parity、owner relation検証を行い、pointer切替の前後でfailureまたはrestartを発生させる
THEN parity未達または参照不整合ではauthorityを切り替えずMigrationBlockedを返し、成功時はpointerをLegacyからSqliteへ一度だけ切り替え、operation identity、terminal result、shutdown Available / Compacted result、counts、deadline、failureを同じpublic queryから再取得できる
AND cutover後のmutationと再起動はSQLiteだけを使い、legacyへのrollback、dual write、record単位fallback、managed backup / restoreのpublic commandを追加しない

## B-099: 通常send operationのprincipal分離

GIVEN principal p-1へ束縛されたoperation identity op-1のAccepted receiptがある
WHEN principal p-2がop-1を同じpayloadでsendまたはoperation identity queryする
THEN TauriとWebSocketはNotFoundを返し、receipt、status、session、message、turn、queue itemをp-2へ返さない
AND p-1のoperationとsessionは変わらず、p-2用operation、human message、turn、queue item、provider effectは0件である

## B-100: Quitの最初のplan writer結果不明

GIVEN quit operation identityとintentはcallerへ返せるが、plan identityをanchorする最初のwriterの結果を確認できない
WHEN request result、current shutdown、known quit operationを取得する
THEN request resultとknown operationは同じoperation identity / intentのtop-level OutcomeUnknownを返し、current shutdownもnormal shutdown不在へ読み替えずOutcomeUnknownを返す
AND frontendは別request identityでquitを自動再実行せず、新しいplan、admission変更、shutdown effectは0件である

## B-101: SessionLifecycle operationのreplayとprincipal分離

GIVEN principal p-1がrequest ID close-1、session s-1、expected revision 4、action CloseをAcceptedされ、responseを失っている
WHEN restart後にp-1が同じrequestとpayloadをreplayし、principal p-2が返されたbackend operation identityを照会する
THEN p-1は初回と同じoperation identity、immutable receipt、current stateを返され、p-2はNotFoundを返される
AND terminal、queue pause、runtime close effectは各最大1件である

## B-102: SessionLifecycle operationのconflictとjoin

GIVEN principal p-1のsession s-1に未解決のArchiveOpen operationがある
WHEN p-1が別request IDで同じsession、expected revision、ArchiveOpenを要求し、さらに別request IDでCloseを要求する
THEN 同じArchiveOpen requestは既存operation identityへ合流し、Close requestはPendingOperationを返す
AND 同じrequest IDを別session、revision、action、backend IDへ再利用した場合はPayloadConflictとなり、新しいoperation、terminal、queue変更、runtime effectは0件である

## B-103: SessionLifecycle operationの10秒結果とstable query

GIVEN active / Idle close、open / closed archive、Idle backend switchの各valid requestと、runtime処理が永久pendingになるcaseがある
WHEN command受理直後、10秒時点、restart後に同じoperation identityを取得する
THEN effect開始前にimmutable Accepted receiptを取得でき、10秒以内にCompletedまたはReconciliationRequiredへ進み、restart後も同じreceipt、state、outcomeを返す
AND 受理前のBusy、PendingOperation、RevisionConflict、InvalidState、Failedではsession、queue、terminal、backend、external effectを変更しない

## B-104: Canonical MessagePart と永続化互換

GIVEN 変更前の既知message part JSON、Claude / Codex F1 fixture、SQLite persistence envelope、Tauri / WebSocket public DTO fixtureがある
WHEN legacy JSONをdomainのcanonical MessagePartへdecodeし、SQLiteへ保存・reloadし、両public surfaceへpresentする
THEN 既知variantのtag、field、順序、optionalityと公開結果は変更前と一致し、session usecase、runtime event、projectionは同じdomain typeを使う
AND persistence DTOとpublic DTOはdomain typeから独立してversion管理され、unknown additive persistence payloadはraw bytesを保持し、unknown required variantはtyped incompatibilityとしてfail closedとなり、usecase layerに同義MessagePart enumを残さない

## 要件IDと検証方法の対応表

| Requirement ID | Behavior ID | Verification Method |
| --- | --- | --- |
| R-001 | B-001, B-002, B-003, B-004, B-005, B-006, B-007, B-008, B-009, B-099 | fake storage / providerでexisting・new-session・queued sendを実行し、response喪失、restart、Tauri / WebSocket並行送信、保存結果不明、Accepted後failure、valid / invalid identity、別principalからのsame operation send / queryを入力する。同一principalのreceipt / disposition不変、別principalのNotFound、message / turn / queue / provider effect増分0を公開queryとprovider記録で比較する |
| R-002 | B-010, B-011 | 受理済みoperationのcontent、image、mention、editor context、target、worktree、configurationを一項目ずつ変更しPayloadConflictとeffect増分0を観測する。受理後session stateだけを変更した同payload再試行では同じreceiptを観測する |
| R-003 | B-008, B-012, B-013 | component testでAccepted、RejectedBeforeCommit、OutcomeUnknown、Accepted後ReconciliationRequired、送信待機中追加入力を投入し、対応snapshotだけのclear、未受理保持、入力復元0、自動send 0をDOMとbackend call記録で観測する |
| R-004 | B-014, B-015, B-016, B-017, B-018, B-019, B-020, B-021, B-095 | fake providerとrestart harnessでsend・permission・create / resume・streaming・session closeに受理前failure、effect直後crash、readback可否、payload欠損、不正effect intentを投入し、受理前effect 0、依存前start 0、exact response最大1、未保存part非表示、same identityのstatus / actionsを観測する |
| R-005 | B-007, B-014, B-018, B-019, B-022, B-023, B-036, B-063, B-064, B-088, B-095 | send、terminal、recovery、session close、shutdownとknown quit operation読取の各保存境界でerror / process crashを発生させ、live / reload / Tauri / WebSocketが変更前、完全確定後、または規定されたOutcomeUnknownだけを返し、authority破損を通常状態へ隠さず別effectが増えないことを比較する |
| R-006 | B-022, B-023, B-024, B-025 | normal、Stop、close、archive、quitのterminalを保存failure / notification failure込みで実行し、parts、assistant message、terminal、session、permission、queueが同時に整合し、pause policyが一致することを観測する |
| R-007 | B-026, B-027 | 同一turnへStop、watchdog、close、Fatal、completionを全順序で競合させ、後続turn開始後に旧eventを投入し、winner reason / parts不変とterminal count 1をlive / reloadで照合する |
| R-008 | B-028, B-029, B-030, B-031, B-087 | fake clockでinterrupt / closeを永久pendingにし10秒terminalとstale result無効化を観測する。Stop payload各field、same-key conflict、別key same-target join、32 / 33件capacity、1 / 128 byte valid identity、0 / 129 byte・non-ASCII・許可文字外identityを入力し、公開Accepted / PayloadConflict / StopCapacityExceeded / InvalidRequest、terminal count、effect増分を比較する |
| R-009 | B-032, B-033, B-034, B-080 | Stop受理保存failure、Accepted後terminal failure、10秒到達、storage復旧、restart / manual競合を実行し、受理前interrupt 0、ReconciliationRequired、Idle / drain抑止、capacity保持、terminal / resolution各1を観測する |
| R-010 | B-035, B-036, B-037, B-038, B-039, B-040, B-090 | 全未完了categoryとowner partitionを個別session未読込で列挙し、201件paging、途中更新、cursor改変 / restart、shutdown plan / epoch association、各crash境界、未解決shutdown、mutation抑止を入力してidentity保持、filter純度、page上限、typed cursor error、effect / message最大1を観測する |
| R-011 | B-041, B-042, B-043, B-044, B-045, B-046, B-094 | data / meta双方を壊したsessionへfailureを発生させ、33件paging、identity別dismiss、512件capacity、stale revision、resolution retry再失敗、UTF-8 byte上限を入力し、embedded SafeOperationFailure、flat field重複0、failure / log correlation identity一致、exact error、同一identity更新、件数 / slot、effect境界を両surfaceで照合する |
| R-012 | B-047, B-048, B-049 | Claude / Codex wire fixtureをproduction public interfaceへ入力し、wire変換とprojection変換を別々にmutation testし、期待golden、独立failure、既存F1維持、network / process起動0をCI記録で観測する |
| R-013 | B-050, B-098 | SQLite fault harnessでmulti-stream batchの各write / commit境界、same-key replay、expected-head conflict、queue件数 / byte上限、signed 64-bit境界、unknown additive / required payload、legacy cutover前後を検査する。変更前または全participant確定後だけ、sequence一回性、OutcomeUnknownのsame-identity解決、MigrationBlocked、cutover後legacy write / fallback 0件を確認する |
| R-014 | B-051, B-052, B-053, B-054, B-055, B-056, B-095, B-101, B-102, B-103 | decision table全surfaceをschema検査し、active / Idle close、open / closed archive、backend switch、view closeへvalid / invalid request ID、same-key replay / conflict、別key join / PendingOperation、別principal query、response喪失、10秒hang、restart、close crashを入力する。同じoperation receipt / outcome、NotFound、terminal有無、queue pause、backend selection、effect最大1を公開command / queryで観測する |
| R-015 | B-039, B-057, B-058, B-059, B-060, B-061, B-062, B-087, B-088, B-091, B-092, B-097, B-100 | 全graceful surfaceからsame / different request identityとintentでquitし、response喪失、SQLite transaction結果不明、same / previous boot nonterminal、migration-safe flight、terminal detail保持中、4096 / 4097 target、page / byte境界を入力する。同じbackend operation、first intent不変、blocking projection、Current / OutcomeUnknown / exact error、effect各最大1を観測する |
| R-016 | B-063, B-064, B-065, B-066, B-067, B-089, B-096, B-097 | fake clockで準備failure、activation結果不明、activation後hang、exit-coupled child、old-flight遅延result、Available→Compacted切替の各公開境界へcrashを投入する。15秒以内のabortまたはexit、開始前effect 0、同一identityのrestart recovery、Compacted後のsame summary / empty entries / DetailsCompacted、fallback 0、重複effect 0を観測する |
| R-017 | B-068, B-069 | 10件 / 1000000件fixtureで各1000 sampleを同一release環境から測定してidentity / page / effect一致とp95 / p99上限を集計し、同時commitと2秒超過でQueryBusy / DeadlineExceeded、partial result 0を観測する |
| R-018 | B-038, B-062, B-070, B-071, B-072, B-073, B-074, B-075, B-076, B-088, B-089, B-098 | legacy fixtureでmigration各checkpoint / authority cutoverの中断・restart、migration-safe quit replay、Tauri / WebSocket parity、WebSocket認証 / resource上限、request ID競合 / reconnect、integer境界、same / previous boot shutdown、authority mismatch、storage / integrity failure、writer結果不明を入力する。重複0、MigrationInProgress、lossless decimal string、Current(None) / exact phase / ReconciliationRequired / Internal / OutcomeUnknownの非混同、cutover後legacy access 0を公開resultで観測する |
| R-019 | B-077 | B-077 trace matrixのexact check / testを記載inputで実行し、各rowのexpected resultとmessage / terminal / queue / notice / external effectの重複0を観測する |
| R-020 | B-026, B-034, B-078, B-079, B-080 | Stop winner / normal・Fatal・close winner、保存failure、restart / retryを組み合わせ、terminal全項目とSucceeded / Superseded resolutionの同時表示、各1件、capacity保持 / 解放を観測する |
| R-021 | B-019, B-021, B-039, B-081, B-082, B-083, B-084, B-085, B-086, B-093 | five action kind、response喪失、restart、時間経過、resource revision更新、details compaction、current resolver failure、未発行 / 改変 / stale / unavailable / target競合、closed classification pair、shutdown全target解決、writer結果不明を入力し、same-action exact replay、exact typed error、effect増分0、plan terminal、ActionOutcomeUnknownを両surfaceで照合する |
| R-022 | B-047, B-048, B-049, B-077, B-104 | compile-timeの単一定義check、legacy JSON / SQLite envelope round-trip、Claude / Codex F1 golden、Tauri / WebSocket presenter golden、unknown version fixtureを実行し、known public shape不変、domain型の単一所有、raw preservation、unknown required variantのfail-closed、usecase同義enum 0件を確認する |
