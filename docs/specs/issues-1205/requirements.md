# Context

- 入力: https://github.com/siro33950/releash/issues/1205
- 補助資料: #1759（[03] Terminal タブ）と `docs/specs/issues-1759/`、#1204（[01] SwiftUI クライアントの器）と `docs/specs/issues-1204/`、#1760（[05] ファイルタブ）と `docs/specs/issues-1760/`、#1762（[09] Workflow 管理タブ）、マイルストーン #78（「UI の決まり」「テストと CI の方針」）、ISSUE に添付された mock（Session タブ、Command タブ）。
- mock は配置と、出す情報と、できる操作の正であり、色と質感の正ではない（マイルストーン #78「UI の決まり」）。
- `AGENTS.md`「サーバがロジックを所有する」: 判定・取得・計算はサーバに置き、client には表示、入力の受付、サーバの呼び出しと購読、表示用の整形だけを許す。
- `AGENTS.md`「通信の原則」: サーバは状態を配信し、client は購読する。client からの単発の呼び出しは、状態を変える操作と、client の入力に対する計算だけに使う。
- proto の互換性は `buf.yaml` の `breaking: WIRE_JSON` で検査する。
- 開発の開始時点は main `72253f80`。
- この開発で人と決めた内容は、ISSUE 本文を書き換えず、この文書と behavior.md・design.md を正とする。ISSUE 本文と食い違うのは次の点である。
  - Session タブの上部の帯と下端の欄（実行木の位置、model、状態と経過、Approve / Abort、Thread の件数、provider session、attempt）は作らない。Approve は #1762 で扱う。これに伴い、Session 単位の Thread の件数と、model と attempt のサーバへの追加も行わない。
  - Command タブは帯を作らず、帯に載せる予定だった shell、exit code、所要時間、Retry / Abort を COMMAND 行に並べる。実行木の位置は出さない。
  - 提出済み・Stop 待ち、プロセスの在否、隔離 worktree の表示は引き継がない。
  - タブを開く経路は、右カラムの Root の一覧とする。Workflow の中の Session と Command のタブは #1762 から開く。
- `docs/specs/issues-1204/` の次の点と食い違う。#1204 の spec は直さず、この文書を正とする。
  - R-023: タブ列の「+」の「開く」は Terminal だけを置く。Session、Workflow、Workflow 管理は置かない。Session と Workflow の開始は Root の一覧の上端から行う。
  - R-036: 右カラムの面は「Root の一覧、ファイル、Source Control、Thread」の順にし、「タスク」の面を除く。
- React の画面は変えない。React は [16] #1765 で撤去する。

# Outcome

対象者: Releash の利用者（開発者）と、Releash を開発する人と agent。

現在の問題:
- Swift.app の Session タブは中身の無い仮の表示で、provider の TUI を使えない。Command の結果を見るタブは無い。
- Swift.app には、worktree の実行木の Root を選んでタブを開く手段が無い。Archive、改名、Resume などのライフサイクルの操作や、Archive 済みの Session と provider の履歴を扱う手段も無い。
- Session を開けるかどうかの判定は domain が行っているが、その結果は画面に届かない。React の画面は、lifecycle と terminalPresence から同じ判定を自分で行っている。
- Command を実行した shell と作業ディレクトリは記録されておらず、画面に出せない。

変更後の状態:
- 利用者は、右カラムの Root の一覧から、選んでいる worktree の実行木の Root を選び、Session タブ、Command タブ、Workflow 管理タブを開ける。
- 利用者は、Session タブで provider の TUI を操作できる。Session を開けないときは、その理由が分かり、止まっていれば Resume できる。
- 利用者は、Command タブでコマンド、作業ディレクトリ、shell、終了コード、所要時間、出力、失敗理由、過去の試行を確かめ、Retry と Abort ができる。
- 利用者は、Root の一覧から Session と Workflow を開始し、Root の改名、Archive、Resume ができる。History から Archive 済みの Session の Restore と Delete、provider の履歴からの Resume ができる。
- Session を開けるかどうかの判定はサーバが配り、Swift.app は判定しない。

