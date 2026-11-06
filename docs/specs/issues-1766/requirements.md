# Context

- 要求の正本は ISSUE #1766「[14] Sparkle による更新と stable の Swift.app への切り替え」（2026-11-05 05:35 UTC 更新の本文）である。
- 補助資料は ISSUE #1764、#2054、#1944、#1736、#1765、Sparkle の文書（publishing、SPUUpdaterDelegate、SPUUpdater、programmatic-setup）である。
- 調査の時点は main `f3a01a6d` である。
- 版は `X.Y.Z` だけで表す。nightly 用の接尾辞、CI の実行番号、版とは別の連番は使わない（ISSUE「バージョンとチャンネル」）。
- 配布するアプリは Swift の `Releash.app` だけになる。Tauri 版の配布を止める。Tauri のコードの撤去は #1765 である。
- 利用者の workflow 定義は `~/Library/Application Support/releash/workflows/` にある（`docs/guide/workflow/common.md:10`）。この場所はデータフォルダと独立で、本番と開発で共通である。
- 新保存先 `releash` は、利用者の workflow 定義フォルダと同じ場所である。定義フォルダはそのまま残り、サーバの状態がその中に置かれる。
- main は PR を経ないと変更できないが、管理者の権限では直接 push できる。stable は管理者の token で tag を push している。
- リポジトリは public である。
- Sparkle の版の比較は `SUStandardVersionComparator` が行い、`-` 以降を捨てる。`X.Y.Z` 同士なら数値の大小で比べる。Apple の `CFBundleVersion` は数字と `.` だけを許す。`0.4.21` のような `X.Y.Z` はどちらにも収まる。
- Sparkle の更新画面に appcast の `link` が出るのは、ダウンロードできない情報だけの更新のときだけである。

# Outcome

対象者は、macOS で Releash の Swift アプリを使う利用者と、Releash を配布する管理者である。

今の Swift アプリは自分を更新できない。
利用者は nightly ごとに GitHub Release から zip を取り、手で入れ替えている。
Tauri 版と Swift 版の両方を配っているため、nightly の時間が長く、利用者はログイン項目と CLI の参照が Tauri 版に残ったまま Swift 版を使っている。
本番のデータフォルダ名が `.app` で終わるため、Finder がアプリとして扱い、利用者はログや設定ファイルに Finder からたどり着けない。

変更後は、Swift の `Releash.app` が Sparkle で自分を更新する。
nightly と stable の配布は `Releash.app` だけになり、配る commit ごとに版が上がる。
利用者は Settings で自動更新と nightly の受け入れを選び、メニューバーから手動確認し、フッターの通知から更新できる。
更新の間もサーバと workflow は動き続ける。
データフォルダは `releash`（開発は `releash.dev`）に移り、Finder で通常のフォルダとして開ける。

# Current Behavior

調査の時点は main `f3a01a6d` である。
ビルド、テスト、実機の更新は実行していない。
GitHub Pages のサイトは無い（Pages の API が 404）。

版と配布:

- 版が上がるのは、stable 公開後に作られる PR と、手動の `Bump Version` workflow だけである。nightly は版を上げない。
- nightly は main の HEAD を配り、直近の公開済み nightly と commit が同じなら日次はスキップする。HEAD に成功した main push の CI が無ければ失敗する。
- そのため、stable `v0.4.19` 以降の nightly はすべて `0.4.20` である（`v0.4.20-nightly.20261015.2` から `v0.4.20-nightly.20261104.1` までの 6 件）。異なる commit が同じ版で配られている。
- nightly の Release には、Tauri 版の dmg、`Releash_{版}_universal.app.tar.gz` とその `.sig`、`latest.json`、Swift 版の `Releash_Preview_{版}_universal.zip` が載る。回帰 E2E と両方のビルドが成功したときだけ公開され、E2E が失敗すると ISSUE が作られる。
- Swift 版の製品名と実行ファイル名は `Releash Preview`、Release の bundle ID は `com.releash.macos`、Debug は `com.releash.macos.dev` である。`CFBundleVersion` は `1` 固定である。
- stable は指定した nightly の commit から Tauri 版だけを再ビルドし、`latest.json` があることを確かめて latest の Release として公開する。公開後に版を `patch` で上げる PR を作る。
- nightly の tag は `v{X.Y.Z}-nightly.{YYYYMMDD}.{N}`、stable は `v{X.Y.Z}` である。nightly の Release は直近 14 件を残す。
- `Releash Preview` の文字列は、メニューバーの tooltip と accessibility label、About と終了の項目、Release の asset 名に出てくる。

