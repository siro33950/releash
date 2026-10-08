# Context

- 要求の正本: https://github.com/siro33950/releash/issues/1907 （[09] CLI の配置の規則を domain に置く）
- 補助資料: #1902（https://github.com/siro33950/releash/issues/1902 ）、マイルストーン #100（https://github.com/siro33950/releash/milestone/100 ）、`src-tauri/releash-desktop/src/infrastructure/platform/cli_install.rs`、`src-tauri/releash-desktop/src/infrastructure/platform/login_item.rs`、`src-tauri/releash-desktop/src/domain/login_item.rs`、`src-tauri/src/infrastructure/platform/path_aliases.rs`
- 本文の「今の作り」の file:line は main `4533af2f` 時点のもの。#1852・#1853 で CLI の配置とログイン項目は Tauri シェルのクレート `src-tauri/releash-desktop/` へ移っている。この文書は main `28df9d01`（#1905 の後）で読み直した事実に基づく。
- symlink の指し先は #1902 で CLI `releash` になっている。`.app` には `releashd` と `releash` が同梱される（`scripts/build-desktop-backend.mjs:20-22`）。
- AGENTS.md の原則: アプリケーションのロジックはサーバ（`releashd`）に置く。client はサーバの呼び出しと購読だけを行い、画面（React）が直接通信する。単発の呼び出しは、状態を変える操作と client の入力に対する計算だけ。
- マイルストーン #78 の方針: ネイティブ UI は Rust のライブラリをリンクせず、同じ規則を Swift に実装しない。#1764 は、ネイティブ UI の CLI の設置とログイン項目の登録で、この ISSUE の規則を使う。
- ログイン項目の登録（SMAppService）は、登録されるアプリ自身のプロセスからしか行えない。登録されるのは呼び出した画面自身の `.app` である。
- 対応プラットフォームは macOS（AGENTS.md「リリース」）。Linux への配布の経路は MS79。
- マイルストーン #100 の条件: どの ISSUE の後でも、Tauri アプリ・CLI・hook が動く状態を保つ。
- 判断の経緯: 要求の整理での質問への回答は、利用者の中央管理セッション releash-81 から受けた。

# Outcome

- 対象者: 設定画面から CLI を設置する利用者、CLI の設置とログイン項目の登録で同じ規則を使うネイティブ UI（#78 の #1764）、Releash の開発者。
- 現在の問題: CLI を置いてよい条件の判断が、Tauri シェルの infrastructure にだけあり、ネイティブ UI から使えない。実行ファイルが一時的な場所にあるかの判定が CLI の設置とログイン項目で別々に書かれ、方法も違う。CLI の設置は読み取り専用のボリュームを見ないため、DMG から直接起動すると、DMG を外すと消える場所への symlink を作る。translocation で設置しなかったときも成功として返すため、呼び出し側が文言で成功と拒否を見分けることになる。
- 変更後の状態: CLI の設置と、ログイン項目を登録してよい場所かの判定は、サーバが持つ 1 つの場所の判定から導かれ、画面もネイティブ UI も同じサーバの呼び出しで使える。一時的な場所や開発用のビルドからは CLI を設置せず、理由を付けた失敗として返す。

# Current Behavior

main `28df9d01` で読んで確かめた挙動。パスは `src-tauri/` 起点。

- 設定画面の「Install CLI command」は Tauri コマンド `install_cli` を呼ぶ（`../src/hooks/useAppSettings.ts:113-119`、`releash-desktop/src/adaptor/controller/command/desktop_lifecycle.rs:86-91`）。usecase は gateway を呼ぶだけ（`releash-desktop/src/usecase/cli_install.rs:10-17`）。
- CLI の設置の判断はすべて Tauri シェルの infrastructure にある（`releash-desktop/src/infrastructure/platform/cli_install.rs`）。
  - debug ビルドでは失敗を返す。文言は `Install the CLI from a release build of Releash.app.`（`:37-39`）。
  - 実行ファイルのパスに `/AppTranslocation/` を含むと、成功を返し、文言は `Releash CLI skipped because app appears to be translocated: <path>`（`:25-29`、`:44`、`:64-68`、`:168-171`）。画面はこれをエラーではなく表示として出す（`../src/hooks/useAppSettings.ts:115`）。
  - 読み取り専用のボリュームかは見ない。
  - `/usr/local/bin/releash` が同じ指し先の symlink なら `already installed` で成功、symlink でないファイルなら上書きせず失敗、それ以外は symlink を作る（`:70-81`）。
  - 直接作れなければ、osascript の管理者権限で作り直し、指し先を確かめる（`:83-104`、`:151-166`）。
  - 指し先は実行ファイルの隣の `releash`（`:40-42`）。
