# Close / quit surface decision table

作成日: 2026-07-19
更新日: 2026-07-19

本書は milestone 84 における view close、Session lifecycle、backend configuration transition、application quit の surface 別意味論の正本である。語彙は [agent-chat-ideal-vocabulary.md](agent-chat-ideal-vocabulary.md)、不変条件は [agent-chat-ideal-lifecycle.md](agent-chat-ideal-lifecycle.md) を参照する。Issue #1499 の実装と integration test は本表に従う。

## 共通規則

- view close、Session close / archive、backend switch、application quit は別のtyped intentであり、相互に代用しない。
- lifecycle closureのscopeは個別`Session`、個別`Workflow`、全targetを束ねる`Application` shutdown planに分ける。別scopeのpartial successを一つのatomic commitと呼ばない。
- view closeは表示だけを閉じ、turn、permission、queue、runtime、workflow executionを変更しない。
- queue itemはclose / quit / backend switchで削除しない。Stop、Session close / archive、backend switch、graceful quit、Fatal、Crashはqueueをpauseまたは既存pauseのままにし、明示resumeまでdrainしない。
- normal Session close / open archiveはstable operation IDを持つ。runtime scopeがある場合だけ`SessionClose` obligationをwrite-aheadしてからruntime closeを開始し、結果不明は確定済みClosed / Archived projectionとqueue pauseを保ったReconciliationRequiredにする。runtime scopeがなければexternal effectやpending recoveryを作らず同じclosureで完了する。
- backend switchはIdleかつpending permission / recovery / external-effect obligationなしの場合だけ受理する。old runtimeがある場合はclose terminal後だけnew configurationをeffectiveにする。結果不明ではold effective configurationとqueue pauseを維持し、new backendを開始しない。
- graceful application quitはすべて同じRust-owned single-flight coordinatorを通る。hard kill / power lossはcoordinatorの入力ではなく、次回起動時のcrash recoveryが扱う。
- provider、runtime、workflowへのexternal I/Oはdurable reservationの確定後、かつinvocation直前のRust-owned dispatch guardが同じidentity / revision / claimを再検証した場合だけ開始する。
- #1499は新しいdurable operation / obligationをrestart後に回収する。#1499導入前から存在し、必要なidentityやeffect proofを一意に再構築できないdangling turnの完全なterminal化は[#1406](https://github.com/siro33950/releash/issues/1406)の境界であり、本書から通常終了を捏造しない。

## Deadline

| Operation | Absolute deadline | Deadline時の扱い |
| --- | --- | --- |
| normal Session close（active） | request ingressから10秒 | 完了、受理前rejection、または同じoperation identityのReconciliationRequired / OutcomeUnknownへ収束する。frontend timeoutから別operationを作らない |
| normal Session close（Idle） | request ingressから10秒 | 完了、受理前rejection、または同じoperation identityのReconciliationRequired / OutcomeUnknownへ収束する。frontend timeoutから別operationを作らない |
| open Session archive（active） | request ingressから10秒 | 完了、受理前rejection、または同じoperation identityのReconciliationRequired / OutcomeUnknownへ収束する。frontend timeoutから別operationを作らない |
| open Session archive（Idle） | request ingressから10秒 | 完了、受理前rejection、または同じoperation identityのReconciliationRequired / OutcomeUnknownへ収束する。frontend timeoutから別operationを作らない |
| closed Session archive | request ingressから10秒 | 完了、受理前rejection、または同じoperation identityのOutcomeUnknownへ収束する。frontend timeoutから別operationを作らない |
| Idle backend switch | request ingressから10秒 | 完了、受理前rejection、または同じoperation identityのReconciliationRequired / OutcomeUnknownへ収束する。frontend timeoutから別operationを作らない |
| Stop | request ingressから10秒 | terminal確定不能なら同じStop identityをReconciliationRequiredとして維持し、Idleやqueue drainへ進めない |
| application quit preparation / activation | quit ingressから13秒cutoff | cutoff後は新しい明示shutdown commandを開始しない |
| application quit全体 | quit ingressから15秒 | pre-activation abortを確定できた場合はprocessを残す。それ以外は同じflightのexit intentでexitし、未完了identityをrestart recoveryへ残す |

deadlineはsame-boot monotonic clockを正本とする。persisted wall clockはrestart分類と監査にだけ使い、restartやresponse遅延で期限を延長しない。blocking I/Oは専用workerへ隔離し、deadline後のlate resultはoperation / epoch / claim guardで別flightへの作用を拒否する。

