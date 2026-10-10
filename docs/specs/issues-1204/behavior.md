## B-001: Swift プロジェクトを生成してビルドできる

GIVEN リポジトリを取得した直後で、`.xcodeproj` は `Releash.xcodeproj/project.xcworkspace/xcshareddata/swiftpm/Package.resolved` を除いてコミットされていない
WHEN `clients/macos/project.yml` から XcodeGen でプロジェクトを生成し、ビルドする
THEN アプリ本体とユニットテストのターゲットが生成され、ビルドが成功する
AND アプリの最低対応は macOS 15 である

## B-002: .app だけでサーバに繋がる

GIVEN サーバと CLI がどこにも配置されていない Mac で、サーバが起動していない
WHEN ビルドした .app を起動する
THEN .app に含まれる CLI がサーバを起動し、Swift.app がそのサーバに接続する
AND .app は `Contents/Helpers/` に `releashd` と `releash` の実行ファイルを含み、Rust のライブラリをリンクしていない

## B-003: 生成した型で通信する

GIVEN `proto/client.proto` の ClientService
WHEN Swift.app がサーバを呼び出す・購読する
THEN 呼び出しと購読は swift-protobuf と connect-swift の生成物を `URLSessionHTTPClient` で使う

## B-004: 発見と起動を CLI に任せる

GIVEN Swift.app が起動した
WHEN サーバの発見・互換判定・起動が要る
THEN Swift.app は同梱した CLI の `releash status --json` と `releash server start` を呼び、その結果に従う
AND Swift.app はサーバのプロセスを強制終了しない
AND 接続中のサーバが止まっても、Swift.app はサーバを起動し直さない

## B-005: 要求の認証

GIVEN Swift.app がサーバに接続した
WHEN Swift.app がサーバへ要求を送る
THEN 要求は operator の token を `Authorization: Bearer` で持ち、`Origin` を持たない

## B-006: 接続できないときの画面と再試行

GIVEN サーバを発見できない、互換でない、または起動できない
WHEN Swift.app が起動する
THEN 画面はそのどれに当たるかの理由と、CLI の案内文を表示する
AND 画面に「再試行」がある

## B-007: 再試行で接続する

GIVEN 接続できないときの画面が出ており、その原因が取り除かれた
WHEN 利用者が「再試行」を押す
THEN Swift.app は発見から起動までをやり直し、サーバに接続してメインの画面を表示する

## B-008: ループバックへの平文接続

GIVEN サーバが `http://127.0.0.1` で待ち受けている
WHEN Swift.app が接続する
THEN ATS に拒否されずに接続できる

## B-009: UI の決まりの定数

GIVEN Swift.app のソース
WHEN 余白と寸法、状態の 3 色、差分の追加と削除の色、共通の style を探す
THEN それぞれが 1 か所にまとまっている
AND 画面はそれ以外の色にシステム色を使う

## B-010: 接続断からの回復

GIVEN Swift.app がサーバに接続して一覧を表示している
WHEN サーバとの接続が切れ、その後サーバに再び接続できるようになる
THEN Swift.app は再接続し、状態を取り直し、購読し直す
AND サイドバーと中央は、切断中に起きた変化を反映した状態になる

## B-011: CI の macOS の job

GIVEN main への pull request
WHEN CI が実行される
THEN macOS の job が Swift のビルドと単体テストを実行する

## B-012: repository のグループの折りたたみ

GIVEN 2 つの repository が登録されている
WHEN 利用者が一方のグループを折りたたみ、Swift.app を起動し直す
THEN 折りたたんだグループは折りたたまれたまま、もう一方は開いたまま表示される

## B-013: worktree のカード

GIVEN repository に main とそれ以外の worktree がある
WHEN サイドバーを表示する
THEN worktree 1 つにつき 1 枚のカードが表示される
AND main の worktree はそれ以外と区別して表示される

## B-014: カードのタイトル

GIVEN branch `feat/login` の worktree がある
WHEN サイドバーを表示する
THEN そのカードのタイトルは `feat/login` である

## B-015: 実行木の行と並び

GIVEN worktree に、archive されていない Session 単体の実行木と Workflow の実行木があり、archive された実行木もある
WHEN サイドバーを表示する
THEN カードには archive されていない実行木ごとに 1 行が表示され、agent 行が workflow 行より先に並ぶ
AND archive された実行木の行は表示されない

## B-016: agent 行

GIVEN worktree に Session 単体の実行木がある
WHEN サイドバーを表示する
THEN agent 行は、その Session の状態の記号、agent のアイコン、名前を表示する

## B-017: workflow 行

GIVEN worktree に Workflow の実行木があり、起動済みの Session の Node が 2 つ、Command の Node が 1 つ、Sequence の Node が 1 つ、Session の Node の過去の試行が 1 つある
WHEN サイドバーを表示する
THEN workflow 行は「<workflow 名> · 3 nodes」を表示する
AND 四角は 2 つで、それぞれの Session の Node の状態の色で塗られる

## B-018: worktree の集約状態

