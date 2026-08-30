# Context

- Primary source は GitHub Issue #1721「[workspace tree] sequence 行のアイコンが Node 種別を表しておらず fanout と対比しない」（https://github.com/siro33950/releash/issues/1721 、state: OPEN、label: enhancement、comment なし）である。
- 追加資料は GitHub Issue #1683（https://github.com/siro33950/releash/issues/1683 ）、`docs/specs/issues-1683/requirements.md`、`docs/specs/issues-1683/behavior.md`、`docs/specs/issues-1683/design.md`、`src/components/workspace/WorkspaceList.tsx`、`src/components/workspace/FanoutRowStatusIcon.tsx`、`src/types/workspace-tree.ts` である。
- Sequence は children を時系列に束ね、辺を所有する合成 Node、Fanout は children を並列に束ねる合成 Node である（`docs/glossary/DOMAIN.md`、`docs/glossary/WORKFLOW.md`）。
- Issue #1683 とその Spec により、Workspace ツリー行の色は Node 種別ではなく状態分類を表す。Issue #1683 の変更ではアイコン形状が Non-goal とされ、commit `59bb632fa` で状態色と pulse の規則が現行実装へ反映された。
- Sequence 行の `ListTree` は、Sequence が公開 DTO に追加された commit `efc76d0ce` で導入された。Issue #1721 に対応済みの commit および `docs/specs/issues-1721` の既存文書は確認できなかった。
- 現在状態の確認は Issue、既存 Spec、用語集、現行コード、既存テスト、Git 履歴の読解によって行った。アプリを起動しての画面確認は行っていない。

# Outcome

Releash の Workspace ツリーで workflow の実行木を確認する開発者が対象である。

現在、Sequence 行の `ListTree` はツリーそのものと同じ形を重ねて表示し、枝分かれする形にも見える。そのため、14px のアイコン形状だけでは、children を時系列に束ねる Sequence と、children を並列に束ねる Fanout を判別しにくい。色は Node の状態分類を表すため、Node 種別の判別には使えない。

変更後は、Sequence 行が複数の点を分岐なしの経路で結ぶ `Waypoints` で表示される。Fanout 行の `GitFork` との形状差により、開発者は状態色に頼らず Sequence と Fanout を判別できる。

# Current Behavior

`src/types/workspace-tree.ts` は、Workspace ツリーの合成 Node を `kind: "sequence"` と `kind: "fanout"` に分けている。トップレベルおよび入れ子の Sequence は、どちらも同じ Sequence 行として表示される。

`src/components/workspace/WorkspaceList.tsx:403-406` は、Fanout 行を `FanoutRowStatusIcon`、それ以外の合成 Node である Sequence 行を `WorkspaceBranchStatusIcon` で描画する。Sequence 行には `ListTree` が渡されている。`src/components/workspace/FanoutRowStatusIcon.tsx:12` は Fanout 行に `GitFork` を渡す。

両方のアイコンは `WorkspaceBranchStatusIcon` により `size-3.5`（14px）で表示され、同じ状態分類に基づく色と pulse を受ける。既存の `src/components/workspace/WorkspaceList.test.tsx` も、Sequence 行に `lucide-list-tree`、Fanout 行に `lucide-git-fork` が表示されることを確認している。

最小の再現手順と実際の出力は次のとおりである。

1. Sequence と Fanout を含む workflow の Workspace ツリーを表示する。
2. Sequence 行と Fanout 行の左端にある Node 種別アイコンを比較する。
3. Sequence 行には枝分かれしたツリー形状の `ListTree`、Fanout 行には分岐形状の `GitFork` が、いずれも14pxかつ状態分類に応じた同じ色規則で表示される。

# Scope / Non-goals

## Scope

- Workspace ツリーに表示されるすべての Sequence 行について、Node 種別を表すアイコン形状を `ListTree` から `Waypoints` へ変更する。
- トップレベルと入れ子のどちらの Sequence 行にも同じ変更を適用する。
- Sequence 行の既存の表示サイズ、状態色、pulse の規則を維持する。

## Non-goals

- Fanout 行の `GitFork` の変更。
- Workspace ツリーの状態分類、色、pulse の規則の変更。
- Session / Command の leaf 行アイコンの変更。
- ツリーの階層、レイアウト、展開・折り畳み操作の変更。
- Sequence / Fanout の workflow 上の意味、実行規則、backend の型および外部インターフェースの変更。
- Sequence の代替アイコン候補を再選定すること。

# Requirements

- R-001: Workspace ツリーのトップレベルおよび入れ子のすべての Sequence 行は、Node 種別アイコンとして `Waypoints` を表示する。
- R-002: Sequence 行の `Waypoints` は14pxで表示され、既存の状態分類に基づく色と pulse の規則をそのまま反映する。
- R-003: Fanout 行は、Node 種別アイコンとして既存の `GitFork` を引き続き表示する。

# Assumptions / Open Questions

なし。
