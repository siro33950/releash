## B-001: 設定画面から CLI を設置する

GIVEN `/Applications` の Releash.app（release ビルド）からサーバが動いている
AND `/usr/local/bin/releash` が無い
WHEN 利用者が設定画面で「Install CLI command」を選ぶ
THEN `/usr/local/bin/releash` が、サーバと同じ Releash.app の中の `releash` を指す symlink になる
AND 設定画面に設置したことが表示される

## B-002: ネイティブ UI と同じ呼び出しで設置する

GIVEN `/Applications` の Releash.app（release ビルド）からサーバが動いている
AND `/usr/local/bin/releash` が無い
WHEN operator の client が Connect で CLI の設置を呼ぶ
THEN 呼び出しは成功する
AND `/usr/local/bin/releash` が、サーバと同じ Releash.app の中の `releash` を指す symlink になる

## B-003: translocation されたアプリからは設置しない

GIVEN translocation された Releash.app からサーバが動いている
WHEN client が CLI の設置を呼ぶ
THEN 呼び出しは `failed_precondition` で失敗し、Releash.app を移動するよう促す理由が返る
AND `/usr/local/bin/releash` は作られない

## B-004: DMG から直接起動したアプリからは設置しない

GIVEN マウントした DMG（読み取り専用のボリューム）の Releash.app から、translocation されずにサーバが動いている
WHEN client が CLI の設置を呼ぶ
THEN 呼び出しは `failed_precondition` で失敗し、Releash.app を移動するよう促す理由が返る
AND `/usr/local/bin/releash` は作られない

## B-005: development ビルドからは設置しない

GIVEN development ビルドのサーバが動いている
WHEN client が CLI の設置を呼ぶ
THEN 呼び出しは `failed_precondition` で失敗し、release ビルドの Releash.app から設置するよう促す理由が返る
AND `/usr/local/bin/releash` は作られない

## B-006: 設置済みなら変えずに成功する

GIVEN `/usr/local/bin/releash` が、動いているサーバと同じ Releash.app の中の `releash` を指す symlink である
WHEN client が CLI の設置を呼ぶ
THEN 呼び出しは成功し、設置済みであることが返る
AND `/usr/local/bin/releash` は変わらない

## B-007: symlink でないファイルは上書きしない

GIVEN `/usr/local/bin/releash` が symlink ではない通常のファイルである
WHEN client が CLI の設置を呼ぶ
THEN 呼び出しは失敗し、上書きしない理由が返る
AND `/usr/local/bin/releash` は変わらない

## B-008: 書き込めないときは管理者権限で設置する

GIVEN 利用者が `/usr/local/bin` に書き込めない
WHEN client が CLI の設置を呼ぶ
THEN 管理者権限を求めるダイアログが出る
AND 利用者が許可すると、`/usr/local/bin/releash` が、サーバと同じ Releash.app の中の `releash` を指す symlink になり、呼び出しは成功する
AND 利用者が拒否すると、呼び出しは失敗し、`/usr/local/bin/releash` は作られない

## B-009: サーバの状態に CLI を設置できるかが載る

GIVEN サーバが動いている
WHEN client が `GetServerInfo` を呼ぶ、またはサーバの状態を購読する
THEN 応答と購読の値は、CLI を設置できるかを含む
AND 設置できないときは、CLI の設置の呼び出しが返すものと同じ理由を含む

## B-010: translocation されたアプリはログイン項目に登録しない

GIVEN translocation された Releash.app の画面が、サーバに接続している
WHEN 利用者が設定画面で「Launch at login」を有効にして保存する
THEN ログイン項目は登録されない
AND 設定画面に Releash.app を移動するよう促す理由が表示される

## B-011: 読み取り専用のボリュームのアプリはログイン項目に登録しない

GIVEN 読み取り専用のボリュームにある Releash.app の画面が、サーバに接続している
WHEN 利用者が設定画面で「Launch at login」を有効にして保存する
THEN ログイン項目は登録されない
AND 設定画面に Releash.app を移動するよう促す理由が表示される

## B-012: ログイン項目の判定は画面の位置で行う

GIVEN `/Applications` の Releash.app から起動したサーバが動いている
AND translocation された別の Releash.app の画面が、そのサーバに接続している
WHEN 利用者が設定画面で「Launch at login」を有効にして保存する
THEN ログイン項目は登録されない
AND 設定画面に Releash.app を移動するよう促す理由が表示される

## B-013: 接続時の復元でも場所で拒否する

GIVEN サーバの「Launch at login」の希望が有効である
AND ログイン項目が登録されていない
AND translocation された Releash.app の画面を起動する
WHEN 画面がサーバに接続する
THEN ログイン項目は登録されない

## B-014: development ビルドでもログイン項目の登録は拒否しない

GIVEN development ビルドの画面が、読み取り専用ではないボリュームの translocation されていない場所にある
WHEN 画面が自分の実行ファイルのパスでログイン項目の登録の可否をサーバに尋ねる
THEN 登録してよいという結果が返る

## B-015: 一時的な場所では CLI の設置とログイン項目の登録が同じ理由で拒否される

GIVEN translocation された Releash.app から、サーバと画面が動いている
WHEN client が CLI の設置を呼ぶ
AND 画面が自分の実行ファイルのパスでログイン項目の登録の可否をサーバに尋ねる
THEN どちらも、translocation されていることを理由に拒否される

## B-016: 設定画面で設置の拒否をエラーとして表示する

GIVEN translocation された Releash.app からサーバと画面が動いている
WHEN 利用者が設定画面で「Install CLI command」を選ぶ
THEN 設定画面に、Releash.app を移動するよう促す理由がエラーとして表示される
AND `/usr/local/bin/releash` は作られない

## 要件IDとBehavior IDの対応表
| Requirement ID | Behavior ID |
| --- | --- |
| R-001 | B-001, B-002 |
| R-002 | B-015 |
| R-003 | B-003, B-004, B-005 |
| R-004 | B-001, B-006, B-007 |
| R-005 | B-008 |
| R-006 | B-009 |
| R-007 | B-010, B-011, B-012, B-013, B-014 |
| R-008 | B-001, B-016 |
