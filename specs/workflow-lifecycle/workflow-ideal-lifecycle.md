# Workflow execution ライフサイクルの理想形

作成日: 2026-07-27

本書は、WorkflowExecution、NodeExecution の canonical fact、workflow obligation、起動時復旧、workflow-owned session への送信、capability が、正常、failure、crash、restart の各経路で守る lifecycle invariant を定義する。型、schema、内部処理順は定義しない。W-I10 のみライフサイクルの表現主体(どの層の何がライフサイクルを表現するか)を定めるモデリング原則であり、これも特定の型形状ではなく責務の所在を契約とする。

現行エンジン(6値 ExecutionStatus)と統一 Node モデル(milestone 86)の実行木の両方に同じ invariant を適用する。エンジン置換時は状態語彙の対応(ExecutionStatus → Node 実行木の状態)を確定した上で本書を維持する。

関連正本:

- [agent-chat-ideal-lifecycle.md](../milestone-84-agent-chat-stabilization/agent-chat-ideal-lifecycle.md)
- [agent-chat-ideal-vocabulary.md](../milestone-84-agent-chat-stabilization/agent-chat-ideal-vocabulary.md)
- [workflow-engine-evolution-plan.md](../../docs/workflow-engine-evolution-plan.md)
- [workflow-engine-model-boundary.md](../../docs/workflow-engine-model-boundary.md)
- [unified-node-model/decisions.md](../unified-node-model/decisions.md)

## 正本分担

- モデル構造境界(WorkflowDefinition と WorkflowExecution の分離、state owner、互換性境界)は workflow-engine-model-boundary.md が正本。
- 実行時ライフサイクル不変条件(W-I 群)と運用形(遷移表、受理マトリクス、復旧カタログ、送信可否表、分類対応表、capability 導出、obligation 遷移)は本書が正本。
- agent session 自体の lifecycle と、obligation / pending recovery / recovery action の一般語彙は milestone 84 正本に従う。本書は workflow が session に関与する境界(WorkflowTurn 送信可否、turn 完了事実の受領、workflow obligation)だけを定める。

## 状態語彙

WorkflowExecution の status は3つの状態集合に分割される。

| 集合 | status | 意味 |
| --- | --- | --- |
| active | running、waiting_approval | live runtime が存在してよい実行中 |
| resumable | interrupted | live runtime を持たない、再開可能な checkpoint |
| finished | completed、failed、aborted | 終端 |

`interrupted` は再開可能な checkpoint であり、既知の観測結果の受け入れを拒む終端ではない。

## 不変条件

### W-I1: 状態集合の明示受理

WorkflowExecution を対象とするすべての操作・復旧・通知適用は、受理する状態集合を active / resumable / finished の3分割語彙で宣言する。2値判定(active か否か)で resumable と finished を同一視しない。受理外状態への到達はサイレント無視ではなく、定義された outcome(AlreadyApplied / NotApplicable / 明示拒否)として着地する。

### W-I2: turn 完了事実の順序非依存な確定

実際に起きた turn の canonical outcome は、interrupt 事実との durable 化順序に依存せず確定できる。completion fact の適用は状態集合ごとに閉じた値域を持つ: active → live 適用 / resumable → record-only 適用(canonical node fact の確定と obligation 消費のみ。live runtime を復活させず、workflow を進行させない。resume 時の projection が確定済み node として再利用する)/ finished → superseded としての終端化。

### W-I3: 復旧の収束保証

すべての起動時復旧項目は有限回の試行で terminal outcome(applied / already-applied / retired)に到達する。transient 失敗(storage I/O 等)と permanent 失敗(対象定義の欠落、provider effect なしでは再生不能、projection 破損)をエラー型で区別し、permanent は retire して復旧キューから外す。無限リトライは transient にのみ許す。

### W-I4: 復旧項目の独立性

復旧は item ごとに独立に成功・失敗し、1項目の恒久失敗が同一パス内の他項目や後段フェーズ(orphan 復旧)を巻き添えにしない。「turn-completion が replay されるまで orphan-interrupt しない」ガードは全体の直列ブロックではなく execution 単位で表現する。復旧の順序・前提は本書が所有し、コードコメントのみに置かない。

### W-I5: 送信可否は workflow lifecycle が所有

