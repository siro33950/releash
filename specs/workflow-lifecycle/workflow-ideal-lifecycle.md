# Workflow ideal lifecycle

この文書は workflow runtime の lifecycle 不変条件、遷移、操作受理、capability 導出を定義する。構造境界は [`../../docs/workflow-engine-model-boundary.md`](../../docs/workflow-engine-model-boundary.md)、構文は [`../../docs/workflow-yaml-syntax.md`](../../docs/workflow-yaml-syntax.md) を正とする。

## 状態語彙と所有者

### 実行木全体

WorkflowExecution が表す実行木全体の status は3値である。

| status | 意味 | 終端 |
| --- | --- | --- |
| Running | 木に継続可能な処理、判断待ち、復旧待ちがある | No |
| Completed | root Node の completion が成立した | Yes |
| Aborted | 人間の abort で木全体を終端した | Yes |

### NodeExecution

Node の詳細状態は NodeExecution が所有する。

| 状態 | 意味 |
| --- | --- |
| Running | Node が実行中 |
| WaitingApproval | 本来の completion が成立し、human Approve を待つ |
| Paused | Session / Command の外部 runtime を停止し、再開可能 |
| Failed | Node attempt が失敗し、retry または resume の判断待ち |
| Completed | Node attempt の completion が成立 |
| Aborted | Node attempt が中断され終端 |
| Interrupted | 外部 runtime の喪失などで継続不能 |

WorkflowExecution は Node の WaitingApproval、Paused、Failed、Interrupted を別の木全体 status として複製しない。これらを含む非終端の木全体は Running であり、capability と reason が人間に次の操作を示す。

## 不変条件

- **W-I1: 集約権威** — workflow aggregate だけが実行木と NodeExecution の transition を受理し、次状態を決める。
- **W-I2: 事実先行** — 外部 effect の観測結果は canonical Node fact として durable 化してから read model へ反映する。
- **W-I3: completion と辺の分離** — completion は Node、完了後の進行は Sequence の辺が所有する。
- **W-I4: 開始済みだけを保持** — 実行木は実際に開始した NodeExecution だけを持ち、定義から expected slot を再生成しない。
- **W-I5: attempt 分離** — Submit、provider Stop、Artifact、失敗、承認は同じ Node attempt に属するものだけを組み合わせる。
- **W-I6: retry の明示性** — retry は元と次の NodeExecution の durable な関係で識別する。同名 loop 再訪や別 Fanout lane を retry とみなさない。
- **W-I7: 結果を推測しない** — process、provider、storage の結果が不明なら完了・失敗を推測せず、recovery reason として観測可能にする。
- **W-I8: 独立収束** — 一つの execution / obligation の失敗は、他 execution の復旧を止めない。transient だけを再試行し、permanent は明示的に終端化する。
- **W-I9: capability の実在性** — read model の capability は、同じ識別子と状態で domain command が実際に受理できる操作だけを true にする。
- **W-I10: surface 非所有** — UI、CLI、API、controller は lifecycle state を所有・再導出しない。

## completion

| Node | 自動 completion | approval 指定時 |
| --- | --- | --- |
| Session | 同一 attempt の Submit と provider Stop が揃う | 二信号後に WaitingApproval、Approve で Completed |
| Command | process 終了と結果確定 | 結果確定後に WaitingApproval、Approve で Completed |
| Fanout | 全 child が決着 | 全 child 決着後に WaitingApproval、Approve で Completed |
| Sequence | 実効終端 child へ到達 | 終端到達後に WaitingApproval、Approve で Completed |

Session の Submit と provider Stop は順不同である。片方だけの状態は Running のまま保持し、後から同じ attempt のもう一方が到着したときだけ completion を評価する。Artifact を宣言した Session では、Contract 検証済み Artifact を含む Submit だけを有効とする。

## 実行木全体の遷移表

| 現在 | 事象 | 次 | 条件 |
| --- | --- | --- | --- |
| Running | root completion 成立 | Completed | root の既定条件と必要な Approve が成立 |
| Running | abort | Aborted | command が受理された |
| Running | Node 開始・完了・失敗・pause・resume・retry | Running | 木全体の終端条件は未成立 |
| Completed | 任意の lifecycle command | Completed | 終端。不変 |
| Aborted | 任意の lifecycle command | Aborted | 終端。不変 |

Completed と Aborted は相互遷移しない。Node が Failed、Paused、WaitingApproval、Interrupted になっても、木全体は Running のまま操作を待つ。

## NodeExecution の主要遷移

