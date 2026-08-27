# Context

## 入力文書

- Primary source: [#1696 [workflow] 単独 Session の実行木に completion signal が届かず、アイコンが青（実行中）から一切遷移しない](https://github.com/siro33950/releash/issues/1696)（OPEN / label: bug）
- 正本: `docs/glossary/DOMAIN.md`（実行木の構造、状態所有、使用禁止語）
- 正本: `docs/architecture/`（Rust レイヤー規約、依存方向）
- 依存元: [#1700 [workflow] agent の活動状態を誰も所有せず、停止した Session が再開しても表示分類が戻らない](https://github.com/siro33950/releash/issues/1700)（OPEN / label: bug）。本 ISSUE を依存として明記する後続 ISSUE であり、agent の活動状態の所有、turn の開始を示す hook の追加、活動に応じた表示分類の復帰を担当する。
- 関連（いずれも CLOSED）: #1683（ツリー行の状態4分類。本症状はこの分類導入後に顕在化した）、#1695（provider Stop 受理の deadlock。同じ Stop ingress 経路）
- spec 配置: `docs/specs/issues-1696`

## 確定済みの背景

正本 `docs/glossary/DOMAIN.md` は単独 Session を特別扱いしないと定めている。

- 単独 Session は Session Node 1個を root とする実行木であり、workflow と同じ再帰構造で Worktree 配下に属する（78-79行）。
- completion signal は NodeExecution が所有し、workflow aggregate だけが transition を決める（111行）。
- 単独 Session の lifecycle を実行木から分離した別の作業モデルにしない（115行）。

ツリー行の色分類は `domain/workspace_tree/value_objects/mod.rs` の `classify_own_status` が決める。

| 分類 | 条件 |
| --- | --- |
| 赤（Failure） | `Failed`、または recovery fence |
| 黄（Attention） | `Waiting`、または `Running` かつ completion signal が `StopReceived` |
| 青（Active） | `Running` |
| 緑（Idle） | 上記以外（`Completed` / `Paused` / `Aborted`） |

Session node の completion 条件は Submit と Stop の2信号で判定する。片方だけを受けた状態（`StopReceived` / `SubmitReceived`）は累積し、`Pending` へ戻らない（`domain/workflow/value_objects/node_execution.rs`、`workflow_execution/mod.rs` の `record_node_completion_signal` / `decide_node_completion_handshake`）。

Releash が provider に登録する hook は、Claude が SessionStart / Stop / StopFailure の3種、Codex が SessionStart / Stop の2種であり（`adaptor/gateway/provider_lifecycle/launch_spec.rs`）、「agent が新しい turn を開始した」ことを示すものは存在しない。この不足と、活動に応じた表示分類の復帰は #1700 が扱う。

詳細設計は本 ISSUE の対応内で決める。

# Outcome

対象者: Releash の Workspace ツリーで作業状態を見ながら単独 Session を使う開発者。

現在の問題: 単独 Session の行は起動から終了まで青（実行中）で固定され、agent が応答を終えて入力待ちになったことをツリーから判別できない。同じ状況で workflow 内の Session node は黄（介入待ち）になるため、単独 Session だけが状態を伝えない。原因は表示側ではなく、単独 Session が書き側で特別経路を担っていることにある。provider の Stop は「親 node があるか」で振り分けられ、親を持たない単独 Session の Stop は自分の実行木に記録されないまま捨てられる。

変更後に実現する状態:

- 単独 Session も workflow 内 Session と同じ completion signal 遷移を示し、agent が入力待ちであることがツリーに現れる。
- 実行木の表現から「単独 Session」という区分が無くなる。completion signal の受理と記録は、木の形にも木の起こされ方にも依存しない。「親がない = 実行木がない」という読み替えが実装から無くなり、同種の乖離が構造として再発しない。
- workflow が Session の lifecycle を所有するかどうかに依存する既存の規則（archive 可否等）は、木の形に依存しない述語として残る。

# Current Behavior

## 再現手順

1. Workspace で単独 Session を開始する。
2. 指示を送り、agent が応答を終えて入力待ちになるまで待つ。
3. Workspace ツリーの当該行のアイコンを見る。

## 実際の出力

- 当該行は青（Active）のまま変わらない。node の status は `Running`、completion signal は `Pending` で固定される。
- provider の Stop hook は届いているが、provider lifecycle slot に記録されるだけで実行木の node の事実にならない。
- 対比: workflow 内の Session node は同じ操作で黄（Attention）になる。
- 単独 Session が青以外へ遷移するのは TUI プロセスが消えたときだけで、`ProcessExited` から `Paused` が導出されて緑（Idle）になる（`domain/workflow/services/fact_replay.rs`）。対話中はプロセスが生きているため起きない。単独 Session の実行木には承認待ちを生む node が無く、`Failed` を導く事実も来ないため、黄・赤にも遷移しない。

## 調査で確認した現行実装

単独 Session の実行木は fact log 上に実在する。session 起動時に `tree_id = node_execution_id = session_id` の木が `TreeRootFact::Session` として append される（`adaptor/gateway/agent_session/agent_session_repository.rs:436-466`）。にもかかわらず、書き側の分岐だけが「親がない」を「木がない」と読み替えている。

| 箇所 | 内容 |
| --- | --- |
| `usecase/provider_lifecycle/ingress.rs:177` | Stop を control plane へ届けるかを `session.tree_parent().is_some()` で判定する。単独 Session の Stop はここで落ち、`NodeStopReceived` の事実にならない |
| `adaptor/gateway/agent_session/agent_session_repository.rs:815-821` | `TreeRootFact::Session` の木から復元した AgentSession は `tree_parent = None` になる。session の「木上の所在」を workflow-child だけが持つ |
| `domain/workflow/services/fact_replay.rs:203-219` | replay 時に session node 1個の WorkflowDefinition を合成して同じ fold に載せる読み側 shim。読み側は統一されているが書き側が統一されていないため足りない |
| `domain/workspace_tree/projection.rs:82-88` | 単独 Session だけ、記録された事実でなく「parent なし・kind = Session・node.id == execution_id」という構造条件で session_id を紐づける特例 fallback |
| `adaptor/gateway/workspace_tree/query_service.rs:65,120,138` | `TreeRootFact::Workflow` / `Session` で読み経路が分岐する |

根本は2つある。1つは `tree_parent: Option<...>` が「親 node の有無」と「実行木への所属の有無」の2つの意味を背負っていること。もう1つは root の事実そのものが `TreeRootFact::Workflow` / `Session` という木の種別を持ち、単独 Session を別種の木として表していることである。

workflow が Session の lifecycle を所有するかに依存する規則は実在する（workflow-owned session の archive / restore / delete / GC 拒否、initial instruction の受理、workflow launch rollback: `domain/agent_session/aggregates/agent_session.rs`）が、「Stop を実行木に届けるか」はそれに依存してよい規則ではない。また現行実装はこの所有を `tree_parent.is_some()`、すなわち親 node の有無で判定しているが、workflow 定義の root（`main`）に kind の制約は無く（`domain/workflow/services/validation.rs:1288,1628` は `entry_node().is_none()` だけを検査する）、`main` が Session node の workflow は定義できる。その Session は親を持たない workflow 所有 Session になるため、親の有無は所有の述語として正しくない。

調査で追加に確認した点として、`ingress.rs:177` の分岐を外すだけでは症状は解消しない。`record_provider_stop`（`domain/workflow/entities/workflow_execution/mod.rs:2611-2634`）は node の `session_id` と受信 session の一致を要求するが、単独 Session の木には node と AgentSession を結ぶ事実（`SessionAttached`）が記録されていない。書き側が木に記録するのは root の `Started` 事実だけで、node と session の紐づけは読み側の特例 fallback が補っている。

# Scope / Non-goals

## 変更する

- provider の completion signal（Stop）を実行木の node の事実として記録する経路。木の形と木の起こされ方に依存しない1本にする。
- 単独 Session の実行木の表現。root の事実が持つ木の種別の区別を廃し、workflow 実行木と同じ構成にする。書き側が記録する事実の構成、および node と AgentSession の紐づけを含む。
- AgentSession が持つ「実行木上の所在」の表現と、workflow が Session の lifecycle を所有するかの判定の分離。
- 上表の乖離5箇所（ingress の分岐、agent_session_repository の復元、fact_replay の読み側 shim、projection の特例 fallback、query_service の `TreeRootFact` 分岐）。

## 変更しない

- `classify_own_status` の分類規則と4色の意味。
- Submit と Stop の2信号で Session node の completion 条件を判定する規則。
- workflow 定義構文、および workflow 実行木の既存の遷移。
- Retry 機能そのもの。workflow の実行として起こされた実行木の既存 retry は維持し、Session の起動として起こされた実行木では explicit retry を受理しない。
- workflow が Session の lifecycle を所有するかに依存する既存の規則（archive / restore / delete / GC / initial instruction の受理 / workflow launch rollback）。
- provider に登録する hook の種類。agent の活動状態の所有、turn の開始を示す hook の追加、活動に応じた表示分類の復帰は #1700 が担当する。
- Terminal、Artifact、承認の各機能。

# Requirements

- R-001: 単独 Session で agent が応答を終えて入力待ちになったとき、その Session node の completion signal 状態が `StopReceived` になり、Workspace ツリーの当該行が青（Active）から黄（Attention）へ遷移する。
- R-002: provider の completion signal が実行木の node に記録されるかどうかは、その Session node が実行木の root であるか child であるか、およびその実行木が workflow の実行として起こされたか Session の起動として起こされたかに依存しない。同一の provider 入力に対して、単独 Session と workflow 内 Session は同じ completion signal 状態遷移を示す。
- R-003: 単独 Session が受け取った Stop は実行木の事実として永続化され、アプリケーション再起動後も当該行は黄（Attention）のままである。
- R-004: 単独 Session の実行木は workflow 実行木と同じ構成の事実で表される。root の事実は木の種別を区別しない。node と AgentSession の紐づけは記録された事実から決まり、「親を持たない Session node である」という構造条件からの推測に依存しない。
- R-005: AgentSession の詳細を返す外部インターフェースで、単独 Session についても、その Session が属する実行木と node execution を取得できる。
- R-006: workflow が Session の lifecycle を所有するかに依存する既存の規則は現行どおり維持される。利用者が Session の起動として起こした実行木の Session は archive / restore / delete / GC の対象であり、workflow の実行として起こされた実行木の Session はいずれも拒否される。この判定は、その Session node が実行木の root であるか child であるかに依存しない。
- R-007: workflow 実行木の既存の振る舞いは変わらない。Session node が Stop だけを受けた状態では黄（Attention）にとどまり、Submit と Stop が揃ったときだけ completion 条件を満たし、`completion: Approval` の node は承認待ちになる。
- R-008: workflow 実行の一覧には workflow の実行として起こされた実行木だけが現れ、workspace の session 一覧には Session の起動として起こされた実行木だけが現れる。
- R-009: Session の起動として起こされた実行木では explicit retry を受理しない。Workspace ツリーに Retry 操作を表示せず、外部インターフェースから直接要求されても新しい attempt を作らず、実行木と AgentSession の状態を変えない。workflow の実行として起こされた実行木の既存 retry は現行どおり受理する。

# Assumptions / Open Questions

## Assumptions

- 本変更以前に event store へ記録された実行木は対象外とする。prototype 段階として非互換を許容し、移行も後方互換の読み取りも行わない。

## Open Questions

なし。
