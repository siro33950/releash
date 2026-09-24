# Design 03

## 開始状態

差分の基準は `main` の `f54e96b6` から派生した `feat/issues/1880`。直前の Design は `docs/specs/issues-1880/design-02.md`。

design-02 の「変える部分」7 件は、この周の開始時点で作業ツリーに実装済みである（未コミット）。`refreshClientOnDisconnect` は `src/`・`scripts/` から消え、`src/lib/client.ts` の detach 失敗の条件は `Code.Canceled` だけになり、`From<EditorError> for String` と `From<FactReadError> for String` は存在せず `find_session_attachment` は `FactReadError` を返し、`hook_health_repository_impl.rs` は `StreamHeadConflict` だけを `Conflict` にして `PayloadConflict` を発生元の分類のまま通し、`AppError::Coded` は `kind` を保持し、`domain/` の分類分岐のテストと `FailureKind` 16 分類の Connect 対応表のテスト（`src-tauri/src/adaptor/protocol/connect_test.rs`）が置かれている。実装済みの内容は再掲しない。

この周までに解消した Thread、`[DEFERRED]` で人間へ渡した Thread、`[REJECTED]` とした Thread はいずれも無い。

開始状態で確認した事実のうち、この周の変える部分の対象になるもの。

- `SafeOperationFailure::new`（`src-tauri/src/domain/local_event/failure.rs:64-84`）は `OutcomeUnknown` を `FailureKind::RestartRequired` に固定しつつ引数 `retryable` をそのまま保存する。`read_only.rs` の `resolve_commit`（`src-tauri/src/adaptor/gateway/local_event_store/read_only.rs:322-336`）は `OutcomeUnknown` と `retryable=true` を組み合わせる。`query_error`（`src-tauri/src/adaptor/gateway/workspace_tree/query_service.rs:542-549`）は `failure.retryable` を `WorkflowError::StorageUnavailable` の `retryable` へ転記し、`src-tauri/src/domain/workflow/error.rs:85-91` が `true` を `Temporary` にする。`agent_session_lifecycle.rs:780-782` も同じ bool で分岐する。
- `usecase/provider_lifecycle/hook_health.rs` の 3 つの記録経路（`115-135`、`148-163`、`175-190`）は `save` の `Conflict` を `for _ in 0..4` で再試行し、上限後に `map_error(Conflict)` を返す。`hook_health_error_test.rs` は `map_error` 単体を呼ぶだけで、`provider_lifecycle_usecase_test.rs:452-465` の `InMemoryHookHealthRepository::save` は常に `Ok` を返すため、`continue` 分岐と上限到達分岐が駆動されない。
- `map_workflow_error`（`src-tauri/src/usecase/agent_session/agent_session_lifecycle.rs:770-784`）の catch-all（`:783`）が `Validation`・`UnauthorizedApprovalTarget`・`IncompatibleStoredEvent`・`External` を `Store(error.failure_kind())` へ詰め、`lifecycle_error`（`src-tauri/src/adaptor/controller/client/agent_session/provider_tui.rs:539-568`、`:561`）が `Store(kind)` を無条件に `Storage failure: {kind:?}` へ変換する。`main` の `f54e96b6` では `Validation` と `UnauthorizedApprovalTarget` は `InvalidOperation`、`IncompatibleStoredEvent` は `Corrupt` で、storage の表示へは落ちていなかった。
- `AgentSessionUsecaseError` は `Conflict`=`RestartRequired`、`ProviderSessionAlreadyOwned`=`StateRequired` と区別する（`src-tauri/src/usecase/agent_session/usecase.rs:354-366`）。一方 `agent_session_launch.rs:1104-1107`、`agent_session_initial_instruction.rs:100-103`、`agent_session_lifecycle.rs:743-746` は両者を自身の `Conflict` へまとめ、その `Conflict` は `RestartRequired` に固定される。`agent_session_rename.rs:85-88` も `AgentSessionRepositoryError` の 2 変種を `Conflict`（`:103` で `RestartRequired`）へまとめる。
- `commit::storage_unavailable`（`src-tauri/src/adaptor/gateway/local_event_store/commit.rs:30-42`）は `rusqlite::Error` を `reader::sqlite_failure_kind` で分類して `SafeOperationFailure` へ載せるが、これを呼ぶテストは `src-tauri/src` 配下に無い。`failure_test.rs` が通すのは `reader::storage_unavailable` だけで、`domain/local_event/batch_test.rs` は `with_failure_kind` を手で設定する。
- `usecase/watcher.rs:37-70` は `RepositoryStateService` 由来の失敗を `error.to_string()` で `UsecaseError::Repository(String)` にし、同ファイル `134-143` が `Repository(_)` を `FailureKind::Internal` に固定する。発生元の `RepositoryStateError`（`src-tauri/src/usecase/repository_state/error.rs:16-26`）は `ScanInvalidated`=`RestartRequired`、`Repository` / `Code` は下位へ委譲する分類を持つ。
- `ProviderTuiCodedError::AgentSessionLaunchUnavailable` と `AgentSessionTerminalUnavailable` は `provider_tui.rs:151-155` で `Temporary` に分類される。この 2 つを作る箇所（`:523`・`:527`・`:555`・`:558`・`:580`）はすべて `launch_error`（`:498-537`）・`lifecycle_error`（`:539-568`）・`read_error`（`:570-590`）の中にあり、各関数は末尾（`:536`・`:567`・`:589`）で usecase error の `failure_kind` を `with_failure_kind` で無条件に適用する。対応する usecase error は `StateRequired`（`agent_session_lifecycle.rs`、`agent_session_launch.rs`、`agent_session_read.rs:129`）であるため、表側の `Temporary` は実経路で到達不能な値であり、`provider_tui_test.rs:376-378` がその値を期待している。