| 現在 | 事象 | 次 |
| --- | --- | --- |
| Running | 自動 completion 成立 | Completed |
| Running | approval 前提条件成立 | WaitingApproval |
| WaitingApproval | approve | Completed |
| Running | stop 対象の外部 runtime 停止 | Paused |
| Paused | resume | 新しい実行または同一 conversation の Running |
| Running | canonical failure | Failed |
| Failed | retry | 新しい attempt の Running |
| Running / Paused | Submit / provider Stop の片方だけが到着した状態で retry | 旧 attempt を Aborted にし、新しい attempt の Running |
| Running / WaitingApproval / Paused / Failed | abort | Aborted |
| Running | runtime 喪失 | Interrupted |
| Interrupted | resume が受理 | 復旧対象の Running |

Retry は既存 attempt の状態を書き換えず、新しい NodeExecution を作る。過去 attempt は決着状態と Artifact を保持する。resume が command の再実行を必要とする場合も、観測可能な新しい attempt として記録する。

## 操作と状態の受理マトリクス

### 実行木全体

| 操作 | Running | Completed | Aborted |
| --- | --- | --- | --- |
| stop | 実行中で stop 可能な Node があれば受理 | 拒否 | 拒否 |
| resume | resume 対象 Node があり recovery fence が無ければ受理 | 拒否 | 拒否 |
| abort | 受理して Aborted | 冪等または拒否 | 冪等 |
| archive | active でなければ受理 | 受理 | 受理 |
| start child / advance edge | aggregate 判定で受理 | 拒否 | 拒否 |

### Node

| 操作 | 受理条件 |
| --- | --- |
| approve | Node が WaitingApproval で、必要な completion 前提が成立 |
| retry | Node が Failed、または Submit / provider Stop の片方だけが到着した Running / Paused attempt で、recovery fence が無い |
| close | Session Node の lifecycle が close を受理可能 |
| Submit | 対象 Session attempt が受理可能で、必要なら Artifact が Contract に適合 |
| provider Stop | 対象 Session attempt が未確定である |

stale な Node ID、別 Worktree の Node ID、既に置き換えられた attempt への操作は拒否する。UI 表示はこの matrix と同じ backend capability を使う。

## capability 導出

### workflow root

- `canStop`: Running の葉 Node があり、その runtime が stop を受理できる。
- `canResume`: Paused または Interrupted の再開対象があり、WaitingApproval と解決不能な recovery fence が無い。
- `canAbort`: 木全体が非終端である。
- `canArchive`: execution が active でなく、archive policy を満たす。
- `resumeUnavailableReason`: resumable だが owner-local recovery fence により再開できない場合の理由。

workflow capability は公開実行木 root に関連付ける。root が Sequence、Fanout、葉 Node のどれであっても同じ execution target を使う。

### Node

- `canApprove`: Node が WaitingApproval である。
- `canRetry`: 対象 attempt が Failed、または Submit / provider Stop の片方だけを持つ Running / Paused であり、同じ retry target に新しい active attempt がなく、recovery fence が無い。
- `canClose`: Session lifecycle が close を受理できる。

### 単独 Session root

- `canArchive` / `canDelete` は AgentSession lifecycle と provider session identity から backend が導出する。
- 操作対象は opaque な Session 参照であり、画面上のラベルや Node ID の解析には使わない。
- archive 済み Session は active 実行木から除き、同じ snapshot の archive 一覧に返す。

## failure と interruption

Node failure は原因を `startup_timeout`、`stale_runtime_timeout`、`model_refusal`、`structured_output_mismatch`、`validation_failure`、`user_abort`、`infrastructure_crash` などの canonical kind で記録する。process・storage の喪失だけを infrastructure crash とし、通常の非ゼロ exit codeや Fanout child の業務上の失敗を木全体の crash に読み替えない。

人間への reason は backend が安全な公開文言へ投影する。provider の raw metadata、内部 ID、Fanout 座標を UI に露出しない。

## recovery obligation

```text
pending ── canonical fact 確定 ──► applied
pending ── fact が既存 ─────────► already-applied
pending ── 明示的な断念 ───────► retired(reason)
```

recovery は pending の間だけ新しい外部 effect を fence する。`retired` は理由を必須とし、履歴を消さない。復旧順序が必要な場合も execution 単位で guard し、全体を直列に止めない。

## シナリオ別保証

| Scenario | 保証 |
| --- | --- |
| Submit 後に provider Stop が遅れて届く | 同じ attempt の二信号が揃った時点で completion を評価 |
| provider Stop 後に Submit が届く | 同上 |
| retry 中に同名 Node の loop 再訪がある | 明示 retry relation の chain だけを retry 履歴にする |
| archive 済み単独 Session | active tree から除き archive 一覧と restore 操作を維持 |
| kill -9 / 電源断後 | durable fact と obligation の突合だけで復旧し、結果を推測しない |
| resume 不能 | `canResume=false` と backend-owned reason を返す |