## Surface decision table

各行は独立したtyped intentの完全な契約である。複数surfaceを一行へまとめず、各cellを別行の暗黙継承で補わない。

| Surface / action | Scope | Admission | Active turn / parts | Permission | Queue | Runtime / provider | Deadline | Persist failure | UI result | Test |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| chat tab close | View | 常にview-localで受理。backend lifecycle commandは発行しない | active turnとpartsを変更しない | 変更しない | 変更しない | runtime / provider effect 0件 | UI event内で即時。backend deadlineなし | durable writeなし。storage failureを生成しない | chat tabだけを閉じる | `close_quit_chat_tab_close_is_view_only` |
| chat panel close | View | 常にview-localで受理。backend lifecycle commandは発行しない | active turnとpartsを変更しない | 変更しない | 変更しない | runtime / provider effect 0件 | UI event内で即時。backend deadlineなし | durable writeなし。storage failureを生成しない | chat panelだけを閉じる | `close_quit_chat_panel_close_is_view_only` |
| workflow node tab close | View | 常にview-localで受理。workflow commandは発行しない | node内のactive turnとpartsを変更しない | 変更しない | 変更しない | runtime / provider / workflow effect 0件 | UI event内で即時。backend deadlineなし | durable writeなし。storage failureを生成しない | workflow node tabだけを閉じ、executionを継続表示可能なままにする | `close_quit_workflow_node_tab_close_is_view_only` |
| workspace close | View | 常にview-localで受理。Session / Workflow lifecycle commandは発行しない | workspace配下のactive turnとpartsを変更しない | 変更しない | 変更しない | runtime / provider / workflow effect 0件 | UI event内で即時。backend deadlineなし | durable writeなし。storage failureを生成しない | workspace viewだけを閉じる | `close_quit_workspace_close_is_view_only` |
| window close | View | 常にview-localで受理。application quit ingressへ変換しない | window内のactive turnとpartsを変更しない | 変更しない | 変更しない | runtime / provider / workflow / process-exit effect 0件 | UI event内で即時。backend deadlineなし | durable writeなし。storage failureを生成しない | 対象windowだけを閉じ、application shutdown overlayを合成しない | `close_quit_window_close_is_view_only` |
| normal Session close（active） | Session | validなrequest ID、session / revision、Closeを検証する。same key / commandはreplay、same key / different commandはPayloadConflict、別key / same unresolved actionはjoin、異actionはPendingOperation | final parts、`Interrupted(SessionClosed)` terminal、Session stateを同じ初期closureで確定する | pending permissionを同じclosureでsettleする | 全itemを保持してPaused。自動drainしない | runtime scopeがあれば`SessionClose`をwrite-ahead後に一回closeする。scopeがなければeffect 0件でcompact success | request ingressのmonotonic T0から10秒 | 初期closureがBeforeCommitなら元stateとeffect 0件を維持し、OutcomeUnknownなら同じoperationを解決する。closure後のruntime結果不明はClosed、terminal、Pausedを保ったReconciliationRequired | terminalとClosedを表示し、Openへ戻さない | `close_quit_active_session_close_commits_terminal_and_pause` |
| normal Session close（Idle） | Session | validなrequest ID、session / revision、Closeを検証する。same key / commandはreplay、same key / different commandはPayloadConflict、別key / same unresolved actionはjoin、異actionはPendingOperation | synthetic terminalとpartsを追加しない | pending permissionが存在する場合はclose closureでsettleし、別操作として送信しない | 全itemを保持してPaused。自動drainしない | runtime scopeがあれば`SessionClose`をwrite-ahead後に一回closeする。scopeがなければeffect 0件でcompact success | request ingressのmonotonic T0から10秒 | 初期closureがBeforeCommitなら元stateとeffect 0件を維持し、OutcomeUnknownなら同じoperationを解決する。closure後のruntime結果不明はClosedとPausedを保ったReconciliationRequired | synthetic terminalなしでClosedを表示する | `close_quit_idle_session_close_has_no_synthetic_terminal` |
| open Session archive（active） | Session | validなrequest ID、session / revision、ArchiveOpenを検証する。same key / commandはreplay、same key / different commandはPayloadConflict、別key / same unresolved actionはjoin、異actionはPendingOperation | final parts、`Interrupted(SessionClosed)` terminal、Archived stateを同じ初期closureで確定する | pending permissionを同じclosureでsettleする | 全itemを保持してPaused。自動drainしない | normal Session closeと同じwrite-ahead / scope closeを使う | request ingressのmonotonic T0から10秒 | 初期closureがBeforeCommitならopen stateとeffect 0件を維持する。closure後のruntime結果不明はArchived、terminal、Pausedを保ったReconciliationRequired | terminalとArchivedを表示し、Openへ戻さない | `close_quit_active_open_archive_commits_terminal_and_pause` |
| open Session archive（Idle） | Session | validなrequest ID、session / revision、ArchiveOpenを検証する。same key / commandはreplay、same key / different commandはPayloadConflict、別key / same unresolved actionはjoin、異actionはPendingOperation | synthetic terminalとpartsを追加しない | pending permissionが存在する場合はarchive closureでsettleし、別操作として送信しない | 全itemを保持してPaused。自動drainしない | normal Session closeと同じwrite-ahead / scope closeを使う。scopeがなければeffect 0件 | request ingressのmonotonic T0から10秒 | 初期closureがBeforeCommitならopen stateとeffect 0件を維持する。closure後のruntime結果不明はArchivedとPausedを保ったReconciliationRequired | synthetic terminalなしでArchivedを表示する | `close_quit_idle_open_archive_has_no_synthetic_terminal` |
| closed Session archive | Session | validなrequest ID、session / revision、ArchiveClosedを検証する。same key / commandはreplay、same key / different commandはPayloadConflict、別key / same unresolved actionはjoin、異actionはPendingOperation | terminalとpartsを変更しない | 変更しない | 変更しない | runtime / provider effect 0件 | request ingressのmonotonic T0から10秒 | archive closureがBeforeCommitならClosedを維持し、writer結果不明は同じoperation identityのOutcomeUnknownとして解決する | Archivedへのprojection変更だけを表示する | `close_quit_closed_archive_changes_projection_only` |
| backend switch（受理） | Session configuration | validなrequest ID、session / revision、SwitchBackend(backend ID)を検証し、Idleかつpending permission / recovery / external-effect obligationなしなら受理する。same key / commandはreplay、same key / different commandはPayloadConflict、別key / same unresolved actionはjoin、異actionはPendingOperation | terminalとpartsを変更しない | pendingなしをguardし、作成・settlementしない | 全itemを保持してPaused。不整合itemはNeedsResolution | old runtimeがあれば`SessionClose`をwrite-aheadして一回closeし、Observed後だけnew backendをeffectiveにする。scopeがなければeffect 0件で切り替える | request ingressのmonotonic T0から10秒 | 初期closure失敗はold effectiveと元queueを維持する。runtime結果不明はold effectiveとPausedを保ったReconciliationRequiredとしnew backendを開始しない | old effectiveからnew effectiveへのack済み遷移、またはold effectiveのままのreconciliationを表示する | `close_quit_idle_backend_switch_is_ack_driven` |
| backend switch（拒否） | Session configuration | validなrequest ID、session / revision、SwitchBackend(backend ID)を検証する。active turnまたはpending permission / recovery / external-effect obligationがあればBusy / PendingOperationとしてbinding / operation 0件で受理しない。same key conflictもPayloadConflictでeffect 0件 | active turnとpartsを変更しない | 変更しない | 変更しない | runtime close / provider start / new backend effect 0件 | guard判定後ただちにtyped rejection。10秒command deadlineを消費しない | lifecycle closureを開始せずdurable stateを変更しない | 操作不可理由を表示し、current backendを維持する | `close_quit_backend_switch_rejects_active_or_pending_session` |
| Cmd-Q | Application | `request_id`は1..=128 bytesの`[A-Za-z0-9._:-]`。不正値はInvalidRequestとしてplan / admission / effect 0件、same ID / same `Exit, 0`はreplay、same ID / different intentはPayloadConflict＋effect 0件、別IDのcurrent flightはfirst intentへjoinする | Activated後だけtargetごとにactive turnのfinal partsとSessionClosed terminalを確定する。Idle targetにsynthetic terminalを追加しない | target closureでsettleする | 全itemを保持してPaused。自動drainしない | global Activated後かつdispatch guard成立後だけtarget別runtime / provider / workflow closeを開始する | quit T0から13秒cutoff、15秒でabortまたはexit | pre-activation BeforeCommit / rollback確認済みfailureだけeffect 0件でabortする。activation OutcomeUnknownまたはActivated後はabortせず同identityのrecoveryを残してexitする | 共通shutdown overlay。pre-activation abort時だけwindowをshow / focusする | `close_quit_cmd_q_routes_to_shared_shutdown` |
| Application menu Quit | Application | `request_id`は1..=128 bytesの`[A-Za-z0-9._:-]`。不正値はInvalidRequestとしてplan / admission / effect 0件、same ID / same `Exit, 0`はreplay、same ID / different intentはPayloadConflict＋effect 0件、別IDのcurrent flightはfirst intentへjoinする | Activated後だけtargetごとにactive turnのfinal partsとSessionClosed terminalを確定する。Idle targetにsynthetic terminalを追加しない | target closureでsettleする | 全itemを保持してPaused。自動drainしない | global Activated後かつdispatch guard成立後だけtarget別runtime / provider / workflow closeを開始する | quit T0から13秒cutoff、15秒でabortまたはexit | pre-activation BeforeCommit / rollback確認済みfailureだけeffect 0件でabortする。activation OutcomeUnknownまたはActivated後はabortせず同identityのrecoveryを残してexitする | 共通shutdown overlay。pre-activation abort時だけwindowをshow / focusする | `close_quit_application_menu_routes_to_shared_shutdown` |
| Dock Quit | Application | native ingressを`Exit`とnative codeへ正規化し、codeなしは0。`request_id`は1..=128 bytesの`[A-Za-z0-9._:-]`。不正値はInvalidRequestとしてplan / admission / effect 0件、same ID / same intentはreplay、same ID / different intentはPayloadConflict＋effect 0件、別IDのcurrent flightはfirst intentへjoinする | Activated後だけtargetごとにactive turnのfinal partsとSessionClosed terminalを確定する。Idle targetにsynthetic terminalを追加しない | target closureでsettleする | 全itemを保持してPaused。自動drainしない | global Activated後かつdispatch guard成立後だけtarget別runtime / provider / workflow closeを開始する | quit T0から13秒cutoff、15秒でabortまたはexit | pre-activation BeforeCommit / rollback確認済みfailureだけeffect 0件でabortする。activation OutcomeUnknownまたはActivated後はabortせず同identityのrecoveryを残してexitする | originを推測せず共通shutdown overlayを表示する | `close_quit_dock_native_exit_uses_shared_shutdown_contract` |
| Tray Quit | Application | `request_id`は1..=128 bytesの`[A-Za-z0-9._:-]`。不正値はInvalidRequestとしてplan / admission / effect 0件、same ID / same `Exit, 0`はreplay、same ID / different intentはPayloadConflict＋effect 0件、別IDのcurrent flightはfirst intentへjoinする | Activated後だけtargetごとにactive turnのfinal partsとSessionClosed terminalを確定する。Idle targetにsynthetic terminalを追加しない | target closureでsettleする | 全itemを保持してPaused。自動drainしない | global Activated後かつdispatch guard成立後だけtarget別runtime / provider / workflow closeを開始する | quit T0から13秒cutoff、15秒でabortまたはexit | pre-activation BeforeCommit / rollback確認済みfailureだけeffect 0件でabortする。activation OutcomeUnknownまたはActivated後はabortせず同identityのrecoveryを残してexitする | 共通shutdown overlayを表示する | `close_quit_tray_routes_to_shared_shutdown` |
| cooperative OS logout / shutdown | Application | native eventが配信された場合だけ`Exit`とnative codeへ正規化し、codeなしは0。`request_id`は1..=128 bytesの`[A-Za-z0-9._:-]`。不正値はInvalidRequestとしてplan / admission / effect 0件、same ID / same intentはreplay、same ID / different intentはPayloadConflict＋effect 0件、別IDのcurrent flightはfirst intentへjoinする | Activated後だけtargetごとにactive turnのfinal partsとSessionClosed terminalを確定する。OSが先に強制終了した場合は完了を推測しない | target closureでsettleする。強制終了時は次回起動で未完了identityを回収する | 全itemを保持してPaused。自動drainしない | event配信中は共通dispatch guard後だけ開始する。OS強制終了は明示effect成功とみなさない | cooperative ingressのT0から13秒cutoff、15秒。OS強制終了自体の猶予は保証しない | pre-activation failureはeffect 0件でabort可能。activation不明、Activated後、OS強制終了は同identityをrecoveryへ残す | 実行可能な間は共通overlay。強制終了後の表示保証は次回起動時だけ | `close_quit_cooperative_os_exit_uses_shared_shutdown_contract` |
| programmatic exit | Application | typed internal ingressの`Exit`とsigned codeを使用する。`request_id`は1..=128 bytesの`[A-Za-z0-9._:-]`。不正値はInvalidRequestとしてplan / admission / effect 0件、same ID / same intentはreplay、same ID / different intentはPayloadConflict＋effect 0件、別IDのcurrent flightはfirst intentへjoinする | Activated後だけtargetごとにactive turnのfinal partsとSessionClosed terminalを確定する。Idle targetにsynthetic terminalを追加しない | target closureでsettleする | 全itemを保持してPaused。自動drainしない | global Activated後かつdispatch guard成立後だけtarget別effectを開始し、permitの`Exit`とcodeをそのまま使う | quit T0から13秒cutoff、15秒でabortまたはexit | pre-activation BeforeCommit / rollback確認済みfailureだけeffect 0件でabortする。activation OutcomeUnknownまたはActivated後は同identityを残してexitする | backend projectionのExitとcodeを共通overlayへ表示する | `close_quit_programmatic_exit_requires_coordinator_permit` |
| programmatic restart | Application | typed internal ingressの`Restart`とsigned codeを使用する。`request_id`は1..=128 bytesの`[A-Za-z0-9._:-]`。不正値はInvalidRequestとしてplan / admission / effect 0件、same ID / same intentはreplay、same ID / different intentはPayloadConflict＋effect 0件、別IDのcurrent flightはfirst intentへjoinする | Activated後だけtargetごとにactive turnのfinal partsとSessionClosed terminalを確定する。Idle targetにsynthetic terminalを追加しない | target closureでsettleする | 全itemを保持してPaused。自動drainしない | global Activated後かつdispatch guard成立後だけtarget別effectを開始し、permitの`Restart`とcodeをExitへ変換しない | quit T0から13秒cutoff、15秒でabortまたはrestart | pre-activation BeforeCommit / rollback確認済みfailureだけeffect 0件でabortする。activation OutcomeUnknownまたはActivated後は同identityを残してrestartする | backend projectionのRestartとcodeを共通overlayへ表示する | `close_quit_programmatic_restart_requires_coordinator_permit` |
| concurrent quit across surfaces | Application | first accepted ingressのintentを固定する。same request ID / different intentはPayloadConflict＋effect 0件、別request IDはintentが異なってもcurrent flightへjoinし、pre-activation effect 0 abort後のnew flightだけ別intentを採用する | first flightのtarget closureだけがactive turnとpartsを変更し、後続requestは追加terminalを作らない | first flightのtarget closureだけがsettleする | first flightが保持してPausedにし、後続requestは変更しない | shutdown plan、target runtime / provider / workflow effect、process exitはfirst flightから各最大1件 | 最初のquit T0から13秒cutoff、15秒。後続requestで延長しない | first flightのpersist結果へ収束し、後続requestから別plan / effectを作らない | 最初のmode / codeと同じ進行resultを全surfaceへ表示する | `close_quit_first_ingress_owns_exit_intent` |
| cooperative quit during bootstrap | Application bootstrap | validな`request_id`と最初のintentをbootstrap-safe single flightへ固定し、normal shutdown plan / targetは作らない。same ID conflictと別ID joinは通常quitと同じ | active turnとpartsを変更せず、normal agent / workflow shutdown commandを作らない | 回答、取消、settlement commandを開始しない | 変更せず、自動drainしない | current bootstrap writeのsettleだけを行い、runtime / provider / workflow shutdown effect 0件 | bootstrap quit T0から13秒settle、15秒でone-shot exit | current writeをCommittedまたはrollback済みBeforeCommitへsettleする。OutcomeUnknownを成功・失敗・未開始へ推測しない | read-only bootstrap progressと同じintentのexit progressを表示する | `close_quit_bootstrap_uses_bounded_exit_without_shutdown_effect` |
| hard kill / power loss | Process crash recovery | coordinator ingress、`request_id`、graceful admissionは存在しない | 保存済みpartsだけを復元する。#1499で識別可能な未完了turnはdurable obligationから回収し、証明不能なlegacy turnは通常terminalを捏造しない | 保存済みpermission stateとexact response obligationから回収し、blind responseしない | itemを削除せずPaused / recovery holdとして復元する | graceful runtime / provider / workflow closeを開始せず、process exitの暗黙作用を成功または未開始へ推測しない | graceful 13秒 / 15秒保証の対象外 | 次回起動でsame identityをOutcomeUnknown / ReconciliationRequiredとして解決し、保存失敗を成功へ格上げしない | 次回起動後にCrash / recovery状態を正規surfaceへ表示する | `close_quit_hard_kill_recovers_as_crash` |

