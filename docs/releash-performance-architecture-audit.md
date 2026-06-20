# Releash 全体再設計監査: 速度・メモリ効率・アーキテクチャ

作成日: 2026-06-20

## 目的

Releash を「Agent Chat / Workflow / Review を中心にした高速な Git ワークベンチ」として再定義し、速度・メモリ効率・責務分離・UI の観点から現状との差分を整理する。

この文書では、既存の主機能である Chat と Workflow は維持する。一方で、Source Control、Diff、Terminal、Remote、GitHub/Notion/MCP などの周辺機能は、主機能を支える最小の形へ振る舞いを変えてよい前提で扱う。

## 結論

現在の最大の問題は、個別の機能の遅さではなく、全体に広がっている「全量を読む、全量を保持する、全量を再送する、全量を再計算する」設計である。

- Git status / diff / diff tree / branch summary が別々に git2 を走らせ、frontend でも tree 化・差分基準選択・patch 生成準備を持っている。
- Agent Chat は session JSON、Rust cache、frontend state、streaming parts、legacy content の複数箇所で同じ本文を保持しやすい。
- streaming は 33ms ごとに累積 parts を clone / consolidate / emit するため、長い応答ほど 1 frame あたりの仕事が増える。
- Terminal は非表示 tab も `forceMount` で保持し、frontend cache に eviction がない。PTY 出力 buffer も Rust runtime と WS bridge の両方にある。
- watcher は component ごとに起動され、Git dir watcher の notify callback 内で branch list sync などの重い read model 生成を行う。
- clean architecture への移行は進んでいるが、`bridge_common.rs`、`lib.rs`、Tauri command fallback surface などに巨大な glue / runtime / compatibility logic が残っている。

改善の中心は、新機能を増やすことではない。Rust 側に state service と read model を集約し、frontend は受け取った view model を表示するだけに戻すこと、そして large data を handle / page / delta で扱うことが必要である。

## あるべき姿

### 1. Rust 所有の Workbench State

frontend は「表示、入力、Tauri command 呼び出し、表示用 format」だけを担う。Git 状態、diff 基準、review file view、hunk id、session paging、workflow run state、terminal lifecycle は Rust 側の usecase / service が所有する。

理想の主な read model は次の通り。

- `get_workbench_snapshot(worktree_path)`:
  - active session summaries
  - workflow run summaries
  - review status tree summary
  - terminal lightweight state
  - stale / loading / limited flags
- `get_review_snapshot(worktree_path, base)`:
  - staged / unstaged / branch-base の file tree
  - diff stats
  - conflict / binary / large file flags
  - stable hunk ids
- `get_review_file_view(file_id | path, section, base, viewport?)`:
  - file metadata
  - hunk groups
  - requested line windows
  - large-file fallback
  - tokenization status
- `get_session_page(session_id, cursor, limit)`:
  - summaries and visible message window
  - attachment refs
  - token / run metadata
- `subscribe_session_stream(session_id, message_id, since_seq)`:
  - delta events, not cumulative payloads

### 2. Repository State Service

1 repo / worktree につき watcher と git scan は 1 系統に集約する。

- recursive file watcher と Git dir watcher を service に集約する。
- debounce 後、versioned snapshot を生成する。
- status、diff stats、branch cards、worktree dirty count、diff tree を同一 cache から返す。
- ignored files は default で返さない。必要な UI だけ opt-in する。
- stats と patch は lazy。file tree 表示に必要な summary と、file open 時の詳細 diff を分ける。
- scan 中に次の変更が来たら古い scan を cancel / supersede する。

### 3. Diff / File Content Pipeline

diff は frontend に full original / modified string を渡してから組み立てるのではなく、Rust が review file view と hunk operation を所有する。

- working tree 読み込みも Tauri FS plugin ではなく Rust command 経由にする。
- text は file size / line count / hunk count の threshold を持つ。
- image は base64 data URL ではなく、temp/resource URL または content-addressed blob ref で渡す。
- stage / unstage は `hunk_id` / `group_id` を渡し、Rust が patch を再構築する。
- Shiki tokenization は visible range または cached result のみ。large file は plain diff / delayed highlight に落とす。
- diff tree は virtualize し、folder expand state を tree rebuild で毎回リセットしない。