# Current Behavior

開発開始時点（main `72253f80`）で確認した状態。

Swift.app:
- タブ列の「+」の「開く」は、Session、Workflow、区切り線、Terminal、Workflow 管理を出す（`clients/macos/Sources/PaneOpenMenu.swift:14-22`）。
- 「+」の「Session」は provider を選んで `CreateAgentSession` を呼び、成功すると Session の種類のタブを開く（`PaneOpenMenu.swift:101-112`）。「Workflow」は定義と依頼文で `StartWorkflow` を呼び、成功すると Workflow 管理の種類のタブを開く（`:114-123`）。
- Session のタブと Workflow 管理のタブの中身は、タブ名だけの仮の表示である（`clients/macos/Sources/PaneWorkspace.swift:180-183`）。Command のタブの種類は無い。
- タブの保存項目は id、種類、ファイルの path・表示・比較だけで、Session や Node を指す項目は無い（`proto/client.proto:1943-1956`）。
- 右カラムの面は、ファイル、Source Control、Thread、タスクの順である（`clients/macos/Sources/RightPanelView.swift:7-24`）。
- サイドバーのカードは実行の要約（`WorktreeExecutionSummary`、`proto/client.proto:2751-2759`）を agent 行と workflow 行で表示するだけで、行からタブは開かない（`clients/macos/Sources/SidebarView.swift:126-162`）。

サーバ:
- 実行木の Root の項目は Sequence、Fanout、Node のどれかで、Node は session と command を `contentKind` で区別する（`server/src/adaptor/presenter/client/workspace.rs:209-260`）。Root の項目に provider は無い。provider は実行の要約にあり、要約の ID は実行の ID で、Root の項目の ID（Node の ID）とは別である（`workspace.rs:126-149`）。
- `StartWorkflow` は実行の ID だけを返す（`proto/client.proto:2679`）。`CreateAgentSession`、`RestoreAgentSession`、`ResumeAgentSessionHistoryCandidate` は Session の ID と Node の ID を返す（`:2629`、`:2668`、`:2670`、`:2945-2948`）。
- Node の詳細は購読 `node-detail`（引数は worktree の path と Node の ID）で届く（`server/src/adaptor/presenter/state_subscription_target.rs:70`、`proto/client.proto:1613-1664`）。Command の content は表示用のコマンド、終了コード、所要時間、stdout、stderr を持つ。`errorReason`、`canRetry`、`canResumeSession`、`canRename` もある。`canAbort` と実行の ID は無い。
- `canAbort` と `canArchive` は実行木の項目の `workflowCapabilities` にだけあり、Root の実行の値を配る（`workspace.rs:285-292`）。
- Command は `/bin/sh -c` で、worktree の path を作業ディレクトリとして実行する（`server/src/infrastructure/process/command_runner.rs:126-130`、`server/src/adaptor/gateway/workflow/workflow_host.rs:1537`）。記録されるのは表示用のコマンドだけで、shell と作業ディレクトリは記録されない（`server/src/domain/workflow/value_objects/node_fact.rs:258-260`）。
- 過去の試行は実行木の Node の `pastAttempts` にあり、1 件ごとに Node の項目（id、title、状態、errorReason、updatedAt など）を持つ（`proto/client.proto:1696-1710`）。
- Session を開く `OpenAgentSession` は `Unit` を返す（`proto/client.proto:2651`）。domain は開いた結果を Attach、Resume、Restore、RemainPaused、Indeterminate、GarbageCollect のどれかに判定する（`server/src/domain/agent_session/aggregates/agent_session.rs:253-260`、`:548-567`）が、controller が結果を捨てており（`server/src/adaptor/controller/client/agent_session/provider_tui.rs:141-145`）、どの購読にも載らない。
- 購読 `agent-session` は `AgentSessionItemDto`（lifecycle、operations、providerSessionId、lastExitAbnormal、terminalPresence など）を配る（`proto/client.proto:886-899`）。
- Archive は、実行木の単位の `ArchiveWorkspaceWorkflowExecution` と Session の単位の `ArchiveAgentSession` があり、どちらも最後に同じ実行木の Archive を行う（`server/src/usecase/workflow/workspace_tree.rs:181-189`、`server/src/usecase/agent_session/agent_session_lifecycle.rs:404-420`）。
- Archive 済みの Session は実行木の `archivedSessions` に、provider の履歴は購読 `session-history` に届く（`proto/client.proto:1681-1685`、`:1806-1824`）。

