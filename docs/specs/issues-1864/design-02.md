# Design 02

## 開始状態

- 差分の基準は branch `feat/issues/1864` の `f3e55b13`（`origin/main` と同一）に、design-01 の実装を加えた未コミットの作業ツリーである。変更済みは次の3ファイル
  - `src-tauri/src/adaptor/gateway/workflow/worktree_context.rs`: 祖先の Node が Sequence / Fanout でないことを理由に `Corrupt("worktree ancestor is not a composite")` を返す判定を削除済み
  - `src-tauri/src/adaptor/gateway/workflow/worktree_context_test.rs`: 祖先 Node の kind を Session に差し替えて `Corrupt` を期待する `leaf-parent` ケースを削除済み
  - `src-tauri/src/adaptor/gateway/workflow/workflow_host/delegate_test.rs`: delegate 構造の回帰テストを追加済み（`delegate_test.rs:51-303`）
- 直前の Design は `docs/specs/issues-1864/design-01.md`
- この周までに解消・見送りとなった Thread はない。`[REJECTED]` または `[DEFERRED]` で resolve した Thread もない

## 変える部分

- delegate 構造の回帰シナリオの配置: `src-tauri/src/adaptor/gateway/workflow/workflow_host/delegate_test.rs:51-303` に置かれている、delegate 配下の Session 開始・終了通知の受理から Fanout 後続 Node の開始までを検証する回帰シナリオを、`docs/architecture/TEST.md:51-57`（「統合テスト `src-tauri/tests/` に配置する。- 複数レイヤーをまたぐシナリオ」）が定める統合テストの配置で扱う。移設後も CI の Rust テスト一式（`cargo test --locked`）で実行されること。根拠: Thread `c3bb7b1a-66c5-4e9e-916f-264b7747d68c`（要旨: 当該テストは `WorkflowRuntimeUsecase`・`ProviderLifecycleIngressUsecase`・`ProviderLifecycleUsecase`・`AgentSessionUsecase` と各領域の実 gateway を同時に組み立て、provider signal の受信から workflow の後続 Node 開始まで横断しているにもかかわらず、workflow gateway の実装併置テストに置かれている）。ルート: 委任

## 固定するルート

- design-01「固定するルート」で固定した回帰テストの対象構造と確認項目を今周も維持する。範囲: `impl`（Sequence）→ `implement`（Session）→ `completion.delegate` → `inner_check`（Sequence）→ `inner_review`（Fanout）→ `reviewer`（Session）という delegate 構造で、作業場所の解決と Session 開始・終了通知の受理を確認する。粒度: 検証対象の構造と確認項目まで
- 今周に新しく固定する実装上の指定はない。移設先のファイル名、テスト名、fixture の組み立て方、acceptance host の公開方法、モック方針は委任

## 変えないもの

- 今周の修正範囲は回帰シナリオの配置の扱いだけにする。design-01 で実装済みの読み側 `execution_worktree_path` の受理条件の変更と、`worktree_context_test.rs` の期待の変更は変更しない。理由: Thread の指摘が配置に限られ、Requirements・Behavior の判断を変えないため

## 未確定・リスク

- R-003 / R-004 を満たすことの確認は、実 provider プロセスを通した再実行では未確認のままである。追加済みの回帰シナリオは fact log と後続 Node の開始まで確認するが、Issue 調査時点から修正後の実行は行われていない。想定が外れた場合、読み側を直しても Fanout が完了待ちのまま残る
- この周に自動判断した箇所はない。`requirements.md` の Assumptions は「なし」であり、未決のまま残した要求もない
- `[DEFERRED]` で人間へ渡した件はない
