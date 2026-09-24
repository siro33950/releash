# Design 02

## 開始状態

差分の基準は `main` の `f54e96b6` から派生した `feat/issues/1880`。直前の Design は `docs/specs/issues-1880/design-01.md`。

design-01 の「変える部分」は、この周の開始時点で作業ツリーに実装済みである（`src-tauri/` を中心に未コミットで 120 ファイル）。実装済みの内容は再掲しない。この周までに解消した Thread、`[DEFERRED]` で人間へ渡した Thread、`[REJECTED]` とした Thread はいずれも無い。

開始状態で確認した事実のうち、この周の変える部分の対象になるもの。

- `src/lib/client.ts` と `scripts/generate-client-protocol.mjs`、`src/generated/client_commands.ts` は design-01 の周で変更されていない。`refreshClientOnDisconnect`（`src/lib/client.ts:130-140`）が `CommandError` detail の有無・`Code.Unavailable`・`Code.Unknown` かつ `cause` が `TypeError` を組み合わせて判定する形のまま残り、呼び出しは `src/lib/client.ts:318`・`src/lib/client.ts:483`・`src/generated/client_commands.ts:3374` の 3 か所、生成元テンプレートは `scripts/generate-client-protocol.mjs:57`。
- `src/lib/client.ts:429-434` が、detach 失敗を無視する条件に `error.code === Code.Canceled` と `!error.findDetails(CommandErrorSchema).length` を組み合わせている。
- `From<EditorError> for String`（`src-tauri/src/domain/external_editor/gateway.rs:35-39`）が今周の実装で追加され、呼び出し元が無い。
- `From<FactReadError> for String`（`src-tauri/src/adaptor/gateway/workflow/fact_log.rs:39-43`）が今周の実装で追加され、`find_session_attachment`（`fact_log.rs:1232-1239`）が `Result<_, String>` を返してこれを使う。受け手は `src-tauri/src/adaptor/gateway/workspace_tree/repository.rs:202` の `fold_query_error`。
- `hook_health_repository_impl.rs:205-207` が `CommitBatchError::PayloadConflict` と `StreamHeadConflict` を同じ `ProviderHookHealthRepositoryError::Conflict` へまとめる。発生元（`src-tauri/src/domain/local_event/batch.rs:139-152`）は前者を `StateRequired`、後者を `RestartRequired` と区別する。
- `src-tauri/src/other/error.rs:100-107` が `AppError::Coded` を理由によらず `FailureKind::StateRequired` にする。
- 今周 `src-tauri/src/domain/` に追加した `ClassifiedFailure` の実装 20 ファイル分について、`failure_kind` の期待値を検証する domain のテストが無い。domain の `*_test.rs` にある `failure_kind` は `NodeExecutionFailureKind` の項目であり、この分類とは別物である。
- `src-tauri/src/adaptor/protocol/connect.rs:26-47` の `code()` は 16 分類を分岐するが、`connect_test.rs:8-33` が検証するのは `Temporary` / `RestartRequired` / `StateRequired` / `Expired` / `Corrupt` / `Internal` / `InvalidInput` の 7 分類。

## 変える部分

- `refreshClientOnDisconnect` を削除する: 定義（`src/lib/client.ts:130-140`）と呼び出し（`src/lib/client.ts:318`、`src/lib/client.ts:483`、`src/generated/client_commands.ts:3374`）、生成元テンプレート（`scripts/generate-client-protocol.mjs:57`）を削除し、`src/generated/client_commands.ts` を再生成する。frontend には分類を読む判定を置かない。根拠: R-005「分類を読む側が、エラーコード・原因の型・メッセージ・エラーに付く detail の有無から分類を決め直すことはない」、B-008、Thread `c163b5ef`。ルート: 削除で解消することと削除の範囲を固定（下記「固定するルート」）。削除後の frontend の形とテストの更新範囲は委任。
- detach 失敗を無視する条件から detail の有無を外す: `src/lib/client.ts:429-434` の条件を `error.code === Code.Canceled` だけにし、`!error.findDetails(CommandErrorSchema).length` を削る。根拠: R-005、requirements.md Scope「`src/lib/client.ts:432` の detail の有無の判定」。ルート: 委任。
- `From<EditorError> for String` を残さない: 今周追加した `src-tauri/src/domain/external_editor/gateway.rs:35-39` の impl を取り消す。`EditorError` の変種と `failure_kind` の分岐、実経路は変えない。根拠: R-008「分類を伴わない値は Connect への変換を通らない」、Thread `f2fc6c2a`。ルート: 2 件の impl を残さないことを固定（下記「固定するルート」）。それ以外は委任。
- `From<FactReadError> for String` を残さず、`find_session_attachment` は `FactReadError` を返す: 今周追加した `src-tauri/src/adaptor/gateway/workflow/fact_log.rs:39-43` の impl を取り消し、`find_session_attachment`（`fact_log.rs:1232-1239`）の戻り値を `FactReadError` にする。`LocalEventQueryError::QueryBusy`（`Temporary`）が `IncompatibleStoredEvent`（`StateRequired`）や `WorkflowError::External`（`Internal`）へ置き換わらないようにする。根拠: R-001、R-002、R-008、B-002、Thread `d76dede1`。ルート: 2 件の impl を残さないことと戻り値の型を固定（下記「固定するルート」）。呼び出し境界の扱い（`workspace_tree/repository.rs:202` の `fold_query_error` 経路、`execution_archive_repository.rs` の `WorkflowError::external` 経路を含む）は委任。
- hook health の commit 経路で conflict の分類を保持する: `hook_health_repository_impl.rs:205-207` が `PayloadConflict` と `StreamHeadConflict` を同じ `Conflict`（`RestartRequired` 固定）へまとめるのをやめ、発生元の分類（`PayloadConflict`=`StateRequired`、`StreamHeadConflict`=`RestartRequired`）を保つ。根拠: R-002、R-005、B-008、Thread `dd4c7988`。ルート: 委任。
- `AppError::Coded` の分類を理由に対応させる: `src-tauri/src/other/error.rs:100-107` が `Coded` を理由によらず `FailureKind::StateRequired` にするのをやめる。`parse_blob_reference`（`src-tauri/src/adaptor/controller/client/code/review.rs:112-114`）が返す `INVALID_REQUEST` のような入力不正が `InvalidInput` として Connect へ届くことを含む。根拠: R-002、R-003「command の失敗が理由によらず `FAILED_PRECONDITION` になることはない」、B-004、Thread `c29ff5a8`。ルート: 委任。
- domain の分類分岐を domain のテストで検証する: 今周 `src-tauri/src/domain/` に追加した `ClassifiedFailure` の実装について、分類値そのものを検証するテストを置く。`EditorError`、`AppConfigError`、`NotionError`、`CodeError`、`TerminalSubscriptionError` を含む。根拠: R-001、R-002、B-001、`docs/architecture/TEST.md` の domain 必須、Thread `0cc2da2b`。ルート: 委任。
- Connect への対応表を全分類で検証する: `src-tauri/src/adaptor/protocol/connect.rs` の `code()` が持つ 16 分類すべてについて、`classified_error` と `command_error` の両経路で Connect のエラーコードの一致を検証する。未検証の 9 分類は `Missing` / `AlreadyPresent` / `Permission` / `Capacity` / `Unsupported` / `Cancelled` / `Unknown` / `OutsideRange` / `AuthenticationRequired`。根拠: R-003、B-004、Thread `d7e633d9`。ルート: 委任。