### 4. Agent Session Storage / Streaming

Agent Chat は append-only / paged storage を基準にする。

- session list は summary index だけを読む。
- session 本文は message page 単位で読む。
- message part は content chunk / attachment ref / tool event として保存し、legacy `content` / `thinking` / `activities` との二重保持をなくす。
- image / binary payload は JSON に base64 inline しない。
- streaming は seq 付き delta を送る。再接続時だけ snapshot を送る。
- Rust runtime の `streaming_parts` は turn 完了時に必ず解放する。
- frontend は visible messages だけを保持し、閉じた session / 非表示 worktree の message body を退避する。

### 5. Runtime Lifecycle

非表示・完了・古い runtime は明示的に解放する。

- Workflow step runtime は完了時に release する。
- Terminal は active tab / active pane を優先し、非表示 tab の xterm は必要時に remount する。
- PTY は idle timeout、max panes、per-worktree cap、output buffer cap を Rust 側で enforce する。
- startup の orphan process cleanup は UI startup を block しない。ただし newly spawned process を誤 kill しない ordering を Rust service で保証する。
- Remote は desktop parity を前提にせず、Chat / Workflow / Review の remote control に絞るか、native 化方針へ寄せる。

### 6. UI 方針

最初の画面は「作業中の AI 変更を把握し、必要なら指示し、review して取り込む」ための workbench とする。

- center: Agent Chat / Workflow の実行状況
- side: Review snapshot / changed files / comments
- bottom: active terminal のみ
- secondary: GitHub/Notion/MCP/settings/remote admin は lazy route / modal / command palette へ退避

UI は多機能な IDE ではなく、AI agent の作業監督・差分確認・承認を最短にする設計へ寄せる。

## 現状との差分

