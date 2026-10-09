# Context

- 入力: https://github.com/siro33950/releash/issues/1204
- 補助資料: #1854（`clients/` の配置）、#1763（[10] Settings）、#1944（[13] フッター）、#2012（[02] E2E）、マイルストーン #100（サーバを画面から独立させる）、マイルストーン #78（ネイティブ UI と「UI の決まり」）、ISSUE に添付された mock 3 枚（全体、Worktree 作成ダイアログ、「+」の「開く」メニュー）。
- タブ列と分割の参考実装: Bonsplit（https://github.com/almonk/bonsplit ）、Ghostty（`macos/Sources/Features/Splits/SplitView.swift`）、CodeEdit、Supacode。
- mock は機能の正であり、UI の細部の正ではない。マイルストーン #78 の「UI の決まり」で、色はシステム色だけを使い、例外は状態の 3 色と差分の追加・削除の色に限る。部品は標準のものを使い、余白は定数で持つ。
- 状態は 3 値で表す。黄は人の番（attention）、青はシステムの番（active）、緑は誰の番でもない（idle）。
- `AGENTS.md`「サーバがロジックを所有する」: アプリケーションロジックはすべてサーバに置き、client には表示、入力の受付、サーバの呼び出しと購読、表示用の整形だけを許す。依頼された振る舞いに必要な既存ロジックはサーバへ移す。
- `AGENTS.md`「通信の原則」: サーバは状態を配信し、client は購読する。client からの単発の呼び出しは、状態を変える操作と、client の入力に対する計算だけに使う。
- #1204 は分割しない。
- この開発で人と決めた内容のうち ISSUE 本文と食い違うもの（worktree の表示名、作成の非同期化と cancel・retry・進行中の行、pin、agent 行の最後のメッセージと経過、右の切替面、Issue・Notion のリンク）は、ISSUE 本文を書き換えず、この文書と behavior.md・design.md を正とする。
- 後続の ISSUE が扱うもの: E2E による確認は [02] #2012、右の切替面は [05]、agent 行・workflow 行からタブを開く操作は [09] と [04]、フッターの中身は [13] #1944、repository の削除は [10] #1763、React の画面の撤去は [16] #1765。

# Outcome

対象者: Releash の利用者（開発者）と、Releash を開発する人と agent。

現在の問題:
- Releash の画面は Tauri と React の画面だけで、ネイティブの macOS クライアントが無い。
- 今の画面は中央に選択した 1 件だけを表示し、分割できない。worktree ごとの実行状況を、一覧で一目に把握できない。
- 削除に強制が要るかの判定や Issue の絞り込みを、React の画面が自分で行っている。

変更後の状態:
- 利用者は、同梱のサーバと CLI を含む SwiftUI の .app を、外部への配置なしに起動できる。
- 利用者は、サイドバーの worktree カードで各 worktree の状態と実行中の Session・Workflow を把握できる。
- 利用者は、中央の pane をタブと分割で並べ、その配置を worktree ごとに復元できる。
- 利用者は、Worktree 作成ダイアログから worktree を作り、続けて Session か Workflow を起動できる。
- 後続の ISSUE は、ここで作った器と UI の決まりの上に中身を足す。

# Current Behavior

開発開始時点（main `efda9bb7`）で確認した状態。

ネイティブクライアント:
- `clients/macos/` は存在しない。

サイドバーと中央（React の画面）:
- 左ペインは `clients/desktop/src/components/workspace/WorkspaceList.tsx` の repository（`:1526`）→ worktree 行（`:585`）→ 実行木の展開で構成される。Session と Workflow の追加は別メニューにある（`:1261-1359`）。
- worktree 行は `branch.name` を表示する（`WorkspaceList.tsx:1078`）。worktree の表示名は無い。
- 中央は選択した 1 件だけを表示し（`clients/desktop/src/screens/MainLayout.tsx:155-192`）、分割は無い。
- worktree ごとの UI 状態はサーバに保存される（`WorkspaceStateDto`、`proto/client.proto:1873-1904`）。項目は今のレイアウト用で、repository のグループ単位の保存先は無い。

