# Design 04

## 開始状態

差分の基準は `main` の `f54e96b6` から派生した `feat/issues/1880`。直前の Design は `docs/specs/issues-1880/design-03.md`。

design-03 の「変える部分」7 件は、この周の開始時点で作業ツリーに実装済みである（未コミット）。`SafeOperationFailure` は `retryable` の bool を持たず `classification` を保持し、`agent_session_launch.rs` / `agent_session_initial_instruction.rs` / `agent_session_lifecycle.rs` は `Conflict(FailureKind)` で発生元の分類を運び `agent_session_rename.rs` は `ProviderSessionAlreadyOwned` を独立の変種として `StateRequired` に分類する。`usecase/watcher.rs` の `UsecaseError::Repository` は `RepositoryStateError` を型のまま持ち分類を委譲し、`ProviderTuiCodedError::AgentSessionLaunchUnavailable` / `AgentSessionTerminalUnavailable` は `FailureKind` を持って 1 か所で分類され、`map_workflow_error` は `Validation` / `UnauthorizedApprovalTarget` / `IncompatibleStoredEvent` / `External` を `Workflow(error)` のまま通す。hook health の再試行の打ち切りは `hook_health_error_test.rs:153-193` が、書込み側 SQLite の分類は `local_event_store/commit_test.rs` が検証する。実装済みの内容は再掲しない。

この周までに解消した Thread、`[DEFERRED]` で人間へ渡した Thread、`[REJECTED]` とした Thread はいずれも無い。

開始状態で確認した事実のうち、この周の変える部分の対象になるもの。

- `WorkflowExternalEditorGateway`（`src-tauri/src/adaptor/gateway/workflow/editor_gateway.rs:53`・`:66`）は `NativeEditorLauncherGateway::open_path` が返す `EditorError` を `error.to_string()` で `WorkflowError::external` にする。`domain/external_editor/gateway.rs:25-33` は `EditorError::Launch` を `StateRequired` と分類する一方、`domain/workflow/error.rs:91` は `WorkflowError::External` を `Internal` に固定する。この経路は `usecase/workflow/mod.rs:468-470` を通り `adaptor/controller/client/workflow/shared.rs:534`・`:559` の client command から Connect へ届く。同じ `EditorError` は external_editor の command 経路では型のまま運ばれる。
- `usecase/provider_lifecycle/hook_health.rs` の 3 つの記録経路（`115-135`、`148-163`、`175-190`）は `repository.save` の戻りのうち `ProviderHookHealthRepositoryError::Conflict` だけを `continue` で同じ位置の再試行に回し、それ以外を即時返却する。`domain/provider_lifecycle/repository.rs:100-111` は `Conflict` を `RestartRequired` と分類し、`hook_health_repository_impl.rs:205-210` は `CommitBatchError::StreamHeadConflict` を `Conflict`・それ以外を `Store(failure_kind())` にし、`domain/local_event/batch.rs:143-147` は `StreamHeadConflict` と `OutcomeUnknown` をともに `RestartRequired` とする。同じ `RestartRequired` の失敗が、変種名によって同じ位置で再試行されるか即時返却されるかに分かれる。design-03 で追加した `hook_health_error_test.rs:153-193` は、この「`Conflict` だけを 4 回再試行する」形を期待値として持つ。
- `adaptor/controller/api/client.rs:68-77` の `desktop_settings()` は `AppConfigUsecase::desktop_settings()` が返す `UsecaseError` を `map_err(String::from)` で文字列にして `Result<Option<wire::DesktopSettings>, String>` を返し、`adaptor/controller/api/client_service.rs:10-12` がその `String` を `AppError::new(error).with_failure_kind(FailureKind::Internal)` で再分類してから `classified_error` へ渡す。発生元の `usecase/app_config/error.rs:17-25` は `InvalidInput`=`InvalidInput`、`AppConfig`=下位委譲の分類を既に持つ。
- `usecase/provider_lifecycle/ingress.rs:436-467` の `map_session_error` と `map_session_repository_error` は `ProviderSessionAlreadyOwned` を `Conflict` と同じ `ProviderLifecycleIngressUsecaseError::Conflict` へまとめ、同ファイル `:469-480` がその `Conflict` を `RestartRequired` に固定する。`adaptor/gateway/agent_session/agent_session_repository.rs:879-883` の `is_owned` も `Conflict` / `ProviderSessionAlreadyOwned` / `Unavailable` を同じ `AgentSessionHistoryGatewayError::Unavailable` へまとめ、`domain/agent_session/provider_history_gateway.rs:52-61` がそれを `Temporary` とする。