## 固定するルート

- `src/lib/client.ts` の分類の再判定は、分類を読む形へ移行するのではなく削除で解消する。範囲: `refreshClientOnDisconnect` の定義（`src/lib/client.ts:130-140`）、呼び出し（`src/lib/client.ts:318`、`src/lib/client.ts:483`、`src/generated/client_commands.ts:3374`）、生成元テンプレート（`scripts/generate-client-protocol.mjs:57`）、および `src/generated/client_commands.ts` の再生成。frontend には分類を読む判定を置かない。粒度: 削除後の frontend の形と、`src/lib/client.test.ts` を含むテストの更新範囲は委任。理由: 人間の決定 D1。接続の生存を分類の軸に載せない。関係: R-005、B-008。
- 今周追加した `From<EditorError> for String`（`src-tauri/src/domain/external_editor/gateway.rs:35-39`）と `From<FactReadError> for String`（`src-tauri/src/adaptor/gateway/workflow/fact_log.rs:39-43`）を 2 件とも残さない。`find_session_attachment`（`fact_log.rs:1232-1239`）は `FactReadError` を返す形にする。範囲: この 2 件の impl と `find_session_attachment` の戻り値の型。粒度: 戻り値の変更に伴う呼び出し境界の扱いは委任。理由: 人間の決定 D3。関係: R-001、R-002、R-008、B-002。
- design-01 で固定した次の 3 つのルートを今周も維持する。内側の層（domain / usecase / gateway）のエラー型は失敗の種類を自前の語彙で持ち、gRPC のステータスコードの語彙を内側の層に入れない。各モジュールの専用のエラー型は残し、統一の 1 型へ置き換えない。Connect のエラーへの変換は 1 か所で行い、その入口は分類を伴うエラーだけを受け取る。

## 変えないもの

- 購読の失敗を受けて `refreshClient` を呼ぶ push ループ（`src/lib/client.ts:192-199`）。理由: D1 が固定したのは `refreshClientOnDisconnect` の削除であり、接続の生存の軸は #1895 / #1879 が持つ。
- `EditorError` の変種、`failure_kind` の分岐、実経路。理由: Thread `f2fc6c2a` は今周追加した impl の取り消しだけを対象とする。
- design-01 で「変えないもの」とした次の対象。client が接続を作り直す条件と desktop 側が daemon を停止したものとして扱う条件（#1891 / #1879 / #1895）、再試行ループそのものの統合（#1889）、CLI / hook が使う HTTP local API のエラー変換と CLI 側の HTTP status の読み替え、`src-tauri/src/adaptor/controller/daemon.rs` の起動時の再開処理。

## 未確定・リスク

- 前周に自動判断した箇所が `requirements.md` の Assumptions に 2 件残る。B-001 の `THEN` から gRPC のステータスコードの無条件要求を外し、gRPC のステータスコードでの表現を B-004 が受け持つ形にしたこと。B-004 の `AND` を、理由ごとにエラーコードが異なることを求める記述から、理由によらず同じエラーコードにまとまらないことを求める記述へ直したこと。今周の検証では Requirements と Behavior に新たな誤り・不足・矛盾は見つからず、自動判断による修正は行っていない。
- 未決のまま残した要求は無い。`[DEFERRED]` で人間へ渡した件も無い。