## Session lifecycle operation contract

view closeを除く4 actionはTauri専用`request_session_lifecycle`一つへ写す。`Close / ArchiveOpen / ArchiveClosed / SwitchBackend { backend_id }`以外は受け付けない。request IDは1..=128 ASCII bytesの`[A-Za-z0-9._:-]`、expected revisionはone-basedである。Close / ArchiveOpen / ArchiveClosedはbackend optionがexactly none、SwitchBackendはnonempty validated backend IDがexactly oneでなければならず、unknown action / option、trailing bytes、revision 0は`InvalidRequest`としてbinding、operation、session、effectを0件にする。backend発行`SessionLifecycleOperationId`へcaller identityのvalidationを流用しない。

same principal / same request ID / same exact commandは同じimmutable receiptとcurrent stateをreplayする。same key / different commandはTauri専用`PayloadConflict(SessionLifecycle { request_id })`で増分0件、different key / same principal / same unresolved session / same normalized actionは既存operationへjoinしてbindingだけを追加し、first receipt / revision guard / action / deadlineを変更しない。SwitchBackendはbackend IDまで同一の場合だけsame actionである。different actionは`PendingOperation`で新binding / operation / effectを0件にする。same-key raceはcaller-key winner一件、different-key raceはsession single-flight winner一件へ収束する。session authorizationを先に検査し、unauthorized command、cross-principal operation query、unknown operation IDは存在を秘匿したNotFoundとする。

