# Design

パスは、`proto/`・`docs/`・`src/`（React）で始まるもの以外は `src-tauri/` 起点。React の `src/` は `../src/` と書く。行番号は main `28df9d01` 時点。

## 変える部分

- 場所の事実: サーバに新しいドメイン `installation` を作り、実行ファイルの置かれた場所の事実（translocation されているか、読み取り専用のボリュームにあるか、ビルド種別）を 1 つの値にする。translocation の判定は 1 つにし、今の 2 つの判定（`releash-desktop/src/infrastructure/platform/cli_install.rs:168-171` の文字列の包含、`releash-desktop/src/infrastructure/platform/login_item.rs:61-64` のパスの要素）を置き換える。根拠: R-002「実行ファイルの場所の判定…は 1 つ」、B-015。ルート: 事実はパスを入力に求める処理 1 つから作る。ボリューム属性とビルド種別の観測は gateway / infrastructure が行い、判定はしない。
- CLI の配置の規則: `installation` に、場所の事実から CLI を設置してよいかと理由を決める規則を置く。translocation・読み取り専用のボリューム・development ビルドのいずれかで拒否する。設置先 `/usr/local/bin/releash` と、指し先（実行ファイルの隣の `releash`）を求める規則も `installation` に置き、usecase と controller は設置先と指し先を持たない。`/usr/local/bin/releash` の既存の状態（同じ指し先の symlink・symlink でないファイル・無いか別の指し先の symlink）から、設置済み・上書きしない・作る、を決める規則も `installation` に置く。根拠: R-003、R-004、B-003〜B-007。ルート: 拒否の理由の文言は domain の規則が決める。translocation と読み取り専用の文言は、ログイン項目の既存の文言（`releash-desktop/src/domain/login_item.rs:33`、`:36`）と同じ「Releash.app を移動する」趣旨にし、development ビルドの文言は今の `Install the CLI from a release build of Releash.app.` の趣旨を保つ。挙動の差: 読み取り専用のボリュームでの CLI の設置は、今は成功するが拒否になる。translocation のときの結果は、今の成功と「スキップ」の表示から失敗に変わる。
- ログイン項目の登録の規則: `installation` に、場所の事実からログイン項目を登録してよいかと理由を決める規則を置く。translocation・読み取り専用のボリュームで拒否し、ビルド種別は見ない。`releash-desktop/src/domain/login_item.rs:25-40` の `RegistrationLocation` とその規則は消す。根拠: R-007、B-010〜B-014。ルート: 画面が渡すパスからはビルド種別が分からない（画面のビルド種別はサーバと同じとは限らない）が、この規則はビルド種別を見ないので問題にならない。
- CLI の設置の実行: サーバの usecase で、自分自身の実行ファイルのパスから場所の事実を求め、CLI の配置の規則を当て、`/usr/local/bin/releash` の観測、symlink の作成、管理者権限での作り直しと指し先の確認を gateway / infrastructure に行わせる。指し先はサーバの実行ファイルの隣の `releash`。根拠: R-001、R-003〜R-005、B-001〜B-008。ルート: infrastructure はファイルの観測と操作（symlink の有無と指し先、symlink の作成、osascript による管理者権限の実行）だけを行い、判定しない。今の `releash-desktop/src/infrastructure/platform/cli_install.rs` の操作の部分をサーバの infrastructure に移す。macOS 固有の部分は `cfg(target_os = "macos")` で区切り、Linux のサーバのビルドを妨げない。
- CLI の設置の RPC: `ClientService` に CLI を設置する RPC（scope は operator）を足す。成功の応答は、作ったか設置済みかと、symlink のパスを返す。場所による拒否は `failed_precondition` で、理由をメッセージにする。根拠: R-001、R-003、B-002〜B-005。ルート: RPC と message の名前は委任。
- ログイン項目の判定の RPC: `ClientService` に、パスを受け取り、ログイン項目を登録してよいかと理由を返す RPC（scope は operator）を足す。client の入力に対する計算の呼び出しである。根拠: R-007、B-012、B-014。ルート: 結果は enum（許可・translocation・読み取り専用のボリューム）と理由の文言で返し、bool と enum を重ねない。RPC と message の名前は委任。
- サーバの状態: `ServerInfo`（`proto/client.proto:2578-2593`）と domain の `DaemonInfo` に、サーバ自身の場所から決まる CLI の設置の可否（enum と理由の文言）を足す。`GetServerInfo` の応答と購読 `daemon_info` は今どおり同じ値から作る。根拠: R-006、B-009。ルート: 載せるのは CLI の設置の可否だけで、ログイン項目の項目は載せない。起動時に場所の事実を求められなくてもサーバは起動し、CLI の設置の可否は「判定できなかった」を表す値と、理由の文言（失敗の文言）で表す。この値は domain の CLI の配置の規則の結果には足さず、`DaemonInfo` が「規則の結果か、求められなかったか」を持つ。enum と message の形は委任。
- 画面の CLI の設置: `../src/hooks/useAppSettings.ts:113-119` の `invokeTauri("install_cli")` を、`../src/lib/client.ts` の呼び出しで新しい RPC を呼ぶ形にする。成功の表示は応答から作り、失敗はエラーとして表示する。根拠: R-008、B-001、B-016。ルート: 委任。
- Tauri シェルの CLI の設置を消す: Tauri コマンド `install_cli`（`releash-desktop/src/adaptor/controller/command/desktop_lifecycle.rs:11`、`:28`、`:86-91`）、`releash-desktop/src/usecase/cli_install.rs`、`releash-desktop/src/adaptor/gateway/cli_install.rs`、`releash-desktop/src/infrastructure/platform/cli_install.rs`、その配線（`releash-desktop/src/desktop.rs:44-45`）と test 支援の公開（`releash-desktop/src/test_support.rs:10-13`）とテストを消す。根拠: R-001。ルート: 委任。
- Tauri シェルのログイン項目: 登録の前の場所の確認（`releash-desktop/src/usecase/login_item.rs:65-69`）を、画面自身の実行ファイルのパスでログイン項目の判定の RPC を呼び、その結果に従う形にする。設定画面からの変更と接続時の復元（`releash-desktop/src/usecase/desktop_lifecycle.rs:69-74`）の両方がこの経路を通る。`releash-desktop/src/infrastructure/platform/login_item.rs:61-79` の `registration_location` と、それを使う gateway（`releash-desktop/src/adaptor/gateway/login_item.rs:9-17`）と test 支援の公開（`releash-desktop/src/integration_test_support.rs:19-20`）を消す。SMAppService による登録・解除・状態の取得はシェルに残す。根拠: R-007、B-010〜B-013。ルート: シェルは判定せず、RPC の結果だけで登録するかを決める。
- ドメイン一覧: `docs/architecture/README.md` の「ドメイン一覧」に `installation` の行を足し、見出しの個数を直す。根拠: 上の `installation` を作ることに伴う目録の更新。ルート: 行の文言は「含まれる責務」の列の書き方に合わせる。
- CLI のガイド: `docs/guide/cli.md:9-12` のインストールの説明を、拒否の条件（translocation・読み取り専用のボリューム・development ビルド）と、拒否が失敗として返ることに合わせる。根拠: R-003、R-008。ルート: 委任。

