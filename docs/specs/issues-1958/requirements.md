# Context

- 要求の正本は GitHub Issue #1958「fix(workspace): Delegate の親が子の実装中も黄になる — 状態の色を「次に誰が動く番か」の3値に作り直す」（https://github.com/siro33950/releash/issues/1958 、milestone なし、comment なし）。
- この ISSUE は #1683（`docs/specs/issues-1683/`）で決めた4色の規則を置き換える。#1683 の「赤＝失敗」は廃止し、失敗は黄に統合する。`docs/specs/issues-1683/` は過去の記録であり、改訂しない。
- #1726（Codex の `permission_request` で回答待ちに張り付く件）は、回答待ちの検出が正しいかどうかの問題であり、この変更とは別に扱う。
- #1917（Delegate の廃止）では、親の Node が完了して agent だけが残る。この変更の規則は、その状態にもそのまま当てはまる形にする。
- 表示状態の導出はサーバ（daemon）が所有する。client は受け取った状態を色として表示するだけにする（`AGENTS.md` アーキテクチャ原則）。
- この文書でいう「失敗」は、現行で `Attention` になる失敗の事実を指す。具体的には、裏で起きた失敗の記録（`background_failure`）と、Command Node の実行中のプロセス消失（`NodeProcessPresence::ConfirmedAbsent`）である。どの失敗を記録として表に出すかの選別（`usecase/failure.rs:140-146` の `requires_attention`）は変えない。

# Outcome

Releash の Workspace ツリーを見て workflow と Session を監督する開発者が対象である。

今の黄は「人の操作が要る」を意味していない。Stop しただけの Session、プロセスが消えた Node、裏で起きた失敗が、すべて同じ黄になる。Delegate の親は Stop すると、子が実装中でも黄になる。そのため開発者は、黄を見ても自分が操作すべきものなのか、ただ止まっているだけなのかを区別できない。

変更後は、行の色が「次に誰が動く番か」を表す3値になる。黄は人の番、青はシステムの番、緑は誰の番でもない。Delegate の親は子が動いている間は青になり、黄は人が操作すべき箇所だけに出る。

# Current Behavior

## 分類の値

- 分類は `WorkspaceNodeStatusClassification` の `Active` / `Attention` / `Idle` / `Unbound` の4値である（`src-tauri/src/domain/workspace_tree/value_objects/mod.rs:53-87`）。proto の `WorkspaceStatusClassification` も `active` / `attention` / `idle` / `unbound` の4値である（`proto/client.proto:2548-2558`）。
- 画面の色は、青＝`active`、黄＝`attention`、緑＝`idle`、灰＝`unbound` である（`src/components/workspace/WorkflowNodeStatusIcon.tsx:8-16`）。`unbound` の leaf 行は、灰色の回転するアイコンで表示される（`src/components/workspace/WorkspaceList.tsx:213-224`）。
- `Unbound` は重さの順で最も軽い。Sequence と Fanout の子がすべて `Unbound` なら、親も `Unbound` になる（`src-tauri/src/domain/workspace_tree/entities/mod.rs:1013-1063`）。

## Node 自身の分類

`classify_own_status`（`src-tauri/src/domain/workspace_tree/value_objects/mod.rs:158-194`）が、次の順で最初に当てはまったものを採る。

1. 裏で起きた失敗がある → `Attention`
2. 完了・中断 → `Idle`
3. Session / Command でプロセスが消えている（`ConfirmedAbsent`） → `Attention`
4. Session が紐づく前 → `Unbound`
5. Session は agent の activity で決まる。`Working` → `Active`、`AwaitingAnswer` と `AwaitingInstruction` → `Attention`
6. 承認待ち → `Attention`
7. それ以外 → `Active`

Stop を受け取ると activity は `AwaitingInstruction` になる（`src-tauri/src/domain/workflow/value_objects/node_fact.rs:289-298`）。そのため、Stop しただけの Session は黄になる。

## 子の集約

