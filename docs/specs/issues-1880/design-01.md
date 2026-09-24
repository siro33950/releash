# Design 01

## 開始状態

初回。差分の基準は `main` の `f54e96b6` から派生した `feat/issues/1880`。既存の Design はない。開始状態の実装は `requirements.md` の Current Behavior が記録したとおりで、Spec 工程でコードは変更していない。未コミットの変更は `docs/specs/issues-1880/` のみ。

開始状態で確認した事実のうち、変える部分の対象になるもの。

- module 専用のエラー型は失敗の種類を持たない（`src-tauri/src/domain/agent_session/repository.rs:5-11`、`src-tauri/src/domain/provider_lifecycle/repository.rs:6-10` ほか）。
- `command_error`（`src-tauri/src/adaptor/controller/api/protocol/connect.rs:4-6`）がすべての command 失敗を `FailedPrecondition` にする。
- `command_error` / `command_error_with_code` の外で `ConnectError` を組み立てる箇所が非テストで 20 か所ある（`api/client.rs` 7、`api/client_service.rs` 6、`api/client_stream.rs` 5、`adaptor/protocol/connect.rs` 2）。
- `From<String> for wire::CommandError`（`src-tauri/src/adaptor/protocol/client/errors.rs:2-9`）により、型を失った文字列が Connect のエラーになる。
- 呼び出し元が分類を判定し直している（`src/lib/client.ts:130-141`、`src-tauri/src/adaptor/gateway/desktop_client.rs:45-62`、`src-tauri/src/adaptor/gateway/provider_lifecycle/event_repository_impl.rs:163` と `173-187`）。
- gateway が store の失敗の理由を単一の変種へ潰す（`src-tauri/src/adaptor/gateway/agent_session/agent_session_repository.rs` の `Unavailable` 16 か所を含む）。

`daemon.rs` の `run_startup_recovery` は開始状態に存在しないことを確認した（`daemon.rs` は 480 行、定義・呼び出しとも無し）。

## 変える部分

- module 専用のエラー型が失敗の種類を持つ: 各エラー型から、その失敗が「その呼び出しだけを再試行してよい」「上の段階からやり直すべき」「状態が直るまで再試行すべきでない」のどれであるかを読めるようにする。根拠: R-001「呼び出し元が受け取るエラーは……分類を持つ」、B-001。ルート: 内側の層は自前の語彙で持つ／専用のエラー型は残す（下記「固定するルート」）。語彙の具体と返す手段は委任。
- 分類を失敗が起きた場所で決める: gateway が store の失敗の理由を単一の変種へ潰すのをやめ、混雑・期限切れ・破損を別の種類にする。対象は `agent_session_repository.rs` の `Unavailable` へ潰す箇所と、`event_repository_impl.rs:163` の `load_stream` 失敗を理由によらず `StorageUnavailable` にする箇所を含む、gateway の同形の変換。根拠: R-002「store が混んでいて失敗した場合と、期限が切れて失敗した場合と、データが壊れていて失敗した場合は、互いに異なる分類になる」、B-002。ルート: 委任。
- store 以外の場所でも分類を理由に対応させる: worktree に実行中の execution が既にあるときの実行開始の失敗を、状態が直るまで再試行すべきでない分類にする。根拠: R-002、B-003。ルート: 委任。
- Connect のエラーへの変換を 1 か所へ統一する: `command_error` / `command_error_with_code` の外で `ConnectError` を組み立てている 20 か所を、その 1 か所へ寄せる。根拠: R-003「Connect のエラーへの変換は 1 か所で行い」、R-008、B-004。ルート: 変換の入口は分類を伴うエラーだけを受け取る（下記「固定するルート」）。その 1 か所の配置は委任。
- 文字列経由の変換経路を無くす: `From<String> for wire::CommandError`（`adaptor/protocol/client/errors.rs:2-9`）により `error.to_string()` された失敗が分類を失ったまま Connect のエラーになる経路を残さない。根拠: R-008「失敗を文字列へ変換して Connect のエラーにする経路は残らない」、B-004。ルート: 同上。
- Connect のエラーコードを分類から決める: `command_error` が失敗の理由によらず `FailedPrecondition` を与えるのをやめ、分類から code を決める。根拠: R-003「command の失敗が理由によらず `FAILED_PRECONDITION` になることはない」、B-004。ルート: 委任。
- `desktop_client.rs` の判定の入力を分類に置き換える: `ResourceExhausted` 以外をすべて daemon の停止とみなす分岐（`src-tauri/src/adaptor/gateway/desktop_client.rs:56`）を削除し、分類を読むだけにする。根拠: R-005「分類を読む側が、エラーコード・原因の型・メッセージから分類を決め直すことはない」、B-008。ルート: 停止とみなす条件そのものは設計し直さない（下記「変えないもの」）。
- `event_repository_impl.rs` の再試行を分類で決める: `resolve_bounded`（`173-187`）が失敗の理由を問わず最大 4 回再試行するのをやめ、再試行するかどうかを分類だけで決める。根拠: R-004「ある失敗を再試行するかどうかは、その失敗の分類だけで決まる」、B-005 / B-006 / B-007。ルート: 委任。再試行ループそのものの統合は行わない（下記「変えないもの」）。
- `client.ts` の判定の入力を分類に置き換える: `refreshClientOnDisconnect`（`src/lib/client.ts:130-141`）が `CommandError` の detail の有無・`Code.Unknown`・`cause` が `TypeError` かを組み合わせて判定するのをやめ、分類を読むだけにする。根拠: R-005、B-008。ルート: 接続を作り直す条件そのものは設計し直さない（下記「変えないもの」）。

