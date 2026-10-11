# Design

## 変える部分
- Swift プロジェクト: `clients/macos/` に XcodeGen の `project.yml` を置き、アプリ本体とユニットテストのターゲットを定義する。`.xcodeproj` は `Releash.xcodeproj/project.xcworkspace/xcshareddata/swiftpm/Package.resolved` を除いて ignore する。根拠: R-001「XcodeGen の `project.yml` で定義した Swift プロジェクトがある」。ルート: 「固定するルート」1
- サーバと CLI の同梱: build phase で cargo を呼んで `releashd` と `releash` をビルドし、.app の `Contents/Helpers/` に helper として入れる。Swift.app が同梱の CLI を呼ぶ path も合わせる。根拠: R-002「.app はサーバ（`releashd`）と CLI（`releash`）の実行ファイルを helper として `Contents/Helpers/` に含み」。ルート: 「固定するルート」1
- 通信のコード生成: `proto/client.proto` から swift-protobuf と connect-swift で Swift のコードを生成し、`URLSessionHTTPClient` で呼ぶ。根拠: R-003。ルート: 「固定するルート」1
- 発見と起動: 同梱の `releash status --json` と `releash server start` を呼び、その結果から接続先と token を得る。根拠: R-004、R-005。ルート: 「固定するルート」1
- 接続できないときの画面: CLI の結果から、発見できない・互換でない・起動できないの区別と案内文を表示し、「再試行」で発見から起動までをやり直す。根拠: R-006「その理由と CLI の案内文を表示し、『再試行』を置く」。ルート: 委任
- ATS: `http://127.0.0.1` への平文の接続を許可する設定を加える。根拠: R-007。ルート: 委任
- 接続の回復: 接続が切れたら再接続し、状態を取り直し、購読し直す。根拠: R-009。ルート: 委任
- UI の決まりの定数: 余白と寸法、状態の 3 色、差分の追加と削除の色、共通の style を 1 か所にまとめる。根拠: R-008。ルート: 委任
- CI: `.github/workflows/ci.yml` に macOS の job を加え、Swift のビルドと単体テストを実行する。根拠: R-010。ルート: 「固定するルート」8
- ビルドに使うツール: XcodeGen、swift-protobuf、connect-swift の生成ツールを `clients/macos/BuildTools/` の Swift package で版を固定して呼び、アプリの依存と同じ版にそろえる。アプリの `Package.resolved` を commit する。根拠: R-001「XcodeGen の `project.yml` で定義した Swift プロジェクトがある」、R-003「swift-protobuf と connect-swift で生成し」、R-010「Swift のビルドと単体テストは手元で成功する」。ルート: 「固定するルート」8
- サイドバー: repository のグループ、worktree カード（Home または branch のアイコンとタイトル、インデントした詳細・agent 行・workflow 行）、引き継ぐ操作と表示、削除の確認を Swift で作る。根拠: R-011〜R-021。ルート: 委任
- 中央: pane ごとのタブ列、ドラッグによる分割と移動、「開く」メニュー、中身の無い Session・Terminal・Workflow 管理・ファイルのタブを Swift で作る。根拠: R-022、R-023、R-025。ルート: 「固定するルート」2
- フッター: 画面の下に中身の無い枠を置く。根拠: R-026。ルート: 委任
- Worktree 作成ダイアログ: Repository、base branch、Advanced の branch 名と既存の branch の一覧、Issue と Notion の task の選択（複数選択と Issue の絞り込みの条件を含む）、「作成後に起動する」を Swift で作る。送信でダイアログを閉じ、失敗は呼び出しの応答で受けて理由を表示する。根拠: R-027〜R-033。ルート: 委任
- 作成から起動まで: worktree の作成と、続く Session または Workflow の起動を、サーバの 1 つの usecase と 1 つの呼び出しにする。複数選択のときの扱いを含む。根拠: R-031、R-032「worktree の作成と起動は、サーバへの 1 回の呼び出しで行われる」、R-033。ルート: 「固定するルート」3
- Issue の絞り込み: Issue の取得の要求に label と milestone の条件を加え、サーバが絞り込む。根拠: R-029「絞り込みはサーバが行う」。ルート: 「固定するルート」4
- worktree カードの値: 実行木ごとの種別（Session 単体か Workflow か）、workflow 行の N、Session の Node ごとの状態をサーバが求め、worktree の一覧で配信する。根拠: R-014〜R-017、R-034。ルート: 「固定するルート」5
- Notion の task の branch 名: branch のプロパティに値が無い task の branch 名を、タイトルからではなく `feat/<task の id>` で作る。タイトルから作る処理は使われなくなったら消す。作成ダイアログは task の選択を task の id で区別する。根拠: R-028「無ければ `feat/<task の id>`」、R-031「task は 1 件ずつ区別して選べる」。ルート: 「固定するルート」9
- worktree の path: branch 名の `/` を `-` に置き換えず、ディレクトリの階層のまま path にする。git の worktree の名前も重ならないように付ける。根拠: R-031「別の branch の worktree は別の path に作られる」。ルート: 「固定するルート」10
- PR の状態: サーバが PR を open・draft・merged・closed の状態つきで取り、worktree の一覧で配信する。merged の判定は今の規則（git で merge 済み、または merged の PR があり open の PR が無い）のまま、取った PR から行う。根拠: R-018「PR のアイコンは PR の状態（open・draft・merged・closed）を区別する」、R-035「開発開始時点より狭くならない」。ルート: 「固定するルート」6
- 削除に強制が要るか: サーバの domain が worktree ごとに判定し、worktree の一覧で配信する。根拠: R-020「削除に強制が要るかはサーバが判定して worktree の一覧で配信し」。ルート: 「固定するルート」7
- 保存する UI 状態: worktree ごとの分割とタブ群の項目と、repository のグループ単位の保存先と折りたたみの項目をサーバに加える。根拠: R-011「折りたたみはグループ単位でサーバに保存され」、R-024。ルート: 「固定するルート」2