worktree の一覧と状態:
- サーバは worktree を main を先頭に、残りを branch 名の順に並べる（`server/src/usecase/repository_usecase.rs:165-174`）。
- branch の情報（`WorkspaceBranch`、`proto/client.proto:2624-2638`）は is_merged・has_pr・pr_number・pr_url・dirty_count を持ち、ahead・behind・upstream と PR の状態を持たない。
- サーバは `gh pr list` を `--state open` と `--state merged` で 1 回ずつ、それぞれ `--limit 100` で呼ぶ（`server/src/adaptor/gateway/git_host/github.rs:128-169`）。`PrInfo` は number と url だけを持つ（`server/src/domain/git_host/value_objects/pr.rs:4-7`）。
- worktree 単位の集約状態は無い（`WorkspaceWorktreeList`、`proto/client.proto:2639-2644`）。集約は実行木の親 Node までで、Idle から始めて最も重い値に畳む（`server/src/domain/workspace_tree/value_objects/mod.rs:156-212`）。
- Session の最後のメッセージは無い。履歴の gateway が読むのはタイトルと最初のプロンプトだけである（`server/src/adaptor/gateway/agent_session/agent_session_history_gateway.rs:120-200`）。

worktree の作成:
- `CreateWorktree`（`proto/client.proto:2526`）は同期の RPC で、要求は repo_path・branch・create_branch・base_branch だけを持つ（`proto/client.proto:629-634`）。表示名を受け取らない。
- 作成の経路に fetch は無い。base を省くと HEAD から作る。worktree の追加に失敗すると、作った branch を消す（`server/src/adaptor/gateway/repository/worktree.rs:337-393`）。
- 作成後の Session と Workflow の起動は別の RPC（`CreateAgentSession` `:2524`、`StartWorkflow` `:2572`）で、画面が順に呼ぶ。
- 作成ダイアログには Plain・Branch・Issue・Notion の 4 つのモードと複数選択がある（`clients/desktop/src/components/workspace/CreateWorktreeModal.tsx:81-86`、`:203-219`）。Plain モードでは利用者が branch 名を直接入れる（`:375-386`）。
- 画面が、選んだ branch が既存かどうかで `createBranch` を決め、選択ごとに `create_worktree` を呼ぶ（`CreateWorktreeModal.tsx:157-201`）。
- Notion の task の branch 名は、branch のプロパティに値があればその値、無ければタイトルから作る。タイトルからは ASCII の英数字だけを残して `feat/<slug>` にするので、日本語だけのタイトル「ログイン」は `feat/` になり、同じタイトルの別の task は同じ branch 名になる（`server/src/adaptor/gateway/notion/service_models.rs:228-240`、`server/src/domain/notion/services.rs:26-42`、`server/src/domain/notion/services_test.rs:91`）。
- worktree の path は `<repo>-worktrees/<branch 名>` で、branch 名の `/` を `-` に置き換える（`server/src/domain/repository/value_objects/worktree_path.rs:16-26`）。そのため `feature/a` と `feature-a` は同じ path になる。git の worktree の名前には path の最後の要素を使う（`server/src/adaptor/gateway/repository/worktree.rs:359-373`）。
- Issue の label と milestone の絞り込みは画面が行う（`CreateWorktreeModal.tsx:488-556`）。Issue の取得の要求は repo_path だけを持つ（`proto/client.proto:771-773`）。

worktree の削除:
- `RemoveWorktree` を受理すると、サーバは削除中として登録して通知し、`is_deleting` が配信される。RPC の応答は削除の完了を待つ（`server/src/usecase/repository_usecase.rs:287-351`）。
- 削除に強制が要るかは、サーバの domain が削除の実行時に判定する（`Worktree::authorize_removal`、`server/src/adaptor/gateway/repository/worktree.rs:432-446`）。一方、確認の出し分けは React の画面が変更の件数から自分で判定する（`clients/desktop/src/components/workspace/DeleteWorktreeDialog.tsx:86-111`）。

# Scope / Non-goals

Scope:
- `clients/macos/` の SwiftUI クライアント（プロジェクト、同梱、生成、発見と起動、接続、失敗の画面、接続の回復、UI の決まりの定数）
- サイドバー（repository のグループ、worktree カード、引き継ぐ操作と表示）
- 中央（pane ごとのタブ列と分割、「開く」メニュー、配置の保存と復元）
- フッターの枠
- Worktree 作成ダイアログと、作成から起動までを 1 つにした操作
- 上記に必要なサーバと proto の追加（worktree カードの値、ahead・behind・upstream、PR の状態、削除に強制が要るかの判定、Issue の絞り込み、UI 状態の項目）
- PR の CI の macOS の job

