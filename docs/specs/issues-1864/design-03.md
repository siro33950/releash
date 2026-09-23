# Design 03

## 開始状態

- 差分の基準は branch `feat/issues/1864` の `f3e55b13`（`origin/main` と同一）に、design-01 と design-02 の実装を加えた未コミットの作業ツリーである。反映済みは次の通り
  - `src-tauri/src/adaptor/gateway/workflow/worktree_context.rs`: 祖先の Node が Sequence / Fanout でないことを理由に `Corrupt("worktree ancestor is not a composite")` を返す判定を削除済み
  - `src-tauri/src/adaptor/gateway/workflow/worktree_context_test.rs`: 祖先 Node の kind を Session に差し替えて `Corrupt` を期待する `leaf-parent` ケースを削除済み
  - `src-tauri/src/workflow_delegate_acceptance.rs`（新規）、`src-tauri/tests/workflow_delegate_acceptance_test.rs`（新規）、`src-tauri/src/lib.rs`、`src-tauri/src/adaptor/gateway/workflow/workflow_host.rs`: delegate 構造の回帰シナリオを統合テストとして配置済み
  - `src-tauri/src/adaptor/gateway/workflow/workflow_host/delegate_test.rs`: design-02 の移設により `f3e55b13` と同一の内容に戻っている
- 直前の Design は `docs/specs/issues-1864/design-02.md`
- この周までに解消・見送りとなった Thread は2件で、いずれも resolve 済みである
  - `3446a764-347a-4f68-a5fd-d26770cea48c`（読み側 `execution_worktree_path` の祖先受理条件）: `[REJECTED]`。人間の決定により退けた
  - `c3bb7b1a-66c5-4e9e-916f-264b7747d68c`（delegate 回帰シナリオの配置）: `[FIX_POLICY]`。design-02 の移設で対応済み
- open Thread は0件である

## 変える部分

なし。Requirements（R-001〜R-006）と Behavior（B-001〜B-006）に今周の追加・削除・統合・緩和はなく、`[FIX_POLICY]` が付いた open Thread もない。design-01 と design-02 が決めた変更はいずれも開始状態で反映済みである。

## 固定するルート

- design-01 で固定した回帰テストの対象構造と確認項目を今周も維持する。範囲: `impl`（Sequence）→ `implement`（Session）→ `completion.delegate` → `inner_check`（Sequence）→ `inner_review`（Fanout）→ `reviewer`（Session）という delegate 構造で、作業場所の解決と Session 開始・終了通知の受理を確認する。粒度: 検証対象の構造と確認項目まで
- design-02 で固定した、この回帰シナリオを `docs/architecture/TEST.md` が定める統合テストの配置（`src-tauri/tests/`）で扱うことを今周も維持する
- 今周に新しく固定する実装上の指定はない

## 変えないもの

- 読み側 `execution_worktree_path` が読み取りエラーとして拒否し続ける対象は、循環した祖先関係と別の実行木の Node を含む祖先関係だけにする。祖先の Node 種別や親子関係の種別による拒否を読み側に持たせない。design-01「変えないもの」を維持する。理由: 実行側 `ExecutionTree::execution_worktree_path`（`src-tauri/src/domain/workflow/entities/workflow_execution/worktree.rs:4-25`）は祖先の Node 種別も親子関係の種別も判定しておらず、読み側だけ厳しくすると同じ事実に対して起動は成功・読み取りは失敗という食い違いが別条件で残り、R-002 と齟齬が出るため
- 実行側（domain、`workflow_host`、`control_plane`）の作業場所の導出は変更しない。design-01「変えないもの」を維持する
- 既に終了が記録されず停止している実行木を先へ進めるための専用の復旧経路は作らない。design-01「変えないもの」を維持する

## 未確定・リスク

- R-003 / R-004 を満たすことの確認は、実 provider プロセスを通した再実行では未確認のままである。追加済みの回帰シナリオは fact log と Fanout 後続 Node の開始まで確認するが、Issue 調査時点から実 provider を通した再実行は行われていない。想定が外れた場合、読み側を直しても Fanout が完了待ちのまま残る
- この周までに自動判断した箇所はない。`requirements.md` の Assumptions / Open Questions は「なし」であり、未決のまま残した要求もない
- `[DEFERRED]` で人間へ渡した件はない