## 固定するルート
1. 基盤
   - XcodeGen の `project.yml` で定義し、`.xcodeproj` はコミットしない。
   - サーバと CLI は build phase で cargo を呼んでビルドし、helper として .app の `Contents/Helpers/` に入れる。`Contents/MacOS/` に置くと、CLI の `releash` がアプリ本体の `Releash` と大文字と小文字だけが違う名前になり、大文字と小文字を区別しないファイルシステムで同じファイルとして扱われるため。Swift.app が同梱の CLI を呼ぶ path も `Contents/Helpers/releash` にする。Rust のライブラリは Swift.app にリンクしない。
   - 通信は swift-protobuf と connect-swift の生成物を `URLSessionHTTPClient` で使う。
   - 発見・互換判定・起動は同梱した CLI を呼んで行い、同じ規則を Swift に実装しない。Swift.app はサーバを監督しない。
2. タブ列と分割
   - Bonsplit（MIT）は fork `https://github.com/siro33950/bonsplit` の `releash` branch を使い、`project.yml` の `revision` で commit を固定する。既存の 9 項目の変更を fork 側に移し、Releash 側にはソースを複製しない。追加の表示修正として、選択タブの線色を公開設定から指定できるようにする。`Resources/Bonsplit-LICENSE` をアプリに同梱する。
   - 「+」から Session の provider 選択、Workflow の定義と依頼文の入力を開く。既存の `createAgentSession` / `startWorkflow` の成功後に対応するタブを開く。開始中は入力とキャンセルを無効にし、失敗は入力画面に表示する。Session の種類を proto・保存形式・Swift の配置型に加える。
   - 右の切替面はファイル・Source Control・Thread・タスクの順とし、中身は見出しだけの仮表示にする。最初はファイルを開き、開閉と選択はウィンドウ内だけで保持する。状態の永続化は行わない。
   - 各 pane のタブ列に「開く」メニューを挿入する拡張口と、保存した配置を復元する API を追加する。SwiftUI と AppKit の境界でも controller を引き渡す。
   - 配置の正はサーバに置く。Bonsplit の状態を既存の `PaneLayout` に変換し、pane と tab の識別子、種類、選択、分割方向、比率を保存する。
   - フッターを含む外枠全体に余白と角丸を付け、外周の枠線は描かない。サイドバー、中央、フッターはタブ境界と同じ色の直線で区切る。左右のカラムは `HSplitView` で幅を変更でき、境界では左右方向のリサイズカーソルを表示する。右カラムを閉じたときは固定幅にする。サイドバーは `List` の sidebar style と折りたたみ可能な `Section` を使う。カードは Button で選択し、選択中のカード内にシステム色の灰色の背景を描く。List 標準の選択背景は使わず、キーボードフォーカスを受け取っても青くしない。Repository の見出しは選択対象から外す。未選択カードの本文は primary の文字色にし、workflow 行の文字は副次的な色にする。カードの先頭行は Home または branch のアイコンと名前だけにし、他の情報は下の行で名前の開始位置にそろえる。worktree の集約状態の丸は表示しない。Session は既存の Lucide Bot、Workflow は既存の Lucide Waypoints をテンプレート画像として同梱し、状態色で表示する。Session と Workflow のアイコン欄の幅と間隔をそろえる。Workflow 配下の Node の状態アイコンは、Workflow 行からさらに一段インデントする。
   - ウィンドウは枠なしの NSWindow を使い、canBecomeKey と canBecomeMain を有効にする。NSHostingView で画面を表示し、ネイティブのタイトルバーがタブのドラッグを奪わないようにする。最初のウィンドウの位置と大きさは AppKit の frame autosave で復元する。左上に AppKit 標準のウィンドウボタンを置き、その右の空白でウィンドウを移動でき、ポインタを重ねると手のカーソルを表示する。タブ列の「+」は最後のタブの直後に置く。各 pane の表示中タブの上端は同じ灰色の線で示し、最後に操作した pane だけを青くしない。
   - コードを取り込むのは MIT、Apache、BSD、ISC のものに限り、表記を付ける。既存の Session と Workflow のアイコンを維持するため、Lucide Bot と Waypoints（ISC）を同梱し、Resources/Lucide-LICENSE を添付する。GPL のコードは写さない。