## 変える部分

- エディタの起動失敗の分類を workflow 経由でも保持する: エディタの起動が失敗したとき、workflow の `open_workflow_in_editor` / `open_facet_in_editor` から観測しても external_editor の command から観測しても同じ分類（`StateRequired`）になるようにし、`EditorError` を文字列へ変換して `WorkflowError::External` にする経路を残さない。根拠: R-002「分類は、失敗が起きた場所の理由に対応する」、R-005、R-008「失敗を文字列へ変換して Connect のエラーにする経路は残らない」、B-008、Thread `828e79db`。ルート: 委任。
- hook health の再試行を分類だけで決める: hook health の記録経路で、同じ位置で再試行するかどうかが失敗の分類だけで決まり、同じ分類の失敗が変種名によって扱いを分けられないようにする。上の段階からやり直すべき分類の失敗は、この位置で再試行されずに呼び出し元へ返る。design-03 で追加した `hook_health_error_test.rs` の再試行の期待値は、変種名ではなく分類に対応する形にする。根拠: R-004「ある失敗を再試行するかどうかは、その失敗の分類だけで決まる」、B-007、Thread `9fb77907`。ルート: 委任。
- `GetServerInfo` の desktop settings の失敗が分類を保持する: desktop settings の取得が失敗したとき、Connect のエラーコードが発生元の `UsecaseError` の分類と一致し、入力不正が `Internal` にまとめられないようにする。この経路で失敗を `String` へ落としてから分類を付け直すコードを残さない。根拠: R-001、R-003「Connect のエラーコードは、その失敗の分類と一致する」、R-008、B-004、Thread `7b0a7601`。ルート: 委任。
- `ProviderSessionAlreadyOwned` の分類を残りの経路でも保持する: provider_lifecycle の ingress 経由でも、provider session の所有を確かめる history gateway 経由でも、`ProviderSessionAlreadyOwned` が `StateRequired` として観測され、`Conflict` 由来の `RestartRequired` や store の `Temporary` と混ざらないようにする。根拠: R-002、R-005、B-008、Thread `3b80debc`（`[STILL_OPEN]`）。ルート: 委任。

## 固定するルート

今周に新しく固定する実装上の指定は無い。design-01 で固定し design-03 で維持した次の 3 つのルートを今周も維持する。

- 内側の層（domain / usecase / gateway）のエラー型は失敗の種類を自前の語彙で持ち、gRPC のステータスコードの語彙を内側の層に入れない。gRPC のステータスコードへの対応付けは adaptor で 1 か所行う。
- 各モジュールの専用のエラー型は残し、統一の 1 型へ置き換えない。
- Connect のエラーへの変換は 1 か所で行い、その入口は分類を伴うエラーだけを受け取る。失敗を文字列へ変換して Connect のエラーにする経路を残さない。

## 変えないもの

- 再試行ループそのものを 1 つの実装へまとめること。理由: design-01「変えないもの」のとおり #1889 が扱う。hook health の変更は再試行の可否の決め方に限る。
- design-01 で「変えないもの」とした次の対象。client が接続を作り直す条件と desktop 側が daemon を停止したものとして扱う条件（#1891 / #1879 / #1895）、CLI / hook が使う HTTP local API のエラー変換と CLI 側の HTTP status の読み替え、`src-tauri/src/adaptor/controller/daemon.rs` の起動時の再開処理。
- 購読の失敗を受けて `refreshClient` を呼ぶ push ループ（`src/lib/client.ts`）。理由: design-03「変えないもの」のとおり、人間の決定 D1 が固定したのは `refreshClientOnDisconnect` の削除であり、接続の生存の軸は #1895 / #1879 が持つ。

## 未確定・リスク

- 前周までに自動判断した箇所が `requirements.md` の Assumptions に 2 件残る。B-001 の `THEN` から gRPC のステータスコードの無条件要求を外し、gRPC のステータスコードでの表現を B-004 が受け持つ形にしたこと。B-004 の `AND` を、理由ごとにエラーコードが異なることを求める記述から、理由によらず同じエラーコードにまとまらないことを求める記述へ直したこと。今周の検証では Requirements と Behavior に新たな誤り・不足・矛盾は見つからず、自動判断による修正は行っていない。
- 未決のまま残した要求は無い。`[DEFERRED]` で人間へ渡した件も無い。
