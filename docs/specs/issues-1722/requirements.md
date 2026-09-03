# Context

## 入力文書

- 正本: GitHub Issue #1722「feat(workspace): Session Node の表示名 — provider タイトルの取り込みと手動 rename、History のタイトル表示」 <https://github.com/siro33950/releash/issues/1722>（label: `enhancement`、state: OPEN、milestone なし、comment なし）
- 参照した規約: `AGENTS.md`、`docs/glossary/DOMAIN.md`
- 参照した既存 Spec: `docs/specs/issues-1662/requirements.md`
- 参照した実装: `src-tauri/src/domain/workflow/value_objects/node_fact.rs`、`src-tauri/src/domain/workspace_tree/`（`entities/mod.rs`、`services.rs`、`value_objects/mod.rs`）、`src-tauri/src/adaptor/gateway/workspace_tree/query_service.rs`、`src-tauri/src/domain/agent_session/`（`aggregates/agent_session.rs`、`provider_history_gateway.rs`）、`src-tauri/src/adaptor/gateway/agent_session/agent_session_history_gateway.rs`、`src-tauri/src/usecase/agent_session/agent_session_history.rs`、`src-tauri/src/infrastructure/provider_history.rs`、`src-tauri/src/adaptor/gateway/provider_lifecycle/payload.rs`、`src-tauri/src/cli/hook.rs`、`src/components/workspace/WorkspaceList.tsx`、`src/types/workspace-tree.ts`、`src/types/agent-session.ts`
- 配置先: `docs/specs/issues-1722`

## 確定済みの背景

### 表示名の現行経路

- Workspace ツリーの Node 行の表示名は Node 名である（`src-tauri/src/domain/workspace_tree/entities/mod.rs:483`）。実行木の各 Node が生成される時点で Node 名がそのまま表示名になる。
- 実行木のルート行の表示名だけは owner（Workflow ノード）の表示名で差し替わる（`src-tauri/src/domain/workspace_tree/services.rs:49`、`src-tauri/src/adaptor/gateway/workspace_tree/query_service.rs:390-417`）。owner の表示名は workflow 定義の `name` である（`src-tauri/src/domain/workspace_tree/entities/mod.rs:282`）。この規則は Issue #1662 の変更で確定した。
- 単独 Session は Session Node 1個を root とする実行木であり（`docs/glossary/DOMAIN.md`）、Releash が合成する定義の `name` と唯一の Node 名がともに `session` である（`src-tauri/src/domain/workflow/value_objects/node_fact.rs:159`）。したがって単独 Session 実行木のルート行は常に `session` と表示される。
- frontend は read model が返した表示名をそのまま描画し、加工しない（`src/components/workspace/WorkspaceList.tsx:213`、`同:408`）。`AGENTS.md`「Rust がロジックを所有する」により、表示名の決定はアプリケーションロジックとして Rust が持つ。
- Workspace ツリーの read model は Tauri command（`src-tauri/src/adaptor/controller/command/workspace_tree.rs`）からのみ公開されており、loopback local API に route を持たない。

### provider タイトルの供給源

- Session Node が参照する provider CLI の継続 identity と lifecycle は AgentSession が持つ（`docs/glossary/DOMAIN.md`）。lifecycle は `open` / `paused` / `archived` の3値であり（`src-tauri/src/domain/agent_session/aggregates/agent_session.rs:8-12`）、transcript への参照を `transcript_ref` として保持する（`同:471`）。
- provider hook の payload はタイトルを含まない。受理する field は `session_id` / `transcript_path` / `hook_event_name` / `error` / `error_details` / `agent_id` / `tool_name` である（`src-tauri/src/adaptor/gateway/provider_lifecycle/payload.rs:117-131`）。したがってタイトルは hook とは別経路で読む必要がある。
- Issue はタイトルの供給源を Claude が transcript の `ai-title`、Codex が thread name と定める。Codex の thread name は Provider history が既に接続している `state_5.sqlite` の `threads` テーブルにある（`src-tauri/src/infrastructure/provider_history.rs:66-70` は同テーブルから `id` / `cwd` / `updated_at` のみを取得している）。
- Issue は取り込みを Rust 側のポーリングで行うことを定める。

### Provider history の現行経路

- Provider history は worktree path を鍵に provider の履歴を列挙する読み取り経路である（`src-tauri/src/usecase/agent_session/agent_session_history.rs`）。gateway は Claude では `~/.claude/projects/<変換済み worktree path>/` 配下の `*.jsonl` の file stem と mtime、Codex では `state_5.sqlite` の `threads` 行から、`provider` / `provider_session_id` / `updated_at_ms` だけを取り出す（`src-tauri/src/adaptor/gateway/agent_session/agent_session_history_gateway.rs:44-91`）。ファイル本文は読まない。
- 公開 DTO は `provider` と `providerSessionId` のみを持つ（`src/types/agent-session.ts:27-30`）。

# Outcome

