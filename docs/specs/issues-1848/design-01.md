# Design 01

## 開始状態

- 差分の基準: `main`（派生点 `4267f22f`）。作業ツリーに未コミットの実装変更は無く、`docs/specs/issues-1848/` の Requirements・Behavior だけが未追跡である。
- 初回。既存の `design-NN.md` は無い。開始時点の挙動は `requirements.md` の Current Behavior を参照する。
- この周までに解消・見送りとなった Thread は無い。

## 変える部分

- 通し番号による応答の破棄の除去: `useReviewSnapshot` が直近に採用した番号より小さい番号の応答を捨てる処理（`acceptedVersionRef` と `src/hooks/useReviewSnapshot.ts:30`、`38-44`、`49-55`、`68-74`、`84` の分岐）を無くし、通し番号が振り直された後の応答も表示へ反映する。根拠: R-001「通し番号が以前より小さい値から振り直された後も、作業ツリーの変更（commit を含む）の後に Review パネルの未コミット差分（Staged / Changes の一覧）が取り直した内容へ更新される」、R-006「取り直した未コミット差分に対して hunk 単位の stage / unstage を行える」、B-001、B-007。ルート: 固定（「固定するルート」の 1 番）
- 破棄処理を確かめているテストの削除: `src/hooks/useReviewSnapshot.test.ts` の `ignores snapshots older than the accepted version` と `accepts snapshot updates with the same version` を削除する。根拠: R-001、B-001。削除する処理だけを確かめているため。ルート: 固定（「固定するルート」の 2 番）
- 打ち切りフラグ `limited` の削除: `SnapshotFlags`（`src-tauri/src/usecase/repository_state/snapshot.rs:14`）、`RepositorySnapshotParts`（同 `77`）、`RepositoryStatusSnapshotDto`（同 `103`）、`RepositoryDiffStatsSnapshotDto`（同 `124`）、`RepositoryBranchCardsSnapshotDto`（同 `145`）、`RepositoryHeadDiffFileTreeSnapshotDto`（同 `169`）、`ReviewSnapshotDto`（`src-tauri/src/usecase/code_dto.rs:180`）、proto（`proto/client.proto` の `limited` 8 箇所）、frontend 型 `ReviewSnapshot`（`src/types/review.ts:37`）、`useReviewSnapshot` の戻り値（`src/hooks/useReviewSnapshot.ts:10`、`124`）から取り除く。根拠: R-005「worktree の状態を表す snapshot（status、diff stats、branch cards、head diff file tree、review snapshot）は、読み手のいない打ち切りフラグ（`limited`）を公開しない」、B-006。ルート: 固定（「固定するルート」の 3 番）
- `repository-snapshot-changed` の送信経路の削除: proto の `Push.repository_snapshot_changed`（`proto/client.proto:13`）と `RepositorySnapshotChangedEvent`（同 `3147-3153`）、`conversions.rs:3235-3247` の変換、`BackendPush::RepositorySnapshotChanged`（`src-tauri/src/adaptor/gateway/push.rs:20`、`70-73`）、`state.rs:291-296` の emit、`RepositorySnapshotChangedEvent`（`src-tauri/src/usecase/repository_state/snapshot.rs:283-300`）と `client_api_acceptance.rs:26` の再公開を取り除く。根拠: R-004「backend は、受け手のいない `repository-snapshot-changed` 通知を client へ送らない」、B-005。ルート: 固定（「固定するルート」の 4 番）
- スキャン開始通知と phase の削除: `mark_refresh_started`（`src-tauri/src/usecase/repository_state/worktree.rs:220-230`）の `notifier.snapshot_changed` 呼び出しと、`SnapshotNotificationPhase`（同 `37-40`）および `SnapshotNotification.phase`（同 `33`）、`state.rs:299-300` の `RefreshStarted` による早期 return を取り除く。`mark_refresh_started` の `refreshing` フラグの設定は残す。根拠: R-004、B-005。`repository-snapshot-changed` の送信を消すと `RefreshStarted` の通知は効果を持たない呼び出しとなり、phase は `SnapshotCommitted` の一値になるため。ルート: 固定（「固定するルート」の 4 番）

## 固定するルート

