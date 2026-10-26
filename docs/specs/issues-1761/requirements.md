# Context

- 要求の正本は ISSUE #1761「[09] 「Thread」の面のネイティブ化と未解決 Thread の一括再指示」である。
- 補助資料は #1760（[05] ファイルタブと「ファイル」の面）、commit `efda9bb7`（ISSUE 本文の「現状」の基準）、Textual（https://github.com/gonzalezreal/textual ）、ISSUE に添付された mock 1 枚である。
- この開発は main の `0f65bf5d` を基点にする。
- マイルストーン #78 の決まりのうち、この開発に関わるもの:
  - ロジックはサーバに置き、Swift.app は表示と操作の受付だけを行う。
  - RPC と購読の形は Swift.app の都合に合わせず、どのクライアントからも同じ形で使えるものにする。
  - ネイティブの画面の中の markdown（Thread の本文）は Textual で表示する。サーバで markdown を構造にして返す仕組みは作らない。
  - 色はシステムの色と状態の 3 色だけを使い、部品は SwiftUI の標準の部品を使う。
- mock は配置と、出す情報と、できる操作の正である。
  mock の右の面は、見出しに未解決と解決済みの件数、送るボタン、Thread の行（位置、本文、最後の発言者と時刻、Comment 数、解決済みの印）、最下部に「General に追加」を持つ。
- Textual は MIT ライセンス、対応は macOS 15 以上、確認した最新版は 0.5.0（2026-06-15 公開）である。
  markdown の整形表示は `StructuredText(markdown:)` が行う。
- ISSUE 本文と異なる決定:
  - 送る Thread: 本文は「未解決 N 件を Session に送る」とする。
    この開発では、Thread の行にチェックボックスを置き、チェックした Thread を送る。未解決か解決済みかは問わない。
  - 送り先の Session: 本文は「対象 Session」とだけ書く。
    この開発では、選択中の worktree の生きている Session の一覧から利用者が一つ指定する。
  - プロンプトの中身: 本文は「行位置付きの 1 つのプロンプト」とだけ書く。
    この開発では、Thread ごとに位置、ID、本文（Thread を立てた最初の Comment の文）を書き、それ以降の Comment は起動環境に対応する CLI の `review get` で確認するよう書く。返信や解決の指示は書かない。
  - 既存の 1 Thread 分の引き渡し文（`BuildReviewThreadHandoff`）は、この新しい文面に揃える。
  - ポップオーバー 2 種を無くす対象は React の画面であり、この開発では React を触らない。
  - 証跡: 本文は手元で確認した証跡を PR に貼るとする。
    証跡を PR に載せる仕組みは利用者の決定で削除済みのため、この開発の要求に含めない。
- 利用者が決めた既存コードの直し方:
  - 指示文を terminal の 1 回の入力として書く形（bracketed paste で包み、改行で送る）は、usecase ではなく gateway が持つ。既存の続行指示と、この開発で足す送信の両方がそれを使う。
  - Thread の引き渡し文の組み立ては domain に 1 つ置き、1 件分も複数件分もそれを使う。

# Outcome

対象者は、Swift.app でコードを読み、Thread を立て、agent に直しを指示する開発者である。

今の Swift.app には、worktree の Thread をまとめて読む面が無い。
Thread はファイルタブの行の間にしか出ず、ファイルに付かない Thread（General）は見る手段も作る手段も無い。
Thread の内容を agent に渡すには、利用者が Thread を一つずつ読み上げるか、React の画面で 1 件ずつクリップボードにコピーして貼り付けるしかない。

変更後は、右カラムの「Thread」の面で、worktree の Thread を未解決と解決済みに分けて読み、返信と解決ができ、General の Thread を作れる。
利用者は、面で Thread をチェックし、生きている Session を一つ指定して、チェックした Thread を 1 つのプロンプトとしてその Session の terminal に書き込める。

# Current Behavior

調査の時点は HEAD `0f65bf5d` である。

Swift.app（`clients/macos/Sources/`）の挙動は次のとおりである。

