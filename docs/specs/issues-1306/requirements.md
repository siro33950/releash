# Requirements

## Type

リファクタリング / 責務境界の整理（terminal pane の state ownership を UI layout と PTY lifecycle に分離する。外部から観測可能な terminal の振る舞いは原則不変）。

関連: #1306 / milestone [12] クリーンアーキテクチャ移行 / Blocks: #878（final dead-code sweep）

## 背景と目的

terminal pane tree は、純粋な UI layout state（frontend が所有してよいもの）と、PTY lifecycle / session reconciliation（Rust が source of truth を所有すべきもの）が、`src/hooks/useTerminalPanes.ts` という単一 hook の中で混在している。

具体的には、`useTerminalPanes` は次の 2 種類の責務を同居させている。

1. **UI layout 操作（frontend が所有してよい）**
   - tab / split pane の layout tree（`tabs` / `paneTree`）
   - focused pane id（`focusedPaneId`）
   - add/close tab、split/close/move pane、focus 移動など純粋な UI 操作
   - UI 上の pane label（`${tabPrefix} N`）
2. **PTY session の正典判定・reconciliation（本来 Rust が所有すべき）**
   - `list_pty_sessions`（`invoke`）の結果を frontend が独自に reconcile し、live でない session_key を持つ pane を「stale」と判定して layout から除去する（`removeStalePanes` / `removePaneBySessionKeyFromState`、行 211-241 / 667-795）。
   - `pty-evicted` event を受けて、表示中 state と `tabStateCache`（module 全体で共有する layout + session cache）の両方から該当 pane を独自 reconcile で除去する（行 174-209、`ensurePtyEvictionMirror` / `removeEvictedSessionFromTabStateCache`、行 627-665）。
   - pane layout cache（`tabStateCache`）と PTY lifecycle（session 生存状態）が同じ cache 構造に同居している。

backend（`src-tauri/src/adaptor/gateway/pty_session/`、`usecase/pty_session/`）は既に PTY session の source of truth を所有しており、`list_pty_sessions`（query_service::list）と `pty-evicted` event（emit_evicted）でその状態を read model / event として公開している。問題は、frontend がそれを「反映する」のではなく、live 判定・stale 判定・eviction 後の整合という **PTY session lifecycle の正典判定を frontend 側で再計算している** ことにある。

本変更の目的は、CLAUDE.md の方針「Rust がロジックを所有する」「状態の所有者を明確にする」「full-retention / full-recompute を避ける」に従い、terminal pane tree について次の境界を確立することである。

- `paneTree.ts` を純粋な UI layout helper として維持する（PTY lifecycle 判定を持ち込まない）。
- `useTerminalPanes` から PTY session の正典判定を外し、backend-owned な session availability read model / event を **反映するだけ** にする。
- pane layout の cache と PTY lifecycle（session 生存）の cache を分離する。
- これにより #878（final dead-code sweep）が前提とする責務境界を満たす。

### 現状の責務混在（コード調査による事実）

- `src/lib/paneTree.ts`（348 行）— `findNode` / `splitPane` / `closePane` / `getAllLeaves` / `getAdjacentPane` 等の **純粋な layout helper のみ**。PTY 判定は含まれていない（=現状すでに pure。本変更では「保つ」ことが要求）。
- `src/types/terminal-pane.ts` — `PaneLeaf` が layout 用 field（`id` / `label`）と PTY binding 用 field（`ptyId` / `sessionKey` / `pendingKill`）を同一型に混載している。
- `src/hooks/useTerminalPanes.ts`（867 行）— layout 操作と PTY reconciliation の両方を実装。後者が以下:
  - `list_pty_sessions` 結果での stale pane 除去（行 211-241、`removeStalePanes` 行 667-702）。
  - `pty-evicted` での pane 除去（表示 state: 行 174-204 / cache mirror: 行 206-209・627-665）。
  - `killPanePty` / `killPaneTreePtys`（`kill_pty` invoke、行 599-610）— pane close 時の PTY 終了副作用。
- `src/hooks/useTerminal.ts`（659 行）— `session_key` / `pty_id` の連携（`get_or_spawn_pty` 等）を担う。`useTerminalPanes` の `updatePaneSessionKey` 経由で pane に PTY binding を書き戻す。

## スコープ

- `src/lib/paneTree.ts` を pure UI layout helper として **維持**する（PTY lifecycle 判定を入れない。劣化させない）。
- `src/hooks/useTerminalPanes.ts` から、PTY session の **正典判定 / reconciliation を除去**する。
  - `list_pty_sessions` 結果を frontend が独自に reconcile して stale pane を算出する経路（`removeStalePanes` 等）を、backend-owned な session availability read model の **反映** に置き換える。
  - `pty-evicted` event の扱いを「backend が決定した eviction を pane layout へ反映する」だけにし、frontend 独自の整合計算（cache mirror 含む）を解消する。
- pane layout cache（`tabStateCache`）と PTY lifecycle（session 生存状態）の cache / state を **分離**する。layout cache が PTY 生存判定で書き換えられない構造にする。
- 「どの session が利用可能か（session availability）」「eviction / stale session reconciliation」を Rust 側の read model / usecase が source of truth として持ち、その reconciliation を Rust の test で検証する。
- frontend の責務を「layout 操作」と「backend からの session availability 反映」に限定し、その境界を frontend test（layout interaction + backend event reflection）で検証する。