- ログイン項目は、実行ファイルのパスの要素に `AppTranslocation` があるか（`releash-desktop/src/infrastructure/platform/login_item.rs:61-64`）と、ボリュームが読み取り専用か（`:65-75`）を Tauri シェルの infrastructure で判定し、domain の規則（`releash-desktop/src/domain/login_item.rs:30-39`）で登録を拒否する。文言は `Move Releash.app to Applications before enabling Launch at login.` と `Releash.app is on a read-only volume. Move it to Applications before enabling Launch at login.`。登録の前に場所を見る経路は、設定画面からの変更と接続時の復元の 2 つ（`releash-desktop/src/usecase/login_item.rs:46-75`、`releash-desktop/src/usecase/desktop_lifecycle.rs:69-74`）。debug ビルドでは拒否しない。
- `releash-dev` は、子プロセス向けの別名としてだけある（`src/infrastructure/platform/path_aliases.rs:15-20`）。`{data_dir}/bin/releash-dev` の wrapper が、同じディレクトリの `releash` を実行する（`:58`、`:162-200`）。利用者のシェルの PATH に置く仕組みは無い。

# Scope / Non-goals

## Scope

- CLI の設置の規則と実行を、Tauri シェルからサーバへ移し、Connect の呼び出しにする。
- 実行ファイルの場所の判定（translocation・読み取り専用のボリューム・ビルド種別）をサーバに 1 つ置き、CLI の設置とログイン項目の登録が同じ判定を使う。
- サーバの状態に、CLI を設置できるかを載せる。
- 設定画面の CLI の設置と、Tauri シェルのログイン項目の登録を、サーバの呼び出しに切り替える。
- `docs/guide/cli.md` のインストールの説明を、変更後の挙動に合わせる。

## Non-goals

- `releash-dev` を利用者のシェルの PATH に置く経路。根拠:
  - debug ビルドの `releash` は、自分で dev の data dir（`com.releash.app.dev`）を選ぶ（`src-tauri/releash-sdk/src/data_dir.rs:17-23`、`:40-48`）。ビルドした `releash` をそのまま実行すれば dev のサーバに届き、`--data-dir` でも指定できる。
  - `releash-dev` は子プロセス（agent・hook）に起動環境を示すための別名で、実体は同じディレクトリの `releash` を指す wrapper である（`src-tauri/src/infrastructure/platform/path_aliases.rs:15-20`、`:58`、`:81-85`、`:162-200`）。
  - CLI の配置の規則は debug ビルドを拒否する。
- ログイン項目の登録そのもの（SMAppService の呼び出し）をサーバへ移すこと。登録は登録されるアプリ自身からしかできない。
- ログイン項目の拒否条件を増やすこと。
- CLI のサブコマンドとしての設置。
- Linux への配布の経路（MS79）。launchd の plist・systemd の unit の提供。

# Requirements

- R-001: CLI の設置は、Connect の呼び出しとしてサーバが行う。Tauri の画面もネイティブ UI も、同じ呼び出しで CLI を設置できる。
- R-002: 実行ファイルの場所の判定（translocation されているか、読み取り専用のボリュームにあるか、ビルド種別）は 1 つで、CLI の設置とログイン項目の登録の可否はどちらもこの判定から導かれる。
- R-003: CLI の設置は、サーバの実行ファイルが translocation されている、読み取り専用のボリュームにある、development ビルドである、のいずれかのとき、`/usr/local/bin/releash` を作らずに `failed_precondition` で失敗し、理由を返す。translocation と読み取り専用のボリュームの理由は、Releash.app を移動するよう促す。
- R-004: CLI の設置は、`/usr/local/bin/releash` が同じ CLI を指す symlink なら、変えずに成功し、設置済みであることを返す。symlink ではないファイルがあれば、上書きせずに失敗する。それ以外は、サーバの実行ファイルの隣の `releash` を指す symlink を作って成功する。
- R-005: CLI の設置は、直接 symlink を作れないとき、管理者権限を求めて作り直し、作った symlink が隣の `releash` を指すことを確かめる。確かめられなければ失敗する。
- R-006: サーバの状態（`GetServerInfo` の応答と、その購読）は、CLI を設置できるかと、できないときの理由を含む。
- R-007: ログイン項目は、登録する画面自身の実行ファイルが translocation されているか、読み取り専用のボリュームにあるときは登録されず、Releash.app を移動するよう促す理由が設定画面に出る。この判定は、画面が自分の実行ファイルのパスを渡してサーバが計算する。設定画面からの変更と接続時の復元の両方でこの判定を使う。ビルド種別では拒否しない。
- R-008: 設定画面の「Install CLI command」は、サーバの呼び出しで CLI を設置し、成功ならその結果を、失敗ならエラーとして理由を表示する。

# Assumptions

- なし
