# Context

## 入力

- 正本: [#1848 \[review\] 接続の張り直し後、commit しても未コミット差分の表示が更新されない](https://github.com/siro33950/releash/issues/1848)
- 実装の参照先: `src/hooks/useReviewSnapshot.ts`、`src/hooks/useReviewSnapshot.test.ts`、`src/hooks/useGitEventRefresh.ts`、`src/lib/client.ts`、`src/components/panels/ReviewPanel.tsx`、`src-tauri/src/usecase/repository_state/worktree.rs`、`src-tauri/src/usecase/repository_state/service.rs`、`src-tauri/src/usecase/repository_state/snapshot.rs`、`src-tauri/src/usecase/review_usecase.rs`、`src-tauri/src/usecase/watcher.rs`、`src-tauri/src/adaptor/gateway/repository/state.rs`、`src-tauri/src/adaptor/gateway/repository/scanner.rs`、`src-tauri/src/adaptor/gateway/push.rs`、`src-tauri/src/adaptor/controller/api/client_service.rs`、`proto/client.proto`
- 過去の記録: `docs/specs/issues-1210/`（通し番号と `stale` / `loading` / `limited` を導入した時の記録）、`docs/specs/issues-1303/`（frontend の番号による破棄を現状維持とした時の記録）。いずれも当時の記録であり、本変更で改訂しない。

## 確定済みの背景

- Review パネルの未コミット差分は `get_review_snapshot` の応答で描画する。応答には worktree ごとの監視状態が持つ通し番号（`version`）が付く。
- 通し番号のカウンタは worktree ごとの監視状態にあり 0 から始まる（`worktree.rs:119`）。購読者が 0 になると監視状態ごと破棄され（`service.rs:183-210`）、次の購読で 0 から作り直される。監視状態が無いときの読み出しは常に 0 を返す（`service.rs:103`）。
- 購読の寿命は client への通知 stream に結び付く。stream が終わると購読が解放され、その購読の監視がすべて止まる（`client_service.rs:31-33`、`watcher.rs:126`）。client は stream が切れると 1 秒後に接続を張り直し、監視を張り直してから全体を取り直す（`src/lib/client.ts:140-197`）。
- 応答の順番の入れ替わりは、最後に投げた取得要求の応答だけを採用する制御（`useReviewSnapshot.ts` の `requestIdRef`）が防いでいる。
- 差分取得（`get_review_file_view`）と画像取得の番号照合は、一覧の番号と backend の現在の番号が「同じかどうか」だけを見る（`review_usecase.rs:247-250`、`853-865`）。番号が振り直されても誤動作しない。
- `limited` は `#1210` で導入した打ち切りフラグで、値を立てる実装が入っておらず production では常に `false` である（`scanner.rs:43-53`）。`SnapshotFlags`（`snapshot.rs:10-14`）から status / diff stats / branch cards / head diff file tree / review snapshot の 5 つの read model と proto（`client.proto` の 8 箇所）へ複製されているが、frontend に読み手が無い（読み出しは `useReviewSnapshot.ts:124` の再公開のみ）。`ReviewFileView` の `limited` はファイル表示の打ち切りを表す別のフラグで、`DiffViewerSection` が読んでいる。
- `repository-snapshot-changed` は backend がスキャンの開始（`RefreshStarted`）と完了（`SnapshotCommitted`）のたびに送っている（`state.rs:290-296`）が、frontend に受け取る処理が無い。`RefreshStarted` の通知はこの送信にしか使われず、送信の直後に早期 return する（`state.rs:299-300`）。スキャン中であることを `stale` / `loading` として読み側へ伝えるのは、通知ではなく `refreshing` フラグである（`worktree.rs:211-218`）。

# Outcome

- 対象者: Releash の Review パネルで未コミット差分を確認する開発者。
- 現在の問題: backend との接続が張り直されると、通し番号が振り直され、frontend が以前より小さい番号の応答を捨てるため、commit しても未コミット差分の表示が更新されない。あわせて、表示中の番号が backend の現在の番号と食い違うため、hunk 単位の stage / unstage が無効のままになる。
- 変更後の状態: 通し番号が振り直された後も、取り直した未コミット差分が表示へ反映され、hunk 単位の操作ができる。あわせて、この変更で読み手が無くなるコードと、受け手のいない通知が残らない。

# Current Behavior

## 症状（2026-09-21 に実機で確認）

1. Review パネルで未コミット差分を表示した状態で、しばらく使い続ける。
2. commit する。
3. Staged / Changes の一覧が commit 前のまま残る。
4. diff base を一度切り替えて戻すと、正しい表示になる。

daemon のログに、監視やスキャンの失敗は出ていない。

## コードから確認した経路

- `useReviewSnapshot` は、直近に採用した番号より小さい番号の応答を捨てる（`src/hooks/useReviewSnapshot.ts:68-73`）。覚えている番号を消すのは、worktree か diff base が変わった時（同 38-44 行）と、取得に失敗した時（同 84 行）だけである。
- commit の検知と frontend への通知は動いており、通知を受けて取り直した応答が frontend で捨てられている。
- `useReviewSnapshot` は `limited` を返すが、受け取る production コードが無い。
- backend は `repository-snapshot-changed` を送るが、frontend に受け取る処理が無い。

## 未確認

- 症状の発生時に、実際に接続の張り直しが起きたかどうか。daemon は購読の開始・終了をログに出さず、frontend の `Client subscription ended` は webview の console にしか出ない。diff base の切り替えで直ることは、上の経路と整合する。
- daemon だけが再起動して UI が生き残った場合も通し番号は 0 に戻るが、この経路で同じ症状が出るかは確認していない。
- 番号の不一致の間、hunk 単位の stage / unstage が無効のままになる点（`review_usecase.rs:247-250`、`src/components/panels/ReviewPanel.tsx:413-416`）は、コードから読み取ったもので実機では確認していない。

# Scope / Non-goals

## 変更するもの

- frontend の「以前より小さい番号の応答を捨てる」処理（`useReviewSnapshot` の `acceptedVersionRef` とそれを扱う分岐）。
- 上の処理を確かめているテスト（`src/hooks/useReviewSnapshot.test.ts` の `ignores snapshots older than the accepted version`、`accepts snapshot updates with the same version`）。
- 打ち切りフラグ `limited`（`SnapshotFlags`、`RepositorySnapshotParts`、status / diff stats / branch cards / head diff file tree / review snapshot の 5 つの read model、proto、frontend 型、`useReviewSnapshot` の戻り値）。
- `repository-snapshot-changed` の通知（client への送信経路、`RefreshStarted` のスキャン開始通知、`SnapshotNotificationPhase`）。

## 変更しないもの

- 差分取得に一覧の番号を渡し、backend の現在の番号と違えば `stale` を返す照合と、`stale` の間 hunk 単位の操作を無効にする扱い。
- 画像取得時の番号の一致の確認。
- backend の通し番号の採番方式（接続をまたいで番号を引き継ぐ、監視状態の寿命を変える、といった変更は行わない）。
- 最後に投げた取得要求の応答だけを採用する制御。
- スキャン中であることを `stale` / `loading` として読み側へ伝える `refreshing` フラグ。
- ファイル表示の打ち切りを表す `ReviewFileView` の `limited`。
- worktree を切り替えたとき表示中の内容が新しい worktree のものへ差し替わる挙動。
- 接続の張り直しが起きたことを後から確認できるようにする変更（daemon のログへの購読の開始・終了の記録）。
- `docs/specs/issues-1210/` と `docs/specs/issues-1303/` の記録。

# Requirements

- R-001: backend との接続が張り直され、通し番号が以前より小さい値から振り直された後も、作業ツリーの変更（commit を含む）の後に Review パネルの未コミット差分（Staged / Changes の一覧）が取り直した内容へ更新される。
- R-002: 同一の接続が続いている間に取得要求の応答の順番が入れ替わっても、表示へ反映されるのは最後に投げた取得要求の応答だけである。
- R-003: 差分取得と画像取得における一覧の番号と backend の現在の番号の照合は維持され、番号が一致しない間は差分が `stale` として返り、hunk 単位の操作が無効になる。
- R-004: backend は、受け手のいない `repository-snapshot-changed` 通知を client へ送らない。
- R-005: worktree の状態を表す snapshot（status、diff stats、branch cards、head diff file tree、review snapshot）は、読み手のいない打ち切りフラグ（`limited`）を公開しない。
- R-006: backend との接続が張り直され、通し番号が振り直された後も、取り直した未コミット差分に対して hunk 単位の stage / unstage を行える。

# Assumptions / Open Questions

## Assumptions

- 番号の不一致の間 hunk 単位の stage / unstage が無効のままになることは、コードから読み取ったもので実機では確認していない。この状態の解消を本変更の対象に含める（R-006）。
- 症状の発生時に接続の張り直しが起きたかどうかは確認できていない。張り直しが起きたことを後から確認できるようにする変更は行わず、通し番号の振り直しで更新が止まる経路の除去だけを本変更の対象とする。

## Open Questions

なし。
