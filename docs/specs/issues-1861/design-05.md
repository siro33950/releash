# Design 05

## 開始状態

- 差分の基準は base ブランチ `main`、派生点は `feat/issues/1861` の `81ec380b`。Design 01〜04 の周の実装は未コミットの作業ツリーにあり、Spec 工程ではコードを変更していないため、この実装をこの周の開始状態とする。
- 直前の Design は `docs/specs/issues-1861/design-04.md`。入力の `context.design_path` は `design-03.md` を指すが、`spec_dir` には `design-04.md` があり、その「変える部分」2件が開始状態で満たされているため、直前の周の Design は `design-04.md` である。
- open Thread は1件（ee6cc9df-2639-4e83-b927-5866cd56a20a）で、`[FIX_POLICY]` が付いている。`[REJECTED]`・`[DEFERRED]` にした Thread はない。前の周までに扱った Thread（dac577a2、ff0fafd5、4359c61e、20465e88、ce4279b6、1bd38ae7、1e36eb0c）は開始時点で open ではない。
- Requirements・Behavior はこの周で変更していない。R-001〜R-016 と B-001〜B-023 に欠番・重複がなく、対応表が全 Requirement ID を記載し各行に Behavior ID が対応していることを確認した。Assumptions / Open Questions は「なし」である。
- Design 04 の「変える部分」2件は開始状態で満たされている。削除後の再読込は Repository 単位の経路を通る（`src/components/workspace/WorkspaceList.tsx:1628`・`:1775`、`src/hooks/useWorkspaceList.ts:59-62`、`src-tauri/src/adaptor/controller/client/workspace_tree_shared.rs:26-31`、`src-tauri/src/usecase/workspace_tree/list.rs:99-105`）。局所操作の通知は対象 Worktree を保って再読込する（`src/hooks/useWorkspaceList.ts:74-84`）。
- Design 04 の「未確定・リスク」1件は開始状態で解消している。Repository 単位の再読込は `begin_repository` が対象 Repository の branches と配下 nodes の generation だけを進め、`complete_branches` は entry 自身の generation と照合して受理する（`src-tauri/src/domain/workspace_tree/refresh.rs:101-112`、`:130-142`）。

## 変える部分

- `useWorkspaceList` の返り値の参照の安定化: `snapshot`・`error`・`refresh`・`refreshWorktree`・`refreshRepository` のいずれも変わらない render で、`useWorkspaceList` が返す `WorkspaceListModel` の参照を維持する。開始状態では `src/hooks/useWorkspaceList.ts:131` が毎 render 新しい object literal を返し、`src/App.tsx:288-310` の `leftNav` の `useMemo` はその object を依存値の先頭に置き（`:302`）、`src/components/workspace/WorkspaceList.tsx:1727` は同じ参照を `WorkspaceListContext.Provider` の `value` に渡し、`src/hooks/useWorkspaceTreeNodes.ts:81` がそれを `useContext` で読む。根拠: Thread ee6cc9df-2639-4e83-b927-5866cd56a20a（blocking、`[FIX_POLICY]`。要旨は、派生点 `81ec380b` の `App.tsx:286-308` では依存値が `useRepoList` の `useState` 配列 `repoPaths` そのもので、値が変わらない render では参照が維持されていたという regression の指摘。受入条件は、Workspace 一覧と無関係な `WorkbenchApp` の state 変更で `leftNav` の `useMemo` が再計算されず `WorkspaceListContext` の value も変わらないこと）。ルート: 委任

## 固定するルート

この周で新しく固定する実装上の指定はない。Design 01 で固定した次の4つを維持する。

- 全体の更新手順と結果の判定を Rust 側に置き、既存の `RepositoryStateService` の走査処理と Workspace の一覧取得処理を再利用する。Frontend は受け取った一覧・更新状態・エラーを表示するだけにする。（Design 01 固定ルート①）
- 一覧を保持する変更と、Session・Workflow の再取得経路の変更を併せて行う。（Design 01 固定ルート②）
- 更新対象は Workspaces の一覧情報に限り、表示中の Session や実行中の Workflow の継続を保つ。（Design 01 固定ルート③）
- 手動更新と自動更新で内部の経路を分けない。（Design 01 固定ルート④）

## 変えないもの

- R-001〜R-016 と B-001〜B-023 の受入条件。返り値の参照を安定させても、一覧の保持、進行表示と重複抑止、失敗表示、新旧判定の観測結果は変わらない。理由: 人間が Thread ee6cc9df の受入条件にそう定めたため。

## 未確定・リスク

- この周で自動判断した箇所はない。Requirements と Behavior の対応に誤り・不足・矛盾は見つからず、両文書を変更していない。未決のまま残した要求はなく、`[DEFERRED]` で人間へ渡した件もない。