resultは`Accepted { receipt, current: InProgress | ReconciliationRequired | Completed } | RejectedBeforeAcceptance { Busy | PendingOperation | RevisionConflict | InvalidState | Failed } | OutcomeUnknown { operation_id }`のclosed setである。writer開始前またはrollback確認済みfailureだけがrejection、writer結果不明だけがOutcomeUnknownである。Accepted後はreceiptを維持し、request ingressから10秒以内にCompletedまたはReconciliationRequiredへ進める。`get_session_lifecycle_operation`はresponse喪失とrestart後もbackend operation IDのdirect authorityから同じreceipt / state / saved outcomeを返し、current session projectionから再構築しない。operation viewが進捗、session read modelがClosed / Archived / effective backendの表示正本であり、frontendは両者から第三のstateを合成しない。BackendSelectedは次sendまでruntime_started=false、closed archiveはqueueを変更せず保存済みqueue_pausedを返す。

bindingはapp-data generationごとexactly oneのowner-only共通keyと`session-lifecycle-exact-request-binding/v1` domainを使う。key bytes `00..1f`、principal `principal_1`、generation `app_1`、request `lifecycle_req_1`、operation `lifecycle_op_1`、session `session_1`、revision `1`、action `close`、backend `none`のKATはinner 38 bytes、full preimage 149 bytes、HMAC-SHA256 `b623c791f1a3f40579ba9713507ab507bdc844dee12d95e4408d673b17eb2217`である。4 actionとbackend ID、revision、principal、request、operation、generation、key、domainのone-field mutationを共有fixtureで拒否する。