| 領域 | 現状の根拠 | 問題 | 改善方向 |
| --- | --- | --- | --- |
| frontend の責務 | `src/hooks/useFileDiffContent.ts` が `readTextFile` で working tree を直接読む | diff 基準選択と file IO が frontend にある | `get_review_file_view` に統合し、Rust が original/modified/source を決める |
| binary diff | `src/hooks/useImageDiff.ts` が `readFile`、byte loop、base64、data URL を生成 | 大きい画像で JS heap に複数 copy が載る | Rust 側 blob ref / temp URL を返し、frontend は URL を表示する |
| hunk 操作 | `src/components/panels/useDiffOperations.ts` が操作時に `compute_diff_hunks` と `generate_group_patch` を再実行 | 既に表示済みの hunk を再計算し、full content を再送する | stable `hunk_id` / `group_id` を Rust に渡して stage/unstage |
| diff tree | `src/hooks/useDiffFileTree.ts` が status 後に `get_status_diff_stats` と `build_diff_file_tree` を別 command で呼ぶ | scan / DTO 生成 / tree 化が分散 | Rust の review snapshot command に統合 |
| Git watcher | `src/hooks/useGitEventRefresh.ts` は caller ごとに `start_watching` する | ReviewPanel と useGitStatus などで watcher が重複 | repo state service が watcher を 1 本化し、subscriber へ snapshot 通知 |
| status | `src-tauri/src/adaptor/gateway/repository/status.rs` は `include_ignored(true)` | frontend は ignored を多くの場合捨てるため転送と CPU が無駄 | ignored は opt-in。default status は review に必要な file のみ |
| diff stats | `get_status_diff_stats` は index diff と worktree diff を取り、untracked content も見る | file list 表示のたびに patch/stat 計算が重い | stats は cache / lazy / threshold 化 |
| branch / worktree | `watcher.rs` の Git dir callback が `list_branches_with_status` を呼ぶ | notify callback 内で dirty count など重い処理が走る | event は invalidate だけにし、background worker が snapshot を更新 |
| mention | `src-tauri/src/adaptor/gateway/code/mention.rs` は query ありで `usize::MAX` まで worktree walk | 入力ごとに全 tree scan しうる | indexed file catalog / capped fuzzy search |
| session list | `SessionStore::list_sessions_filtered` は初回に全 session JSON を cache し、summary 化する | session 数と本文量に比例して startup/list が重くなる | summary index と body paging を分離 |
| session get | `SessionStore::get_session` は `ChatSession` 全体 clone | message 本文と attachment が毎回 clone される | page API と ref-counted/chunked body |
| session save | `save_session` は full session を pretty JSON で temp write / rename | streaming 中に巨大 JSON を繰り返し全量保存 | append-only event log + compact、または per-message chunk file |
| session fork | `fork_session` が `messages.clone()` | 長い会話を即時 full copy | copy-on-write / parent ref / selected range fork |
| streaming persist | `persist_streaming_parts` が `parts_to_legacy` と `parts.to_vec()` 後に full save | parts と legacy content が二重化し、1s ごとに全量書く | delta persist、legacy 廃止、完了時 compact |
| streaming emit | `prepare_streaming_flush` が `proc.streaming_parts.clone()` を `consolidate_parts` | 応答が長いほど 33ms tick の clone 量が増える | seq delta emit。snapshot は reconnect 用のみ |
| frontend chat | `useAgentSdkListeners` / reducer は累積 parts を session state に反映 | React state に長大 message array / parts が載る | visible window + message store + delta apply |
| terminal UI | `TerminalTabPanel.tsx` は inactive tab も `forceMount` | 非表示 xterm / pane が残る | active tab 以外は lightweight state に退避 |
| terminal cache | `useTerminalPanes.ts` の `tabStateCache` は eviction なし | worktree を跨いで terminal layout が増え続ける | LRU / per-worktree cap / close on inactivity |
| PTY buffers | Rust PTY runtime と `WsBroadcaster` がそれぞれ 64KB 級 buffer を持つ | terminal 数に比例して buffer が二重化 | owner を決め、remote subscriber 有無で WS buffer を動的化 |
| startup | `src-tauri/src/lib.rs` が orphan cleanup thread を spawn して `join` | startup path が cleanup に block される | init ordering を service 化し async cleanup |
| command surface | `src-tauri/src/adaptor/controller/command/mod.rs` に大きな fallback `generate_handler` が残る | domain command と旧 command が混在 | command を bounded context ごとに縮小し、compat command を削除 |
| god module | `bridge_common.rs` は streaming、process、session IO、permission、tests を含む巨大 module | 変更影響範囲が広く、性能修正の局所性が低い | runtime/process/stream/session_persist/event_emit/recovery に分割 |

## 優先ロードマップ

### M0: 計測と予算を固定する

まず、改善前後を比較できる最低限の数値を入れる。

- startup time、first window ready、first repo snapshot ready
- Git status scan duration、diff stats duration、review file open duration
- Tauri event payload size、WS payload size、streaming emit interval / dropped frame count
- session list duration、session get duration、session save bytes
- Rust RSS、WebView JS heap、xterm count、active PTY count

初期の性能予算案:

- app startup: orphan cleanup による visible startup block なし
- repo snapshot: 中規模 repo で 200ms 台を目標、長い scan は stale snapshot を返す
- file diff open: 小/中ファイル 500ms 未満、大ファイルは即 fallback 表示
- streaming event: 通常 frame payload は 64KB 未満、累積 snapshot は reconnect 時のみ
- session list: session 本文量に依存しない
- terminal: worktree あたり mounted xterm 数に上限

### M1: Git / Diff hot path を Rust read model に寄せる

1. `RepositoryStateService` を追加し、watcher / status / stats / branch / worktree dirty count を集約する。
2. `get_review_snapshot` を追加し、`useGitStatus` + `useDiffFileTree` + `get_status_diff_stats` の分散呼び出しを置き換える。
3. `get_review_file_view` を追加し、`useFileDiffContent` / `useImageDiff` の direct FS 読みを削除する。
4. `stage_hunk_by_id` / `unstage_hunk_by_id` を追加し、frontend から full content と patch generation を外す。
5. ignored / untracked content / rename detection / tokenization に threshold を入れる。