## 非スコープ

- terminal UI redesign（見た目・操作系の刷新）。
- shell / PTY backend の新機能追加。
- WebSocket migration（#1131）。本変更は現行の Tauri `invoke` / `listen` 経路の上で責務境界を整理する。
- PTY session の保存形式・registry 構造・eviction ポリシー（idle timeout、cap、gc 条件）自体の変更。本変更は既存の backend-owned state を「frontend が再計算しない」形にすることに閉じる。
- terminal の出力バッファリング / streaming（`get_pty_buffered_output` 等）の挙動変更。
- pane label の命名規則・表示フォーマットの変更。
- `useTerminal.ts` の PTY spawn / IO ロジックそのものの再設計（pane binding の受け渡し境界が変わる範囲を除く）。

## 要求事項

1. `paneTree.ts` は pure UI layout helper として維持され、domain / PTY lifecycle decision を持たないこと。
2. `useTerminalPanes` が backend-owned な PTY session state（どの session が live か、どれが evict されたか）を **再計算しない**こと。frontend は backend が提供する session availability read model / event を反映するのみとする。
3. `list_pty_sessions` の結果と `pty-evicted` event を、frontend が独自に reconcile（stale 判定・cache mirror）しない形にすること。session の live/stale/evicted の判定は Rust 側を source of truth とし、frontend はその結果を layout に適用するだけとする。
4. pane layout cache と PTY lifecycle cache が分離され、layout cache（tab/pane 構成・focus）が PTY 生存判定によって暗黙に書き換えられないこと。
5. Rust 側に、PTY session availability / eviction reconciliation を検証する test が存在すること。
6. frontend test が、(a) tab/pane layout interaction（add/close/split/move/focus）と、(b) backend event（session 消失 / eviction）に対する pane layout への反映、の両方を検証していること。
7. 外部から観測可能な terminal の振る舞いを壊さないこと。具体的には:
   - tab/pane の追加・分割・移動・クローズ・focus 移動の挙動と上限（`MAX_TABS=8` / `MAX_PANES_PER_TAB=4`）。
   - PTY が evict / 消失した pane が、最終的に layout から適切に除去（または最後の 1 pane の場合はクリア）される結果整合。
   - pane close 時に対応する PTY が終了される副作用（`kill_pty`）。
   - 表示中の tab/pane state と、tab を切り替えた際に復元される cache state（`tabStateCache`）の整合。
8. `pnpm lint` / `pnpm test` / `pnpm build` / `cargo fmt --check` / `cargo clippy -- -D warnings` / `cargo test` が通ること。

## 受け入れ基準の概要

- `paneTree.ts` に PTY lifecycle / session 判定ロジックが存在しないことを、コードレビューと既存 `paneTree.test.ts` で確認できる。
- `useTerminalPanes` が `list_pty_sessions` 結果や `pty-evicted` event から「どの pane を消すか」を独自に算出する経路を持たず、backend-owned read model / event の反映に限定されていることを確認できる。
- pane layout cache が PTY 生存判定で直接書き換えられない（layout cache と lifecycle cache が分離されている）ことを確認できる。
- Rust 側に PTY session availability / eviction reconciliation の test が追加・維持されている。
- frontend test が layout interaction と backend event reflection の両方をカバーしている。
- 既存の外部観測可能な振る舞い（tab/pane 操作・上限、evict 後の pane 整合、close 時の PTY 終了、cache 復元）が、既存・追加テストで維持される。
- 上記すべての lint / test / build コマンドが通る。

## 仮定

- A1. `paneTree.ts` は現状すでに pure UI layout helper であるため、本変更での主作業対象は `useTerminalPanes.ts`（および必要に応じて `terminal-pane.ts` の型整理、`useTerminal.ts` との binding 受け渡し境界）である。
- A2. backend は既に PTY session の source of truth を所有し、`list_pty_sessions`（query_service::list）と `pty-evicted` event（emit_evicted）で session availability を公開している。本変更は原則この既存 IPC / event 面を再利用し、frontend 側の独自 reconciliation を撤去する方向で実現する。新たな backend command / event 形を追加するか否か、追加する場合の read model 形状は design.md で決定する。
- A3. pane が自分に紐づく `sessionKey` / `ptyId` を「参照」として保持すること自体は layout-adjacent な binding として許容する。本変更が禁ずるのは、その binding の **live/stale/evicted 判定を frontend が再計算する**ことである。binding 情報を `PaneLeaf` に残すか、別の lifecycle state へ分離するかは design.md で決定する。
- A4. eviction / session 消失時に「stale pane を layout から除去する」という最終的な layout 反映は frontend 側の layout 操作として残る。Rust が source of truth として持つのは「どの session が available / evicted か（reconciliation 判定）」であり、frontend はその判定結果を layout に適用するだけとする。判定（Rust）と適用（frontend）の境界の正確な API 形は design.md で確定する。
- A5. 本変更は terminal の外部観測可能な振る舞いを不変に保つことを優先し、内部の責務再配置に閉じる。挙動差分が避けられない箇所が見つかった場合は behavior.md / design.md で明示して合意する。
- A6. 検証は CI と同じコマンド（`pnpm lint` / `pnpm test` / `pnpm build` / `cargo fmt --check` / `cargo clippy -- -D warnings` / `cargo test`）で行う。

## Open Questions

なし。