## Application quit ingress

Tauri 2の`RunEvent::ExitRequested`が提供するのは`code: Option<i32>`であり、Cmd-Q、Dock Quit、OS logout / shutdownのoriginを常に識別できるわけではない。production code、event、telemetryで識別不能なoriginを推測しない。

全surfaceはadaptorでwire DTOをdecodeした後、coordinator admission前にcanonical `ShutdownExitIntent { mode: Exit | Restart, code: i32 }`へ正規化する。`ShutdownExitIntentV1`はTauri / WebSocket adaptor DTOに限り、domain / usecase / lifecycleの正本へ逆流させない。

typed Tauri / WebSocket / internal callerは`request_id`を一度だけ生成して渡す。`RunEvent::ExitRequested`のようにwire event自体がidentityを持たないnative ingressでは、受信adaptorがcurrent installation principalのcallerとしてvalidなopaque `request_id`をevent受信時に一度だけ生成し、coordinator admission、same-flight join、result replayまで保持する。origin、exit code、wall clockからidentityを導出したり、同じnative eventの再処理ごとに再生成したりしない。

| Ingress | Normalized intent |
| --- | --- |
| Cmd-Q | `Exit, 0` |
| Application menu Quit | `Exit, 0` |
| Tray Quit | `Exit, 0` |
| Dock Quit、native codeなし | `Exit, 0` |
| Dock Quit、native codeあり | `Exit, same code` |
| cooperative OS logout / shutdown、native codeなし | `Exit, 0` |
| cooperative OS logout / shutdown、native codeあり | `Exit, same code` |
| programmatic exit | typed ingressが指定する`Exit`とcode |
| programmatic restart | typed ingressが指定する`Restart`とcode |

