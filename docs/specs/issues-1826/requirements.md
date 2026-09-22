# Context

- 正本: [#1826 \[workflow\] アーカイブを実行木の 1 つの操作にし、アーカイブした実行と worktree が消えた実行を必ず終了させる](https://github.com/siro33950/releash/issues/1826)
- 関連 Issue: [#1839](https://github.com/siro33950/releash/issues/1839)、[#1840](https://github.com/siro33950/releash/issues/1840)、[#1836](https://github.com/siro33950/releash/issues/1836)、[#1845](https://github.com/siro33950/releash/issues/1845)
- `docs/glossary/DOMAIN.md` は、Workflow と単独 Session をともに Worktree 配下の実行木と定義し、単独 Session を Session Node 1 個を root とする実行木と定義している
- 先行条件の #1839 は `b642f94d feat(workflow): プロセス在否に基づくNode再開と起動再試行を導入 (#1860)` で現行 `main` に取り込まれている。現行の Abort は worktree フォルダの存在を要求せず、Command の Retry は実行 worktree のフォルダを要求する
- 永続化の正本は事実ログである。一方、Workflow として起動した実行木の Archive だけは `src-tauri/src/adaptor/gateway/workflow/execution_archive_repository.rs` の `workflow_execution_archives.json` に記録され、単独 Session の Archive / Restore は root Node の `archive_requested` / `restore_requested` として事実ログへ記録される
- 現行の Workspace tree と履歴は `src-tauri/src/adaptor/gateway/workspace_tree/query_service.rs` で、Workflow の Archive を専用 JSON から、単独 Session の Archive を事実ログから別々に導出している
- 現行 GC は `src-tauri/src/usecase/app_data_gc/mod.rs` と `src-tauri/src/adaptor/gateway/app_data_gc/mod.rs` で、Git の worktree 一覧を基準に付属データの削除候補を決め、リポジトリを読めない場合は workspace 依存の削除を行わない。実行木自体は Archive しない
- Releash 内の worktree 削除は `src-tauri/src/usecase/repository_usecase.rs` で、Terminal の停止、Git worktree の削除、base branch 設定の後始末を行うが、実行木の Abort / Archive は行わない
- #1845 は worktree フォルダ削除を画面の待機対象から外す変更を扱う。ただし、削除前の実行木 Archive は本 Issue が先に同期的に完了させる前提である
- Worktree の削除開始後は、その Worktree に対する外部からの状態変更を受け付けず、読み取りを許可する。削除処理または GC が実行木を片付けるために行う Abort と Archive は、削除・消失処理の内部遷移として許可する（Review Thread `d2b46389-20be-448f-b8aa-7797c73df881`）

# Outcome

対象者は、Releash で Workflow または単独 Session を実行し、不要になった実行木や worktree を整理する利用者である。

現在は Workflow と単独 Session で Archive の操作と記録先が分かれ、Workflow の Archive は実行状態を変えない。このため、実行中のまま一覧から隠れた実行木が同じ worktree の起動枠を塞ぐ。また、Git の worktree 一覧から消えた worktree の実行木は画面から操作できず、GC や Releash 内の worktree 削除でも終了・Archive されない。

変更後は、起動方法にかかわらず実行木を対象とする 1 つの Archive 操作になり、Archive 済みの実行木は必ず終了している。Git の worktree 一覧から消えた worktree の実行木は自動で Archive され、Releash 内で worktree を削除するときもフォルダ削除前に実行木とそのプロセスが片付く。Archive の状態と理由は事実ログから一貫して読める。

Releash 内で worktree の削除が始まった後は、その worktree への外部からの状態変更によって新しい実行や未 Archive の実行木が生じず、削除中も状態を読み取れる。

# Current Behavior

## Workflow と単独 Session の Archive が分かれている

- Workflow の Archive は `archive_workspace_workflow_execution` が対象と worktree の対応だけを確認し、`workflow_execution_archives.json` に `manual` の記録を追加する。実行状態の確認も Abort も行わない
- Workflow の Archive ボタンは `Completed` または `Aborted` のときだけ有効である。ただし、この制約は画面表示だけで、バックエンドの Archive 入口には無い
- 単独 Session は別の `archive_agent_session` を使う。provider session ID が無い場合は Archive せず `delete_confirmation_required` を返す。Archive できた場合は `archive_requested` を事実ログへ記録する
- `archive_requested` は AgentSession の lifecycle を Archived にするが、WorkflowExecution の状態導出では無視される

最小の再現は、実行中の WorkflowExecution に画面以外の入口から `archive_workspace_workflow_execution` を実行し、Workspace tree と実行一覧を読み直すことである。実行木は Workspace tree から消える一方、実行状態は `running` のままになる。正本 Issue の 2026-09-16 の実測では「実行中かつ Archive 済みで一覧に出ない」実行が 18 件あり、例として `ac1e31a3-96bd-45bc-a3ce-4488d9ec5114` は Archive 後も同じ worktree の新規 Workflow 起動を塞いでいた。

## 消えた worktree の実行木を片付けない

- GC は Git の worktree 一覧から消えた workspace の付属データを削除候補にするが、その worktree に属する実行木へ Archive を要求しない
- Releash 内の worktree 削除は Terminal を止めてから Git worktree とフォルダを削除するが、Workflow の Command プロセスを止めず、実行木を Archive しない
- worktree が一覧から消えると Workspace 自体が画面に現れないため、そこに属する実行木を画面から Archive できない

## #1839 後の Abort / Retry

- 現行の Abort は execution ID から永続化済み状態を読み、worktree フォルダを検査せずに実行木を `Aborted` へ遷移させる
- Command の Retry は、そのフォルダでプロセスを起動し直すため、実行 worktree のフォルダが無ければ受理されない

# Scope / Non-goals

## 変更する対象

- Workflow として起動した実行木と単独 Session として起動した実行木の Archive 操作
- 実行中の実行木を Archive するときの Abort と、Archive 完了後の表示・起動枠
- Archive 状態、Archive 時刻、Archive 理由を導出する永続記録
- `workflow_execution_archives.json` の既存記録の事実ログへの移行と、同ファイルへの読み書きの廃止
- Workspace tree の Archive ボタンと、実行中の実行木を Archive するときの画面上の確認
- Git の worktree 一覧から消えた worktree の実行木を GC が Archive する処理
- Releash 内で worktree を削除する前に、その worktree の実行木を Archive する処理
- Releash 内で worktree の削除を開始した後の、対象 worktree に対する外部からの状態変更の拒否と読み取りの維持
- Archive 済みの実行木を Restore する操作と、Restore 後の単独 Session の手動 Resume
- 上記の変更が既存の画面・Connect / Tauri command・local API / CLI の共有 backend state へ反映される範囲

## 変更しない対象

- Node の状態、Resume / Retry、起動時の自動再試行など、#1839 で確定・実装済みのライフサイクル
- Abort が worktree フォルダの存在を要求しないことと、Retry が実行 worktree のフォルダを要求すること
- 起動時の Workflow 状態の常駐・復元方法と、削除開始前の通常状態における worktree 単位の起動排他。#1840 が扱う
- 実行木の自然完了の事実化と、保存済み定義を読めない実行の扱い。#1836 が扱う
- worktree フォルダの削除方法と、削除を画面の待機対象から外す変更。#1845 が扱う
- Git の worktree 一覧以外の情報を worktree 消失判定へ追加すること

# Requirements

- R-001: Workflow として起動した実行木と単独 Session として起動した実行木は、起動方法に依存しない同じ Archive 操作で Archive できる
- R-002: Archive は実行木の現在の状態にかかわらず要求できる。終了していない実行木は Abort された後にだけ Archive 済みとなり、Archive 済みかつ実行中の状態は存在しない
- R-003: Archive 状態、Archive 時刻、Archive 理由は事実ログから導出される。利用者が実行木を Archive した記録は理由 `manual` を持ち、`workflow_execution_archives.json` は Archive の読み書きに使われない
- R-004: `workflow_execution_archives.json` にある既存の Archive 記録は事実ログへ移され、既存の Archive 状態、Archive 時刻、Archive 理由が失われない。移行対象が終了していない場合も、移行後に Archive 済みかつ実行中の状態を残さない
- R-005: 画面は、表示中の Archive されていない実行木に現在の状態を問わず Archive 操作を提示する。終了していない実行木では Archive により Abort されることを実行前に確認し、終了済みの実行木では確認を挟まない
- R-006: 画面以外の Archive 入口は利用者確認の状態や手順を持たず、要求を受けると Archive を実行する
- R-007: GC の worktree 消失判定は Git の worktree 一覧だけを正とし、フォルダの有無を判定に使わない。Git の一覧にある worktree の実行木は Archive せず、一覧から消えた worktree の実行木は Archive する
- R-008: GC が Git の worktree 一覧から消えた worktree の実行木を Archive するとき、終了していない実行木は Abort され、Archive 理由は `worktree_removed` となる。対象リポジトリを読めず一覧を確定できない場合、そのリポジトリの実行木は Archive しない
- R-009: Releash から worktree を削除するとき、その worktree に属する実行木は理由 `worktree_removed` で Git worktree とフォルダの削除より先に Archive され、実行中の Command プロセスはフォルダ削除より先に停止する
- R-010: worktree フォルダが存在しなくても実行木を Abort できる一方、Command の Retry は実行 worktree のフォルダが存在する場合にだけ実行できる
- R-011: 所属 worktree が利用可能な Archive 済みの実行木は Restore できる。Restore は Archive を解除するが、実行木の終了状態を変えずプロセスを自動で起動しない。Restore した単独 Session の再開は、利用者が手動で Resume を要求したときに行う
- R-012: Releash による worktree の削除が始まった後は、対象 worktree に対する外部からの状態変更要求は受理されず、読み取りは引き続き行える。削除処理および GC が R-007〜R-009 を満たすために行う Abort と Archive は、この制約の対象外とする

# Assumptions / Open Questions

なし。