### M2: Agent Chat の memory model を作り直す

1. #1194 の turn 完了時 `streaming_parts` 解放を先に入れる。
2. #1195 の frontend message body 退避 / virtualize を入れる。
3. session summary index を追加し、list API が full session cache に依存しないようにする。
4. `get_session_page` と message paging を追加する。
5. streaming を cumulative parts から seq delta に変える。
6. `parts_to_legacy` を compatibility 出力へ限定し、保存正典から外す。
7. image / attachment は content-addressed file ref にする。

### M3: Runtime lifecycle を締める

1. #1196 と `docs/spec/issues-929.md` の workflow execution / step runtime release を完了する。
2. terminal inactive tab を unmount し、pane state と PTY lifecycle を分離する。
3. PTY / workflow / agent process の idle timeout と cap を Rust 側で enforce する。
4. `cleanup_orphan_processes` を startup blocking path から外す。
5. remote subscriber がないときは remote 用 buffer / broadcast を最小化する。

### M4: Clean Architecture 移行を性能改善と結びつける

1. #72 の既存移行を進めつつ、read model / state service を usecase として明確化する。
2. `bridge_common.rs` を runtime/process/stream/persist/recovery に分割する。
3. `lib.rs` の composition root から startup workflow を module 化する。
4. read command に write side-effect を持たせない。
5. 旧 Tauri command を棚卸しし、frontend から使われないものを #878 と合わせて消す。

### M5: UI を Chat / Workflow / Review 中心に再編する