Non-goals:
- pin（カードの pin のアイコンと、保存する UI 状態の項目を含む）
- 作成の cancel と retry、進行中の行と失敗した行、作成を受け付けるジョブと、進捗・失敗を配信する購読
- 作成前の fetch。worktree はローカルの ref から作る
- agent 行の最後のメッセージと経過時間
- worktree の表示名（作成ダイアログの欄、サーバへの保存）
- Issue と Notion の task へのリンクの保存
- 右の切替面（[05]）
- タブの中身（Terminal・Workflow 管理・ファイル。後続の ISSUE）
- agent 行・workflow 行からタブを開く操作（[09]、[04]）
- フッターの中身（[13] #1944）
- repository の削除（[10] #1763）
- テーマと文字の大きさの設定
- E2E による確認（[02] #2012）
- React の画面の変更。削除の確認の出し分けを React が自分で判定する処理は、撤去（[16] #1765）まで残す
- ISSUE 本文の書き換え
- mise の導入と、手元の buf の版の固定（#2037）。Renovate への移行（#2038）

# Requirements

基盤:
- R-001: `clients/macos/` に、XcodeGen の `project.yml` で定義した Swift プロジェクトがある。ターゲットはアプリ本体とユニットテストで、`.xcodeproj` は、`Releash.xcodeproj/project.xcworkspace/xcshareddata/swiftpm/Package.resolved` を除いてコミットしない。最低対応は macOS 15。
- R-002: .app はサーバ（`releashd`）と CLI（`releash`）の実行ファイルを helper として `Contents/Helpers/` に含み、外部への配置なしに起動できる。Rust のライブラリは Swift.app にリンクしない。
- R-003: Swift の通信の型と呼び出しは `proto/client.proto` から swift-protobuf と connect-swift で生成し、HTTP クライアントに `URLSessionHTTPClient` を使う。
- R-004: Swift.app は、サーバの発見・互換判定・起動を、同梱した CLI（`releash status --json`、`releash server start`）を呼んで行う。同じ規則を Swift に実装しない。Swift.app はサーバを監督しない（起動し直さず、強制終了しない）。
- R-005: Swift.app のサーバへの要求は `Authorization: Bearer <operator の token>` を持ち、`Origin` を持たない。
- R-006: サーバを発見できない・互換でない・起動できないとき、画面はその理由と CLI の案内文を表示し、「再試行」を置く。「再試行」を押すと、発見（`releash status --json`）から起動（`releash server start`）までをやり直す。
- R-007: `http://127.0.0.1` への平文の接続が ATS で許可されている。
- R-008: 余白と寸法の定数、意味を持つ色（状態の 3 色、差分の追加と削除）、共通の style が 1 か所にまとまっている。それ以外の色はシステム色、部品は標準のものを使う。
- R-009: サーバとの接続が切れると、Swift.app は再接続し、状態を取り直し、購読し直して回復する。
- R-010: PR の CI に macOS の job があり、Swift のビルドと単体テストを実行する。Swift のビルドと単体テストは手元で成功する。

