# Context

- 要求の正本: [siro33950/releash#2011](https://github.com/siro33950/releash/issues/2011)
- 補助資料: [#1763](https://github.com/siro33950/releash/issues/1763)、マイルストーン [#78](https://github.com/siro33950/releash/milestone/78) の「確定した設計」と「UI の決まり」、[Textual](https://github.com/gonzalezreal/textual)
- 対象の commit: main `99a6a2af`

確定済みの制約は次のとおりである。

- workflow 定義と Facet の画面は、一覧、詳細、診断と「外部エディタで開く」だけを持つ。編集は外部エディタで行う（マイルストーン #78「確定した設計」）。
- Swift.app の画面は SwiftUI の標準の部品と標準の style を使い、色はシステムの色、文字はシステムの文字の段階を使う（マイルストーン #78「UI の決まり」）。
- Swift.app は自前でロジックを持たず、読みは proto の購読、操作は RPC で行う。RPC と購読の形は Swift の画面の都合に合わせず、どのクライアントからも同じ形で使えるものにする（`AGENTS.md`「サーバがロジックを所有する」、マイルストーン #78「確定した設計」）。
- 画面の置き場所は Settings のタブにする。タブの中は、一覧の面から定義の面、Facet の面へ順に重ねて戻れる形（`NavigationStack`）にし、Node の内容は定義の面の中で行を開いて見せる（利用者の決定）。Settings の寸法は今の固定値のままにする。
- 定義のフォルダを開く操作は、引数なしの RPC にし、サーバが自分の持つフォルダを開く（利用者の決定）。
- 定義と Facet を外部エディタで開くときは、定義のフォルダとそのファイルの 2 つのパスを一度にエディタへ渡し、エディタがフォルダをワークスペースとして開いた上でファイルを表示する形にする（利用者の決定）。
- 定義のソースと Facet の本文は画面に出さず、外部エディタで見る。画面は定義の Node 構成と、Node と Facet の参照関係を見せる（利用者の決定）。ISSUE #2011「やること」の「Facet の詳細（本文）」と「markdown は Textual で表示する」、マイルストーン #78「確定した設計」の「定義と Facet の詳細の markdown は Textual で表示する」は、この開発では適用しない（利用者の決定）。
- 「builtin の定義と Facet は外部エディタで開けない」という判断と Facet の種類の解釈を、gateway とテスト専用の重複実装から domain へ移す（利用者の決定）。
- 定義と Facet が builtin かどうかは、名前ではなく実際の取得元で判断する。利用者のファイルを読み込んだ場合は、builtin と同名でも利用者のものとして扱う（利用者の決定）。

# Outcome

対象者は Swift.app を使う開発者である。

Swift.app には workflow 定義と Facet を見る画面が無く、開発者は定義の一覧、定義ごとの Node の構成、Node が参照する Facet、Facet がどの定義のどの Node から参照されているか、定義と Facet の診断のどれも Swift.app から見られない。React を撤去すると、アプリの中でこれらを見る手段が無くなる。
定義と Facet を外部エディタで開く操作は、ファイル 1 つだけをエディタに渡すため、エディタはフォルダをワークスペースとして開かず、同じフォルダの他の定義や Facet へ移れない。
定義のフォルダそのものを外部エディタで開く入口は、どのクライアントにも無い。

変更後、開発者は Swift.app の Settings で定義と Facet の一覧と診断を見られ、定義を開くと Node の構成と各 Node が参照する Facet を、Facet を開くとその参照元の定義と Node を、互いに行き来しながら見られる。定義と Facet を外部エディタで開くと、エディタが定義のフォルダをワークスペースとして開いた上でそのファイルを表示する。定義のフォルダだけを外部エディタで開くこともできる。

# Current Behavior

Swift.app の Settings は General、Repositories、Providers、System の 4 タブで、幅 560、高さ 560 の固定寸法である（`clients/macos/Sources/Settings/SettingsView.swift:6-16`、`clients/macos/Sources/Style.swift:19-20`）。定義と Facet に関する表示は無く、RPC `OpenWorkflowInEditor`、`OpenFacetInEditor`、`OpenFolderInEditor` を呼ぶ箇所も無い。外部エディタを使うのはファイルタブの `OpenInEditor` だけである（`clients/macos/Sources/FileTabPane.swift:144-151`）。

React の画面は Settings の「Automation」節にあり、Workflows と Facets の 2 タブを持つ（`clients/desktop/src/components/panels/AutomationSection.tsx`）。一覧は名前、説明、builtin の印、形式（yaml / lua）、読み取りエラー、診断の件数を出し、定義の詳細は説明、診断、YAML のソース、Node の一覧を、Facet の詳細は本文、診断、本文から抽出した `{{ }}` の変数、参照元の一覧を出す（`automation/WorkflowList.tsx`、`WorkflowDetail.tsx:20-72`、`FacetList.tsx`、`FacetDetail.tsx`）。YAML の編集、新規作成、複製、削除、Facet の編集と preview もできる。

React が読む購読と、サーバが返す値は次のとおりである（`clients/desktop/src/hooks/useAutomation.ts:17-62`、`server/src/usecase/state_subscription/reads.rs:342-349`、`:432-451`、`proto/client.proto`）。

| 購読 | 引数 | 値 |
| --- | --- | --- |
| `workflows` | なし | `ListWorkflowSummaryDto`。builtin と利用者の定義の両方。各項目は名前、説明、builtin、実行中、形式、読み取りエラー（`proto/client.proto:1930-1942`） |
| `workflow` | 定義の名前 | 解釈済みの定義（`WorkflowDto`。名前、説明、builtin、形式、Node の一覧）。解釈できないときは空（`:1240-1247`） |
| `workflow-source` | 定義の名前 | ソースの文字列。YAML、Lua、builtin のいずれも返す（`server/src/adaptor/gateway/workflow/definition_repository.rs:171-179`） |
| `facets` | 種類（`policy` / `knowledge` / `instruction`） | `ListFacetSummaryDto`。builtin と利用者の Facet の両方。各項目は key、種類、説明、builtin（`:1889-1899`、`server/src/adaptor/presenter/state_subscription_target.rs:360-367`） |
| `facet` | 種類、key | 本文の文字列、または読み取りの失敗 |
| `diagnostics` | なし | `DiagnosticReport`。診断の項目（code、severity、stage、位置、message、対象の定義名、Node 名、Facet の種類と key、field）、定義名ごとの件数、`種類/key` ごとの Facet の件数、Facet の参照元（`:734-800`） |

RPC `OpenWorkflowInEditor` と `OpenFacetInEditor` は、定義または Facet のファイル 1 つのパスを解決し、選択中の外部エディタに渡す。builtin の定義と Facet は拒否する（`server/src/adaptor/gateway/workflow/editor_gateway.rs:43-94`）。エディタの起動は `open` crate で、macOS では `/usr/bin/open -a <エディタ> -- <パス>` をパス 1 つで実行する。エディタが未選択のときは `/usr/bin/open -- <パス>` で、フォルダは Finder、ファイルは既定のアプリが開く（`server/src/adaptor/gateway/external_editor/launcher_impl.rs:8-31`）。

定義と Facet のフォルダは 1 つで、サーバが起動時に環境変数 `RELEASH_WORKFLOWS_DIR` か既定の `<config_dir>/releash/workflows` を解決して持つ（`server/src/adaptor/controller/daemon.rs:435`、`server/src/adaptor/gateway/workflow/storage.rs:132-137`）。Facet の base dir も同じフォルダである（`editor_gateway.rs:36`）。このフォルダは、利用者が定義か Facet を保存するまで存在しないことがある（`storage.rs:139-144` の `ensure_dir` が保存時に作る）。RPC `OpenFolderInEditor` はクライアントが渡したパスを開くもので、サーバが持つフォルダを開く RPC は無い（`server/src/usecase/external_editor/open_usecase.rs:15-23`）。購読 `daemon-info` の `ServerInfo` にもフォルダの情報は無い（`proto/client.proto:2761-2773`）。

「builtin は外部エディタで開けない」という判断とパスの解決は gateway にあり（`editor_gateway.rs:74-94`）、usecase は Facet の種類を `dir_name()` の文字列で渡し、gateway が別名を含めて `FacetKind` に戻している（`server/src/usecase/workflow/mod.rs:269-271`、`editor_gateway.rs:98-108`）。同じ判断とパスの解決が、テスト専用の `open_facet_in_editor_inner` にもう一つある（`server/src/adaptor/controller/client/workflow/mod.rs:120-142`、呼び出し元は `server/tests/internal/adaptor_controller_client_workflow_mod.rs:242-290`）。

# Scope / Non-goals

変更する対象:

- Swift.app の Settings に足す、定義と Facet のタブ。定義の一覧と詳細（Node の構成と参照する Facet）、Facet の一覧と詳細（参照元の定義と Node）、診断の表示、「外部エディタで開く」と定義のフォルダを開く入口。
- サーバの RPC の追加。定義のフォルダを選択中の外部エディタで開く、引数なしの RPC。
- RPC `OpenWorkflowInEditor` と `OpenFacetInEditor` の振る舞いの変更。定義のフォルダとファイルの 2 つのパスを一度にエディタへ渡す。
- エディタ起動の gateway の変更。複数のパスを一度にエディタへ渡せるようにする。
- 「builtin の定義と Facet は外部エディタで開けない」という判断と Facet の種類の解釈を domain へ移し、usecase から gateway への受け渡しを `FacetKind` にし、テスト専用の `open_facet_in_editor_inner` とそのテストを消す。
- 定義と Facet を扱う既存処理の責務の修正。取得した Entity が業務の判断を持つ形にし、既存の保存・複製・削除などの経路も対象にする。
- 「定義の一覧が表示され、定義を選ぶと詳細と診断が表示される」を確かめる回帰の E2E（XCUITest）。
- 診断における builtin 定義の読み込みの扱い。利用者の定義と同じく、解釈できる builtin 定義は Facet の参照検証に失敗しても診断付きの定義として残す。

変更しない対象:

- 定義のソースの表示と、Facet の本文の表示。購読 `workflow-source` と `facet` はこの画面では使わない。
- 定義の保存、新規作成、複製、削除と、Facet の保存、新規作成、複製、削除、preview の利用者向け機能の追加・削除。これらの RPC（`SaveWorkflowSource`、`DeleteWorkflow`、`DuplicateWorkflow`、`SaveFacet`、`DeleteFacet`、`DuplicateFacet`、`RenderFacetPreview`）はサーバに残す。削除するかは #2036 が決める。
- Facet の本文から `{{ }}` の変数を抽出する表示。
- RPC `OpenFolderInEditor` と `OpenInEditor`。
- React の画面（#1765 で撤去する）。React は変更後の RPC をそのまま呼び続ける。
- 購読 `workflows`、`workflow`、`workflow-source`、`facets`、`facet`、`diagnostics` の形。
- 診断の内容と判定の規則。builtin 定義の読み込みの扱い（「変更する対象」参照）を除く。

# Requirements

- R-001: 開発者は、Swift.app の Settings に定義と Facet のタブを見つけ、そこで定義の一覧と Facet の一覧を切り替えて見られる。
- R-002: 定義の一覧には、builtin と利用者の定義の両方が並び、各定義について名前、説明、builtin であるか、実行中であるか、形式（yaml / lua）、読み取りエラーがあればその内容、診断の件数（error と info の別）が表示される。
- R-003: 開発者が一覧から定義を選ぶと、その定義の説明、その定義に対する診断の一覧（R-006）、Node の一覧が表示される。Node ごとに名前と kind と、定義が持つ内容（command、session の provider / model / permission、参照する Facet を枠（policy / knowledge / instruction）ごとに、Fanout と Sequence の子 Node の名前、artifact、input、completion、worktree）が見られる。Node が参照する Facet を選ぶと、その Facet の詳細（R-005）へ移れる。定義を解釈できないときは、診断の一覧だけが表示される。
- R-004: Facet の一覧は、種類（policy / knowledge / instruction）ごとに切り替えられ、builtin と利用者の Facet の両方が並ぶ。各 Facet について key、説明、builtin であるか、診断の件数（error と info の別）が表示される。
- R-005: 開発者が Facet を選ぶと、その key、種類、説明、その Facet に対する診断の一覧（R-006）、参照元の一覧が表示される。参照元の一覧は、その Facet を参照する全定義について、定義名、Node 名、枠（policy / knowledge / instruction）を示し、一つの Facet を複数の定義や複数の Node が参照していればその全部が並ぶ。参照元を選ぶと、その定義の詳細（R-003）へ移れる。本文は表示しない。
- R-006: 診断の一覧の各項目は、code、severity（error / info）、stage、位置（ソース名と行・列。位置を持たない項目は省く）、message を示す。
- R-007: 開発者は、利用者の定義と Facet を選択中の外部エディタで開ける。サーバは定義のフォルダとそのファイルの 2 つのパスを一度にエディタへ渡す。builtin の定義と Facet には、この操作が無い。サーバは builtin に対する要求を拒否し、その理由を返す。
- R-008: 開発者は、定義のフォルダを選択中の外部エディタで開ける。クライアントはパスを渡さず、サーバが自分の持つ定義のフォルダを開く。フォルダが存在しないときは、サーバがフォルダを作ってから開く。
- R-009: 定義または Facet のファイルが外部で変わると、Swift.app の一覧、詳細、診断の表示は、サーバの購読が届ける新しい値に追従する。
- R-010: React の画面と CLI は、`OpenWorkflowInEditor` と `OpenFacetInEditor` を変更前と同じ引数で呼べる。proto の変更は RPC の追加だけで、既存の message と RPC の形は変わらない。

- R-011: 定義と Facet の builtin 表示、および編集・保存・削除の可否は、実際の取得元に基づく。利用者のファイルから取得したものは、builtin と同名であることだけを理由に builtin と表示されたり、編集・保存・削除を拒否されたりしない。名前の妥当性や複製先の重複など、取得元とは別の規則は適用される。

# Assumptions

- 画面の置き場所は Settings のタブで、一覧の面から定義の面、Facet の面へ順に重ねて戻れる形にする。Node の内容は定義の面の中で行を開いて見せる。
- 定義のフォルダを開く操作は引数なしの RPC で、サーバが自分の持つフォルダを開く。
- 定義と Facet を外部エディタで開くときは、定義のフォルダとファイルの 2 つのパスを一度にエディタへ渡す。2 つのパスをどう扱うかはエディタの振る舞いで、サーバは保証しない。
- 定義のソースと Facet の本文は画面に出さず、外部エディタで見る。ISSUE #2011 とマイルストーン #78 の markdown 表示に関する記述は、この開発では適用しない。
- 「builtin は外部エディタで開けない」という判断と Facet の種類の解釈は domain へ移し、テスト専用の重複実装は消す。

# Invariants

なし。

# Risks

- 既知のエディタ（VS Code、Cursor、Zed、Sublime Text、TextMate、Nova、BBEdit、CotEditor）が `/usr/bin/open -a` 経由でフォルダとファイルの 2 つのパスを受けたとき、フォルダをワークスペースとして開いた上でファイルを表示するかは、実機で確かめていない。エディタがファイルだけを開く、またはフォルダとファイルを別のウィンドウで開く場合、R-007 の「フォルダをワークスペースとして開いた上でファイルを表示する」結果にならない。