アプリの更新:

- Swift アプリに、更新を確認する操作、更新の設定、更新の画面は無い。
- フッターのサーバ表示は、画面とサーバの版 `X.Y.Z` が違うときに「サーバが古いため、再起動で更新できます」または「クライアントが古いです」を出す。版が同じなら、commit が違っても出ない。
- メニューバーの項目は、実行中の workflow、開く、設定、About、終了である。
- Settings は General、Repositories、Providers、Workflows、System の 5 タブで、System にはログイン項目、CLI の設置、Telemetry の節がある。サーバに接続していないと、General の「Diff base の既定」以外の設定は変更できない。
- Quit は画面のプロセスだけを終え、サーバは残る。次に画面を起動すると、動いているサーバに接続する（#1764）。
- 「ログイン時に起動」が ON のとき、ログイン項目の登録が失われていれば、起動時に登録し直す（#1764）。

データフォルダ:

- 既定のデータフォルダは、本番が `~/Library/Application Support/com.releash.app`、開発が `com.releash.app.dev` である（`docs/guide/cli.md:27`）。`--data-dir`、`RELEASH_DATA_DIR`、既定値の順で決まる。
- `releash status` は、データフォルダの `client-api.json` が動いているサーバを指していれば running を返す。`releash server start` とアプリは、サーバが無ければそのデータフォルダで起動する。既定以外の場所にあるサーバは探さない。
- データフォルダの `bin/releash` は、データフォルダとアプリの絶対パスを含んでいる。この Mac では `com.releash.app` と `/Applications/Releash Preview.app` を指している。
- この Mac の本番のデータフォルダには、設定、event store、ログ、端末の記録、review comment、workspace の状態など 26 項目があり、event store は 354MB である。Tauri 版が置いたロックとソケットも残っている。
- `~/Library/Application Support/releash/` は既にあり、利用者の workflow 定義 `workflows/`、`prompts/`、0 バイトの `local-event-store.sqlite3` が入っている。

再現手順（データフォルダ）:

1. 本番の Releash を起動する。
2. Finder で `~/Library/Application Support` を開き、`com.releash.app` をダブルクリックする。
3. 中に入れず、`open "~/Library/Application Support/com.releash.app"` は「The application cannot be opened because its executable is missing.」で失敗する（#1736）。

再現手順（更新）:

1. `Releash Preview.app` 0.4.20 を起動する。
2. メニューバー、Settings、フッターのどこにも更新を確認する操作は無い。新しい nightly が公開されても、アプリは何も表示しない。

この Mac の状態（2026-11-05）:

- `/Applications/Releash.app` は Tauri 版 0.4.19（`com.releash.app`）、`/Applications/Releash Preview.app` は Swift 版 0.4.20（`com.releash.macos`、`CFBundleVersion` は `1`）である。
- 動いているのは Swift 版とその `releashd` で、データは `com.releash.app` を使っている。
- Tauri 版のログイン項目 `com.releash.app`（LaunchAgent）は有効のままである。`/usr/local/bin/releash` は Tauri 版の `Contents/MacOS/releash` を指している。

# Scope / Non-goals

変更するもの:

- 版の付け方。配る commit ごとに `patch` を上げ、nightly が版上げの commit を main に入れてから配る。
- nightly と stable の workflow。Tauri 版の配布を外し、Swift の `Releash.app` を配り、appcast を GitHub Pages に公開する。
- Swift アプリの名前。`Releash Preview.app` を `Releash.app` にする。
- Swift アプリの更新。Sparkle の組み込み、更新の設定、メニューバーの手動確認、フッターの通知。
- 更新設定をアプリが所有する境界の、プロジェクト規約（`AGENTS.md`）への明記。
- データフォルダの既定名と、既存データの移行。
- `AGENTS.md` のリリースの節と `docs/guide/cli.md` のデータフォルダの既定値。

変更しないもの:

- Tauri のコードの撤去、版上げ対象からの `package.json` と `tauri.conf.json` の除外（#1765）。
- 旧 Tauri 版から Swift 版への自動更新。
- アプリ更新のための、稼働中サーバの自動停止。
- bundle ID（`com.releash.macos`、`com.releash.macos.dev`）。
- 利用者の workflow 定義の場所（`~/Library/Application Support/releash/workflows/`）。
- 利用者向けの手動導入の手順の文書。管理者が不要と決めた。
- Sparkle の更新画面での変更内容（リリースノート）の表示。
- 明示指定されたデータフォルダ（`--data-dir`、`RELEASH_DATA_DIR`）の移動。
- nightly の tag の形式と、nightly の Release を直近 14 件残す規則。

# Requirements

版:

- R-001: nightly が新しい commit を配るとき、配る版は直前に配った版より `patch` が 1 つ大きい `X.Y.Z` である。配るアプリの `CFBundleShortVersionString` と `CFBundleVersion`、同梱の `releashd` と `releash` の版、appcast の項目の `sparkle:shortVersionString` と `sparkle:version` は、すべて同じ `X.Y.Z` である。
- R-002: 版上げは main の commit として残る。nightly は版を上げた commit を main に入れ、その commit から配る。翌日に main の HEAD がその commit のままなら、nightly は配らない。
- R-003: 同じ commit の再試行、再配布、stable への昇格では版が変わらない。stable は選んだ nightly と同じ commit、同じ版で作る。
- R-004: stable の公開後に版上げの PR を作らない。stable の公開で main の版は変わらない。

配布:

- R-005: nightly は、署名・公証済みの universal の `Releash.app` を zip にした asset だけを prerelease の Release に載せる。Tauri 版の成果物と `latest.json` は載らない。配る commit に成功した main push の CI が無ければ nightly は失敗し、回帰 E2E が失敗したら配らず ISSUE を作る。
- R-006: stable は、指定した nightly の commit から `Releash.app` を再ビルド、署名、公証し、latest の Release として公開する。公開の条件は、Swift の asset と appcast がそろっていることである。
- R-007: 配布名は `Releash.app`、実行ファイル名は `Releash` である。メニューバー、About、終了の項目、tooltip、Release の asset 名に `Releash Preview` が残らない。
- R-008: appcast は `https://siro33950.github.io/releash/appcast.xml` で取得できる。項目は最新の stable（チャンネル無し）と最新の nightly（`sparkle:channel` が `nightly`）の各 1 件で、同じ版が stable に昇格したら stable の項目にまとめる。各項目の enclosure は公開済みの Release の asset を指し、その EdDSA 署名はアプリに組み込んだ公開鍵で検証できる。
- R-009: nightly と stable が同時に公開されても、appcast の片方の項目を失わない。appcast の項目が指す asset は、公開された時点で取得できる。appcast の公開に失敗した workflow は成功にならず、再実行で同じ版の公開を完了できる。nightly の Release を消しても、appcast は消した asset を指さない。
- R-010: GitHub Release の prerelease の区分は、nightly が prerelease、stable が latest である。

アプリの更新:

- R-011: アプリは Sparkle で自分を更新する。更新はアプリの差し替えと再起動だけを行い、動いているサーバと workflow を止めない。再起動したアプリは動いているサーバに接続する。サーバの版がアプリより古いときは、既存のフッターの「再起動で更新できます」で案内する。
- R-012: Settings の System タブに「更新」の節があり、「自動更新」（初期値 ON）と「nightly アップデートを受け入れる」（初期値 OFF）がある。値は再起動後も保持され、サーバに接続していなくても変更できる。
- R-013: 自動更新が ON のとき、アプリは自動で確認とダウンロードを行い、利用者がアプリを終了したときに適用する。利用中の画面を自動で終了しない。利用者が Sparkle の画面で「今すぐ」を選べば、その場で適用して再起動する。
- R-014: 更新候補になるのは、現在の版より新しい版だけである。「nightly アップデートを受け入れる」が ON のときだけ nightly の項目が候補になり、stable の項目はどちらの設定でも候補になる。OFF に戻しても古い stable へ戻さない。同じ版の stable への昇格は候補にならない。
- R-015: メニューバーに「アップデートを確認」があり、Sparkle の手動確認を開く。確認中は項目が無効になる。
- R-016: 新しい版が見つかると、フッターにその版と「更新」ボタンが出る。ボタンは Sparkle の標準の更新画面を開く。自動ダウンロードの間はボタンを無効にし、終わると押せるようになる。更新画面には版だけが出て、変更内容やリンクは出ない。ダウンロードの進行、適用の確認、失敗の表示は Sparkle の標準の画面である。
- R-017: Debug のアプリ（`com.releash.macos.dev`）は appcast を読まず、更新を確認しない。
- R-018: `AGENTS.md` の「サーバがロジックを所有する」の節に、アプリ自身の更新設定（自動更新、nightly の受け入れ）を例外として挙げる。

データフォルダ:

- R-019: 既定のデータフォルダは、本番が `~/Library/Application Support/releash`、開発が `~/Library/Application Support/releash.dev` である。サーバ、CLI、`docs/guide/cli.md` が同じ値を示す。
- R-020: 旧保存先にサーバが動いているとき、CLI とアプリはそのサーバに接続する。データを移さず、別のサーバを起動しない。
- R-021: 旧保存先のサーバが止まった後の次の起動で、旧保存先のデータ（設定、event store とその関連ファイル、ログ、その他のファイル）を新保存先へ移してから起動する。`bin/releash` の wrapper は新保存先のパスで書き直される。本番と開発は独立して移行する。
- R-022: 新旧の両方にデータがあるときは移さず、理由を表示して止まる。新保存先の `workflows/` と `prompts/`（利用者の定義）、および中身が空のファイルは、データとみなさない。
- R-023: 同時に 2 つ起動しても、移行の途中で失敗しても、再実行しても、データを失わず、サーバを二重に起動しない。旧サーバの停止を確かめられないときは移行しない。
- R-024: 移行後の保存先は Finder で通常のフォルダとして開ける。

名前の変更:

- R-025: `Releash Preview.app` を `Releash.app` に置き換えた後、ログイン項目は `Releash.app` を起動する。

# Assumptions

- Sparkle の鍵対は管理者が作り、秘密鍵を 1Password に置き、公開鍵を merge 前に用意する。
- GitHub Pages の source を「GitHub Actions」にするリポジトリ設定の変更は、merge 前に管理者が行う。
- nightly は、管理者の権限で版上げの commit を main に直接 push する。
- 旧 Tauri 版 `Releash.app` が利用者の Mac に残っていても、その扱いは利用者に任せる。

# Invariants

なし。

# Risks

- ログイン項目の登録が、アプリのパスの変更（`Releash Preview.app` から `Releash.app`）の後に新しい `Releash.app` を指すかは、実機で確かめていない。古いパスを指したままになる場合、R-025 を満たせない。
- Sparkle がアプリを差し替えるとき、動いている `releashd` は古いバンドルから起動したプロセスのままである。差し替え後に `bin/releash` の wrapper が指す CLI は新しい版になり、サーバより新しい CLI が動く。この組み合わせで CLI の操作が拒否される経路があれば、R-011 の「サーバと workflow を止めない」が利用者には止まったように見える。