workflow-owned session へ turn を送れるかは workflow 側の実行文脈(node が initial / repair / 継続 turn を送る正当な checkpoint にあるか)が決める。session 側 gate が検査してよいのは「終端(Closed / Archived)でない」「未解決 recovery がない」「quiescent(実行中・待機中 turn がない)」のみ。`SessionState::Done` は「直近 turn が正常完了した」という履歴 projection であり、送信拒否の根拠にしない。送信可否は単一の述語として定義され、複数の gate が別々の状態解釈を持たない。

### W-I6: 失敗分類の忠実性

失敗は発生地点の意味(業務的拒否 / 一時的失敗 / 恒久インフラ喪失)を境界を越えて保持する。typed failure を文字列化して包括変種に落とさない。この原則は failure_kind と `ExecutionInterruptionReason` の両方に適用する: `crash` / `infrastructure_crash` はプロセス・ストレージの喪失にのみ使い、「分類不能な非ゼロ exit_code」「fanout の部分失敗」「graceful shutdown 中の command cancel」「admission gate の業務的拒否」の総称にしない。retryable と宣言する失敗には、少なくとも一つの実効的な再試行経路を対応させる。

### W-I7: capability の正直さ

UI へ公開する capability(canResume / canStop / canAbort)は「操作が実際に受理される前提条件」から導出し、既知の未解決 obligation / recovery を反映する。押すと必ず失敗するボタンを出さない。解決不能な場合は理由付きで resume 不可を明示する(unified-node-model の worktree 台帳突合が確立した「resume 不可を明示」パターンと同型)。

### W-I8: 事実の忠実性

復旧・再開・通知適用は durable event と既知 observation のみから状態を再構築し、結果不明の turn / node を成功・未開始・失敗へ推測で倒さない。観測済みの turn 結果から canonical node fact を導出せずに離脱しない。適用できなかった completion fact を無言で捨てない(破棄するなら retired として記録する)。graceful shutdown と crash は別 intent であり、graceful 経路が crash 事実を durable 化してはならない。

### W-I9: workflow obligation の閉じたライフサイクル

workflow が関与する durable obligation(turn-completion / shutdown)は milestone 84 の Durable obligation 語彙(owner、purpose、現在状態、安全な observation、利用可能な action)に従い、pending → (applied | already-applied | retired) の閉じた遷移を持つ。effect admission のブロックは「pending である間」だけ有効で、retire により解除される。obligation の存在と遷移は本書に記述され、暗黙機構にしない。

### W-I10: ライフサイクルの Entity 表現とエンジンの domain 所在

workflow engine とは、ライフサイクル(状態・遷移・受理・不変条件)を自身のメソッドとして表現する WorkflowExecution 集約と、実行進行の決定を行う domain service 群(次 node 決定、failure policy 適用、fanout 展開、approval 解決)の総体であり、domain に存在する。状態遷移は集約のメソッド経由でのみ起き、フィールドの直接書き換えによる遷移経路を持たない。操作×状態 受理マトリクスは各遷移メソッドの事前条件と outcome の仕様である。turn 完了事実の確定は「観測結果を渡すと終端 fact を含む決定が返る」単一の遷移として表現され、fact なしの中間状態は集約上存在できない。usecase はエンジンを駆動する手順(観測事実の受領、集約への適用、決定の永続化と副作用の指示、トランザクション境界)を所有し、adaptor は外界(event store、agent session runtime、通知)への接続だけを持つ。engine を名乗る型・モジュールが adaptor 層に存在しない。執行層を engine と呼ばない。

## 運用形

### ExecutionStatus 遷移表

| イベント | 適用可能な状態 | 遷移先 |
| --- | --- | --- |
| execution_started | (新規) | running |
| approval_requested | running | waiting_approval |
| approval_resolved | waiting_approval | running |
| execution_completed | active | completed |
| execution_failed | active | failed |
| execution_aborted | active、interrupted | aborted |
| execution_interrupted(reason) | active(interrupted への再適用は冪等) | interrupted |
| execution_resumed | interrupted | running |
| node / fanout / stall / artifact 系イベント | active | status 不変(NodeExecution 側の fact) |

適用可能でない状態へのイベントは replay においても無言で捨てず、W-I1 の定義された outcome(記録上の矛盾としての顕在化を含む)として扱う。

### 操作×状態 受理マトリクス

各セルは集約の遷移メソッドの事前条件と outcome の仕様である(W-I10)。「明示拒否」は理由を返す受理前 rejection、「NotApplicable」は定義済みの正常 no-op。空欄(未定義)を持たない。