quit commandの`request_id`は1..=128 bytesの`[A-Za-z0-9._:-]`に限定したcaller指定stable identityである。空、129 bytes以上、non-ASCII、または文字集合外は`InvalidRequest`としてplan / admission変更 / effectを0件にする。同じprincipal / request ID / same intentは同じapplication quit operationとresultをreplayし、同じrequest IDへ異なるmode / codeを渡した場合はresult variantやexact 19種の`SessionOperationFailureKind`へ加えず、WebSocketまたはTauriの共通`PayloadConflict { identity: ApplicationQuit { request_id } }`としてplan / effectを0件にする。`request_id`を`operation_id`へ改名しない。別request IDでcurrent flightへ到着した後続requestはintentが異なってもfirst accepted flightへjoinし、そのflightのintentを書き換えない。pre-activation effect 0 abortが確定した後のnew flightだけが別intentを採用できる。

automatic Phase 0 bootstrap中はnormal shutdown plan、target、agent / workflow shutdown commandを作らない。cooperative quitだけをbootstrap-safe single flightへ渡し、最初のintentを固定したままcurrent writeをsettleし、15秒以内にexitする。OutcomeUnknownをbootstrap成功・失敗・未開始へ推測しない。

## Application shutdown contract

1. 最初のquitはglobal mutation admissionを閉じ、同じflightのstable operation identityとexit intentを確定する。
2. quitより前に受付済みのmutationを、Committedまたはrollback済みBeforeCommitへsettleする。writer結果不明を未受理へ格下げしない。
3. targetはquit開始時にopenなactive / Idle Sessionと進行中Workflowである。関連runtime / childはowner targetのsubordinate effectであり別targetへ数えない。closed / archived Sessionとunowned runtimeの既存recoveryはtargetにせず、pending recovery snapshotへ残す。
4. targetは最大4096件。1 pageは最大128件かつencoded 1 MiB以下。上限超過は`CapacityExceeded`または`ResponseTooLarge`としてpre-activation、effect 0件でabortする。
5. 全targetをrestart後も識別できるPrepared planとして保存した後だけglobal activationを試みる。partial preparationをshutdown開始の証拠にしない。
6. activation commitを確認したcurrent coordinatorだけがtarget別reservationを行う。reservation後もdispatch guardを通らない明示commandは開始しない。
7. pre-activation BeforeCommitとrollbackを確認し、Failed / Cancelled fenceを確定できた場合だけeffect 0件でabortし、admissionを再開する。
8. activation writerのOutcomeUnknownは未開始へ格下げせず、明示command 0件のまま同identityをresolveする。15秒でも不明ならactivation-possible recovery exitとする。
9. Activated後はabortやadmission reopenを行わない。cutoffまでにterminalへ進まないtargetを同じidentityのReconciliationRequiredとして残し、15秒以内にexitする。
10. process exit自体がpipe close、job object、parent-death signalへ与えた作用を0件または成功と推測せず、該当targetを`ExitCoupledOutcomeUnknown`としてrestart readbackへ送る。