## 変える部分

- `SafeOperationFailure` の `retryable` から分類を再計算する経路を無くす: `OutcomeUnknown` の失敗が `workspace_tree/query_service.rs` 経由でも `agent_session_lifecycle.rs` 経由でも `RestartRequired` として観測されるようにし、`retryable` の bool から分類を導出する経路を残さない。根拠: R-001、R-005、B-008、Thread `c2221b13`。ルート: 委任。
- `ProviderSessionAlreadyOwned` の分類を下流 usecase で保持する: `launch`・`initial_instruction`・`lifecycle`・`rename` のどの経路から観測しても `StateRequired` になり、`Conflict` 由来の `RestartRequired` と混ざらないようにする。根拠: R-002、R-005、B-008、Thread `3b80debc`。ルート: 委任。
- watcher が repository の失敗の分類を保持する: `RepositoryStateService` が `ScanInvalidated` を返したとき watcher 経由でも `RestartRequired` として観測され、下位 store の一時失敗は `Temporary` として観測されるようにし、repository の失敗を文字列化して分類を落とす経路を残さない。根拠: R-001、R-002、R-008、Thread `a28de71d`。ルート: 委任。
- `AgentSessionLaunchUnavailable` / `AgentSessionTerminalUnavailable` の分類を 1 か所にする: この 2 つに対応する分類が 1 か所にだけ存在し、Connect へ届く分類と一致するようにする。同じ失敗に異なる分類を主張するコードとテストを残さない。根拠: R-005、R-002、B-008、Thread `4fe4797b`。ルート: 委任。
- 非 store の `WorkflowError` が storage の失敗として表示されるのをやめる: `WorkflowError::Validation`・`UnauthorizedApprovalTarget`・`IncompatibleStoredEvent`・`External` が agent_session lifecycle 経由で失敗したとき、利用者へ返るメッセージが storage の失敗を表さないようにする。それぞれの分類（`InvalidInput` / `Permission` / `StateRequired` / `Internal`）は発生元の値のまま Connect へ届く。根拠: `requirements.md` の Scope（失敗の原因の表示の変更は対象に含まれない）、R-002、Thread `5d8bc3ab`。ルート: 委任。
- hook health の再試行の打ち切りを usecase テストで検証する: `repository.save` が `Conflict` を返し続けるとき、試行回数が 4 回で止まることと、呼び出し元へ返る失敗の分類が `RestartRequired` であることを、実経路を通して検証する。根拠: R-004、B-006、`docs/architecture/TEST.md` の usecase 必須、Thread `237b3a62`。ルート: 委任。
- 書込み側の SQLite 失敗の分類を gateway テストで検証する: `commit::storage_unavailable` を実際に通し、`SQLITE_BUSY` / `SQLITE_LOCKED` が `Temporary`、`SQLITE_CORRUPT` / `SQLITE_NOTADB` が `Corrupt`、`SQLITE_READONLY` / `SQLITE_FULL` が `StateRequired`、その他が `Internal` になることを検証する。根拠: R-002、B-002、`docs/architecture/TEST.md` の adaptor/gateway 必須、Thread `49ebdac9`。ルート: 委任。

## 固定するルート

今周に新しく固定する実装上の指定は無い。design-01 で固定した次の 3 つのルートを今周も維持する。

- 内側の層（domain / usecase / gateway）のエラー型は失敗の種類を自前の語彙で持ち、gRPC のステータスコードの語彙を内側の層に入れない。gRPC のステータスコードへの対応付けは adaptor で 1 か所行う。
- 各モジュールの専用のエラー型は残し、統一の 1 型へ置き換えない。
- Connect のエラーへの変換は 1 か所で行い、その入口は分類を伴うエラーだけを受け取る。失敗を文字列へ変換して Connect のエラーにする経路を残さない。

design-02 で固定した 2 つのルート（`src/lib/client.ts` の分類の再判定を削除で解消すること、今周追加した `From<EditorError> for String` と `From<FactReadError> for String` を残さないこと）は、開始状態で満たされているため今周の対象ではない。

## 変えないもの

- 購読の失敗を受けて `refreshClient` を呼ぶ push ループ（`src/lib/client.ts`）。理由: 人間の決定 D1 が固定したのは `refreshClientOnDisconnect` の削除であり、接続の生存の軸は #1895 / #1879 が持つ。
- design-01 で「変えないもの」とした次の対象。client が接続を作り直す条件と desktop 側が daemon を停止したものとして扱う条件（#1891 / #1879 / #1895）、再試行ループそのものの統合（#1889）、CLI / hook が使う HTTP local API のエラー変換と CLI 側の HTTP status の読み替え、`src-tauri/src/adaptor/controller/daemon.rs` の起動時の再開処理。

## 未確定・リスク

- 前周に自動判断した箇所が `requirements.md` の Assumptions に 2 件残る。B-001 の `THEN` から gRPC のステータスコードの無条件要求を外し、gRPC のステータスコードでの表現を B-004 が受け持つ形にしたこと。B-004 の `AND` を、理由ごとにエラーコードが異なることを求める記述から、理由によらず同じエラーコードにまとまらないことを求める記述へ直したこと。今周の検証では Requirements と Behavior に新たな誤り・不足・矛盾は見つからず、自動判断による修正は行っていない。
- 未決のまま残した要求は無い。`[DEFERRED]` で人間へ渡した件も無い。
