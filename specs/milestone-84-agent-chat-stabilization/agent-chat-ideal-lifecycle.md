# Agent セッションライフサイクルの理想形

作成日: 2026-07-07
更新日: 2026-07-29

本書は Session、turn、permission、queue、configuration、Goal、close、quit が、正常、failure、crash、restart の各経路で守る lifecycle invariant を定義する。型、schema、内部処理順は定義しない。

関連正本:

- [agent-chat-ideal-vocabulary.md](agent-chat-ideal-vocabulary.md)
- [agent-chat-ideal-presentation.md](agent-chat-ideal-presentation.md)
- [close-quit-decision-table.md](close-quit-decision-table.md)
- [Issue #1499 Primary Spec](../../docs/specs/issues-1499/requirements.md)

## #1499 との境界

#1499 は固定 path の SQLite store を normal runtime の唯一の persistence authority とする。本書の invariant は SQLite に受理された operation と state に適用する。変更前の file-store data は探索・読込せず、その data だけに存在する未完了 turn を推測で terminal 化しない。

Phase 0、F2、F3、D3などの計画labelと、廃止したfile-storeの物理用語はlifecycle stateではない。通常の SQLite schema evolution と、provider / configuration / watch の各初期化を legacy-data migration と混同しない。

## 設計原則

- **L-P1（durable-first）**: live 表示は reload 後の read model と同じ結果へ収束する。
- **L-P2（明示的な終端）**: turn、permission、tool、queue、operation は忘れられた進行中状態を残さない。
- **L-P3（失敗の着地）**: canonical state の失敗を握りつぶさず、受理前 rejection または受理済み reconciliation へ着地させる。
- **L-P4（backend parity）**: Stop、liveness、permission、terminal の保証水準は provider に依存しない。
- **L-P5（Rust authority）**: configuration、Goal、available action、recovery、shutdown の判断は Rust が所有する。
- **L-P6（atomic visibility）**: 一つの利用者操作として不可分な state は一括して可視になり、partial read model を公開しない。
- **L-P7（domain lifecycle authority）**: Session のライフサイクル（状態・遷移・受理条件・不変条件）は domain の集約がメソッドとして表現する。状態遷移は集約の操作経由でのみ起き、他層が状態を直接書き換える経路を作らない。usecase は駆動手順とトランザクション境界、gateway は外部世界の都合と内側の言語の相互変換、infrastructure は外部世界の都合をその形のまま扱うことに限り、controller は入口に留まる。adaptor / usecase の手続きに受理判定の独自解釈を置かない。

## 不変条件

### I1: turn 終端保証

normal completion、interrupt、Fatal、normal Session close、open Session archive、graceful application shutdown は、final parts、TurnResult、未解決 permission / tool、queue の outcome を一つの canonical terminal へ収束させる。view close は turn 終端を起こさない。

### I2: クラッシュ回収

SQLite に受理済みの未完了 work は startup で bounded に発見し、同じ identity と既知 observation から回復する。結果不明を成功または未開始へ推測せず、closed / archived Session を自動 reopen しない。

### I3: streaming flush 保証

streaming の live delivery が欠落または遅延しても、最終的な parts は canonical read model から復元できる。partial parts で保存済みの完全な content を上書きしない。

### I4: queue の永続と一貫性

Accepted queue item は restart、close、backend switch を跨いで同じ identity と input を保つ。cancelled item は履歴に cancelled として残し、復元 context から除外する。通常完了以外では queue を pause し、暗黙 drain しない。

### I5: interrupt 保証

Stop は provider の内部 turn identity がまだ観測できない時点でも失われず、request から 10 秒以内に terminal または同じ Stop identity の結果確認必要状態へ到達する。Stop 後の queue は pause する。

### I6: ユーザー入力の無損失

send または steer が未受理、拒否、結果不明の場合、本文と添付を保持する。Accepted 後だけ対応する attempt の input を clear し、結果不明を別 identity で自動再送しない。

### I7: permission の有効性

permission request は Pending、Responding、Resolved、Cancelled、ReconciliationRequired を利用者が区別できる。provider が取り下げた request は直ちに操作不能になり、exact response の実効性を確認できない場合は blind resend しない。

### I8: ack 駆動と failure 可視化

provider または storage の ack 前に user-visible state を成功へ進めない。受理前 failure と受理後 failure を分け、対象 scope に安全な説明と解決操作を表示する。external effect は開始直前に canonical intent と現在の owner を Rust-owned boundary で再確認し、stale なら開始しない。

### I9: resume 回復の統一

provider session の resume failure は backend に依存せず、保存済み conversation と configuration を保った recoverable outcome へ着地する。回復不能な provider state を current として使い続けない。

### I10: backend stdout の頑健性

識別済み diagnostic、未知 content、malformed / incompatible control message を区別する。応答が必要な control message を無言で捨てず、protocol incompatibility は安全に turn 開始を止める。

### I11: 生存シグナルと stall 判定

thinking、tool、provider keep-alive など意味的な進行を backend 共通の liveness として扱う。正常な長考を backend 差で stall と誤判定せず、stall は terminal と同一視しない。

### I12: エラーの着地保証

turn failure、idle runtime failure、workflow failure reason は対象の durable surface へ届き、live と reload で同じ意味を持つ。log だけに残して成功扱いしない。

### I13: 排他と lock の規約

同じ Session / operation の競合は一つの owner が調停する。外部 I/O を待つ間に他 state の進行を不必要に止めず、late result は元 operation にだけ作用する。

### I14: Agent 実行設定の ack・revision 保証

provider、model、mode、ReasoningEffort は selected、effective、pending、reconciliation を区別する。利用者更新は revision と capability を検証し、provider ack と canonical commit 後だけ effective にする。silent fallback を行わない。

### I15: Goal lifecycle と回復保証

Goal は configuration と独立し、Session ごとに current 最大一つを持つ。set、edit、pause、resume、clear、completed、failed、blocked を evidence と provider capability に基づいて遷移させ、restart / resume / workflow 継続で identity と status を保つ。

### I16: ReasoningEffort capability と反映時点

ReasoningEffort の option、default、availability、反映時点は provider / model capability に基づく。selected と effective / unknown を分け、unsupported value を別値へ silent clamp しない。TokenUsage や budget を代替値にしない。

### I17: close / quit surface authority と bounded shutdown

view close、Session lifecycle、backend switch、application quit は [close-quit-decision-table.md](close-quit-decision-table.md) の異なる intent とする。Session lifecycle と Stop は 10 秒、graceful application quit は 15 秒の利用者可視 deadline を持つ。shutdown summary と detail は同じ canonical shutdown state から得る。

startup failure 中は normal Session / Workflow / quit operation が存在しない。Rust-owned safe failure と process-local cooperative exit だけを提供し、durable migration progress や特殊 shutdown state を作らない。初回作成残骸は durable な initial-create evidence があり、normal admission が一度も開いていない場合だけ再利用可能と判断する。既存の空 file だけを根拠に再初期化しない。

## turn 状態と利用者操作

| State | Allowed user action | Required outcome |
| --- | --- | --- |
| Idle | send、valid configuration / Goal change、close | Accepted operation または受理前 rejection |
| Starting | Stop、operation readback | start または Stop の同じ identity へ収束 |
| Streaming | Stop、queue、supported steer | input を失わず Accepted / rejected / unknown を区別 |
| Waiting permission | answer、Stop | exact response または Cancelled / reconciliation |
| Pending terminal | readback、safe recovery action | normal Idle と区別し queue を開始しない |
| Reconciliation required | Rust が提示する action | blind retry せず同じ identity を解決 |
| Closed / Archived | history、allowed archive / recovery | provider を自動 resume しない |

## シナリオ別保証

| Scenario | Observable guarantee |
| --- | --- |
| send response loss | same identity で同じ receipt / state |
| Stop immediately after send | 10 秒以内に terminal または reconciliation |
| crash during streaming | reload で既知 parts と crash / recovery state |
| permission answer during crash | exact intent を同じ identity で readback、blind resend なし |
| queue then restart | queue item と input を復元、暗黙 drain なし |
| close active Session | final parts、SessionClosed、permission settlement、queue pause |
| backend switch result unknown | old effective backend と queue pause、new backend 自動開始なし |
| quit with slow target | 15 秒以内に exit / restart、未完了 identity を保持 |
| SQLite first-create crash | incomplete initial creation を同じ path で安全に再試行 |
| initialized SQLite corruption | normal workbench を開かず、既存 store を変更しない |
| SQLite startup attempt blocked | writer lockは待たず、SQLite busy waitは最大2秒、同一process内で自動再試行しない |
| old file-store present | production lifecycle 全体で非参照・無変更 |

## backend 差の吸収規約

| Concern | Common guarantee |
| --- | --- |
| interrupt | request を失わず 10 秒以内に同じ outcome |
| reasoning / liveness | provider が公開した進行を同じ surface へ反映 |
| permission | exact question / answer semantics と取消を保持 |
| resume | recoverable state または明示 failure |
| mode / Goal / effort | capability、selected / effective、effects を表示 |
| protocol drift | session / turn を開始せず ProtocolIncompatible |

## トレーサビリティ

| Problem group | Invariants |
| --- | --- |
| RT-1〜RT-8 | I1〜I4、I8、I12 |
| OB-1〜OB-8 | I4〜I6、I9 |
| CL / CX control loss | I7〜I12 |
| configuration / Goal | I14〜I16 |
| #1499 R-001〜R-013 | I1〜I13 |
| #1499 R-014〜R-021 | I1、I2、I4、I5、I7、I17 |
| #1499 R-022 | L-P4、L-P6 |

## 設計判断

- **L-D1**: queue は durable lifecycle とし、通常 surface は active / recent state だけを表示する。
- **L-D2**: Stop deadline は request から 10 秒で、response delay や restart により延長しない。
- **L-D3**: recovery discovery は pending work の bounded inventory を使い、全 Session / event historyへ fallback しない。
- **L-D4**: cancelled queue message は transcript に mark して残し、復元 context から除外する。
- **L-D5**: Stop 後の queue は常に pause し、再開は明示操作にする。
- **L-D6**: configuration は selected / effective / pending / reconciliation を分離する。
- **L-D7**: Goal は configuration と独立した Session-scoped lifecycle にする。
- **L-D8**: ReasoningEffort は provider / model capability 駆動にし、TokenUsage と分離する。
- **L-D9**: workflow template、resolved launch configuration、Session configuration を別 scope にする。
- **L-D10**: queue item は受理時の execution configuration と Goal reference を保持し、暗黙 rebase しない。
- **L-D11**: close と quit の意味は decision table を唯一の surface 正本にする。
- **L-D12**: backend switch は Idle 限定で、active turn を終了させる操作にしない。
- **L-D13**: native exit の識別不能な origin を推測しない。
- **L-D14**: application quit は一つの Rust-owned flight とし、15 秒以内に abort / exit / restart を決める。
- **L-D15**: fixed SQLite path の create / open / schema evolution / validation とinitial-create evidenceの完了後だけnormal admissionを開き、startup failureはclosedなsafe classificationとprocess-local exitだけを提供する。
- **L-D16**: SQLite が唯一の persistence authority であり、legacy data用の互換・切替・進捗・特殊終了機構を持たない。