shutdown phaseは`Preparing | Prepared | Activated | Quiescing | Completed | Failed | Cancelled | ReconciliationRequired`のclosed 8値である。全targetがterminalならplan phaseはCompletedとし、non-success targetやpreexisting recoveryはsummary outcome `ExitedWithRecovery`へ表す。

same-boot current flightには後続quitをjoinさせる。process flightがなくてもsame-boot / previous-boot nonterminal planまたはunresolved shutdown scope fenceが残る場合、新quitはAccepted前に`PreviousShutdownReconciliationRequired`を返し、新plan / effectを作らない。old detail compaction待ちなら`PreviousShutdownCompactionPending`を返す。`RetryQuit`はsame-boot pre-activation effect-0 Failed、durable fence、admission Open、store Healthyを同じsnapshotで証明できる場合だけ提示する。

## Shutdown readback

- current surfaceはTauri `get_application_shutdown` / WebSocket `GetApplicationShutdown`の共通query / presenterが返すclosed `CurrentApplicationShutdownResult`である。
- known quit operationはTauri `get_application_quit_operation` / WebSocket `GetApplicationQuitOperation`からtop-level `ApplicationQuitOperationView`として読む。Acceptedの`current`だけがOptionを持たない`CurrentApplicationQuitOperationResult`を保持し、closed `ApplicationQuitProjection::Shutdown | Bootstrap`を区別する。Shutdown locatorはhash-valid live rootまたはimmutable compact archiveのclosed unionで解決し、archive-onlyでも同じTerminal / Compactedを返し、双方存在時はsemantic parityを必須にする。双方不在または不一致はInternalである。bootstrap-safe flightをnormal shutdown planまたは`Current(None)`へfallbackしない。
- 通常起動とinitial root commit前の受理前rejectionは`Current(None)`、same-boot Accepted flightはterminalなFailed / Cancelledを含め`Current(Some(projection))`、保存結果不明は`OutcomeUnknown`である。
- fresh bootではprevious-boot nonterminal planだけを同identityのReconciliationRequiredとして`Current(Some(...))`にする。previous-boot terminal planは`Current(None)`で、history queryから読む。
- hash-validなcomplete rootがexactly oneでplan ID / epoch / exit intentを一意にanchorでき、plan pointer等の冗長semantic identityだけが矛盾する場合に限って`ShutdownAuthorityMismatch`を持つ同identityのReconciliationRequiredを返す。shutdown authorityのcommit結果を確認できない場合だけembedded `OutcomeUnknown`を返し、query storage / decode / envelope・self-hash / pointer-to-root hash failure、required record欠損、state composite・activation lineage integrity failure、identity unanchorable / ambiguousはTauriの`GetApplicationShutdownApplicationError::Internal`またはWebSocketの`AgentSessionWsErrorV1::Internal`として返して`Current(None)`やembedded resultへ隠さない。
- `get_shutdown_plan / GetShutdownPlan`はstable target order、最大128件 / encoded 1 MiBのpageを返す。cursorはplan / epoch / revision / snapshotへ束縛し、`CursorExpired`後にold pageとfresh pageを連結しない。`QueryBusy` / `DeadlineExceeded`ではpartial countsやpartial pageを返さない。
- current pending recovery queryと、planが固定したpreexisting recovery snapshot queryは別authorityである。frontendはcurrent一覧をhistorical snapshotの代用にしない。
- `resolve_pending_recovery_action / resolve_shutdown_target_action`はbackend発行action IDをstable identityとする。Completed decisionはcurrent resource / plan detailを再構築せず保存済みreceipt / safe resultをexact replayする。fresh actionまたはnonterminal joinだけがcurrent revision / root / state guardを検証する。
- shutdown target actionのSucceeded / ConfirmedNoEffect / Ambiguousはkind固有owner state、target result、action receiptを同じclosureで確定する。最後のtargetではplan Completed、summary、pointerも同じclosureで確定し、target resultだけのpartial completionを作らない。