| 操作 / 適用 | active | resumable | finished |
| --- | --- | --- | --- |
| stop | 受理 → interrupted(stop) | 明示拒否(停止済み) | 明示拒否 |
| resume | 明示拒否 | 受理 → running | 明示拒否 |
| abort | 受理 → aborted | 受理 → aborted | NotApplicable(冪等) |
| approve / reject | waiting_approval のみ受理 | 明示拒否 | 明示拒否 |
| Artifact submit | 対象 node が受理可能なときのみ受理 | 明示拒否 | 明示拒否 |
| turn 完了事実の適用 | live 適用 | record-only 適用 | superseded 終端化 |
| orphan-interrupt(起動時) | 受理 → interrupted(orphan) | NotApplicable(metadata reconcile のみ) | NotApplicable(metadata reconcile のみ) |
| WorkflowTurn 送信 | workflow 文脈が正当なときのみ受理 | 明示拒否 | 明示拒否 |
| stall 観測 | 記録 | NotApplicable | NotApplicable |

### turn 完了事実の確定

turn 完了通知を受けたら、execution をどう遷移させるかの判断より前に、observed result から canonical node fact(node_completed / node_failed)を必ず導出し durable 化する(W-I8)。interrupt はその後に続く別の fact である。fanout child の失敗はまず child の canonical fact であり、execution 全体をどうするかは fanout の failure disposition が決める。execution の interrupt(crash)は真のインフラ喪失に限る(W-I6)。

### interruption reason × 発生源

| reason | 唯一の発生源 |
| --- | --- |
| crash | プロセス・ストレージの喪失を自己観測した実行継続不能 |
| stale | 実行中 runtime の応答期限超過 |
| stop | 利用者の明示停止 |
| orphan | 起動時に前プロセスの active 残骸として発見 |

禁止: 分類不能な非ゼロ exit_code、fanout の部分失敗、graceful shutdown 中の command cancel を crash として記録しない。child turn の失敗理由を execution の interrupt 理由へ転写しない。

### 起動時復旧カタログ

| 項目 | 対象 | per-item outcome | permanent 判定 |
| --- | --- | --- | --- |
| workflow turn-completion 復旧 | pending な turn-completion obligation | applied / already-applied / retired(reason) | 対象定義の欠落、provider effect なしで再生不能、projection 破損 |
| orphan 復旧 | 非終端 metadata の execution | interrupted(orphan) 化 / event log との reconcile / stale 予約の除去 | — |

- 順序は turn-completion → orphan。ガードは execution 単位: execution X の turn-completion が未解決の間、X を orphan-interrupt しない。他 execution の復旧には影響しない(W-I4)。
- item の失敗は同一ページ・同一パスの他 item を止めない。
- リトライは transient にのみ許す(W-I3)。復旧の完了は bounded な quiescence 判定による。

### SessionState × WorkflowTurn 送信可否

送信可 = open ∧ quiescent ∧ 未解決 recovery なし。quiescent は SessionState projection ではなく実 turn 状態(実行中 turn なし ∧ 待機 turn なし)から判定する。この述語は単一定義である(W-I5)。

| SessionState | open | 備考 |
| --- | --- | --- |
| Idle | ✓ | 初回 turn の既定状態 |
| Active | ✓ | open だが quiescent でないため実質待機 |
| Done | ✓ | 直近 turn 正常完了の履歴。repair・継続 turn の正当な対象 |
| Error | ✓ | 直近 turn 異常終了の履歴。現行の送信元には現れない(open として扱い、送るかどうかは workflow 文脈が決める) |
| Closed | ✗ | 終端 |
| Archived | ✗ | 終端 |

### failure_kind × 発生源

| kind | 発生源 |
| --- | --- |
| startup_timeout | runtime 起動の期限超過 |
| stale_runtime_timeout | 実行中 runtime の応答期限超過 |
| model_refusal | provider の明示的拒否 |
| structured_output_mismatch | 必須 Artifact の未提出・schema 不一致(repair 経路の送信失敗を含む) |
| validation_failure | workflow 定義・入力の検証失敗 |
| user_abort | 利用者中断 |
| infrastructure_crash | プロセス・ストレージの喪失のみ |

### capability 導出規則