3. 作成から起動まで
   - 1 つの usecase にする。
   - 呼び出しは同期のままにし、worktree の追加と起動が終わってから成功か失敗を返す。作成を受け付けるジョブと、進捗・失敗を配信する購読は作らない。
   - 作られた worktree は、既存の worktree の一覧の購読で画面に届く。失敗は呼び出しの応答で画面に届く。
   - 作成前に fetch せず、ローカルの ref から作る。
4. Issue の絞り込み
   - label と milestone の条件を Issue の取得に渡し、サーバが絞り込む。
5. worktree カードの値
   - archive されていない実行木ごとに、種別、workflow 行の N、Session の Node ごとの状態を domain が求める。
   - worktree 単位の集約状態と、ahead・behind・upstream は求めず、配信もしない。カードに表示しないためである。
6. PR の状態
   - `PrInfo` を広げ、PR の状態と draft を持たせる。
   - 取り方は、`gh pr list` を状態ごとに `--state open`・`--state merged`・`--state closed` の 3 回、それぞれ `--limit 100` で呼ぶ。open の取得で draft も取る。open と merged の取り方は今（`server/src/adaptor/gateway/git_host/github.rs:128-169`）と同じ範囲になり、R-035 の範囲を狭めない。`--state all` の 1 回にすると、同じ上限では全状態で直近の 100 件しか取れず、今より狭くなるため使わない。
7. 削除に強制が要るか
   - サーバの domain が判定し、worktree の一覧の購読に載せる。Swift.app は判定しない。