GIVEN worktree に、状態が黄の実行木と緑の実行木がある
WHEN サイドバーを表示する
THEN カードの状態記号は黄である

## B-019: 実行木の無いカード

GIVEN worktree に archive されていない実行木が無い
WHEN サイドバーを表示する
THEN カードは状態記号を表示しない

## B-020: ahead・behind と PR の表示

GIVEN worktree の branch に upstream があり、upstream より 2 commit 進み 1 commit 遅れていて、その branch の draft の PR がある
WHEN サイドバーを表示する
THEN カードの 2 行目は ahead 2 と behind 1、draft を表す PR のアイコンを表示し、branch 名を表示しない

## B-021: upstream の無い branch

GIVEN worktree の branch に upstream が無い
WHEN サイドバーを表示する
THEN カードの 2 行目は ahead と behind を表示しない

## B-047: 2 行目に出すものが無いカード

GIVEN worktree の branch に upstream が無く、その branch の PR も無い
WHEN サイドバーを表示する
THEN カードは 2 行目を表示しない

## B-022: PR の状態の区別

GIVEN open・draft・merged・closed の PR をそれぞれ持つ branch の worktree がある
WHEN サイドバーを表示する
THEN 各カードの PR のアイコンは、4 つの状態を互いに区別できる

## B-023: 引き継ぐ操作と表示

GIVEN サイドバーを表示している
WHEN 利用者が repository を追加する、または一覧を再読み込みする
THEN 一覧に反映される
AND 一覧の読み込みに失敗した repository には失敗が表示される
AND 各カードには変更の件数が表示され、変更の件数または PR を取れないときは「Changes unavailable」または「PR unavailable」が表示される

## B-024: 既存の worktree への追加

GIVEN worktree のカードがある
WHEN 利用者がそのカードから Session を追加する、または依頼文を入れて Workflow を追加する
THEN その worktree で Session または Workflow が起動し、カードに行が加わる

## B-025: 強制の要らない削除

GIVEN サーバが、強制なしで削除できると配信している worktree がある
WHEN 利用者がそのカードの削除を選ぶ
THEN 強制を求めない確認が出る
AND 確認すると、カードは削除中と表示された後、一覧から消える

## B-026: 強制の要る削除

GIVEN サーバが、削除に強制が要ると配信している worktree がある
WHEN 利用者がそのカードの削除を選ぶ
THEN 強制を求める確認が出る
AND 確認すると、カードは削除中と表示された後、一覧から消える

## B-027: 行のクリック

GIVEN worktree A のカードに agent 行と workflow 行があり、中央に worktree B のタブ群が表示されている
WHEN 利用者が A の agent 行または workflow 行をクリックする
THEN A のカードが選ばれ、中央に A のタブ群が表示される

## B-028: タブのドラッグで分割する

GIVEN pane に 2 つのタブがある
WHEN 利用者が一方のタブを pane の右端、左端、上端、下端のいずれかへドラッグする
THEN pane がその向きに分割され、ドラッグしたタブは新しい pane に移る

## B-029: タブを別の pane へ移す・閉じる

GIVEN 2 つの pane がある
WHEN 利用者が一方の pane のタブを他方の pane のタブ列へドラッグする
THEN タブはドラッグ先の pane へ移る
AND 利用者はタブを閉じられる

## B-030: 「開く」メニュー

GIVEN pane のタブ列がある
WHEN 利用者が「+」を押す
THEN Terminal・Workflow 管理・ファイルを選べるメニューが出る
AND どれかを選ぶと、その種類のタブが中身の無い仮の表示で開く

## B-031: 配置の保存と復元

GIVEN worktree A で pane を分割し、Terminal・Workflow 管理・ファイルのタブを開いた
WHEN 利用者が別の worktree を選んでから A を選び直す、または Swift.app を起動し直して A を選ぶ
THEN A の分割とタブ群が、開いていたときと同じ配置で表示される

## B-032: タブ列と分割の出所

GIVEN Swift.app の依存とソース
WHEN タブ列と分割の実装を確かめる
THEN 外部のライブラリに依存していない
AND 取り込んだコードは MIT のものだけで、ライセンスの表記がある

## B-033: フッターの枠

GIVEN Swift.app がメインの画面を表示している
WHEN 画面を見る
THEN 画面の下にフッターの枠がある

## B-034: 作成ダイアログの入力

GIVEN Worktree 作成ダイアログを開いた
WHEN 利用者がダイアログを見る
THEN Repository、base branch、「作成後に起動する」を入れられる
AND branch 名は Advanced を開くと入れられる
AND 表示名の欄は無い

## B-035: Issue から作る

GIVEN Worktree 作成ダイアログを開いた
WHEN 利用者が Issue を 1 つ選んで送信する
THEN その Issue から導いた branch 名で worktree が作られる

## B-036: Notion の task から作る

GIVEN Worktree 作成ダイアログを開いた
WHEN 利用者が Notion の task を 1 つ選んで送信する
THEN その task から導いた branch 名で worktree が作られる

## B-048: branch のプロパティが無い Notion の task