## 固定するルート

- 内側の層（domain / usecase / gateway）のエラー型は、失敗の種類を自前の語彙で持つ。gRPC のステータスコードの語彙を内側の層に入れない。gRPC のステータスコードへの対応付けは adaptor で 1 か所行う。範囲: 分類の表現と、変換を担う層の割り当てまで。粒度: 語彙の具体、置き場所、対応表のモジュール配置は委任。理由: error code は内側が所有し、transport の code への変換は外側が行うという Clean Architecture のプラクティス。関係: R-001、R-003、R-005、R-008。
- 各モジュールの専用のエラー型は残し、統一の 1 型へ置き換えない。範囲: エラー型の構成。粒度: どのエラー型がどう失敗の種類を返すかは委任。理由: 正本の指示。関係: R-001。
- Connect のエラーへの変換は 1 か所で行い、その入口は分類を伴うエラーだけを受け取る。失敗を文字列へ変換して Connect のエラーにする経路を残さない。範囲: 変換経路の構成。粒度: その 1 か所の具体的な配置は委任。理由: 正本の指示に加え、分類を持たせる対象の範囲を実装で強制するため。関係: R-003、R-008。

## 変えないもの

- client が接続を作り直す条件と、desktop 側が daemon を停止したものとして扱う条件。判定の入力を分類に置き換えるだけにする。理由: 条件そのものは #1891 / #1879 / #1895 が扱う。
- 再試行ループそのものを 1 つの実装へまとめること。理由: #1889 が扱う。
- CLI / hook が使う HTTP local API のエラー変換（`src-tauri/src/adaptor/controller/api/error.rs`）、CLI 側での HTTP status の読み替え（`src-tauri/src/cli/api_client.rs:183-195`）、`LocalApiClientError::Unavailable` のときファイル直読みへ切り替える条件。理由: この ISSUE は UI と daemon の間の Connect 経路に閉じる。
- `src-tauri/src/adaptor/controller/daemon.rs` の起動時の再開処理。理由: 正本が挙げた `run_startup_recovery` は #1867（commit `5e2ef0a1`）で削除済みで、対象が残っていない。

## 未確定・リスク

- 自動判断で Behavior を 2 か所直した（`requirements.md` の Assumptions に記録）。B-001 の `THEN` から gRPC のステータスコードの無条件要求を外し、gRPC のステータスコードでの表現は B-004 が受け持つ形にした。B-004 の `AND` を、理由ごとにエラーコードが異なることを求める記述から、理由によらず同じエラーコードにまとまらないことを求める記述へ直した。
- daemon に届かずに終わる失敗（fetch 自体の失敗など）に、daemon が決めた分類が付かない。`@connectrpc/connect` 2.2 はこれを `Code.Unknown` などで表し、開始状態の `client.ts:130-141` は `cause` が `TypeError` かを見て接続の作り直しを判断している。R-005 はこの判定を禁じるため、この種の失敗に分類をどう与えるかが決まらないと、接続を作り直す場面が開始状態から変わり、「条件そのものを設計し直さない」（#1891 / #1879 / #1895 の範囲）と両立しなくなる。
- `src/lib/client.ts:432` は `Code.Canceled` と `CommandError` detail の有無を組み合わせて、取り消しを無視するかを決めている。Scope の削除対象には入っていないが、detail の有無を見る点は R-005 の「分類を決め直さない」に触れうる。開始状態のまま残すか削るかが決まっていない。