8. ビルドに使うツールと CI
   - 生成ツール（xcodegen、protoc-gen-swift、protoc-gen-connect-swift）は BuildTools で版を固定し、グローバルな環境（brew、PATH、`/private/tmp` など）には入れない。実装担当はツールを入れず、`/private/tmp` に取得済みの xcodegen・swift-protobuf・connect-swift も使わない。Xcode は利用者が入れる。
   - buf は、#1204 では CI の buf（`bufbuild/buf-action` の `version: 1.47.2`）だけを固定する。手元の buf は `AGENTS.md` の「buf は CI と同じ 1.47.2 を使う」に従い、`generate.sh` は手元では PATH の buf を呼ぶ。手元の buf の版を固定する仕組みは #2037 で入れる。
   - `clients/macos/BuildTools/Package.swift` に XcodeGen、swift-protobuf、connect-swift を exact の版で書き、`Package.resolved` と一緒に commit する。呼び出しは `swift run --package-path clients/macos/BuildTools <tool>` とする。`buf.gen.yaml` の `local:` はコマンドの列（例: `["swift", "run", "--package-path", "clients/macos/BuildTools", "protoc-gen-swift"]`）で書き、PATH を使わない。
   - `project.yml` の `exactVersion` と BuildTools の版を同じにする（connect-swift 1.0.0、swift-protobuf 1.38.1）。`generate.sh` の版の文字列の検査は、`Package.resolved` で版が決まるので消す。
   - `.gitignore` は `.xcodeproj` の中身を ignore したまま、`Releash.xcodeproj/project.xcworkspace/xcshareddata/swiftpm/Package.resolved` だけを `!` で戻して commit する。親のディレクトリ（`Releash.xcodeproj/`、`project.xcworkspace/`、`xcshareddata/`、`swiftpm/`）は丸ごとではなく、それぞれの中身を ignore する形にする。CI の `xcodebuild` には `-disableAutomaticPackageResolution` を付ける。
   - `clients/macos/.xcode-version` に `27.0` と書く（最新の安定版の Xcode）。CI の macos-native job は `runs-on: xcode-27` とし（既定の Xcode が 27.0 の image）、このファイルを読んで setup-xcode の `xcode-version` に渡す。
   - CI の macos-native job の buf は `bufbuild/buf-action` の `version: 1.47.2` で入れ、lint の job とそろえる。`brew install xcodegen buf swift-protobuf` と connect-swift の取得は消す。
   - CI では、依存の解決をビルドと別の段に分ける。`-clonedSourcePackagesDirPath` の置き場所と BuildTools の `.build` を、それぞれの `Package.resolved` のハッシュをキーにしてキャッシュする。

9. Notion の task の branch 名
   - task の branch のプロパティに値があれば、今どおりその値を使う。
   - 値が無いときは `feat/{id}` とする。id は `NotionTask.id`（Notion の page の id。`server/src/domain/notion/value_objects.rs:28`、gateway は page の `"id"` から取る `server/src/adaptor/gateway/notion/service_models.rs:191-195`）。
   - タイトルから作る `notion_task_title_branch_name`（`server/src/domain/notion/services.rs:26-42`）は、使われなくなったら消す。
   - 作成ダイアログの task の選択は、branch 名ではなく task の id で区別する。

10. worktree の path
   - path は `<repo>-worktrees/<branch 名>` とし、branch 名の `/` をディレクトリの階層にする（例: `feature/a` は `<repo>-worktrees/feature/a`、`feature-a` は `<repo>-worktrees/feature-a`）。`/` を `-` に置き換える `branch_to_dir`（`server/src/domain/repository/value_objects/worktree_path.rs:16-26`）はやめる。
   - git は、ある branch 名が別の branch 名の親の階層になる組み合わせ（`feature` と `feature/a`）を作らせない。ref をディレクトリの階層で持つためである。そのため、branch 名の階層をそのまま path にすれば、別の branch が同じ path になることは無い。
   - 既に作られている worktree の path は変えない。path は git から読む。
   - git の worktree の名前（`$GIT_DIR/worktrees/<名前>`）も重ならないようにする。path の最後の要素だけを名前にすると、`feature/a` と `bugfix/a` が同じ名前 `a` になる。`git worktree add` と同じく、名前が既にあれば番号を付けるなどして重ならない名前を付ける。名前の付け方の細部は実装に任せる（`repo.worktree` に渡す名前。`server/src/adaptor/gateway/repository/worktree.rs:359-373`）。

## 変えないもの
- React の画面: 削除の確認の出し分けを React が自分で判定する処理（`clients/desktop/src/components/workspace/DeleteWorktreeDialog.tsx:86-111`）は、撤去（[16] #1765）まで今のまま残す。

## 未確定・リスク
なし