- 対象者: Releash の Workspace で複数の Session と workflow を並行して動かし、ツリーおよび Provider history から作業を選び分ける開発者。
- 現在の問題: 単独 Session 実行木のルート行がどれも `session` と表示され、行から中身を区別できない。Node の表示名を人が変更する経路も無い。Provider history も provider 名と provider session id の並びであり、再開したい会話を選べない。加えて、AgentSession が bind される前の Session Node の行は、まだ何も観測していないにもかかわらず介入待ちとして表示される。
- 変更後に実現する状態: 単独 Session 実行木のルート行に provider が生成したセッションタイトルが表示され、開発者は行の表示名から会話の中身を判別できる。任意の Session Node は人が付けた名前へ変更でき、その名前は provider タイトルの更新に上書きされない。Provider history も各行がタイトルで表示され、再開する会話を内容から選べる。bind 前の Session Node の行はそれと分かる状態で表示され、その間は表示名を変更できないことが行から読み取れる。

# Current Behavior

## 再現手順

1. Releash を起動し、任意の worktree を選択する。
2. その worktree で Claude の単独 Session を起動し、`ai-title` が生成されるまで会話を進める。
3. Workspace ツリーの、手順 2 で起動した実行木のルート行を見る。
4. 同じ worktree で 2本目の単独 Session を起動し、Workspace ツリーの2行を見比べる。
5. 手順 3 の行の表示名を人手で変更する経路を探す（行のクリック、ホバー時のボタン、行のメニュー）。
6. 任意の builtin workflow を起動し、その実行木に含まれる Session Node の行を、provider に接続する前と後で見比べる。
7. AgentSession の起動メニューから Provider history の一覧を開く。

## 実際の出力

- 手順 3: ルート行は `session` と表示される。
- 手順 4: 2行とも `session` と表示され、行の表示名からは区別できない。provider が生成したタイトルは表示されない。
- 手順 5: 表示名を変更する操作は存在しない。行に付くのは Archive / Delete / Close と workflow 操作メニューだけである。
- 手順 6: Session Node の行は workflow 定義の Node 名のまま表示され、変更する操作は存在しない。AgentSession が bind される前も、bind された後と同じ黄（介入待ち）の状態表示になる。
- 手順 7: 各行が `claude 4f3a9b21-…`（provider 名と provider session id）の形で表示される。タイトルも会話内容も表示されない。

## 確認方法と根拠

現在の挙動は、表示名の投影経路と Provider history の読み取り経路をコード上で追って確認した。アプリを起動しての画面確認は行っていない。

- ルート行が `session` になること: 合成定義の `name` と Node 名がともに `session`（`src-tauri/src/domain/workflow/value_objects/node_fact.rs:159`）であり、ルート行の表示名は owner の表示名 = 定義の `name` で差し替わる（`src-tauri/src/domain/workspace_tree/services.rs:49`、`src-tauri/src/adaptor/gateway/workspace_tree/query_service.rs:390-417`、`src-tauri/src/domain/workspace_tree/entities/mod.rs:282`）。
- workflow 内 Session Node の行が Node 名になること: `src-tauri/src/domain/workspace_tree/entities/mod.rs:483`。
- 表示名を変更する経路が存在しないこと: `rename` に相当する Tauri command、usecase、domain 操作は存在しない（`src-tauri/src` 全体の検索で該当なし）。frontend の行にも該当する操作は無い（`src/components/workspace/WorkspaceList.tsx:213-283`）。
- provider タイトルを読む経路が存在しないこと: `ai-title` / `aiTitle` / `thread_name` / `session_index` を読む実装は存在しない（`src-tauri/src` 全体の検索で該当なし）。Provider history の gateway もファイル本文と `threads.name` を読まない（`src-tauri/src/adaptor/gateway/agent_session/agent_session_history_gateway.rs:44-91`、`src-tauri/src/infrastructure/provider_history.rs:66-70`）。
- bind 前の Session Node が黄（介入待ち）で表示されること: 行の生成時に `activity` へ既定値が入り（`src-tauri/src/domain/workspace_tree/entities/mod.rs:482`）、`AgentSessionActivity` の既定は `AwaitingInstruction`（`src-tauri/src/domain/workflow/value_objects/node_fact.rs:271-272`）、Session Node の `AwaitingInstruction` は `Attention` に分類される（`src-tauri/src/domain/workspace_tree/value_objects/mod.rs:178-185`）。単独 Session 実行木は root started と `session_attached` を同時に書くため（`同 node_fact.rs:196-206`）、bind 前の行は workflow 実行木の Session Node にのみ現れる。
- Provider history の行表示: `src/components/workspace/WorkspaceList.tsx:1257`。

# Scope / Non-goals

## 変更するもの

