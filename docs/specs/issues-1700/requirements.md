# Context

## 入力文書

- Primary source: [#1700 [workflow] agent の活動状態を誰も所有せず、停止した Session が再開しても表示分類が戻らない](https://github.com/siro33950/releash/issues/1700)（OPEN / label: bug / milestone なし / comment なし）。追加の自由文指示はない。
- 正本: `docs/glossary/DOMAIN.md`（Session と AgentSession の語彙の分離、実行木の状態所有）、`docs/architecture/`（Rust レイヤー規約）。
- 依存（充足済み）: [#1696 単独 Session の実行木に completion signal が届かない](https://github.com/siro33950/releash/issues/1696)。CLOSED、main へ merge 済み（`4fd3e7a69`）。`usecase/provider_lifecycle/ingress.rs` から `tree_parent.is_some()` 分岐は消えており、provider の Stop は木の形に依存せず実行木の事実になる。spec は `docs/specs/issues-1696`。
- 関連（いずれも CLOSED）: [#1683 ツリー行の状態4分類](https://github.com/siro33950/releash/issues/1683)（spec: `docs/specs/issues-1683`）、[#1682 Node 終端時の provider 停止](https://github.com/siro33950/releash/pull/1682)、[#1695 provider Stop 受理の deadlock](https://github.com/siro33950/releash/issues/1695)。
- spec 配置: `docs/specs/issues-1700`。

## 確定済みの背景

`docs/glossary/DOMAIN.md` は次を定めている。

- Session は Node の正規語、AgentSession は provider CLI identity を扱う実装境界の正規語であり、同じ概念ではない（42行目）。
- WorkflowExecution は木全体の `Running` / `Completed` / `Aborted` を所有し、`WaitingApproval` / `Paused` / `Failed` / `Interrupted` と completion signal は NodeExecution が所有する（111行目）。
- AgentSession は provider、provider session identity、transcript reference、Terminal ownership、open / paused / archived lifecycle を持つ。

「agent が今動いているか」を所有する概念はこの一覧に存在しない。Command Node には Session が無く活動状態も存在しないため、Node 状態側に活動を持たせると Command Node が持てない値を Node 状態が持つことになる。

表示4分類（青=実行中 / 黄=介入が必要 / 赤=失敗 / 緑=動いていない）と、親行（Sequence / Fanout）の重大度集約（赤 > 黄 > 青 > 緑）は #1683 が導入した現行の正である。本 ISSUE は #1683 の leaf 分類規則のうち「承認待ち → 黄」「実行中かつ Stop 信号のみ受領 → 黄」の2条件を上書きし、集約規則は据え置く。

Workspace ツリーおよび Workspace ノード詳細は Tauri command 経路でだけ提供される。local API と CLI は返していない。AgentSession 詳細も Tauri command だけが返す。

provider の hook 事情は公式ドキュメントで確認した。

| hook event | Claude Code | Codex CLI |
| --- | --- | --- |
| `SessionStart` / `Stop` | あり | あり |
| `UserPromptSubmit` / `PreToolUse` / `PostToolUse` / `PermissionRequest` | あり | あり |
| `PostToolUseFailure` / `StopFailure` | あり | 記載なし |

Releash が現在登録しているのは Claude が `SessionStart` / `Stop` / `StopFailure` の3種、Codex が `SessionStart` / `Stop` の2種である（`adaptor/gateway/provider_lifecycle/launch_spec.rs`）。受理側も `ProviderLifecycleSignalKind` の3種に閉じており（`domain/provider_lifecycle/value_objects/provider_lifecycle_signal.rs`）、Codex の `StopFailure` は未対応イベントとして拒否される。

# Outcome

対象者は、Workspace ツリーで作業状態を見ながら agent と対話する開発者である。

現在、Session Node の行の色は agent の活動に追従しない。agent が応答を終えて黄になった行は、追加指示を送って agent が再び動き出しても黄のままである。permission 承認や質問で agent が止まっても行は青のままで、実行中と区別できない。承認待ちの Node は agent と対話中でも黄になる。そのため開発者は、ツリーの色から「今 agent が動いているのか」「自分の回答を待っているのか」を判断できず、terminal を開いて中身を見るまで実態が分からない。色が状態を伝えないことで、複数の Node を並行して回しているときに手が空いた Node を見つけられない。

変更後は、Session Node の行の色が agent の活動に追従する。agent が動いている間は青、agent が人の回答（permission 承認、質問への回答）または次の指示を待って止まっている間は黄になり、応答終了と再開のたびに黄と青を往復する。承認待ちであるかどうかは色に出さず、承認操作の可否と Node 詳細が引き受ける。provider プロセスが未完了のまま正常終了しなかった Session は緑ではなく赤になり、正常終了した Session だけが緑になる。開発者はツリーを見るだけで、手が空いている Node と自分の介入を待っている Node を特定できる。

# Current Behavior

## 表示分類の決定規則

ツリー行の色は `WorkspaceTreeNode::classify_own_status`（`domain/workspace_tree/value_objects/mod.rs`）が決める。入力は Node 状態、completion signal 状態、recovery fence の有無の3つである。

| 分類 | 条件 |
| --- | --- |
| 赤（Failure） | recovery fence あり、または Node 状態が `Failed` |
| 黄（Attention） | Node 状態が `Waiting`（承認待ち）、または `Running` かつ completion signal が `StopReceived` |
| 青（Active） | Node 状態が `Running` |
| 緑（Idle） | 上記以外（`Completed` / `Paused` / `Aborted`） |

分類は runtime snapshot 反映のたびに再計算される（`domain/workspace_tree/projection.rs` の `recompute_status_classifications`）。ツリー DTO はこの4分類だけを `status` として返し（`adaptor/gateway/workspace_tree/query_service.rs`）、詳細 DTO が詳細状態と4分類の両方を返す。

## 症状1: 黄から青へ戻らない

再現手順:

1. Session Node（workflow の子、または単独 Session）で agent に指示を出す。
2. agent が応答を終えるまで待つ。
3. terminal から追加指示を送り、agent が再び動き出したことを terminal の出力で確認する。
4. ツリーの当該行の色を見る。

実際の出力: 手順2で行は青から黄へ変わり、手順3以降も黄のまま変わらない。

黄を決めているのは `completion_signals == StopReceived` である。completion signal 状態は `Pending` → `SubmitReceived` / `StopReceived` → `Ready` の単調遷移しか持たず、`StopReceived` から `Pending` へ戻る遷移が存在しない（`domain/workflow/value_objects/node_execution.rs`、`domain/workflow/entities/workflow_execution/mod.rs` の `record_completion_signal`）。解けるのは Submit が来て `Ready` になったときだけで、`Ready` になった Session Node は完了へ進むため、対話継続中に青へ戻る経路がない。

加えて、agent が再び動き出したことを表す事実が Releash に入らない。登録済み hook は `SessionStart` / `Stop` / `StopFailure` だけで、agent への追加指示は terminal surface への書き込みとして流れるだけであり、事実にならない。

## 症状2: permission 待ち・質問待ちが青のまま

再現手順:

1. permission が `Manual` の Session Node で、agent に承認が必要な操作を伴う指示を出す。
2. provider の承認ダイアログが terminal に表示された状態で、ツリーの当該行の色を見る。

実際の出力: 行は青のままである。`PermissionRequest` に相当する hook を登録していないため、agent が止まって人の回答を待っている事実が入らず、Node 状態は `Running`、completion signal は `Pending` のままである。`Stop` が届くまで実行中に見える。

## 症状3: 承認待ちの Node が対話中でも黄

再現手順:

1. `completion` が承認を要求する Session Node で agent が Artifact を提出し、Node が承認待ちになる。
2. 承認せずに terminal から追加指示を送り、agent との対話を続ける。
3. ツリーの当該行の色を見る。

実際の出力: 手順1で Node 状態が `Waiting` になり行は黄になる。手順2以降も黄のままである。承認しないまま対話を続けるのは通常の使い方だが、この間も介入待ちとして表示される。

## 症状4: 異常終了した Session が緑

`NodeFact::ProcessExited` の replay は、Command Node では exit code で分岐して 0 なら完了、非 0 なら失敗、不明なら中断としているのに対し、Session Node では `failure_reason` を見ず、Node が実行中かつ completion signal が `Pending` / `SubmitReceived` のときに一律 `Paused` を導出する（`domain/workflow/services/fact_replay.rs` の `NodeFact::ProcessExited` 分岐）。その結果、provider プロセスがクラッシュした Session Node も緑（動いていない）として表示され、意図して止めた Session と区別できない。

区別に必要な事実は既に記録されている。AgentSession の停止記録は `last_exit_abnormal` に応じて `ProcessExitedFact.failure_reason` を埋め（`adaptor/gateway/agent_session/agent_session_repository.rs`）、Node 完了に伴う正常停止は `stop_for_terminal_execution_tree_node` が `last_exit_abnormal = false` を立てる（`domain/agent_session/aggregates/agent_session.rs`）。

## 既に存在する別の「活動」表現

AgentSession の読み取りモデルには `activity`（`running` / `idle`）が既にある。これは terminal surface の最終出力からの経過が3秒未満かどうかで読み取り時に導出する値であり（`domain/terminal_surface/value_objects/terminal_activity.rs` の `TerminalActivity::classify`、`usecase/agent_session/agent_session_read.rs` の `with_activity`）、事実として永続化されない。permission 待ちと入力待ちを区別せず、どちらも `idle` になる。この値は Tauri command 経由で frontend の型（`src/types/agent-session.ts`）まで届いているが、production の component はこれを一切参照しておらず、表示に使われていない。Workspace ツリーの色もこの値を読んでいない。

# Scope / Non-goals

## Scope

- AgentSession が持つ、provider の活動状態（agent が動いている / 人の回答を待っている / 次の指示を待っている）の所有と、その状態遷移を表す事実の記録。
- Claude Code と Codex CLI に登録する hook の種類、および受理側が扱う provider lifecycle signal の種別。
- Session を持つ Node の表示4分類の導出規則。Node 状態と Session の活動状態の両方を入力にする。
- Session Node の `ProcessExited` から導出する Node 状態を、正常終了と異常終了に分けること。
- 正常終了しなかった Session Node の resume 可否。
- AgentSession の読み取りモデルが持つ既存の `activity`（terminal 出力の recency から読み取り時に導出する `running` / `idle`）の削除。
- 上記に伴う Workspace ツリー、Workspace ノード詳細、AgentSession 詳細の外部インターフェースの応答。

## Non-goals

- 表示4分類の意味（青=実行中 / 黄=介入が必要 / 赤=失敗 / 緑=動いていない）の変更。
- 親行（Sequence / Fanout）の重大度集約規則の変更。#1683 の規則を維持する。
- Command Node の表示分類の変更。
- NodeExecution が所有する completion signal の変更。Submit と Stop の2信号で Session Node の完了条件を判定する規則、および両信号の受領記録は現行どおりとする。
- AgentSession の `open` / `paused` / `archived` lifecycle の変更。活動状態は lifecycle を置き換えない。
- approve / retry / stop / abort / archive の操作可否判定、および resume 不能理由の変更。resume は R-021 が定める範囲でのみ変わる。
- provider 自身の承認 UI、質問 UI の変更。Releash 側が承認や回答を代行しない。
- terminal、Artifact、承認機能そのものの変更。
- 本変更以前に event store へ記録された実行木の移行、および後方互換の読み取り。
- local API と CLI の応答。Workspace ツリー、Workspace ノード詳細、AgentSession 詳細のいずれも返していない。

# Requirements

- R-001: Node が実行中または承認待ちの Session Node は、その Session で観測された活動状態が `Working` である間、ツリー行が青（実行中）になる。
- R-002: agent が応答を終えて次の指示を待っている間、Session Node のツリー行は黄（介入が必要）になる。
- R-003: 黄になった Session Node は、ユーザーが追加指示を送って agent が再び動き出したとき青へ戻る。応答終了と再開を繰り返す間、行は黄と青を往復する。往復の回数に上限はない。
- R-004: agent が permission の承認または質問への回答を待って止まっている間、Session Node のツリー行は黄になる。provider の承認ダイアログまたは質問が表示されている間に青にならない。
- R-005: 承認待ちの Session Node は、agent が動いている間は青になる。承認しないまま対話を続けても黄にならない。その Node が承認を要求していることは、承認操作の可否と Node 詳細から引き続き判別できる。
- R-006: Session Node のツリー行の色は completion signal の受領状況に依存しない。Stop を受領済みでも agent が動いていれば青になり、Stop を未受領でも agent が止まっていれば黄になる。
- R-007: Node が失敗している、または recovery fence を持つ Session Node のツリー行は、Session の活動状態にかかわらず赤になる。Node が完了・中止・停止に達している Session Node は、Session の活動状態にかかわらず緑になる。
- R-008: provider プロセスが未完了のまま正常終了しなかった Session Node のツリー行は赤になる。provider プロセスが正常終了した実行中（承認待ちではない）の Session Node、および Node の完了に伴って Releash が停止した Session Node は緑になる。承認待ちの Session Node は provider プロセスが正常終了しても承認待ちのまま残り、agent が次の指示を待っているため R-002 と同じく黄になる。
- R-009: Command Node のツリー行の色は、この変更の前後で変わらない。実行中は青、承認待ちは黄になる。
- R-010: 親行（Sequence / Fanout）のツリー行の色は、自分自身と配下の子の分類を合わせた最も重い分類になる。この集約規則は変更の前後で変わらない。
- R-011: R-001 から R-008 の振る舞いは、Claude Code の Session と Codex CLI の Session の双方で同じように成立する。
- R-012: R-001 から R-008 の振る舞いは、Session Node が実行木の root であるか child であるか、およびその実行木が workflow の実行として起こされたか Session の起動として起こされたかに依存しない。
- R-013: Releash は agent の活動状態を観測するために provider の承認判定へ介入しない。permission の承認可否は provider 自身が決め、承認 UI は現行どおり表示される。
- R-014: 記録される活動状態の事実の件数は、実際に起きた活動状態の遷移の回数と等しい。同一の活動状態を繰り返し観測しても事実は追記されない。Stop に伴う `AwaitingInstruction` への遷移は既存の `StopReceived` 事実が担い、同じ遷移を表す活動観測の事実を重ねて追記しない。
- R-015: アプリケーションを再起動しても、記録済みの活動状態から R-001 から R-008 と同じ色が再現される。
- R-016: Session Node の完了判定は現行どおり Submit と Stop の2信号が揃うことを条件とする。活動状態は完了判定に影響しない。
- R-017: approve / retry / stop / resume / abort / archive の操作可否、および resume 不能理由の内容は、Node 状態の導出が変わることによる影響と R-021 が定める resume を除き、この変更の前後で変わらない。活動状態は操作可否の入力にならない。
- R-018: AgentSession の archive / restore / delete / GC / initial instruction の受理可否は、現行どおり `open` / `paused` / `archived` の lifecycle に従う。活動状態はこれらの判定に影響しない。
- R-019: local API と CLI の応答は、この変更の前後で変わらない。
- R-020: Session が起動してから最初の活動が観測されるまでの間、Session Node のツリー行は R-002 と同じく黄になる。停止した provider プロセスを resume した直後も同じ扱いとする。
- R-021: provider プロセスが正常終了しなかった Session Node は resume できる。resume の提示と受理は、その実行木が workflow の実行として起こされたか Session の起動として起こされたかに依存しない。停止している provider プロセスは既存の provider session を使って復旧し、復旧が成立した場合にだけ Node を実行中へ戻す。provider の復旧に失敗した場合は resume を失敗として返し、Node を resume 前の失敗状態に保つ。provider が動作中の Paused Session Node は、provider を再起動せず現行どおり Node を実行中へ戻す。
- R-022: AgentSession の応答は、terminal 出力の recency から導出する `activity` を返さない。

# Assumptions / Open Questions

## Assumptions

なし。

## Open Questions

なし。
