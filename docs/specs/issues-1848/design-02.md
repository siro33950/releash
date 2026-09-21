# Design 02

## 開始状態

- 差分の基準: `main`（派生点 `4267f22f`）。作業ツリーには design-01.md の「変える部分」を実装した未コミットの変更がある。
- 直前の Design: `docs/specs/issues-1848/design-01.md`。同 Design の「固定するルート」1〜5 と「変えないもの」は解除せず維持する。
- この周までに解消・見送りとなった Thread は無い。`[DEFERRED]`（不採用）・`[REJECTED]`（不成立）・resolve 済みの Thread はいずれも無い。
- `requirements.md` の R-001〜R-006 と `behavior.md` の B-001〜B-007 は、この周で変更していない。

## 変える部分

- `refreshing` フラグを立てる操作の集約: `refreshing` を `true` にする操作の実装を一箇所だけにし、スキャン開始（`src-tauri/src/usecase/repository_state/worker.rs:79`）とテスト（`src-tauri/src/usecase/repository_state/worktree.rs:497`）の呼び出し元が同じ実装を経由する状態にする。根拠: Thread `74eac82a-62b3-409f-abb6-63cc697a9ba3`（`[FIX_POLICY]`）。design-01.md の固定ルート 4 による `RefreshStarted` 通知の削除で `mark_refresh_started`（`worktree.rs:212-214`）の固有処理が無くなり、`set_refreshing(true)`（同 `143-145`）と同一の `refreshing.store(..., Ordering::SeqCst)` を二箇所で実装している。`docs/architecture/README.md:32`「同じ操作の実装は 1 つに集約する。」に反する。ルート: 委任
- `snapshot.rs` / `state.rs` のテスト配置の集約: `src-tauri/src/usecase/repository_state/snapshot.rs` と `src-tauri/src/adaptor/gateway/repository/state.rs` のそれぞれについて、当該実装のテストが `docs/architecture/TEST.md`「配置」の定める `<impl>_test.rs` / `<impl>_tests` 一箇所にまとまり、inline の `mod tests` と外部 test module が併存しない状態にする。根拠: Thread `2ea2b65a-db92-4781-aa74-9294bc69bb63`（`[FIX_POLICY]`）。この周の前に `snapshot_test.rs` / `state_test.rs` を追加した一方で、基準コミット `4267f22f` から存在する inline の `mod tests`（`snapshot.rs:180-245`、`state.rs:319-478`）を残したため、同一実装のテストが二箇所に分散している。ルート: 委任

## 固定するルート

- 今周で新たに固定したルートは無い。上記 2 件はいずれも「ルート: 委任」である。`mark_refresh_started` を廃して `set_refreshing(true)` へ寄せるか `mark_refresh_started` を `set_refreshing` の委譲にするか、呼び出し元の扱い、inline の `mod tests` を `<impl>_test.rs` 側へ移すか追加分を inline へ寄せた上で規約の配置へ作り直すか、テストヘルパーや import の再構成は、いずれも指定しない。
- design-01.md の「固定するルート」1〜5 を今周も維持する。解除はしない。

## 変えないもの

- `refreshing` フラグの読み側への伝わり方。スキャン中に `stale` / `loading` として読み側へ伝える導出（`src-tauri/src/usecase/repository_state/worktree.rs:203-210` の `snapshot_for_read`）は変えない。理由: R-003 / B-003 が定める観測可能な挙動であり、集約の対象は setter の実装箇所だけであるため。
- テストが確かめている被覆。`RepositoryHeadDiffFileTreeSnapshotDto` の tree と staged / changes の件数、watcher から通知までの経路（いずれも基準コミットから存在する被覆）と、B-005 の `repository-snapshot-changed` 非送信、B-006 の `limited` 非公開（この周の前に追加した被覆）を、いずれも落とさない。理由: 配置の集約であり、検証内容の変更ではないため。
- design-01.md の「変えないもの」に挙げた対象。理由: 同 Design の判断を今周で解除しないため。

## 未確定・リスク

- 自動判断した箇所は無い。Requirements・Behavior に誤り・不足・矛盾は見つからず、両ファイルを修正していない。`requirements.md` の Assumptions に「自動判断」の記載は無い。
- 未決のまま残した要求は無い。`[DEFERRED]` で人間へ渡した件も無い。
- design-01.md の「未確定・リスク」に挙げた 2 件（R-001 の原因が実機で裏付けられていないこと、R-006 / B-007 の前提が実機で確認されていないこと）は、この周でも未解消のまま継続する。今周の変える部分 2 件は、いずれもその裏付けに依存しない。