- 子の分類を親に集約するのは Sequence と Fanout だけである（`src-tauri/src/domain/workspace_tree/entities/mod.rs:1024-1027`）。Session と Command は、裏で起きた失敗の `Attention` だけを子から受け取る（同 1047-1051）。
- Delegate の子は、親の Session Node の子として同じツリーに置かれる（`src-tauri/src/domain/workspace_tree/entities/mod.rs:385-419`）。しかし Session は子を集約しないので、子が動いていても親の色には反映されない。
- Delegate の親は、Artifact を提出して Stop した後、子の完了を待つ（`DelegatePhase::WaitingChild`、`src-tauri/src/domain/workflow/entities/workflow_execution/delegate.rs:11-19`）。この段階はツリーの分類に使われていない。判定関数 `delegate_waits_for_child`（同 34-42）は `#[cfg(test)]` である。

再現: Delegate を持つ Session Node が Artifact を提出して Stop し、子の Session が `Working` の間、親の行は `Attention`（黄）、子の行は `Active`（青）になる。

## Workflow の外で直接起動した Session

- Workflow の外で直接起動した Session は、Session Node を一つだけ持つ実行木として記録される（`src-tauri/src/domain/workflow/value_objects/node_fact.rs:195-221`、`launched_as: Session`）。
- Session Node は、Submit と Stop の両方を受け取るまで完了しない（`src-tauri/src/domain/workflow/entities/workflow_execution/mod.rs:240-263`）。Workflow の外の Session も、両方がそろったときにだけ自動で完了する（同 2755-2760）。そのため、Stop した後も Submit を待ち続ける。

## Session Node のヘッダー

Session Node の詳細のヘッダーは、`WorkflowNodeStatusIcon` で状態アイコンを表示している（`src/components/panels/NodeContentView/NodeContentView.tsx:159-165`）。このアイコンは詳細 DTO の `statusClassification`（`proto/client.proto:1626`、`src-tauri/src/usecase/workflow/workspace_tree.rs:143`）を色に使う。`src` の中で `statusClassification` を読むのは、このアイコンだけである。

# Scope / Non-goals

## Scope

- ツリー行の表示状態を、黄・青・緑の3値で導出し直す。
- Session と Node がそれぞれ出す色の規則と、行の色の集約の規則。
- Delegate の親が子の完了を待っている段階を、表示状態の導出に使う。
- Workflow の外で新しく起動する Session の Node を、完了した状態で起動する。
- ツリー DTO と proto の表示状態を3値にする。
- Session Node の詳細のヘッダーの状態アイコンを削除する。

## Non-goals

- すでに記録されている Workflow の外の Session で、Node が Submit を待ったまま完了していないものの記録の移行。これらは記録どおり、Submit 待ちの Node として扱う。
- 回答待ちの検出の正しさ（#1726）。
- どの失敗を記録として表に出すかの選別（`requires_attention`）。
- Delegate の廃止（#1917）。
- Session Node の詳細のヘッダーにある、Submit・Stop の待ちの文言とプロセスの有無の表示。

# Requirements

- R-001: ツリー行の表示状態は、黄（人の番）・青（システムの番）・緑（誰の番でもない）の3値だけで表される。ツリー DTO と proto の表示状態にも、この3値だけが載る。失敗の赤と、Session が紐づく前の灰は、独立した値として存在しない。
- R-002: Session が出す色は、Node の状態に関係なく、agent の状態だけで決まる。紐づく前と動作中は青、回答待ち（質問・許可）は黄、Stop した状態は緑である。
- R-003: Node が出す色は次のとおりである。承認待ちと失敗は黄。agent の作業を待っているのに agent が止まっている（Stop したが Submit がない、またはプロセスが消えた）場合は黄。Command が実行中なら青。完了・中断は緑。Delegate の親が子の完了を待っている間は、Node 自身は色を出さない。
- R-004: 行の色は、Node が出す色、紐づく Session が出す色、子の行の色のうち、最も重いもの（黄 ＞ 青 ＞ 緑）である。Session・Sequence・Fanout の行のすべてに同じ規則が使われる。
- R-005: Workflow の外で新しく起動する Session は、Node が完了した状態で起動する。その行の色は、Session が出す色だけで決まる。Stop した後とプロセスが消えた後は緑になる。
- R-006: Session Node の詳細のヘッダーに、状態アイコンは表示されない。

# Assumptions

なし