1. default workbench を Chat / Workflow / Review の 3 領域にする。
2. Source Control は Review snapshot の一部へ統合する。
3. Terminal は active worktree / active task の補助 view とし、常時多数 mount しない。
4. Remote は mobile desktop parity ではなく、Chat / Workflow / Review の監督に絞る。
5. Native UI (#76) は長期選択肢として残すが、Rust read model / delta stream / paging は native 化前にも必須とする。

## 既存 Issue / Milestone との重複・関連

GitHub の open Issue と Milestone を確認した。今回の監査は既存の複数 Issue と重なるが、横断的な性能予算と状態モデルを扱う umbrella がまだ不足している。

### 直接重複する Issue

- [#1191 Codex会話中にメモリ枯渇でアプリがクラッシュする回帰](https://github.com/siro33950/releash/issues/1191)
  - 本文書の M2 / M3 と直接重複する親 Issue。先行対応として `31b4729f` / #1197 で A 群の純粋削減は完了済み。
- [#1194 メモリ削減: ターン完了時に streaming_parts を解放する](https://github.com/siro33950/releash/issues/1194)
  - M2 の最初に実施するべき即効性のある修正。
- [#1195 メモリ削減: フロント agent チャットの全メッセージ保持を退避/仮想化する](https://github.com/siro33950/releash/issues/1195)
  - frontend memory の主要対策。ただし message paging API と組み合わせないと効果が限定的。
- [#1196 メモリ削減: terminal 後の workflow execution を解放する](https://github.com/siro33950/releash/issues/1196)
  - M3 の runtime lifecycle 対策と重複。
- [#1192 Agent: ターン完了後に2通目を送ると無反応](https://github.com/siro33950/releash/issues/1192)
  - bridge process の死活検知と再 spawn は M3 の runtime lifecycle / recovery と合わせて扱う。
- [#1190 Session復帰時に会話コンテキストを失う](https://github.com/siro33950/releash/issues/1190)
  - session storage の正典、runtime state、再開時 context 復元を分ける M2 の設計と関連する。
- [#767 フロントエンドロジックをRustに移行する](https://github.com/siro33950/releash/issues/767)
  - M1 / M2 / M4 の根本方針と重複。今回の監査では「何を Rust に移すか」を Git/diff/session/streaming の read model として具体化した。

### Architecture 系 Milestone / Issue

- Milestone [#72 `[12] クリーンアーキテクチャ移行`](https://github.com/siro33950/releash/milestone/72) は open 15 / closed 21。
  - #1170, #1171, #1172, #1173, #1176, #1130-#1134, #980, #985, #986, #878 などが関連。
  - 本文書の M4 と重なる。ただし #72 は構造移行が中心で、performance budget、state service、全量 payload 削減、frontend memory policy は別 Issue として追加した方がよい。
- `docs/specs/feat-issues-977/design.md`
  - agent_session bounded context、persistent state と runtime state の分離、status derivation の方針が M2 / M4 と一致する。
- `docs/specs/feat-issues-978/design.md`
  - workflow clean architecture 移行が M3 / M4 と一致する。
- [#878 デッドコード削除](https://github.com/siro33950/releash/issues/878)
  - command surface 削減と合わせて実施する。

### Streaming / Workflow local spec

- `docs/spec/issues-970.md`
  - AgentChat streaming の遅延・詰まり対策。33ms coalescing と累積置換 payload は当時の解だが、今回の監査では memory の観点から delta stream へ進める。
- `docs/spec/issues-929.md`
  - workflow step runtime release の仕様。#1196 と合わせて M3 に取り込む。

### UI / Remote / Native 関連

- Milestone [#76 ネイティブ UI 化（Rust コア温存 + SwiftUI）](https://github.com/siro33950/releash/milestone/76) と [#1187](https://github.com/siro33950/releash/issues/1187)
  - WebView / DOM / JS / Shiki / xterm.js の性能限界という認識は本文書と一致する。
  - ただし native 化は戦略的選択肢であり、Rust read model、paging、delta stream、runtime caps は native 化前にも必要。
- Milestone [#52 Mobile Remote Environment](https://github.com/siro33950/releash/milestone/52) は open 13 / closed 0。
  - #866-#877, #1000 が関連。`src/remote/` の維持・削除・native 化方針が未整理。
  - 本文書では mobile remote を desktop parity から切り離し、Chat / Workflow / Review の監督に絞る方針を推奨する。
- Milestone [#50 Diffレビュービュー](https://github.com/siro33950/releash/milestone/50) は open 2 / closed 37。
  - [#788](https://github.com/siro33950/releash/issues/788), [#805](https://github.com/siro33950/releash/issues/805) が残っている。
  - `docs/spec/issues-784.md` の UI 簡素化方針とも関連する。今回の監査では Review を Source Control の上位概念として再定義する。
- [#858 ワークスペース切替をコンポーネント保持方式に変更する](https://github.com/siro33950/releash/issues/858)
  - display:none で保持する方針は UX には効くが、メモリ削減方針と衝突しうる。bounded keep-alive / LRU / active core view 限定が必要。
- [#1189 横断ビュー: 全worktreeのChat/Workflowをタイルグリッドで同時表示](https://github.com/siro33950/releash/issues/1189)
  - 全 worktree の Chat / Workflow を hydrated component として同時保持すると memory 方針と衝突する。summary tile + active tile hydration に限定する必要がある。

### 隣接 Milestone

- Milestone [#56 READ専用コードインテリジェンス](https://github.com/siro33950/releash/milestone/56) は open 7 / closed 0。
  - CodeIntel は有用だが、RepositoryStateService / file catalog / cache budget の上に載せないとさらに scan と memory を増やす。
- Milestone [#54 ローカルLLM対応](https://github.com/siro33950/releash/milestone/54) は open 4 / closed 0。
  - Agent runtime と session storage の圧力が増えるため、M2 の memory model と runtime caps を先行させるべき。
- Milestone [#51 チャットUI改善](https://github.com/siro33950/releash/milestone/51) は open 1 / closed 28。
  - UI 改善は M2 の paging / virtualization と整合させる。

## 追加した Issue

既存 Issue に吸収しきれない横断 work item は、独立マイルストーン
「[性能・メモリ効率改善（Workbench State / Read Model）](https://github.com/siro33950/releash/milestone/80)」
に新規 Issue として切った。

1. [#1209 Performance budget / telemetry を追加する](https://github.com/siro33950/releash/issues/1209)
   - startup、repo snapshot、diff open、stream payload、session IO、JS heap、Rust RSS を測る。
2. [#1210 RepositoryStateService を導入し watcher / status / stats を 1 系統化する](https://github.com/siro33950/releash/issues/1210)
   - watcher / status / stats / branch / worktree dirty count / diff tree を 1 系統化する。
3. [#1211 ReviewSnapshot / ReviewFileView command を追加し diff 表示を Rust read model に寄せる](https://github.com/siro33950/releash/issues/1211)
   - frontend direct FS read と diff tree orchestration を削除する。
4. [#1212 Hunk operation を id-based にし frontend patch 再生成を削除する](https://github.com/siro33950/releash/issues/1212)
   - frontend で full content から patch を再生成しない。
5. [#1213 Agent session summary index と message paging を導入する](https://github.com/siro33950/releash/issues/1213)
   - list と get が full session cache / clone に依存しないようにする。
6. [#1214 Agent streaming を cumulative snapshot から seq delta に移行する](https://github.com/siro33950/releash/issues/1214)
   - reconnect snapshot と通常 delta を分ける。
7. [#1215 Terminal / PTY lifecycle cap を導入する](https://github.com/siro33950/releash/issues/1215)
   - inactive tab unmount、LRU、idle timeout、buffer owner の整理。
8. [#1216 Startup orphan cleanup を non-blocking service 化する](https://github.com/siro33950/releash/issues/1216)
   - 誤 kill を避けつつ visible startup を block しない。
9. [#1217 bridge_common.rs を runtime / stream / persist / recovery に分割する](https://github.com/siro33950/releash/issues/1217)
   - stream、process registry、session persistence、permission、recovery、tests を分ける。
10. [#1218 Remote scope decision: src/remote を削除・縮小・native 化のどれに寄せるか決定する](https://github.com/siro33950/releash/issues/1218)
   - `src/remote/` を削除・縮小・native 化のどれに寄せるか決定する。

## 実装順の推奨

最短で効く順序は次の通り。#1191 は先行対応済みなので、この順序では「再発防止の構造化」と「未解決の直接対策」を進める。

1. #1194 / #1195 / #1196: 既存のメモリ削減分割 Issue を片付け、turn / frontend message / workflow runtime の残留を止める。
2. #1192 / #1178: bridge process death detection、respawn、stuck Thinking recovery を入れ、runtime lifecycle の復旧性を上げる。
3. #1209: 計測を入れ、session save bytes / streaming event bytes / JS heap / Rust RSS を見えるようにする。
4. #1210 / #1211 / #1212: RepositoryStateService、ReviewSnapshot / ReviewFileView、id-based hunk operation を入れ、Git / diff の重複 scan と frontend patch 再生成を止める。
5. #1213 / #1190: Agent session storage を summary index + paging に変え、復帰時 context restoration と保存正典を揃える。
6. #1214: streaming delta protocol に移り、通常 payload が応答全体長に比例して増えないようにする。
7. #1215 / #1216: Terminal / PTY lifecycle cap と startup orphan cleanup non-blocking 化で hidden runtime と startup block を抑える。
8. #1217: `bridge_common.rs` を分割し、runtime / stream / persist / recovery を局所化する。
9. #1218: Remote scope を決定し、desktop parity ではなく Chat / Workflow / Review 監督に寄せるか、削除・native 化へ整理する。
10. #72 / #767 / #878: 既存の Clean Architecture 移行と frontend logic Rust 移行に合流し、旧 command surface と dead code を縮める。

## 判断基準

今後の設計判断は、次の基準を満たすかで見る。

- frontend に domain logic / file IO / diff construction / Git orchestration がない。
- 画面に見えていない data body は frontend state に常駐しない。
- long-running stream は payload size が応答全体長に比例して増えない。
- Git scan は component 数に比例して増えない。
- list API は item body の総量に比例しない。
- hidden terminal / workflow / remote runtime は明示的に budget 管理される。
- read command は write side-effect を持たない。
- large file / large session / many worktree / many terminal の degraded mode が定義されている。