- canStop = active
- canAbort = active ∨ resumable
- canResume = resumable ∧ 所有 session に解決不能な未解決 recovery がない
- canResume が false かつ resumable のとき、その理由を read model に含める(W-I7)
- capability は「操作が実際に受理される前提条件」から導出する。受理されない操作を可能と表示しない

### workflow obligation の遷移

```text
pending ── canonical fact の確定と同時 ──→ applied
pending ── fact が既に存在 ─────────────→ already-applied
pending ── retire(reason) ──────────────→ retired
              reason ∈ { superseded, unrecoverable }
```

- superseded: 事実は既に決着しており適用が不要(finished な execution への late completion 等)。
- unrecoverable: provider effect なしには canonical outcome を導出できず断念(Artifact 未提出の成功 turn、projection 破損等)。
- effect admission のブロックは pending の間のみ。retired は履歴として残り、reason に応じて提示される。
- retire は milestone 84 の Recovery action「Keep for manual resolution」からの明示解決に対応する durable な遷移であり、情報を失わない。

### unrecoverable retire を含む execution の resume

resume は、unrecoverable として retire された attempt の存在と「前回 attempt の外部作用(worktree 変更等)が未確定であること」を利用者に提示し、明示的な進行指示を要件とする。進行指示後、当該 node は新しい attempt として再実行される。無警告での自動再実行を行わない(W-I8)。

## シナリオ別保証

| Scenario | Observable guarantee |
| --- | --- |
| fanout child が exit_code≠0 で turn 完了 | child の canonical fact が durable 化され、obligation は同一処理内で消費される |
| turn 完了事実の確定前に execution が interrupted 化 | completion fact は record-only 適用で確定し、resume が確定済み node として再利用 |
| 復旧不能な obligation が残存 | 有限回で retired(reason) に到達し、resume・orphan 復旧・session 操作を塞がない |
| アプリ終了時に fanout 実行中 | graceful shutdown は crash 事実を書かず、次回起動の orphan 復旧が interrupted(orphan) へ導く |
| 必須 Artifact 未提出で turn 正常完了 | repair turn が送信される。上限到達時は structured_output_mismatch で失敗する |
| resume 不能な execution | canResume は false になり、理由が read model から観測できる |
| kill -9 / 電源断後の再起動 | 復旧は durable event と obligation の突合のみで進み、結果不明を推測しない |

## トレーサビリティ

| Problem | Invariants |
| --- | --- |
| [#1557](https://github.com/siro33950/releash/issues/1557)(repair turn 送信不能) | W-I1、W-I5、W-I6 |
| [#1558](https://github.com/siro33950/releash/issues/1558)(turn-completion 復旧恒久失敗) | W-I1〜W-I4、W-I7〜W-I9 |
| [#1559](https://github.com/siro33950/releash/issues/1559) 根本原因1(状態集合の暗黙前提) | W-I1、W-I5 |
| 同 根本原因2(復旧の収束・独立性) | W-I3、W-I4 |
| 同 根本原因3(失敗分類) | W-I6、W-I8 |
| 同 根本原因4(capability) | W-I7 |
| 同 根本原因5(仕様の空白) | 本書全体 |
| 同 根本原因6(エンジンの所在) | W-I10 |

## 設計判断

- **W-D1**: obligation の retire は単一状態 retired + reason 必須(superseded / unrecoverable)とし、別状態に分裂させない。
- **W-D2**: unrecoverable retire を含む execution の resume は、提示と明示的な進行指示を要件とする。
- **W-D3**: 正本分担は「モデル構造境界 = workflow-engine-model-boundary.md / 実行時ライフサイクル不変条件 = 本書」。model-boundary の不変条件「engine 以外が workflow state transition を決めない」は、W-I10 により「集約以外が state transition を決めない(engine = domain の集約 + 決定サービス群。執行層は遷移を判断しない)」へ精密化される。
- **W-D4**: 集約のメソッド契約(W-I 準拠の遷移群と受理マトリクス)を安定境界とし、内部表現は現行モデル(6値 status)のまま。milestone 86 移行時は内部表現だけを実行木へ差し替え、本書と契約を維持する。
- **W-D5**: contract violation の write-ahead(送信失敗でも repair_attempt が消費される)は現状維持で開始し、attempt 消費を送信受理に紐付ける変更は将来の course-correct として open にする。
- **W-D6**: 復旧順序「turn-completion → orphan」は execution 単位ガードとして維持し、全体を直列にブロックしない。