GIVEN branch のプロパティに値が無く、タイトルが「ログイン」の Notion の task がある
WHEN 利用者がその task を選んで送信する
THEN `feat/<その task の id>` の branch で worktree が作られる

## B-049: 同じタイトルの Notion の task

GIVEN branch のプロパティに値が無く、タイトルが同じ 2 つの Notion の task がある
WHEN 利用者が一方の task を選ぶ
THEN もう一方の task は選ばれていない
AND 両方を選んで送信すると、task ごとに別の branch で worktree が 1 つずつ作られる

## B-050: `/` の有無だけが違う branch 名

GIVEN Worktree 作成ダイアログで、新しい branch 名 `feature/a` と `feature-a` を両方選んだ
WHEN 利用者が送信する
THEN `<repo>-worktrees/feature/a` と `<repo>-worktrees/feature-a` の別々の path に worktree が 2 つ作られる

## B-037: Issue の絞り込み

GIVEN repository に、label または milestone の異なる Issue がある
WHEN 利用者が Issue の一覧を label と milestone で絞り込む
THEN 一覧には条件に合う Issue だけが表示される

## B-038: 既存の branch から作る

GIVEN worktree を持たない既存の branch と、worktree を持つ既存の branch がある
WHEN 利用者が Advanced の既存の branch の一覧を開く
THEN 一覧には worktree を持たない branch だけが表示される
AND 選んで送信すると、その branch の worktree が作られる

## B-039: branch 名が無いと送信できない

GIVEN Worktree 作成ダイアログで Issue も task も選んでいない
WHEN Advanced で branch 名を入れておらず、既存の branch も選んでいない
THEN 送信できない

## B-040: 複数選択

GIVEN Worktree 作成ダイアログで Issue を 2 つ選び、「作成後に起動する」で Session を選んだ
WHEN 利用者が送信する
THEN 各 Issue から導いた branch 名で worktree が 2 つ作られる
AND それぞれの worktree で Session が起動する

## B-041: 複数の既存の branch

GIVEN Worktree 作成ダイアログで既存の branch を 2 つ選んだ
WHEN 利用者が送信する
THEN 選んだ branch ごとに worktree が 1 つずつ作られる

## B-042: 作成して Workflow を起動する

GIVEN Worktree 作成ダイアログで「作成後に起動する」に Workflow を選び、依頼文を入れた
WHEN 利用者が送信する
THEN worktree が作られ、その worktree でその依頼文の Workflow が起動する

## B-043: 送信してすぐ閉じる

GIVEN Worktree 作成ダイアログに必要な値を入れた
WHEN 利用者が送信する
THEN ダイアログはすぐ閉じ、利用者は作成の完了を待たずにほかの操作ができる
AND 作成が終わると、作られた worktree のカードがサイドバーに現れる

## B-044: 作成の失敗

GIVEN Worktree 作成ダイアログで、worktree の作成または起動が失敗する値を入れた
WHEN 利用者が送信する
THEN 画面は失敗の理由を表示する

## B-045: サーバとの境界

GIVEN この開発で追加したサーバの呼び出しと購読
WHEN その形と、Swift.app の処理を確かめる
THEN 呼び出しと購読は Swift.app に固有の値を持たない
AND 状態は購読で配信され、単発の呼び出しは状態を変える操作か入力に対する計算である
AND 集約状態、削除に強制が要るか、Issue の絞り込み、ahead と behind、PR の状態は、Swift.app ではなくサーバが求める

## B-046: PR の表示の範囲を狭めない

GIVEN 開発開始時点の Releash で、PR のアイコンまたは merged の判定が出ている branch の worktree がある
WHEN 同じ repository と PR の状態で、Swift.app のサイドバーを表示する
THEN その worktree のカードにも、PR のアイコンまたは merged の判定が出る

## 要件IDとBehavior IDの対応表
| Requirement ID | Behavior ID |
| --- | --- |
| R-001 | B-001 |
| R-002 | B-002 |
| R-003 | B-003 |
| R-004 | B-004 |
| R-005 | B-005 |
| R-006 | B-006, B-007 |
| R-007 | B-008 |
| R-008 | B-009 |
| R-009 | B-010 |
| R-010 | B-011 |
| R-011 | B-012 |
| R-012 | B-013 |
| R-013 | B-014 |
| R-014 | B-015 |
| R-015 | B-016 |
| R-016 | B-017 |
| R-017 | B-018, B-019 |
| R-018 | B-020, B-021, B-022, B-047 |
| R-035 | B-046 |
| R-019 | B-023, B-024 |
| R-020 | B-025, B-026 |
| R-021 | B-027 |
| R-022 | B-028, B-029 |
| R-023 | B-030 |
| R-024 | B-031 |
| R-025 | B-032 |
| R-026 | B-033 |
| R-027 | B-034 |
| R-028 | B-035, B-036, B-048 |
| R-029 | B-037 |
| R-030 | B-038, B-039 |
| R-031 | B-040, B-041, B-049, B-050 |
| R-032 | B-040, B-042 |
| R-033 | B-043, B-044 |
| R-034 | B-045 |
