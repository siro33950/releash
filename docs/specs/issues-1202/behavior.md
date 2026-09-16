## B-001: daemon の headless 起動

GIVEN desktop が起動していない
WHEN daemon を単独で起動する
THEN ウィンドウ、メニュー、トレイアイコンは表示されない
AND daemon は desktop とは別のプロセスとして動作を続ける

## B-002: daemon の実行ファイルの Tauri 非依存

daemon の実行ファイルは、Tauri を含まずにビルドされる。

## B-003: 単独起動した daemon への接続と command 実行

GIVEN desktop なしで単独起動した daemon が起動を完了している
WHEN クライアントが data_dir の discovery file に記載された port の 127.0.0.1 へ、クライアント token で認証してクライアント ws 接続し、command の要求エンベロープを送る
THEN 接続は受け付けられる
AND 同じ `request_id` の応答エンベロープが返る
AND 応答の結果は、変更前の desktop で同じ command を実行した結果と同じである

## B-004: 外部 daemon に対する desktop の機能

GIVEN daemon が desktop とは別のプロセスで動作し、desktop がクライアント ws で接続している
WHEN 利用者が repository、code・review、comment、agent session、terminal、workflow、workspace tree、設定・Notion・git host・外部エディタの操作を行う
THEN 変更前と同じ結果が画面へ反映される

## B-005: 外部 daemon に対する UI shell の機能

GIVEN daemon が desktop とは別のプロセスで動作し、desktop がクライアント ws で接続している
WHEN 利用者がフォルダ選択 dialog、URL を開く操作、更新の確認と relaunch、ログイン時の起動の切替、ファイルのドロップ、メニュー（Quit を除く）、ウィンドウを閉じる操作、トレイ（Quit を除く）を使う
THEN 変更前と同じ結果になる

## B-006: backend 状態の push の経路

GIVEN desktop が daemon にクライアント ws で接続している
WHEN daemon 側で workflow execution、agent session、ファイル、git status、repository path、branch 一覧、repository snapshot、review comment のいずれかの状態が変わる
THEN desktop はその push をクライアント ws で受け取り、変更後の状態が画面へ反映される
AND その push は Tauri event で配信されない

## B-007: data_dir の維持

GIVEN 変更前の desktop が、ビルド種別（release / dev / performance）に対応する data_dir（`<OS のデータディレクトリ>/com.releash.app`、`com.releash.app.dev`、`com.releash.app.performance`）に workspace、workflow execution、agent session の履歴、review comment、設定を保存している
WHEN 同じビルド種別の daemon を起動し、desktop を接続する
THEN 保存されていたデータが画面へ反映される
AND daemon の discovery file はその data_dir に書き出される

## B-008: master token の非露出

GIVEN desktop が daemon にクライアント ws で接続している
WHEN renderer がクライアント ws の接続情報を取得して認証する
THEN renderer が受け取る token は discovery file の master token と異なる
AND renderer は master token を受け取らない

## B-009: loopback 限定の待ち受け

daemon が local API とクライアント ws を待ち受けている間、127.0.0.1 以外のアドレスへの接続は受け付けられない。

## B-010: CLI と provider hook

GIVEN daemon が起動を完了している
WHEN performance build で `RELEASH_DATA_DIR` が未設定または空である場合を除き、CLI の `releash workflow` / `releash review`、または provider hook の `releash hook` を実行する
THEN daemon の local API または data_dir に到達し、変更前と同じ結果が返る

## B-011: `/usr/local/bin/releash`

GIVEN release build の Releash を起動した
WHEN `/usr/local/bin/releash workflow status <execution_id>` を実行する
THEN Releash の CLI として実行され、変更前と同じ結果が返る

## B-012: 子プロセスからの CLI alias

GIVEN daemon が起動した terminal または agent session の子プロセスがある
WHEN その子プロセスからビルド種別の alias（release: `releash`、dev: `releash-dev`）で CLI を実行する
THEN Releash の CLI として実行される
AND その CLI は daemon と同じ data_dir を使う

## B-013: daemon の再起動後の再接続

GIVEN desktop が daemon に接続しており、結果を確定できない変更要求がある
WHEN その daemon が停止し、別のインスタンスとして起動し直される
THEN desktop は新しいインスタンスへ再接続し、現在の状態が画面へ反映される
AND その変更要求は結果不明と表示される
AND その変更要求は安全を確認できないまま再実行されない

## B-014: 起動済みの daemon への desktop の接続

GIVEN desktop と同じビルド種別の daemon が起動を完了している
WHEN desktop を起動する
THEN desktop は新たに daemon を起動しない
AND desktop はその daemon にクライアント ws で接続し、利用可能になる

## B-015: daemon の単独起動手段の非公開

Releash の CLI のヘルプと利用者向けガイド（`docs/guide/`）には、daemon を単独起動する手段が現れない。

## B-016: Exit または Restart の意図による daemon の終了

GIVEN daemon が起動を完了し、クライアントがクライアント ws で接続している
WHEN クライアントが終了（Exit）または再起動（Restart）の意図で application quit を要求する
THEN daemon は変更前と同じ一括停止（workflow runtime / terminal surface / provider exit observer / local API）を行う
AND その一括停止の完了後に daemon のプロセスが終了する

## B-017: performance build の CLI と provider hook の既定 data_dir

GIVEN performance build の daemon が起動を完了している
WHEN `RELEASH_DATA_DIR` が未設定または空の状態で、performance build の CLI の `releash workflow` / `releash review`、または provider hook の `releash hook` を実行する
THEN その CLI と provider hook は、daemon と同じ `<OS のデータディレクトリ>/com.releash.app.performance` を data_dir として使う
AND daemon の local API または data_dir に到達する

## 要件IDとBehavior IDの対応表
| Requirement ID | Behavior ID |
| --- | --- |
| R-001 | B-001, B-002 |
| R-002 | B-003 |
| R-003 | B-004, B-005 |
| R-004 | B-006 |
| R-005 | B-007 |
| R-006 | B-008 |
| R-007 | B-003, B-009 |
| R-008 | B-010, B-017 |
| R-009 | B-011 |
| R-010 | B-012 |
| R-011 | B-013 |
| R-012 | B-014 |
| R-013 | B-015 |
| R-015 | B-016 |
