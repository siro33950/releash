# Requirements

## Type

性能・メモリ効率改善。マイルストーン「性能・メモリ効率改善（Workbench State / Read Model）」の M3「Runtime lifecycle を締める」の一部として、Terminal / PTY のライフサイクルに cap・eviction・idle timeout を導入する。

関連: #1215 / #1196（CLOSED, workflow runtime 解放）/ #858（ワークスペース切替のコンポーネント保持方式）/ `docs/releash-performance-architecture-audit.md` M3（正本ドキュメント）

## 背景と目的

`docs/releash-performance-architecture-audit.md` の M3 で指摘された runtime lifecycle 課題のうち、Terminal / PTY 領域を対象とする。現状の調査で以下の無制限増大・二重保持が確認されている。

- **frontend xterm の常時マウント**: `TerminalTabPanel.tsx`（L298）が inactive tab を `forceMount` し、`data-[state=inactive]:hidden` で隠すのみ。inactive な分割ペイン（`PaneLeafContainer.tsx` L186）も含め、xterm インスタンスが DOM 上に残り続ける。MAX_TABS=8 × MAX_PANES_PER_TAB=4（`useTerminalPanes.ts` L17-18）の上限まで mounted xterm が積み上がる。
- **frontend cache の eviction 不在**: `useTerminal.ts` の `sessionKeyCache`（L31, `Map<cwd, sessionKey>`）に eviction 機構がなく、`tabStateCache`（`useTerminalPanes.ts` L45）にも明示的廃棄がない。
- **PTY output buffer の二重保持**: Rust runtime 側（`backend_impl.rs` の `PtyRuntime.output_buffer`、`OUTPUT_BUFFER_CAPACITY = 64KB` リングバッファ）と WS bridge 側（`ws_bridge.rs` の `pty_output_buffers`、`PTY_OUTPUT_BUFFER_SIZE = 64KB` リングバッファ）が PTY ごとに独立して同一出力を保持している（合計 ~128KB/PTY）。
- **PTY registry の cap 不在**: `PtySessionRegistry`（`pty_session_registry.rs`）は `HashMap<u64, PtySession>` で、max panes / per-worktree cap を持たない。GC は `gc_ptys_for_worktree`（keep_session_keys ベースの手動呼び出し）のみで、自動 eviction がない。
- **idle timeout の不在**: alive な PTY への idle timeout はなく、プロセス終了後 5 分の delayed cleanup（`backend_impl.rs` L118-128）のみ存在する。
- **remote subscriber 不在時の buffer 継続**: WS bridge の per-PTY buffer は subscriber がいなくても 64KB まで蓄積し続け、`remove_pty_output_buffer` の明示呼び出しでのみ解放される。

本変更の目的は、Terminal / PTY のメモリ常駐量とピークを境界づけ、active terminal の UX を壊さずに inactive terminal を軽量状態へ退避・復元できるようにすること。

## スコープ

ロジックは Rust に集約し、frontend は表示・入力受付・invoke 呼び出しに徹する（`.claude/rules/rust-first-logic.md` 準拠）。cap / eviction / idle timeout の enforcement は Rust 側で行い、frontend は Rust が公開する状態を表示・復元する。

### 1. frontend xterm の remount（mounted 数の境界づけ）

- active tab / active pane 以外の xterm は DOM から unmount し、必要時（再アクティブ化時）に remount する。
- mounted xterm 数が hidden terminal 数に比例して増え続けないことを保証する。
- unmount された terminal は、再表示時に Rust が保持する lightweight state（後述）から見た目とスクロールバックを復元する。

### 2. terminal pane state の lifecycle policy

- terminal pane state に LRU / per-worktree cap / idle timeout を導入する。
- inactive pane state を軽量表現へ退避し、cap 超過・idle 超過したものを eviction する。
- lifecycle policy（cap 値・idle 時間・LRU の単位）は Rust 側で enforce する。

### 3. PTY lifecycle の Rust 側 enforcement

- max panes（全体）、per-worktree cap、output buffer cap を Rust 側で enforce する。
- alive PTY に対する idle timeout を導入し、idle 超過 PTY を解放対象にする。
- PTY output buffer の owner を一意に定め、Rust runtime と WS bridge の二重保持を解消する。

### 4. remote buffer / broadcast の最小化

- WS remote subscriber がいないときは、remote 用の PTY output buffer 蓄積 / broadcast を最小化する。
- subscriber 接続時に必要なスクロールバックを供給できる前提を維持する。

## 非スコープ

