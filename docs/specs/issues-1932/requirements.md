# Context

- 正本: https://github.com/siro33950/releash/issues/1932
- 参照: `docs/glossary/WORKFLOW.md`（「Session の delegate」）、`docs/architecture/USECASE.md`（「失敗」）、#1890、#1917、#1928、#1930

確定済みの背景・制約:

- delegate は、親 Session が Artifact を提出したときに child を実行し、その結果を同じ provider session へ返す仕組みである。Session は child の実行中も完了せず、結果注入後も同じ NodeExecution・attempt・AgentSession のまま続行する。
- 親 Session の Artifact 提出は、delegate が提出を受け付ける段階にあるときだけ受理される。
- 結果が未注入なら、親 Session Node の Resume 時にその結果を返す。Resume は、Node のプロセスが無いと確認できたときだけ行える。この経路は同じ結果をもう一度 provider へ送る。
- 集約の版が競合したときに読み直して再実行するかは、業務の手順のやり直しとして usecase が決める。
- 作業列は、同じ鍵の処理が終わるまで待つ。
- 作業列は、やり直しても直らない失敗を「要対応」として示し、やり直しが続いている失敗は回数と最初・最後の時刻を伴う記録として示す。競合は手を打たなくても収まれば通るため、後者に当たる。
- 失敗の記録は操作と対象の組で持たれ、読み側は対象の完全一致で引く。画面の Node の行は、workspace tree が作る opaque な id（`node-w-{execution_id}-{sha256(semantic_key)}`）を対象にして購読する。Node の起動や Command の失敗は、生の node execution の id を対象にして記録されている。どの失敗がどの Node の行に属するかは、読み取りの結果を組み立てる判断であり、サーバの query service が持つ。
- #1930 は作業列を usecase から出すこと、および各ドメインの失敗表示の状態を domain から無くすことを扱う。本変更で業務の手順のやり直しが 1 か所増えることは #1930 に記載されている。
- #1917 は delegate の廃止を扱う。

# Outcome

- 対象者: delegate を持つ workflow を実行する利用者。
- 現在の問題: child 完了時の結果の注入が保存の競合で記録されず、provider には結果が届いているのに親 Session が Artifact を再提出できなくなる。再提出は `cannot accept Artifact in its current state` で拒否され、provider のプロセスを終了して Resume するまで作業を進められない。その復旧手段は画面から分からず、同じ結果が provider の会話へ二度届く。
- 変更後に実現する状態: 送付の後に保存が競合しても、送付を繰り返さずに記録の保存だけがやり直され、記録できた時点で Resume なしに次の Artifact 提出が受理される。やり直しが続いている間の失敗は破棄されず、対象の Node から観測できる。

# Current Behavior

main `635ee8d6` 時点で確認した。

- `usecase/workflow/delegate.rs:44` で実行を読み、`:63` で provider を復元し、`:66` で結果を送り、`:76` で読んだ状態を `before` として保存する。読みと保存の間に同じ実行へ別の記録が入ると、保存は競合（`WorkflowRuntimeError::Conflict`）で失敗する。
- 呼び出し元 `adaptor/gateway/workflow/workflow_host.rs:1086-1097` と `:1354-1364` は、競合を警告に出して `continue` する。provider には結果が送られたまま、delegate は注入前の段階に残る。
- `adaptor/gateway/workflow/workflow_host/delegate.rs:67-73` は、起動用のロックを送付と保存の全体で保持する。`run_runtime_activation` を通さないため、ロックを持っている間は Abort の中止要求に応答しない。
- 注入前の段階では `domain/workflow/entities/workflow_execution/delegate.rs:89` が Artifact 提出を受け付けない。
- Resume は Node のプロセスが無いと確認できたときだけ可能（`domain/workflow/value_objects/node_execution.rs:156-157`）。プロセスが動いている間は、注入をやり直す経路（`adaptor/gateway/workflow/workflow_host/node_startup.rs:246`）に到達できない。

観測された再現（2026-09-25、workflow 実行 `8ddbea76-5570-44e8-89c4-c2838c952c0e`、親 Session `planner` の node execution `c3874241-93a8-4a37-9f48-07ee95a8f989`）:

1. child（Sequence `converge`）が完了し、親へ `session_continuation_admitted` が記録される。`delegate_result_injected` は記録されない。
2. 同時刻に、同じ実行へ child 内の別 Session の `process_exited` が記録されている。
3. daemon のログ: `workflow 8ddbea76-...: delegate result injection into c3874241-... was not applied: conflict: execution '8ddbea76-...' changed before commit`
4. 以降、親 Session への Artifact 提出は `--json` / `--file` のいずれも `cannot accept Artifact in its current state` で拒否される。
5. provider のプロセスを終了してから Resume すると結果が再注入され、提出を受け付ける状態に戻る。

同じ実行の中の別の delegate（`implement`）では、`session_continuation_admitted` と `delegate_result_injected` が両方記録されている。

# Scope / Non-goals

変更する:

- delegate の結果注入の業務手順（provider への送付と、注入済みの記録の分離。競合したときの保存のやり直し）。
- 注入の入口と、その組み立て。
- 競合を警告だけで捨てている 2 か所の呼び出し元。
- 失敗の記録を Node の行へ対応付ける読み取り。Node の行の id から、その Node に属する node execution の id を引いて照合する。
- 関連するテスト。

変更しない:

- 失敗の記録の購読の契約と画面。画面は Node の行の id を渡す形のままとする。
- Resume 経路（`resume_session_process`）が注入の失敗を呼び出し元へ返し、親 Node の状態を確定しない形。
- 期限と取り消しの振る舞い（処理を待ってから期限を確かめる形のまま）。
- 共通の仕組み（common、作業列）の追加と変更。
- Node の失敗として精算するかの判定。`settle_runtime_failure_for_node` は具体のエラーの値で競合を弾き、`record_node_start_failure` は domain の分類で弾いており、store 由来の同じ分類の失敗について一致しない。この不一致は今回の対象外とし、別の ISSUE も作らない。
- 送付の後・記録の前にプロセスが落ちたときの二重送信。起動時に注入をやり直すと同じ結果を二度送る可能性があるが、この変更の前からある問題として別に扱う。
- 作業列を usecase から出すこと。#1930 が扱う。
- delegate の廃止。#1917 が扱う。

# Requirements

- R-001: provider への送付が成功した後に保存が競合しても、注入済みの記録が保存される。保存のやり直しで provider への送付は繰り返されない。
- R-002: 注入済みの記録が保存された後、親 Session は provider のプロセスの終了と Resume を要さずに次の Artifact 提出を受理する。
- R-003: 注入済みの記録を保存するとき、同じ実行に記録された他の事実は失われない。
- R-004: 対象の注入が既に注入済みである、親 Session の待っている注入が別のものへ変わっている、workflow が Abort された、のいずれかの場合、古い更新を適用せず、保存のやり直しを終える。
- R-005: 注入済みの記録の保存が競合し、やり直しが続いている間も、workflow の Abort は完了する。
- R-006: 注入済みの記録の保存が競合する間、保存のやり直しが続く。その間の失敗は、回数と最初・最後の時刻を伴う記録として、対象の Node から観測できる。

# Assumptions / Open Questions

- Resume 経路が注入の失敗を呼び出し元へ返す形を変更対象外とすることは、要求の正本が Resume 経路の失敗の返し方を変更対象に挙げていないことから導いた（自動判断）。