- Session Node の表示名の決定規則。手動 rename、provider のセッションタイトル、既定値の3段の優先順位を持たせる。
- Session Node の手動 rename。名前の受付、Releash 側での保持、Workspace ツリーの表示への反映。
- provider のセッションタイトルの取り込み。Claude は transcript の `ai-title`、Codex は thread name を供給源とし、Rust 側のポーリングで取り込む。
- Session Node が AgentSession に bind されるまでの間の状態表示。Workspace ツリーの状態分類、色、アイコンを対象にする。
- Provider history 一覧の行表示。provider のセッションタイトル、最初のユーザープロンプト、provider 名と短縮 id の順で表示する。

## 変更しないもの

- Session Node 以外の rename。Sequence 行、Fanout 行、および workflow 実行木のルート行（workflow 名を表示する行）は対象にしない。
- Claude の `custom-title` の読み取り、および provider 側への名前の書き込み。Codex の `SetThreadName` は自動生成の thread name と同じ保存先にあり供給源では区別できないため、独立した読み取り対象として扱わず、手動 rename の優先順位によって表示を保護する。
- Fanout / retry で並ぶ同名 Node の表示上の区別。
- Workspace ツリーの行への provider 判別表示。
- workflow 実行木のルート行に workflow 名を表示する現行規則（Issue #1662 で確定）。
- Provider history の取得範囲、並び順、ページング、および再開操作。
- archive 済み AgentSession 一覧の表示名。
- Claude / Codex 以外の provider への対応。
- workflow 定義側の Node 名、および定義構文。
- bind 済みの Session Node、Command Node、Sequence 行、Fanout 行、および workflow 実行木のルート行の状態分類。
- Workspace ツリーの階層、展開・折り畳み操作。

# Requirements

- R-001: Session Node の表示名は、手動 rename ＞ provider のセッションタイトル ＞ 既定値 の優先順位で決まる。上位の段に値がある間、下位の段の値は表示されない。
- R-002: workflow 実行木に含まれる Session Node は、workflow 定義の Node 名を手動 rename 段の初期値として持つ。rename されない限り、provider のセッションタイトルを取得しても Node 名のまま表示される。
- R-003: 単独 Session 実行木のルート行は手動 rename 段に初期値を持たない。provider のセッションタイトルを取得した後は、rename されていなければそのタイトルを表示する。
- R-004: 単独 Session 実行木のルート行は、provider のセッションタイトルが未取得かつ rename されていない間、現行と同じ既定値 `session` を表示する。
- R-005: Fanout または retry によって並ぶ同名の Node は、表示名も同名のままとする。表示名に序数その他の区別を付けない。
- R-006: 利用者は Session Node の表示名を任意の名前へ手動で変更できる。変更後は Workspace ツリーの当該行がその名前で表示される。
- R-007: 手動 rename の対象は Session Node のみとする。Sequence 行、Fanout 行、および workflow 実行木のルート行は rename できない。
- R-008: 手動 rename した表示名は Releash が保持し、アプリケーションを再起動しても維持される。provider のセッションタイトルが後から取得または更新されても、手動 rename した表示名は変わらない。
- R-009: 手動 rename の名前は Releash 側だけが持ち、provider 側へ名前を書き込まない。Claude の `custom-title` は読み取らない。Codex の thread name は自動生成名と `SetThreadName` が同じ保存先を使い供給源の側で区別できないため、Codex について「カスタム名を読み取らない」ことは保証せず、手動 rename が provider のセッションタイトルより優先される R-001 によって、rename 済みの表示名が Codex 側の名前に上書きされないことを保証する。
- R-010: provider のセッションタイトルの供給源は、Claude が transcript の `ai-title`、Codex が thread name とする。provider hook の payload はタイトルを含まないため、hook とは別の経路で読む。
- R-011: provider のセッションタイトルの定期的な取り込みは、活動中の AgentSession のみを対象とする。provider session が終了した AgentSession、および `paused` / `archived` になった AgentSession のタイトルは再取得しない。
- R-012: 定期的な取り込みの間隔は、タイトルが未取得の AgentSession で 20 秒、取得済みの AgentSession で 5 分とする。
- R-013: タイトルの読み取りは最小に保つ。transcript の全走査を行わない。
- R-014: Provider history 一覧の各行は、provider のセッションタイトル、最初のユーザープロンプトの冒頭、provider 名と短縮した provider session id の順に、最初に存在する値を表示する。provider session id をそのまま表示しない。
- R-015: Session Node は、AgentSession が bind されるまでの間、表示名を変更できない。bind された後は変更できる。
- R-016: Session Node は、AgentSession が bind されるまでの間、bind 後の状態表示および他のどの状態表示とも区別できる状態で Workspace ツリーに表示される。
- R-017: Sequence 行および Fanout 行の状態を決めるときは、bind 待ちの状態に分類された Session Node を集約対象の子から除外する。bind 前に終了して Failure または Idle に分類された Session Node は除外しない。除外後に集約対象の子が残る場合は、親自身と残った子に既存の集約規則を適用し、集約対象の子が残らない場合に限り、親行は bind 前の状態を表示する。

# Assumptions / Open Questions

なし。