React の画面:
- Node の画面は、Session なら provider の TUI、Command ならコマンド、状態、終了コード、所要時間、stdout、stderr を出す（`clients/desktop/src/components/panels/NodeContentView/NodeContentView.tsx:68-93`、`:216-255`）。
- Session を開けるかどうかを、lifecycle と terminalPresence から自分で判定し、止まっていれば `canResumeSession` のとき `ResumeWorkspaceSessionNode` の Resume を出す（`clients/desktop/src/components/panels/AgentSessionPanel/AgentSessionPanel.tsx:76-82`、`NodeContentView.tsx:288-313`）。
- 実行木の Root の「×」で、`canAbort` なら「Archive execution? / Archiving will Abort this execution and stop its processes.」の確認を出してから、`ArchiveWorkspaceWorkflowExecution` で Archive する（`clients/desktop/src/components/workspace/WorkspaceList.tsx:395-406`、`:884-903`、`:1426-1455`）。
- worktree のメニューの SessionHistory に Archive 済みの Session（選ぶと Restore、Delete も可）と provider の履歴（選ぶと Resume、追加読み込み）を、WorkflowHistory に Archive 済みの Workflow を並べる（`WorkspaceList.tsx:1129-1235`）。

# Scope / Non-goals

Scope:
- 右カラムの Root の一覧の面（行、行を押したときのタブ、New Session、New Workflow、History、行の右クリックメニュー）と、面の並びの変更
- タブ列の「+」の「開く」の項目の変更
- Session タブ（provider の TUI、開けなかったときの表示、Resume）
- Command タブ（COMMAND 行、STDOUT、STDERR、失敗理由、過去の試行、Retry、Abort）
- タブの保存項目への、Node を指す ID と Command の種類の追加（proto、保存形式）
- Session を開けるかどうかの判定の配信、Command の shell と作業ディレクトリの記録と配信、Root の一覧と Command タブに要る値の配信（サーバ）
- Session タブと Command タブの回帰の E2E

Non-goals:
- Session タブの上部の帯と下端の欄（実行木の位置、model、状態と経過、Approve / Abort、Thread の件数、provider session、attempt）
- Session 単位の Thread の件数、Thread の書き手の区別への session_id の追加、provider からの model の取得、attempt の配信
- Command タブの実行木の位置の表示
- 提出済み・Stop 待ち、プロセスの在否、隔離 worktree の表示の引き継ぎ
- Workflow の中の Session と Command のタブを開く操作、Approve、Workflow 管理タブの中身、History の Workflow の Restore（[09] #1762）
- タブを自動で開く動作
- タブを閉じたときに Session や実行を止めること
- Root の一覧の行の右クリックメニューの Restore と Delete
- `ArchiveAgentSession` の利用
- 過去の試行への attempt の番号と終了コードの追加
- React の画面の変更
- ISSUE 本文と `docs/specs/issues-1204/` の書き換え

# Requirements