## Failure policy

- initial Session close / archive closureを確定できない場合は元stateを保ち、runtime closeを開始しない。closure確定後のruntime close結果不明はClosed / Archivedとqueue pauseを保ったReconciliationRequiredにする。
- application quitはknown pre-activation failure、またはactivation BeforeCommit＋rollback＋terminal fenceを確認できた場合だけabortする。activation OutcomeUnknown / Activated後は未完了identityを残してexitする。
- graceful exit eventが届かなかった場合はgraceful完了を推測せずhard-kill recoveryを適用する。
- provider shutdownはcancellation-safeなforce-kill / reap portを持ち、内側のgraceful waitを外側deadlineより短くする。
- late resultは元operation / planへだけ収束し、別Session、別turn、別shutdown flight、new backend startを変更しない。

## Traceability

| Requirement | Canonical clauses |
| --- | --- |
| #1499 R-014 | Surface decision table |
| #1499 R-015 | Application quit ingress、Application shutdown contract、Shutdown readback |
| #1499 R-016 | Deadline、Application shutdown contract、Shutdown readback、Failure policy |
| #1499 R-017 | bounded target / pending recovery lookup |
| #1499 R-018 | local-store migration中のgate、migration-safe quit、Tauri / WebSocket共通readback |
| #1499 R-020 | Session close / archiveとStop競合時のcanonical terminal closure |
| #1499 R-021 | recovery / shutdown target action、last-target finalization |
| lifecycle I1 / I4 / I7 / I17 | 本書全体 |
