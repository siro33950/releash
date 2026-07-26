# Close / quit surface decision table

作成日: 2026-07-19
更新日: 2026-07-24

本書は milestone 84 における close、archive、backend switch、application quit の surface ごとの意味を定める。共通語彙は [agent-chat-ideal-vocabulary.md](agent-chat-ideal-vocabulary.md)、不変条件は [agent-chat-ideal-lifecycle.md](agent-chat-ideal-lifecycle.md)、Issue #1499 の観測可能なシナリオは [behavior.md](../../docs/specs/issues-1499/behavior.md) を正本とする。

## 共通規則

- view close、Session close / archive、backend switch、application quit は別の intent であり、相互に代用しない。
- view close は表示だけを閉じ、turn、permission、queue、runtime、workflow executionを変更しない。
- Session close / archive と backend switch は、同じ操作の再送を同じ結果へ収束させる。未解決の同種操作には合流し、競合する操作は新しい作用を始めず拒否する。
- lifecycle 操作は queue item を削除せず、明示的に再開されるまで pause を維持する。
- external effect は、その作用を開始してよいことが永続化され、開始直前にも有効性が確認された場合だけ開始する。
- application quit は入口によらず同じ Rust-owned coordinator を通る。同時要求は最初に受理した終了意図へ合流し、期限を延長しない。
- hard kill と power loss は graceful quit ではなく、次回起動時の crash recovery が扱う。
- startup failure 中の終了は normal backend admission を経由しない process-local な一回限りの操作であり、永続的な quit 操作や代替進捗状態を作らない。

## 利用者に保証する期限

| 操作 | 期限 | 期限時の結果 |
| --- | --- | --- |
| Stop、Session close / archive、受理可能な backend switch | request ingressから10秒 | 完了、作用開始前の拒否、または同じ操作の結果確認待ちへ収束する。表示側の timeout から別操作を作らない |
| application quit | 最初のrequest ingressから15秒 | 安全に中止できる場合は process を残して理由を表示する。それ以外は未完了操作を次回起動時に回収可能なまま終了する |
| startup failure中のQuit | request ingressから15秒 | durable stateを作らず、process-local exit effectを一度だけdispatchする |

## Surface decision table