サイドバー:
- R-011: サイドバーは worktree を repository ごとのグループに分けて表示し、グループを折りたためる。折りたたみはグループ単位でサーバに保存され、Swift.app を起動し直しても復元される。
- R-012: サイドバーは、各 repository の既存の worktree を 1 worktree につき 1 枚のカードで表示する。作成中や作成に失敗した worktree の行は無い。main の worktree とそれ以外を区別して表示する。
- R-013: カードのタイトルは、その worktree の branch 名である。
- R-014: カードは、その worktree に属する archive されていない実行木を 1 つずつ行で表示する。Session 単体の実行木は agent 行、Workflow の実行木は workflow 行になる。agent 行を先に、workflow 行を後に並べる。同じ種別の中の順序は定めない。
- R-015: agent 行は、状態記号（3 値）、agent のアイコン、名前を表示する。
- R-016: workflow 行は「workflow 名 · N nodes」と、Session の Node ごとの四角を表示する。N は起動済みの Session と Command の Node の数で、Sequence と Fanout の Node、過去の試行は数えない。四角は起動済みの Session の Node 1 つにつき 1 つで、その Node の状態の 3 値の色で塗る。過去の試行は数えず、灰色の四角は無い。
- R-017: カードは worktree の集約状態を状態記号で表示する。集約状態は、archive されていない実行木が 1 つ以上あるときだけ値を持ち、それらの状態のうち最も重いもの（黄、青、緑の順に重い）である。実行木が 1 つも無いカードは状態記号を表示しない。
- R-018: カードの 2 行目は、ahead と behind、PR のアイコンを表示し、branch 名を表示しない。ahead と behind は upstream があるときだけ表示する。出すものが無いとき（upstream も PR も無いとき）は、2 行目を表示しない。PR のアイコンは PR の状態（open・draft・merged・closed）を区別する。
- R-035: PR のアイコンと merged の判定が出る branch の範囲は、開発開始時点より狭くならない。
- R-019: サイドバーで次の操作と表示ができる。repository の追加、一覧の再読み込み、読み込みの失敗の表示、変更の件数、「Changes unavailable」と「PR unavailable」の表示、既存の worktree への Session と Workflow の追加（Workflow は依頼文の入力を含む）。
- R-020: サイドバーから worktree を削除できる。削除に強制が要るかはサーバが判定して worktree の一覧で配信し、Swift.app はその値に従って、強制を求める確認とそうでない確認を出し分ける。削除中の worktree は削除中と表示される。
- R-021: agent 行と workflow 行をクリックすると、その行の worktree のカードを選んだときと同じく、その worktree のタブ群が中央に表示される。

中央とフッター:
- R-022: 中央は pane ごとにタブ列を持つ。タブのドラッグで、pane を左右・上下に分割でき、タブを別の pane へ移せる。タブを閉じられる。
- R-023: タブ列の「+」は「開く」メニューを出し、Terminal・Workflow 管理・ファイルを選べる。選ぶと、その種類のタブが中身の無い仮の表示で開く。開く・閉じる・移す・分割する・保存と復元は、3 つの種類のどれでもできる。
- R-024: 分割とタブ群は worktree ごとにサーバへ保存され、その worktree を選び直したときと Swift.app を起動し直したときに復元される。
- R-025: タブ列と分割は外部のライブラリを使わずに作る。取り込むコードは MIT のものだけで、ライセンスの表記を付ける。GPL のコードは写さない。
- R-026: 画面の下にフッターの枠がある。

Worktree の作成:
- R-027: Worktree 作成ダイアログで、Repository と base branch を入れ、branch 名は Advanced で入れる。ダイアログに表示名の欄は無い。
- R-028: ダイアログで GitHub の Issue か Notion の task を選ぶと、branch 名はその Issue か task から導かれる。Notion の task の branch 名は、task の branch のプロパティに値があればその値、無ければ `feat/<task の id>`（task の id は Notion の page の id）である。Issue と task は branch 名を決めるためだけに使い、リンクとしては保存しない。
- R-029: Issue を選ぶ一覧は label と milestone で絞り込める。絞り込みはサーバが行う。
- R-030: Advanced では、新しい branch 名を入れるか、worktree を持たない既存の branch を一覧から選べる。Issue も task も選ばないときは、branch 名を入れるか選ぶまで送信できない。
- R-031: Issue、task、既存の branch は複数選べる。task は 1 件ずつ区別して選べる。選んだものごとに worktree が 1 つ作られ、別の branch の worktree は別の path に作られる。同じ「作成後に起動する」がそれぞれに適用される。
- R-032: 「作成後に起動する」で、起動しない・Session（provider を選ぶ）・Workflow（依頼文を入れる）のどれかを選べる。worktree の作成と起動は、サーバへの 1 回の呼び出しで行われる。
- R-033: 送信するとダイアログはすぐ閉じ、利用者は作成の完了を待たずに操作を続けられる。作られた worktree は、worktree の一覧の配信によってサイドバーにカードとして現れる。作成または起動に失敗すると、画面は失敗の理由を表示する。

サーバとの境界:
- R-034: この開発で追加するサーバの呼び出しと購読は、特定の client に依存しない形である。状態は購読で配信し、単発の呼び出しは状態を変える操作と client の入力に対する計算だけに使う。判定・分類・集約はサーバの domain が行い、Swift.app は行わない。

# Assumptions

なし