Root の一覧:
- R-001: 右カラムの面は、Root の一覧、ファイル、Source Control、Thread の順に並ぶ。「タスク」の面は無い。
- R-002: Root の一覧は、選んでいる worktree の実行木の Root のうち、Archive されていないものだけを 1 行ずつ並べる。各行は、Root の種類（Session、Command、Sequence、Fanout）のアイコンを状態の 3 値の色で塗り、タイトルを並べる。Session の Root は、タイトルの前に provider の表示名を出す。
- R-003: 行を押すと、Root の種類に応じたタブを開く。Session なら Session タブ、Command なら Command タブ、Sequence と Fanout なら Workflow 管理タブである。
- R-004: 一覧の上端の右に、「…」と「+」のアイコンのボタンがある。「+」のメニューに New Session と New Workflow が、「…」のメニューに History がある。New Session は provider を、New Workflow は定義を、ダイアログを出さずにメニューの中で選び、選んでいる worktree で開始する。依頼文は渡さない。History はダイアログで開く。開始に成功すると、開始した Root について、一覧の行を押したときと同じ振る舞いをする。
- R-005: History は、Archive 済みの Session と provider の履歴を並べる。Archive 済みの Session を選ぶと Restore し、Delete もできる。provider の履歴を選ぶと Resume し、さらに古い履歴を追加で読める。Restore または Resume に成功すると、その Root について、一覧の行を押したときと同じ振る舞いをする。
- R-006: 行の右クリックメニューには、改名、Archive、Resume を置く。項目は、サーバが配る値でその Root に使えるときだけ出し、使えない項目は出さない。
- R-007: Archive は、どの種類の Root にも同じ操作で働く。その Root の実行が中止できる状態なら、Archive する前に、実行を中止して Archive することの確認を出す。確認で取りやめると何もしない。
- R-008: 改名すると、一覧の行と、その Root のタブのタブ名が新しい名前になる。

タブ:
- R-009: タブ列の「+」の「開く」は Terminal だけを出す。
- R-010: Session タブと Command タブは、Node と結び付けて保存される。worktree を選び直したときと Swift.app を起動し直したときに、同じ Node の内容を表示する。
- R-011: Session タブまたは Command タブを閉じても、Session と実行は止まらず、閉じる前の確認も出ない。

Session タブ:
- R-012: Session タブは、provider の TUI を表示し、入力を受け付ける。
- R-013: Session を開けないとき、Session タブは TUI の代わりに、止まっていること、判定できないこと、または Archive されていることを表示する。止まっていて Resume できるときは Resume の操作を出し、押すと Session が再開して TUI を表示する。
- R-014: Session を開けるかどうか（開いている、止まっている、判定できない など）は、サーバが判定して購読で配る。Swift.app は lifecycle や terminalPresence から判定しない。

Command タブ:
- R-015: Command タブの COMMAND 行に、実行したコマンド、作業ディレクトリ、shell、終了コード、所要時間を表示する。shell と作業ディレクトリは、サーバが実行時に記録して配る。
- R-016: Command タブは、stdout と stderr の内容を分けて表示し、失敗していれば失敗理由を表示する。
- R-017: COMMAND 行に Retry と Abort の操作がある。Retry はサーバがその Node に Retry できるとするときだけ、Abort はその実行が中止できる状態のときだけ出す。Abort は、その Node の実行を中止する。
- R-018: Command タブに、過去の試行を時刻と失敗理由で並べる。過去の試行を押すと、その試行の Command タブを開く。

サーバとの境界:
- R-019: この開発で追加・変更するサーバの呼び出しと購読は、特定の client に依存しない形である。状態は購読で配信し、単発の呼び出しは状態を変える操作と client の入力に対する計算だけに使う。Root の一覧の provider、開始した Workflow の Root の特定、Command タブの Abort の可否と対象は、サーバが配る値で決まり、Swift.app が別々に届いた値を突き合わせて求めない。
- R-020: proto の変更は `buf.yaml` の breaking 検査（WIRE_JSON）を通る。

検証:
- R-021: Session タブに provider の TUI が表示され、入力できることを確かめる回帰の E2E がある。
- R-022: Root が Command の実行について、Command タブにコマンドの出力と終了コードが表示されることを確かめる回帰の E2E がある。

# Assumptions

なし