## 固定するルート

- 場所の事実を求める処理と、CLI の配置・ログイン項目の登録の 2 つの規則は、サーバの新しいドメイン `installation` に置く。事実は 1 つの値、規則は目的ごとに 2 つ。client（Tauri の画面とシェル、ネイティブ UI）は規則を持たず、サーバの結果に従う。
- CLI の設置の可否は、サーバ自身の実行ファイルのパスで判定する。symlink の指し先がサーバの隣の `releash` なので、同じ `.app` の位置で判定することになる。
- ログイン項目の登録の可否は、画面が渡す画面自身の実行ファイルのパスで判定する。登録されるのは呼び出した画面自身の `.app` であり、サーバの位置で代用すると、サーバと画面が別の `.app` から動いているときに要求を満たさないため。
- ログイン項目の判定は、client の入力に対する計算の単発の呼び出しにする。CLI の設置の可否は、サーバの状態として `ServerInfo` に載せる。
- CLI の設置の拒否は、どの理由でも `failed_precondition` の失敗で返し、文言で区別する。成功に「スキップ」の表示を付ける形はやめる。
- macOS 以外（Linux のサーバのビルドと CI）での振る舞い:
  - 場所の事実: translocation は macOS だけの概念なので、macOS 以外では常に「されていない」。読み取り専用のボリュームの観測は OS ごとの実装（macOS は statfs の `MNT_RDONLY`、Linux は statvfs の `ST_RDONLY` 相当）で、判定は同じ規則。ビルド種別は OS に依らない。
  - CLI の設置の RPC とログイン項目の判定の RPC は、macOS 以外でも存在し、同じ規則で判定する。管理者権限での作り直し（osascript）は macOS だけで、macOS 以外では直接作れなければそのまま失敗する。
  - `cfg` で区切るのは、translocation の観測、読み取り専用のボリュームの観測の OS ごとの実装、osascript の実行だけ。`installation` の domain と usecase、gateway は区切らず、それらのテストは Linux の CI でも同じに通る。
- 管理者権限での作り直しは残し、サーバのプロセスから osascript を実行する。管理者権限の作成の口は設置先と指し先だけを受けて実行の結果だけを返し、直接の作成と管理者権限の作成の両方が失敗したときの失敗は usecase が両方の失敗を持つ値として返す。RPC の要求が期限切れや取り消しで終わったら osascript を止め、symlink を作らない。releashd は CLI・画面から setsid した子プロセスとして起動される（`releash-sdk/src/daemon.rs:93-110`）が、launchd の GUI セッション（Aqua）に属する。確認は `launchctl managername`（2026-10-08、macOS 26）。launchd（LaunchAgent / LaunchDaemon）から起動した場合はこの ISSUE の範囲外とする。

## 変えないもの

- ログイン項目の拒否条件は translocation と読み取り専用のボリュームの 2 つのまま。ビルド種別では拒否しない。ISSUE 本文が 1 つにすると定めたのは translocation の判定で、ログイン項目の拒否条件を増やすことは求めていないため。
- SMAppService によるログイン項目の登録・解除・状態の取得は Tauri シェルに残す。登録は登録されるアプリ自身からしかできないため。

## 未確定・リスク

- RPC の要求が終わって osascript を止めたときに、管理者権限の認証ダイアログが閉じるかは確かめていない。確かめるには実機で認証ダイアログを出す必要がある。