- 右カラムの面は「Node」「ファイル」「Source Control」「Thread」の切替を持つが、中身があるのは「Node」と「ファイル」だけである（`RightPanelView.swift:7-25`、`:57-63`）。「Thread」を選ぶと空の領域が出る。
- ファイルタブは、開いているファイルの Thread を購読 `file-threads` で受け、行の間に出す（`FileTabPane.swift:58-60`、`:191-212`）。
  返信は `AppendReviewComment`、解決は `ResolveReviewThread` を呼ぶ（`:218-239`）。
  Thread の作成は、行を選んで帯の「Thread作成」から行う（`:243-271`）。
- Session タブは `agentSessionID` を持ち、前面の pane と選択中のタブを `PaneWorkspace` が持つ（`PaneWorkspace.swift:224-226`）。
- Swift の依存は Bonsplit、SwiftTerm、Connect、SwiftProtobuf で、Textual は入っていない（`clients/macos/project.yml:5-17`）。
  同梱する依存のライセンスは `clients/macos/Resources/` に `<名前>-LICENSE` として置き、`project.yml` の resources で bundle に入れる（`:34-41`）。
- markdown を整形して出す箇所は無い。

サーバ（`server/src/`）と proto（`proto/client.proto`）は次のとおりである。

- Thread の作成、返信、解決、削除の RPC は既存である（`CreateReviewThread` `:2677`、`AppendReviewComment` `:2666`、`ResolveReviewThread` `:2714`、`DeleteReviewThread` `:2684`）。
- worktree の全 Thread は購読 `review-threads`（引数は worktree の path）で届く（`adaptor/presenter/state_subscription_target.rs:137`、`usecase/state_subscription/reads.rs:417-425`）。
  Thread は更新時刻の新しい順に並ぶ（`domain/comment/mod.rs:647`）。
  Thread の作成、返信、解決、削除は購読に通知される（`usecase/comment/mod.rs:69-77`）。
- Thread の位置は `ReviewTarget`（`file_path`、`line_number`、`end_line`、`side`、`end_side`）で、`file_path` が無い Thread が General である（`domain/comment/mod.rs:100-106`）。
  解決済みの Thread には返信できない（`domain/comment/mod.rs:491-501`）。
- 1 Thread 分の引き渡し文は `BuildReviewThreadHandoff`（`:2670`）が返す。
  文面は usecase の `build_review_thread_handoff_message` が組み立て、冒頭の一文と `releash review get --session-id "$RELEASH_SESSION_ID" <id>` だけで、位置と本文を含まない（`usecase/comment/mod.rs:249-257`）。
  呼び出し元は React だけで、返った文字列をクリップボードにコピーする（`clients/desktop/src/contexts/ReviewThreadHandoffContext.tsx:70-100`）。
- サーバの中から Session の terminal へ書く経路は `AgentSessionInitialInstructionUsecase::dispatch_continuation` だけである（`usecase/agent_session/agent_session_initial_instruction.rs:43-88`）。
  usecase の `write_instruction` が指示文を bracketed paste（`\u{1b}[200~` と `\u{1b}[201~`）で包み、`\r` を付けて `ProviderAgentTerminalInputGateway::write` に渡す（`:81-88`）。
  gateway の実装は受けた文字列をそのまま terminal に書く（`adaptor/gateway/agent_session/provider_agent_terminal_gateway.rs:99-107`）。
  この経路は workflow が起動した Session に限る（`domain/agent_session/aggregates/agent_session.rs:694-697`）。
- 複数の Thread を 1 つのプロンプトにまとめて Session の terminal に書く usecase と RPC は無い。
- Session の生死は `AgentSessionItemDto.availability`（`open` / `paused` / `indeterminate` / `archived`、`:899-908`）で届く。
  worktree の実行木の Session Node は `WorkspaceNode.session_id`（`:1723`）で Session を指す。

回帰の E2E（`clients/macos/UITests/WorkbenchTests.swift`）に、「Thread」の面を扱うテストは無い。
fixture は General の Thread を作れる（`UITests/Fixture.swift:139`、`:180-186`）。
provider の TUI の fixture は、受けた入力を `received-<n>:<入力>` と画面に出す（`server/src/test_support/agent_tui_fixture.rs:194-201`）。

# Scope / Non-goals

変更するもの:

- 右カラムの「Thread」の面（一覧、未解決と解決済みの切替、返信、解決、General の作成、チェック、送信）。
- Thread の本文と Comment の markdown の整形表示（Textual の導入とライセンスの同梱）。
- サーバ: チェックした Thread を 1 つのプロンプトにして、指定した Session の terminal に書く usecase と RPC。
- サーバ: Thread の引き渡し文の組み立てを domain に置き、1 件分（`BuildReviewThreadHandoff`）もそれを使う。
- サーバ: 指示文を terminal の 1 回の入力として書く形を gateway に移し、既存の続行指示もそれを使う。
- 回帰の E2E。

変更しないもの:

- Thread の削除。
- ファイルタブの行の間の Thread（#1760 で作った表示と操作）。
- Thread の行からファイルタブを開く操作。
- 複数の Session への同時送信。
- 送った事実の記録（Comment の追記や送信履歴）。
- React の画面と Tauri シェル（ポップオーバー 2 種は #1765 で React ごと消える）。
- 購読 `review-threads` の形。
- 証跡を PR に載せる仕組み。

# Requirements

- R-001: 右カラムに「Thread」の面があり、選択中の worktree の Thread が一覧で出る。各行に、位置（ファイルの path と行の範囲、ファイルに付かない Thread は `General`）、Thread の本文（最初の Comment の文）、Comment の数、最後の Comment の発言者と時刻が出る。
- R-002: 面は未解決と解決済みを切り替えて出す。見出しに未解決の件数と解決済みの件数が出る。解決済みの行には解決済みであることが分かる印が出る。
- R-003: 利用者は、未解決の Thread の行から返信でき、Thread を解決できる。解決した Thread は解決済みの側に移る。
- R-004: Thread の本文と Comment の markdown は、見出し、箇条書き、コード、強調が整形されて出る。
- R-005: 利用者は、面の「General に追加」からファイルに付かない Thread を作れる。作った Thread は未解決の側に出る。
- R-006: Thread の行にチェックボックスがあり、未解決と解決済みのどちらの Thread もチェックできる。既定は未チェックである。チェックした件数が送るボタンに出る。
- R-007: 送るボタンは、選択中の worktree の生きている Session（`availability` が `open`）の一覧を出し、利用者はその中から一つを指定する。生きている Session が無いときは送れない。
- R-008: 利用者が Session を指定して送ると、チェックした Thread が 1 つのプロンプトとしてその Session の terminal に書き込まれ、Session の TUI は 1 回の入力として受ける。プロンプトは、冒頭の一文、Thread ごとの位置、ID、本文（最初の Comment の文）、それ以降の Comment を `<CLI名> review get --session-id "$RELEASH_SESSION_ID" <id>` で確認するよう書いた一文からなる。CLI 名はサーバの起動環境に合わせ、本番では `releash`、開発では `releash-dev` とする。返信や解決の指示は含まない。
- R-009: どのクライアントも、Thread の ID の列と Session の ID を指定して R-008 の送信を呼べる。サーバは、生きていない Session と、指定した worktree に無い Thread の ID を拒む。
- R-010: `BuildReviewThreadHandoff` が返す 1 Thread 分の引き渡し文は、R-008 のプロンプトと同じ形（Thread が 1 件）になる。
- R-011: Thread の作成、返信、解決、削除が起きると、利用者の操作なしに面に反映される。
- R-012: 回帰の E2E が、「Thread」の面に Thread が出て返信と解決ができることと、チェックした Thread を Session の terminal に送れることを確かめる。

# Assumptions

なし

# Invariants

- 既存の続行指示（delegate の子の結果を親 Session へ送る指示）は、terminal に同じ形（bracketed paste で包み、改行で送る）で届く。利用者が、この包みを gateway へ移し、既存の経路と新しい経路の両方がそれを使うと決めたため。

# Risks

- Textual の整形表示が、システムの色と明暗の設定に合うかは確かめていない。合わない場合は R-004 を満たしつつ #78 の UI の決まりを守れない。
- XCUITest から Textual が描いた本文を文字列で探せるかは確かめていない。探せない場合は R-012 の確かめ方が変わる。