| Surface / action | Scope | 受理条件 | domain への作用 | failure / restart 後 | 利用者に見える結果 |
| --- | --- | --- | --- | --- | --- |
| chat tab close | View | 常に受理 | chat tab の表示だけを閉じる | 永続化や recovery を開始しない | 対象 tab が閉じる |
| chat panel close | View | 常に受理 | panel の表示だけを閉じる | 永続化や recovery を開始しない | 対象 panel が閉じる |
| workflow node tab close | View | 常に受理 | node tab の表示だけを閉じ、workflow execution は継続する | 永続化や recovery を開始しない | 対象 tab が閉じる |
| workspace close | View | 常に受理 | workspace view だけを閉じる | Session / Workflow lifecycle を開始しない | 対象 view が閉じる |
| window close | View | 常に受理 | 対象 window だけを閉じる | application quit へ変換しない | 対象 window が閉じる |
| active Session close | Session | revision と競合操作を検証して受理 | 既存 parts、interrupted terminal、permission settlement、Closed、queue pause を一つの意味的確定として扱い、必要な runtime close を一度だけ開始する | 作用開始前の保存失敗では元状態を保つ。runtime 結果不明では Closed と pause を保って結果確認待ちにする | terminal と Closed、または同じ操作の確認待ちを表示する |
| Idle Session close | Session | revision と競合操作を検証して受理 | synthetic terminal を追加せず Closed と queue pause を確定し、必要な runtime close を一度だけ開始する | active close と同じ安全規則を使う | synthetic terminal なしの Closed を表示する |
| active open Session archive | Session | revision と競合操作を検証して受理 | active close と同じ終端化を行い、最終状態を Archived にする | 作用開始前の保存失敗では open を保つ。結果不明では Archived と pause を保つ | terminal と Archived、または確認待ちを表示する |
| Idle open Session archive | Session | revision と競合操作を検証して受理 | synthetic terminal を追加せず Archived と queue pause を確定する | active archive と同じ安全規則を使う | synthetic terminal なしの Archived を表示する |
| closed Session archive | Session | revision と競合操作を検証して受理 | parts、terminal、permission、queue、runtime を変更せず Archived へ移す | 保存結果不明は同じ操作として解決する | Archived への変更だけを表示する |
| backend switch | Session configuration | Idle で、未解決の permission、recovery、external effect がない場合だけ受理 | old runtime の終了が確認された後だけ new backend を有効にし、queue は pause する | 結果不明では old backend を有効なまま保ち、new backend を開始しない | 新 backend の確定、拒否理由、または old backend のままの確認待ちを表示する |
| Cmd-Q / menu / Dock / Tray Quit | Application | 共通 quit intent として受理 | 全 target の保存済み shutdown 状態を一つの意味的集約として確定し、その後に必要な終了作用を開始する | 安全に中止できない不明結果は同じ quit として次回起動時に回収する | 共通 shutdown 表示と同じ結果を返す |
| cooperative OS logout / shutdown | Application | event を受信できた場合に共通 quit として受理 | 共通 quit と同じ。OS が先に強制終了した場合は成功を推測しない | 保存済み状態から次回起動時に回収する | 実行可能な間だけ共通 shutdown 表示を行う |
| programmatic exit / restart | Application | 呼出元の exit / restart intent を保持して共通 quit として受理 | 共通 quit と同じ。exit と restart を相互変換しない | 同じ intent と quit identity で回収する | 共通 shutdown 表示に確定した intent を示す |
| concurrent quit | Application | 最初に受理した intent へ後続要求を合流 | shutdown と process effect を重複させない | 最初の quit の結果へ収束する | 全 surface に同じ進行と結果を表示する |
| cooperative quit during SQLite startup failure | Startup failure | Rust startup outcome が Failed の場合だけprocess-local one-shotとして受理 | SQLiteを再openせず、Session、Workflow、queue、permission、durable quit stateを作らず終了effectを一度だけdispatchする | 重複要求は同じprocess-local dispatchへjoinする。次回起動は同じ固定SQLite pathを通常のstartup規則で扱う | safe classification、correlation、次回launch時の扱い、Quitだけを表示する |
| hard kill / power loss | Crash recovery | graceful admission なし | 保存済み状態だけを根拠に未完了操作を回収し、未保存の成功や terminal を捏造しない | 次回起動で Crash または結果確認待ちとして示す | 起動後の正規 surface に recovery 状態を表示する |

## Application shutdown contract

- shutdown の保存、保存後検証、external-effect gate、current readback、history readback、target pagination は、SQLiteの同じplan / ordered target rowsから得る一つのsemantic shutdown aggregateとrevisionを共有する。
- current と history は同じ aggregate の異なる view であり、別の authority や独立した真偽判定を持たない。
- target ごとの結果は aggregate の一部として保存される。全 target が terminal になるまで、全体を完了と表示しない。
- quit 受理後の保存結果が不明な場合、同じ quit を問い合わせて解決する。空の current や新しい quit に置き換えない。
- public collection access は同じ aggregate revision の結果だけを返す。利用者に一貫した結果を返せない場合は partial success を返さない。
- page file、page reference、root hash、root page、current recovery collectionを保存検証・effect gate・paginationの代替authorityにしない。
- startup failure 中は normal shutdown aggregate が存在しないため、存在するかのような current/history projection を作らない。

## Traceability

| Requirement | 本書の責務 |
| --- | --- |
| #1499 R-014 | view close、Session lifecycle、backend switch の区別とrequest ingressから10秒の結果 |
| #1499 R-015 | application quit の共通入口と single-flight |
| #1499 R-016 | 最初のrequest ingressから15秒のapplication quit結果 |
| #1499 R-017 | shutdown aggregate の一貫した public readback |
| #1499 R-018 | startup failure 中の process-local exit |
| #1499 R-020 | Session close / archive と Stop の canonical terminal |
| #1499 R-021 | recovery action と shutdown target の一貫した確定 |
| lifecycle I1 / I4 / I7 / I17 | scope 分離、重複作用防止、failure、quit |