1. `useReviewSnapshot` の `acceptedVersionRef` と、それを扱う分岐を削除する（`src/hooks/useReviewSnapshot.ts:30`、`38-44`、`49-55`、`68-74`、`84`）。範囲: frontend の当該 hook のみ。粒度: 対象の識別子まで指定。理由: 応答の順番の入れ替わりは `requestIdRef` が既に防いでおり、監視状態が生きている間は backend の番号が減らないため、この処理が働くのは番号が振り直されたときだけである。
2. `src/hooks/useReviewSnapshot.test.ts` の `ignores snapshots older than the accepted version` と `accepts snapshot updates with the same version` を削除する。`resets accepted version when rootPath changes` は削除せず、rootPath 切り替えの検証として残す（名前は実態に合わせてよい）。範囲: 当該テストファイルの 3 件。粒度: テスト名まで指定。理由: 前 2 件は削除する処理を確かめているため。後者は `acceptedVersionRef` に依存しない被覆を含み、削除の理由が当てはまらないため。
3. 打ち切りフラグ `limited` を、`SnapshotFlags`・`RepositorySnapshotParts`・`RepositoryStatusSnapshotDto`・`RepositoryDiffStatsSnapshotDto`・`RepositoryBranchCardsSnapshotDto`・`RepositoryHeadDiffFileTreeSnapshotDto`・`ReviewSnapshotDto`・proto（`client.proto` の 8 箇所）・frontend 型（`ReviewSnapshot`）・`useReviewSnapshot` の戻り値から削除する。範囲: snapshot の `limited` のみで、`ReviewFileView` の `limited` は対象外。粒度: 対象の型まで指定。理由: production で常に `false` であり、読み手も書き手も無いため。
4. `repository-snapshot-changed` を、client への送信経路（proto の event、`conversions.rs` の変換、`BackendPush::RepositorySnapshotChanged`、emit、`RepositorySnapshotChangedEvent`）、`mark_refresh_started` の `RefreshStarted` 通知呼び出し、`SnapshotNotificationPhase` まで削除する。`refreshing` フラグと `stale` / `loading` の導出は変えない。範囲: 通知の経路の終端まで。粒度: 経路の終端まで指定。理由: frontend に受け取る処理が無く、送信を消すと `RefreshStarted` の通知が効果を持たない呼び出しになるため。
5. `get_review_file_view` と画像取得の番号照合（`review_usecase.rs:247-250`、`853-865`）、backend の通し番号の採番方式、`useReviewSnapshot` の `requestIdRef` は変えない。範囲: 維持対象の指定。粒度: 対象の識別子まで指定。理由: 番号照合は同じかどうかしか見ないため、番号が振り直されても誤動作しない。

## 変えないもの

- 差分取得（`get_review_file_view`）と画像取得の番号照合（`review_usecase.rs:247-250`、`853-865`）。理由: 一覧の番号と backend の現在の番号が同じかどうかしか見ないため、番号が振り直されても誤動作しない。
- `stale` の間 hunk 単位の操作を無効にする扱い（`src/components/panels/ReviewPanel.tsx:413-416`）。理由: 番号照合の結果を表示側で受ける経路であり、本変更の対象ではない。
- backend の通し番号の採番方式。接続をまたいで番号を引き継ぐ、監視状態の寿命を変える、といった変更は行わない。理由: 本変更は frontend 側の破棄処理の除去で解決するため。
- 最後に投げた取得要求の応答だけを採用する制御（`useReviewSnapshot` の `requestIdRef`）。理由: 応答の順番の入れ替わりはこの制御が防いでいる。
- スキャン中であることを `stale` / `loading` として読み側へ伝える `refreshing` フラグ（`worktree.rs:211-218`）と、その導出。理由: `RefreshStarted` の通知とは別の経路であり、通知を削除しても残る必要がある。
- ファイル表示の打ち切りを表す `ReviewFileView` の `limited`（`code_dto.rs:232`、`281`、`src/types/review.ts:73`、`111`）。理由: snapshot の `limited` とは別のフラグで、`DiffViewerSection` が読んでいる。
- worktree を切り替えたとき表示中の内容が新しい worktree のものへ差し替わる挙動。理由: `useReviewSnapshot.test.ts` の `resets accepted version when rootPath changes` が確かめている被覆であり、落とさない。
- 接続の張り直しが起きたことを後から確認できるようにする変更（daemon のログへの購読の開始・終了の記録）。理由: 観測性は本変更と独立した関心事で、変更の境界を明確にするため。
- `docs/specs/issues-1210/` と `docs/specs/issues-1303/`。理由: 当時の記録であるため。

## 未確定・リスク

- R-001 の原因の特定が実機で裏付けられていない。症状の発生時に実際に接続の張り直しが起きたかどうかを確認できておらず、daemon は購読の開始・終了をログに出さない。張り直し以外の経路で通し番号の食い違いが起きていた場合、`acceptedVersionRef` の除去だけでは R-001 の症状が解消しない可能性がある。
- R-006 / B-007 の前提（番号が一致しない間は hunk 単位の stage / unstage が無効のままになる）は `review_usecase.rs:247-250` と `ReviewPanel.tsx:413-416` から読み取ったもので、実機では確認していない。前提が外れる場合、B-007 は `acceptedVersionRef` の除去とは別の要因で満たされる／満たされないことになる。
- 自動判断した箇所は無い。Requirements・Behavior に誤り・不足・矛盾は見つからず、両ファイルを修正していない。
- 未決のまま残した要求は無い。`[DEFERRED]` で人間へ渡した件も無い。