- workflow runtime / WorkflowExecution の解放（#1196 で対応済み、CLOSED）。
- agent process（agent チャット bridge）の idle timeout / cap。M3 では言及されるが本 Issue は Terminal / PTY に限定する。
- `cleanup_orphan_processes` を startup blocking path から外す対応（M3 項目 4。別 Issue）。
- ワークスペース切替方式そのものの変更（#858）。本 Issue は terminal/PTY のライフサイクルに限定し、#858 の display:none 方式採用可否には踏み込まない。
- terminal の機能追加（新シェル統合、新ペイン操作 UI 等）。
- agent チャット streaming / session storage の計測・最適化（#1209 / #1213 / #1195 の領域）。

## 要求事項

R1. inactive な terminal tab / pane の xterm は unmount され、mounted xterm 数が hidden terminal 数に比例して増え続けない。

R2. unmount された terminal を再アクティブ化したとき、active terminal と同等の UX で（直近スクロールバック・サイズ・カーソル状態を含めて）復元できる。復元元は Rust が保持する lightweight state とする。

R3. terminal pane state に LRU / per-worktree cap / idle timeout を導入し、cap・idle を超えた pane state を eviction する。cap 到達時は最も古い idle terminal を LRU で自動 eviction して新規を許可し、active terminal は eviction 対象から除外する。これらのポリシーは Rust 側で enforce する。

R4. PTY に対して max panes（全体）、per-worktree cap、output buffer cap を Rust 側で enforce する。

R5. alive PTY に idle timeout を導入し、idle 超過した PTY を解放できる。

R6. PTY output buffer の正本 owner を一意に定め、Rust runtime と WS bridge の二重保持を解消する。buffer の所在と責務がコード上明確である。

R7. WS remote subscriber がいないとき、remote 用 buffer 蓄積 / broadcast を最小化する。subscriber 接続時には必要なスクロールバックを供給できる。

R8. active terminal の入出力・操作 UX を壊さない（cap / eviction / idle timeout 導入によって active terminal の応答性・履歴が劣化しない）。

## 受け入れ基準の概要

- AC1: hidden terminal を増やしても mounted xterm 数が比例して増え続けない（R1）。
- AC2: tab / pane を切り替えて戻ったとき、terminal の表示内容（直近出力・サイズ）が失われず復元される（R2, R8）。
- AC3: PTY output buffer の owner が単一で、Rust runtime と WS bridge の二重保持が解消されている（R6）。
- AC4: per-worktree cap / max panes / idle timeout に到達したとき、Rust 側のポリシーに従って PTY と pane state が解放される（R3, R4, R5）。
- AC5: remote subscriber が不在のとき remote 用 buffer / broadcast が最小化され、subscriber 接続時には必要なスクロールバックが供給される（R7）。
- AC6: active terminal の UX（入出力・履歴・サイズ追随）が劣化しない（R8）。

## 仮定

- A1. spec ディレクトリは `docs/specs/issues-1215` とする（直近 Issue の命名慣例 `issues-NNN` に従う）。
- A2.（合意済み）cap / idle timeout の具体値は design.md でチューニング可能なパラメータとして定める。初期値は暫定とし、per-worktree cap = 現行 MAX_PANES_PER_TAB×MAX_TABS 相当、alive PTY idle timeout = 5 分（既存 delayed cleanup を参考）、output buffer cap = 64KB を採用する。
- A3.（合意済み）PTY output buffer の正本 owner は Rust runtime 側（`PtyRuntime.output_buffer`）に一本化する。WS bridge は独立バッファ（`pty_output_buffers`）を廃止し、subscriber 接続時に runtime buffer から取得・転送する。
- A4. lightweight state（再 mount 時の復元元）は Rust が保持する PTY output buffer（リングバッファ）＋ pane のサイズ/メタ情報を基本とし、frontend は復元時にこれを invoke で取得して xterm に書き戻す。
- A5. 既存上限定数（frontend の MAX_TABS=8 / MAX_PANES_PER_TAB=4）は維持しつつ、cap enforcement の正本を Rust 側へ移す。
- A6.（合意済み）cap 到達時は LRU で最も古い idle terminal を自動 eviction（解放/kill）して新規を許可する。active terminal は eviction 対象から除外する。
- A7.（合意済み）inactive terminal の再 mount 時の復元範囲は、output buffer cap（64KB ring）相当の直近スクロールバックまでとする。フルスクロールバックの保持・復元は要求しない。

## Open Questions

なし（すべて解消済み）。
