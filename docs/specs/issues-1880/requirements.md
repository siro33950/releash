# Context

- 正本: [#1880 \[01\] 失敗の分類をエラーの値に持たせる](https://github.com/siro33950/releash/issues/1880)
- 所属: [milestone #99「02. UI と daemon の間の通信の仕組みを一本化する」](https://github.com/siro33950/releash/milestone/99) の Wave 01。
- 分類の基準は [gRPC Status Codes](https://grpc.github.io/grpc/core/md_doc_statuscodes.html) の公式の定義に従う。正本が示す使い分けは、その呼び出しだけを再試行してよいなら `UNAVAILABLE`、上の段階からやり直すべきなら `ABORTED`、状態が直るまで再試行すべきでないなら `FAILED_PRECONDITION`。
- 同じ milestone の別 ISSUE が、この変更に隣接する対象を持つ。再試行ループの統合は #1889、client のつなぎ直しの統合は #1891、接続状態の一元化は #1895、生存確認の標準化は #1879、呼び出しの期限と取り消しは #1883。
- UI と daemon の間の通信は Connect（unary + server streaming）で、HTTP local API は CLI / hook 用に残る。
- 参照する既存実装: `src/lib/client.ts`、`src-tauri/src/adaptor/gateway/desktop_client.rs`、`src-tauri/src/adaptor/controller/daemon.rs`、`src-tauri/src/adaptor/gateway/provider_lifecycle/event_repository_impl.rs`、`src-tauri/src/adaptor/gateway/agent_session/agent_session_repository.rs`、`src-tauri/src/adaptor/controller/api/protocol/connect.rs`、`src-tauri/src/adaptor/controller/api/error.rs`。

# Outcome

Releash を変更する開発者と、失敗したときの挙動を受け取る利用者が対象である。

現在は、失敗が一時的なものか、状態が直るまで失敗し続けるものかを、エラーの値が持っていない。判定は呼び出し元ごとに、エラーコード・原因の型・メッセージから独自の基準で行われており、基準は呼び出し元ごとに食い違う。その結果、直らない失敗を再試行し続けたり、一時的な失敗で接続全体を作り直したり、混雑と破損を同じ失敗として扱ったりする。

変更後は、分類が失敗の起きた場所で決まり、エラーの値そのものがその分類を運ぶ。呼び出し元は分類を読むだけで、同じ原因の失敗はどの経路でも同じ扱いになる。

# Current Behavior

2026-09-24 に `feat/issues/1880`（base `main` の `f54e96b6`）で確認した。

- module 専用のエラー型は分類を持たない。`AgentSessionRepositoryError`（`src-tauri/src/domain/agent_session/repository.rs:5-11`）は `Conflict` / `ProviderSessionAlreadyOwned` / `InvalidRequest` / `Corrupt` / `Unavailable` の 5 変種で、再試行してよいかの情報を持たない。`ProviderLifecycleRepositoryError`（`src-tauri/src/domain/provider_lifecycle/repository.rs:6-10`）は `InvalidInput` / `StorageUnavailable` / `Corrupt` の 3 変種で同様。エラー型を持つファイルは `src-tauri/src/` 配下に 101 個ある（内訳は domain 42、usecase 27、adaptor 21、infrastructure 8、cli 2、other 1）。
- Connect のエラーコードは分類に基づかない。`src-tauri/src/adaptor/controller/api/protocol/connect.rs:4-6` の `command_error` が、すべての command 失敗を `FailedPrecondition` にする。別のコードを与えるのは同時実行枠を超えたときの `ResourceExhausted` だけで、`src-tauri/src/adaptor/controller/api/client.rs:80-91` と `src-tauri/src/adaptor/controller/api/client_stream.rs:123` の 2 か所にある。
- Connect のエラーへの変換は 1 か所にない。`command_error` の外で `connectrpc::ConnectError` を直接組み立てる箇所が非テストで 20 か所あり（`src-tauri/src/adaptor/controller/api/client.rs` 7、`client_service.rs` 6、`client_stream.rs` 5、`src-tauri/src/adaptor/protocol/connect.rs` 2）、`internal` / `invalid_argument` / `not_found` / `already_exists` / `resource_exhausted` / `unavailable` をその場で選んでいる。また `src-tauri/src/adaptor/protocol/client/errors.rs:2-9` の `From<String> for wire::CommandError` により、`error.to_string()` された失敗が型を失ったまま Connect のエラーになる経路がある。
- HTTP local API は別の変換を持つ。`src-tauri/src/adaptor/controller/api/error.rs:35-90` が module のエラー変種から axum の `StatusCode` を直接決めており、Connect 側の変換とは独立している。
- 呼び出し元が分類を判定し直している。
  - `src/lib/client.ts:130-141` の `refreshClientOnDisconnect` は、`CommandError` の detail を持たず、かつ `Code.Unavailable` または `Code.Unknown` かつ `cause` が `TypeError` のときだけ接続を作り直す。呼び出し元は `src/generated/client_commands.ts:3374`、`src/lib/client.ts:318`、`src/lib/client.ts:483` の 3 か所。これとは別に `src/lib/client.ts:305-313` が `Code.ResourceExhausted` を 1 秒後に再開する条件として読み、`src/lib/client.ts:432` が `Code.Canceled` を読んでいる。
  - `src-tauri/src/adaptor/gateway/desktop_client.rs:45-62` は 5 秒ごとの `get_server_info` が失敗したとき、`ResourceExhausted` 以外をすべて daemon の停止とみなし、監視を終了して失敗理由を記録する。`connected()` はその task が終わっているかだけを見る。
  - `src-tauri/src/adaptor/gateway/provider_lifecycle/event_repository_impl.rs:173-187` の `resolve_bounded` は、失敗の理由を問わず 10ms から倍化する間隔で最大 4 回再試行し、尽きたら `StorageUnavailable` にする。同ファイル `163` は `load_stream` の失敗を理由を問わず `StorageUnavailable` にする。
- store の失敗は理由を区別せず 1 つの変種へ潰れる。`src-tauri/src/adaptor/gateway/agent_session/agent_session_repository.rs` には `AgentSessionRepositoryError::Unavailable` へ潰す箇所が 16 か所あり、`558-575` の `find` / `find_for_activity` もそこに含まれる。gateway 全体では `map_err(|_| ...Unavailable)` の形が 61 か所ある。SQLite 接続は `busy_timeout` を 2 秒に設定しており（`src-tauri/src/adaptor/gateway/local_event_store/connection.rs:46`）、混雑による待ち切れも、データの破損も、その他の失敗も、同じ `Unavailable` になる。
- 正本が現状として挙げる `src-tauri/src/adaptor/controller/daemon.rs:470-505`（すべての失敗を一時的とみなす `run_startup_recovery`）は、現在のコードに存在しない。#1867（commit `5e2ef0a1`、2026-09-23）で削除済みで、現在の `daemon.rs` は 480 行、`run_startup_recovery` の定義・呼び出しとも残っていない。削除前の実装（`5e2ef0a1~1` の同ファイル 469-503）は、`Err` を理由によらず一時的として扱い、上限付きの backoff で再試行し続けていた。

# Scope / Non-goals

変更する。

- エラーの値が分類を運ぶこと。
- 分類を、失敗が起きた場所で決めること。store の混雑・期限切れ・破損を別の分類にすることを含む。
- Connect のエラーへの変換を 1 か所へ統一し、その入口が分類を伴うエラーだけを受け取ること。`command_error` の外で `ConnectError` を直接組み立てている 20 か所と、`From<String> for wire::CommandError` による文字列経由の変換を含む。
- Connect のエラーコードを、この分類から決めること。
- 呼び出し元が分類を判定し直しているコードの削除。対象は `src/lib/client.ts:130-141` の `refreshClientOnDisconnect`（定義と、`src/lib/client.ts:318`・`src/lib/client.ts:483`・`src/generated/client_commands.ts:3374` の呼び出し、および生成元 `scripts/generate-client-protocol.mjs:57` のテンプレート内の呼び出しを含む）、`src/lib/client.ts:432` の detail の有無の判定、`src-tauri/src/adaptor/gateway/desktop_client.rs:56`、`src-tauri/src/adaptor/gateway/provider_lifecycle/event_repository_impl.rs:177-183`。frontend には分類を読む判定を置かない。
- store の失敗を `Unavailable` に潰す変換の削除。

変更しない。

- 再試行ループそのものを 1 つの実装へまとめること（#1889）。
- client のつなぎ直しを 1 つの実装へまとめること（#1891）。
- 接続の状態を client の 1 か所に持たせること（#1895）。
- 画面からサーバの生存を確かめる仕組み（#1879）。
- client が接続を作り直す条件と、desktop 側が daemon を停止したものとして扱う条件の設計（#1891 / #1879 / #1895）。今回は判定の入力を分類に置き換えるか、判定自体を削除するかに留め、条件そのものを設計し直さない。
- 購読が無い状態で unary の呼び出しだけが失敗したときに client を作り直すこと（#1895）。接続の生存は分類とは別の軸として扱う。
- 呼び出しの期限と取り消しの導入（#1883）。
- CLI / hook が使う HTTP local API のエラー変換（`src-tauri/src/adaptor/controller/api/error.rs`）と、CLI 側での HTTP status の読み替え（`src-tauri/src/cli/api_client.rs:183-195`）、および `LocalApiClientError::Unavailable` のときファイル直読みへ切り替える条件。
- `src-tauri/src/adaptor/controller/daemon.rs` の起動時の再開処理。正本が挙げた対象は削除済みで、この変更に残る対象がない。

# Requirements

- R-001: 失敗したとき、呼び出し元が受け取るエラーは、その失敗が「その呼び出しだけを再試行してよい」「上の段階からやり直すべき」「状態が直るまで再試行すべきでない」のどれであるかを示す分類を持つ。Connect を経由して観測される分類は gRPC のステータスコードで表す。分類を持つ対象は、Connect への変換に渡されるエラーと、再試行および daemon の生存の判断に渡されるエラーである。
- R-002: 分類は、失敗が起きた場所の理由に対応する。store が混んでいて失敗した場合と、期限が切れて失敗した場合と、データが壊れていて失敗した場合は、互いに異なる分類になる。
- R-003: Connect のエラーへの変換は 1 か所で行い、そこで分類からエラーコードを決める。Connect のエラーコードは、その失敗の分類と一致する。command の失敗が理由によらず `FAILED_PRECONDITION` になることはない。
- R-004: ある失敗を再試行するかどうかは、その失敗の分類だけで決まる。状態が直るまで再試行すべきでない分類の失敗は、再試行されない。
- R-005: 同じ原因の失敗は、Connect を経由するどの呼び出しから観測しても同じ分類になる。分類を読む側が、エラーコード・原因の型・メッセージ・エラーに付く detail の有無から分類を決め直すことはない。
- R-008: 分類を伴わない値は Connect への変換を通らない。失敗を文字列へ変換して Connect のエラーにする経路は残らない。

# Assumptions / Open Questions

- 自動判断: B-001 の `THEN` を「gRPC のステータスコードで表した分類を伴う」から「分類を伴う」へ直し、対応表の R-001 に B-004 を加えた。R-001 は gRPC のステータスコードでの表現を Connect を経由して観測される場合に限っており、内側の層に gRPC の語彙を入れないことが固定されたルートであるため、無条件に gRPC のステータスコードを要求する B-001 は R-001 と矛盾していた。gRPC のステータスコードでの表現は B-004 が受け持つ。
- 自動判断: B-004 の `AND 理由の異なる command の失敗が、同じエラーコードにまとまることはない` を `AND 理由によらずすべての command の失敗が同じエラーコードになることはない` へ直した。R-001 が分類を 3 つに定め、R-002 が理由ごとに分類が対応するとしている以上、異なる理由が同じ分類になることは要求の前提であり、理由ごとにエラーコードが異なることを求める記述は R-001・R-003 と矛盾していた。直した記述は R-003 の「command の失敗が理由によらず `FAILED_PRECONDITION` になることはない」に対応する。
