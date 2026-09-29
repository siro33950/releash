# Design 01

## 開始状態

初回。差分の基準は `main` の `9524a6fa`（#1951 の PR #1957 をマージした時点）。作業ブランチ `feat/issues/1952` の派生点は `636bb2f0` で、#1951 を含まない。開始時点の挙動は `requirements.md` の Current Behavior を正とする。閉じた Thread は無い。

## 変える部分

- daemon の合図の配信条件: 購読の有無と snapshot の作成待ちにかかわらず、stream ごとに合図が流れるようにする。根拠: R-001「開いている購読の stream ごとに、購読の有無と snapshot の作成を待っているかどうかにかかわらず、一定の間隔で client へ合図を流す」。ルート: 委任
- 無音と判断する時間: `proto/client.proto:2707` の `state_stream_silence_ms` を 30000 から 20000 にする。根拠: R-007「無音と判断する時間は proto の規則として 20 秒とし、画面とシェルは同じ値を読む」。ルート: proto の service option の値として持ち、シェル側に別の値を置かない
- シェルの生存の判定: 数える対象を無音と stream の切断だけにし、連続 2 回で「居ない」とする。無音の打ち切りに `ATTEMPT_LIMIT` を使うのをやめる。根拠: R-002・R-003。ルート: 連続失敗の回数は 2 とする。無音の時間は proto の規則を読む
- シェルの stream: 生存確認用と設定用の 2 本を 1 本にし、その stream で `desktop-settings` を購読する。根拠: R-004「シェルは daemon との購読の stream を 1 本だけ開き、その stream で生存の合図と desktop 設定の両方を受け取る」。ルート: 設定専用の stream と、`desktop_client.rs:116-125` の 1 秒固定のループを残さない
- シェルのつなぎ直し: 待ちと、つなぎ直しの対象にする失敗を proto の規則から決める。根拠: R-005。ルート: proto の `connection_backoff` と `reconnect_status_code` を読む。同じ値の定数を Rust 側に置かない
- シェルの単発の呼び出しの既定の期限: `desktop_client.rs:31` の 30 秒をやめる。根拠: R-006「シェルが期限を指定せずに送る単発の呼び出しには、proto の規則で定めた既定の期限が当たる」。ルート: proto の `default_timeout_ms` を読む
- シェルの失敗の記録先と監督への渡し方: `desktop.rs:78-81` の `FailurePresenter` と `FailureRecordStore` をシェルの組み立てから外し、生存の失敗を分類付きで監督へ渡す。根拠: R-008「分類を保ったまま daemon の監督へ渡り、画面から観測できる。どこからも読めない記録先には書かない」。ルート: シェル側に読み取りの経路を新設せず、画面は既存の daemon の状態から読む

## 固定するルート

- 無音と判断する時間、つなぎ直しの待ち、つなぎ直しの対象にする失敗、単発の呼び出しの既定の期限は、proto の service option を正とし、シェルはそれを読む。同じ値を Rust 側の定数として持たない。
- 無音と判断する時間の値は 20 秒とし、画面とシェルで分けない。
- 「居ない」と判定する連続失敗の回数は 2 とする。
- シェルの stream は 1 本にし、その stream で `desktop-settings` を購読する。設定専用の stream は残さない。
- シェルに失敗の記録先を作らない。生存の失敗は監督が持ち、画面は既存の daemon の状態から読む。
- `RetryBackoff::SERVICE`（`src-tauri/src/common/retry.rs:15`）と proto の `connection_backoff` の値の重複は、この周では直さない。

## 変えないもの

- 合図の間隔（10 秒、`src-tauri/src/infrastructure/state_subscription.rs:14`）。届く頻度は変えず、届かない状態を無くすだけにするため。
- daemon の中の繰り返し処理が使う `ATTEMPT_LIMIT`（`src-tauri/src/usecase/failure.rs:58`）の値。シェルの生存の判定から切り離すだけで、daemon 側の打ち切り時間は今回の対象ではないため。
- 画面（`src/lib/client.ts`）のコード。無音と判断する時間は proto の値を読んでいるため、値の変更だけで反映されるため。

## 未確定・リスク

- Rust から proto の service option を読む経路。`client_descriptor.bin` が実行時に `DescriptorPool` へ読み込まれていること（`src-tauri/src/adaptor/presenter/client/json.rs:5-12`）と、`prost-reflect` が依存にあること（`src-tauri/Cargo.toml:83`）は確かめたが、そこから service の custom option を読み出せることは実際に動かして確かめていない。読み出せない場合、R-005・R-006・R-007 の「proto の規則を読む」を満たせない。
