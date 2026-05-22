# issues-1019: Workflow Engine Evolution - Mutating CLI

関連: [GitHub Issue #1019](https://github.com/siro33950/releash/issues/1019) / マイルストーン [06] / [`workflow-engine-evolution-plan.md`](../workflow-engine-evolution-plan.md) / [`workflow-engine-model-boundary.md`](../workflow-engine-model-boundary.md) / 先行 [issues-1011](./issues-1011.md) / [issues-1013](./issues-1013.md) / [issues-1015](./issues-1015.md)

## 要求

**種別**: 新機能

**ゴール**: 外部 caller（人間オペレータ / Agent）が `run_id` を主語として workflow run の state を変化させる CLI 経路を確立し、CLI からの外部入口を観測（[05]）と操作（[06]）の両面で run_id 主語に揃える。具体的には以下が満たされること。

- CLI から approve / reject / abort が run_id 主語で実行できる。
  - `releash workflow approve <run-id>`（任意で対象 node の限定および承認コメントを添える）
  - `releash workflow reject <run-id>`（任意で対象 node の限定。却下理由は必須）
  - `releash workflow abort <run-id>`（任意で対象 node の限定）
- 上記 CLI command は、read-only CLI（[05]）と同じ **engine と IPC せず file を仲介とする** 世界観を踏襲する。CLI は pending command を file として書き出し、稼働中の Releash app の watcher がそれを拾って既存の typed `WorkflowCommand` 入口（`ApproveNode` / `RejectNode` / `AbortRun`）へ受け渡す。
- **CLI の完了基準は「受理キュー投入」までで統一する**。CLI は engine 側での受理結果を待たずに、pending command の書き出しが完了した時点で完了とみなす。稼働中アプリで engine への到達まで観測できた場合は、付加情報として利用者に伝える（完了基準そのものを変えない）。
- 稼働中アプリ経由で engine に届いた要求には、既存 engine の認可・冪等性・stale target 判定（[04] / [issues-1013](./issues-1013.md) で確立）がそのまま適用される。CLI 経由の操作と UI 経由の操作は engine から見て等価に扱われ、同一意図の command は呼び出し経路に依らず同じ許可・拒否・重複扱いとなる。
- 既存 UI approval / abort 経路（`approve_workflow_step` / `abort_workflow` Tauri command）は本 issue では破壊しない。CLI 経路の追加によって既存 UI 動作に regression を起こさない。
- アプリ非稼働時の挙動が明示的に定義されている。CLI による pending command の file 書き出しは成功し、CLI はその時点で完了する。engine への到達は、有効期限内であれば次回 app 起動 + watcher pickup 時となる。
- pending command の重複処理 / 古い未処理キューの累積を防ぐ運用境界が定義されている。各 pending command は engine により一度だけ処理され、処理済みは pickup 経路から除外される。古い未処理要求は無期限に滞留せず、最終的に engine に到達しなくなる（TTL / 起動時 cleanup の具体実装手段は実装に委ねる）。
- engine に到達した CLI 経由要求の事実が `WorkflowEvent` 列に typed event として記録され、observer（[05] read-only CLI / API）から「いつ / どの経路から / 何が要求されたか」が `run_id` を主語に観測できる。TTL 等により engine に到達しなかった pending command の観測責務は本 issue では持たない。
- **`--reason` / `--comment` は機密情報を含めない前提で扱う**。これらの自由記述データは pending command file・`WorkflowEvent` log・観測 API・CLI 表示に平文で露出する。秘匿が必要な情報を入力しない運用前提を要求側で固定する（暗号化・マスキング等の追加対策は本 issue で導入しない）。
- 既存 Tauri 入口（`abort_workflow` / `approve_workflow_step`）はすでに run_id 主語化済み（[04] / [issues-1013](./issues-1013.md)）であり、本 issue は CLI 入口の追加に閉じる。plan doc が言う「旧 `worktree_path` command は wrapper として維持する」は [04] 時点で破壊的置換が選択され適用済みのため、本 issue 範囲では該当しない。

**スコープ外**: 以下は本 issue の対象外。

- `releash workflow run <workflow-name> <task>` 等の **新規 run 起動 CLI** は本 issue では対象外とする。plan doc [06] の作業項目には起動 CLI が含まれておらず、起動は worktree 選定 / 初期 task / permission_mode 等の追加要素を伴うため、別 issue として切り出す。
- structured output 提出 CLI（[08]）: `WorkflowCommand::SubmitOutput` / `releash workflow output submit|validate|get`。
- Workflow Panel / Command Center UI（[07]）: 本 issue では UI パネル新規追加は行わない。既存 approval UI は破壊しない範囲で動作させる。
- bash node 実行系統（[13]）。
- main-agent narrator への typed event 配信と user decision 経路整備（[16]）。
- engine 内部 typed command の他 variant 追加（`SubmitOutput` 等）。

**現状温存**: 既存の UI approval / abort 経路（`approve_workflow_step` / `abort_workflow` Tauri command と関連フロントエンド）は本 issue では破壊しない。同じ engine `dispatch` を経由するため、CLI と UI が並行して同じ run に対して command を発行しても、engine 側の冪等性・stale target 判定で整合的に処理される。agent 返信テキスト解釈に基づく engine 内部の分岐ロジック（aggregate の LGTM マッチング、`<workflow_output>` 抽出など）は本 issue では廃止対象としない（[issues-1013](./issues-1013.md) / [issues-1015](./issues-1015.md) と同じ方針を踏襲）。

**背景**: マイルストーン [04] までで workflow state を変化させる入口は `WorkflowCommand` 型に typed 化され、Tauri command 入口は run_id を主語に揃った（[issues-1013](./issues-1013.md)）。続く [05] では外部 caller が `run_id` を主語に workflow run を **観測** できる経路（read-only API + CLI）が確立し、CLI は engine と IPC せず `workflow_runs/` 配下と `workflows/` YAML を直接読む file-direct 構成を採用した（[issues-1015](./issues-1015.md)）。

一方、外部 caller が `run_id` を主語に workflow run の state を **変化** させる経路は依然として Tauri command（UI / Remote セッション）に閉じており、CLI / Agent が「同じ run_id 主語で同じ command を発行する」ための入口が存在しない。今後の [07] Workflow Panel / [08] OutputForm CLI / [15] Skill / [16] Main Agent Mediation はいずれも「CLI と UI が同一 typed command 境界を共有する」「同一 run_id を主語に観測と操作が対称に行える」ことを前提とする。

本 issue は read-only ([05]) と対称な形で mutating 経路を追加し、CLI 入口・UI 入口・Agent 入口を engine から見て経路非依存に扱える土台を完成させる。Archon が CLI / Slack / Telegram など複数 Platform Adapter から file/DB-direct で engine に到達する構成を採っていることを参考に、Releash でも CLI は「pending command を file として書き出し、稼働中アプリの watcher が拾って typed dispatch する」file-direct 路線で統一する。

**前提**:

- マイルストーン [05] Read-Only Run APIs + CLI 完了済み（[issues-1015](./issues-1015.md)）。`src-tauri/src/cli/mod.rs` に `releash workflow list|runs|status|logs` の CLI scaffolding（`clap` ベース）が存在し、`workflow_runs/` および `workflows/` への file-direct 読み出し経路と projection helper（`event_projection::reconstruct_state_from_events`）が共有可能な状態にある。
- マイルストーン [04] Command / Event Boundary 完了済み（[issues-1013](./issues-1013.md)）。`WorkflowCommand` typed 入口（`StartRun` / `AbortRun` / `ApproveNode` / `RejectNode` / `CompleteNode` / `FailNode`）と `WorkflowEvent` 語彙、`WorkflowEngine::dispatch` 単一入口、command 受理サイクル内 atomic rollback 境界が確立済み。
- マイルストーン [03] Run Store / Run ID 完了済み（[issues-1011](./issues-1011.md)）。`run_id` を一次キーとする run metadata / event log の永続化基盤が成立済み。
- 既存 Tauri command `abort_workflow(run_id)` / `approve_workflow_step(run_id, decision, step_name)` はすでに `WorkflowCommand` 経由で engine に dispatch されており、本 issue で新規導入する CLI 経路はこの dispatch を file-direct + watcher 経由で再利用する。

## 関連マイルストーン上の位置

- 直接の依存元: [04] Command / Event Boundary / [05] Read-Only Run APIs + CLI。
- 直接の依存先: [07] Workflow Panel / [08] OutputForm CLI / [15] Skill / [16] Main Agent Mediation。
- 本 issue は user decision 系 3 command（approve / reject / abort）の CLI 入口追加に閉じる。新規 run 起動 CLI / structured output 提出 CLI / UI パネル / agent 経路整備はそれぞれ別マイルストーンに委ねる。

## 振る舞い定義

```gherkin
Feature: CLI 経由で workflow run の state を変化させる経路

  Rule: 外部 caller は run_id を主語に workflow run の state 変化を要求できる
    Scenario: 進行中 run に対して承認を要求する
      Given 承認を待っている workflow run が存在する
      When 外部 caller が当該 run に対して承認を要求する
      Then 当該 run への承認要求が workflow engine に受理される

    Scenario: 承認コメントを添えて承認を要求する
      Given 承認を待っている workflow run が存在する
      When 外部 caller が承認コメントを添えて当該 run に対して承認を要求する
      Then 当該 run への承認要求が承認コメントとともに workflow engine に受理される

    Scenario: 進行中 run に対して却下を要求する
      Given 承認を待っている workflow run が存在する
      When 外部 caller が却下理由を添えて当該 run に対して却下を要求する
      Then 当該 run への却下要求が workflow engine に受理される

    Scenario: 進行中 run に対して中止を要求する
      Given 進行中の workflow run が存在する
      When 外部 caller が当該 run に対して中止を要求する
      Then 当該 run への中止要求が workflow engine に受理される

  Rule: 対象 node を限定した要求も run 全体への要求も同等に受理される
    Scenario: 対象 node を限定して要求する
      Given 承認を待っている workflow run が存在する
      When 外部 caller が対象 node を限定して当該 run に対して state 変化を要求する
      Then 当該要求は対象 node を限定した要求として workflow engine に受理される

    Scenario: run 全体に対して要求する
      Given 承認を待っている workflow run が存在する
      When 外部 caller が対象 node を限定せずに当該 run に対して state 変化を要求する
      Then 当該要求は run 全体への要求として workflow engine に受理される

  Rule: 却下要求には却下理由が伴う
    Scenario: 却下理由を伴わない却下要求は成立しない
      Given 承認を待っている workflow run が存在する
      When 外部 caller が却下理由を伴わずに当該 run に対して却下を要求する
      Then 当該却下要求は却下要求として成立せず利用者に受理されない

  Rule: 同一意図の state 変化要求は呼び出し経路に依らず同じ振る舞いを示す
    Scenario: CLI 経由と UI 経由の同一意図要求は engine から見て等価に扱われる
      Given 同一の workflow run に対して同一意図の state 変化要求が複数経路から発行される
      When 各経路からの要求が workflow engine に届く
      Then 要求の許可・拒否・重複扱いは呼び出し経路に依らず同一となる

  Rule: 各 state 変化要求は engine により一度だけ処理される
    Scenario: 同じ要求が engine に二度処理されない
      Given 外部 caller が state 変化要求を発行している
      When 当該要求が workflow engine に届く
      Then 同一の要求が engine に二度処理されることはない

  Rule: 古い未処理要求は無期限に滞留せず、最終的に engine に到達しなくなる
    Scenario: 有効期限を過ぎた要求
      Given 外部 caller の state 変化要求が受理キューに積まれたまま有効期限を過ぎている
      When 以降のいかなる時点でも当該要求は workflow engine に到達しない
      Then 古い未処理要求は無期限に滞留せず engine による処理対象から除外される

  Rule: 要求が engine に到達したことを観測できた場合は外部 caller に付加情報として伝わる
    Scenario: workflow engine が稼働している状態での要求
      Given workflow engine が稼働している
      When 外部 caller が state 変化を要求する
      Then 要求が engine まで届いたことを観測できた場合はその旨が付加情報として外部 caller に伝わる

    Scenario: workflow engine が稼働していない状態での要求
      Given workflow engine が稼働していない
      When 外部 caller が state 変化を要求する
      Then 要求が受理キューに積まれた旨が外部 caller に伝わる
      And engine の次回稼働時に有効期限内であれば当該要求は engine に到達する

  Rule: engine に到達した CLI 経由要求は観測経路から事実として観測できる
    Scenario: engine に到達した CLI 経由要求が observer から観測できる
      Given 外部 caller の CLI 経由 state 変化要求が workflow engine に到達した
      When 観測者が当該 run の事実列を要求する
      Then いつ・どの経路から・何が要求されたかが run_id を主語に観測できる

  Rule: 外部 caller が提供する自由記述データは観測経路から平文として観測できる
    Scenario: 観測者は CLI 要求の reason / comment を平文で観測できる
      Given 外部 caller が reason または comment を伴って state 変化を要求した
      When 観測者が当該 run の事実列を要求する
      Then 当該 reason および comment は平文で観測できる
```

## アーキテクチャ概要

本 issue は read-only ([05]) と対称な mutating 経路を、CLI から engine への file-direct 受け渡し（pending command file を仲介し、稼働中アプリの watcher が拾って既存 typed dispatch に流す）として確立する。CLI は engine と直接 IPC せず、Archon 事例に倣う file-direct 構成を踏襲する。既存 UI / Tauri command の dispatch 経路（`approve_workflow_step` / `abort_workflow`）と engine 内部の認可・冪等性・stale target 判定は本 issue では拡張せず、CLI 経由の要求も同一 `WorkflowEngine::dispatch_external` 入口で処理する。

### 責務配置

- **CLI mutating 入口（`src-tauri/src/cli/` 拡張）**: `releash workflow approve|reject|abort <run-id>` の clap サブコマンド追加、引数バリデーション（`run-id` UUID 形式、`reject` サブコマンドでの `--reason` 必須化、`approve` での `--comment` 任意化、`--node` の任意限定）、pending command の file 書き出し、書き出し結果（受理キュー投入完了 / 稼働中アプリで engine 到達まで観測できた場合の付加情報）の caller 向け出力整形。**`--reason` 必須化は CLI 入口で完結させ、engine 側は『reject 要求には reason が必ず伴う』前提で受理する**（engine が reason 欠落を理由に拒否する経路は持たない）。担当しない: engine への直接 dispatch、観測経路の重複実装、authorization 判断、stale target 判定。
- **pending command store（新設モジュール、`src-tauri/src/workflow/` 配下に file-direct 仲介層を切る）**: pending command を表現する typed payload（approve / reject / abort、対象 `run_id`、任意 `node_name`、reject reason、approve コメント、要求元経路、生成 timestamp）と、その file 形式での書き込み・読み出し・処理済み除外・古い未処理エントリの cleanup ポリシー（TTL）を担う。担当しない: engine 内部の state mutation、`WorkflowCommand` enum 拡張、event 発行。
- **watcher 接続点（`src-tauri/src/watcher.rs` または専用 watcher の追加）**: pending command store のディレクトリ変更を debounce 検知し、新規エントリを順に dispatcher adapter に渡す。担当しない: file 形式パース、`WorkflowCommand` 組み立て、エラーリトライ戦略の決定。
- **dispatcher adapter（`src-tauri/src/workflow/commands.rs` または新設 adapter）**: pending payload を `WorkflowCommand::ApproveNode` / `RejectNode` / `AbortRun` に変換し、`WorkflowEngine::dispatch_external` に渡す。dispatch 完了後に pending entry を「処理済み」として除外マーキングする責務をここで持つ。担当しない: engine 内部の認可・冪等性・stale target 判定（既存 engine 側に閉じる）、CLI 出力整形、`WorkflowCommand` の internal variant 取り扱い。
- **`workflow/command.rs`（既存・変更なし）**: 既存 typed 入口 `ApproveNode` / `RejectNode` / `AbortRun` をそのまま再利用する。CLI 経路のために新規 variant は追加しない。
- **`workflow/engine.rs`（既存挙動を変えず、新規 typed event variant の受理・記録経路を拡張）**: 既存 `dispatch_external` を CLI 経路でも単一入口として再利用し、認可・冪等性・stale target 判定はそのまま適用する。これに加えて、`dispatch_external` のサイクル内で「CLI 経由で mutation が要求された事実」を表す新規 typed event variant を `WorkflowEvent` 列に追記する経路を拡張する。担当しない: 既存 state mutation ロジックの変更、経路別の分岐。
- **`workflow/event.rs`（既存・拡張）**: 「CLI 経由で mutation が要求された事実」を append-only に記録する typed event を追加する（経路種別・要求内容・要求時刻を `run_id` 主語で表現する）。担当しない: 既存 `ApprovalResolved` / `RunAborted` 等の発行点を変えること（受理時の事実列は既存通り）。
- **観測経路（[05]）**: 本 issue が追加する新規 event variant も既存 `get_workflow_run_log` / `releash workflow logs` から透過的に観測できる。担当しない: CLI mutation 要求のための専用観測 API を追加すること。
- **フロントエンド（`src/`）**: 変更なし。既存 UI approval / abort 経路はそのまま維持する（破壊しない境界）。

### データ/通信フロー

- **CLI 経由 approve / reject / abort（アプリ稼働中）**: 利用者 → `releash workflow {approve|reject|abort} <run-id> [--node N] [--reason R] [--comment C]` → CLI が引数バリデーション → pending command store に file として書き出し → ここで CLI は「受理キューに投入された」旨を返して完了する。以降は稼働中アプリ側の非同期フロー: 稼働中 watcher が変更検知 → dispatcher adapter が pending payload → `WorkflowCommand` 変換 → `WorkflowEngine::dispatch_external` → engine の認可・冪等性・stale target 判定 → state mutation + `WorkflowEvent::ApprovalResolved` / `RunAborted` 等の既存 event 列追記 + 「CLI 経由要求の事実」typed event の追記 → pending entry を処理済みとして除外。CLI 完了基準はこの非同期フローの結果を待たず、書き出し完了の時点で固定する。engine 到達まで CLI 側で観測できた場合は付加情報として併せて返してよいが、完了判定そのものは受理キュー投入で確定する。
- **CLI 経由 approve / reject / abort（アプリ非稼働中）**: 利用者 → CLI → pending command store に書き出すまでで CLI は完了 → CLI 出力は「受理キューに投入された」旨を返す → 次回アプリ起動 + watcher pickup 時に、有効期限内であれば上記稼働中フローに合流する。
- **観測経路（既存）**: 観測者 → `releash workflow logs <run-id>` または `get_workflow_run_log` → NDJSON 読込 → 「CLI 経由要求の事実」を含む `WorkflowEvent` 列が返る。
- **UI 経由 approve / reject / abort（既存・温存）**: UI → `approve_workflow_step` / `abort_workflow` Tauri command → `dispatch_external` → 同一 engine 処理。CLI 経路とは独立に同じ engine 入口に到達するため、engine 視点では経路非依存。
- **重複 dispatch 防止フロー**: pending entry は dispatcher adapter で「処理中→処理済み」マーキングされ、watcher 再発火や同一 file の二重検知でも engine への二重 dispatch にはならない。古い未処理 pending entry は TTL / 起動時 cleanup ポリシーに従って除外される。

### 状態 Owner

- **pending command の集合**: pending command store のファイル群が一次 owner。CLI が writer、watcher + dispatcher adapter が reader / consumer。engine は pending 集合を直接参照しない（変換後の `WorkflowCommand` だけを受け取る）。
- **pending entry の処理済み状態**: dispatcher adapter が owner。file 削除 or 専用 marker file / 別ディレクトリ移動などの実装手段は委ねるが、唯一の判定権威であること。
- **`WorkflowEvent` 列（既存）**: engine が一次 owner として保持する append-only NDJSON。CLI mutation 要求の事実も engine が `dispatch_external` のサイクル内で追記する。CLI / 観測経路は読み取り側として `WorkflowEventLog` を介す。
- **active run の state（既存）**: engine の in-memory map + `workflow_runs/{run_id}.json`。本 issue は state Owner を変えず、書き込み起動点として CLI 経路を追加するだけ。
- **CLI 出力の完了報告と engine 到達観測（付加情報）**: CLI が owner。CLI 完了基準は「受理キュー投入」で固定であり、書き出し完了をもって利用者に完了を返す責務を CLI 側に閉じる。これに加えて「engine 到達まで観測できたか」を付加情報として利用者に伝えるかは実装に委ねる（観測手段として IPC 不在 / lock file / app 起動 marker / `workflow_runs/` 側 event 列観測などのいずれを採用するかも実装に委ねる）。完了基準そのものは付加情報の有無に依存しない。

### 境界

- **CLI と engine の直接 IPC 不在境界**: CLI は engine プロセスとソケット / pipe 等で直接通信しない。仲介は pending command store の file のみ。これにより [05] の file-direct 構成と対称になり、CLI はデスクトップアプリ非稼働でも書き込みまで完遂できる。
- **typed boundary 共有境界**: CLI 経路で組み立てる command は `WorkflowCommand::ApproveNode` / `RejectNode` / `AbortRun` の既存 3 variant に閉じる。CLI 経路のための新規 variant 追加や engine 内部 typed 化（`CompleteNode` / `FailNode`）への波及はしない（[05] 完了境界の温存）。internal-only variant の到達拒否境界は [05] のまま。**`AbortRun` / pending payload / dispatcher adapter は node 限定 abort（対象 node の任意限定を伴う中止要求）を表現できる前提とし、abort の `--node` 限定を pending payload から `AbortRun` 受理までエンドツーエンドで保持する**。`AbortRun` 既存 variant が node 限定を表現できない場合は本前提を満たすよう拡張する（CLI 経路のための新規 variant 追加ではなく、既存 variant の表現範囲調整として扱う）。
- **engine 認可 / 冪等性 / stale target 判定の経路非依存境界**: CLI 経由でも UI 経由でも、engine 視点では同一 `dispatch_external` を経由するため、認可・冪等性・stale target 判定が経路に依らず同一に適用される。CLI 経路独自の認可層や stale 判定は導入しない。
- **CLI 認証境界（[05] 踏襲）**: CLI は同一デバイス所有者の OS user 権限下で動作する前提とし、pending command store の OS ファイル権限に依拠する。CLI 専用の追加認証層は導入しない。リモートセッション経由の CLI mutating は本 issue 対象外。
- **CLI 入力の信頼境界（[05] 踏襲）**: CLI が caller から受け取る `run-id` / `--node` / `--reason` / `--comment` のみを外部入力として扱い、書式バリデーションを経た上で pending payload に詰める。pending command store の file 内容は engine-owned かつ同一デバイス内の信頼済み入力として watcher / dispatcher adapter は扱う（書き手は CLI 自身、読み手は watcher）。
- **一度だけ処理境界**: dispatcher adapter で pending entry を「処理中→処理済み」マーキングする境界の中で重複 dispatch を遮断する。watcher の重複発火、同一 file の再検知、アプリ再起動時の resume はこの境界の内側に閉じる。
- **TTL / cleanup 境界**: 古い未処理 pending command は TTL に従い除外される。除外判定の生起点は (a) 起動時 cleanup、(b) watcher pickup 時の age check、のいずれか／両方を取りうる（実装に委ねる）。除外された pending entry は engine に到達しない。
- **mutation 要求の観測経路境界**: 「CLI からの mutation 要求の事実」を `WorkflowEvent` の新規 typed event variant として追記する。既存の `ApprovalResolved` / `RunAborted` 等は state 変化が engine に受理された後の事実を記録する役割を保つ。両者は別 variant として併存させ、観測経路（[05] read-only API / CLI）から透過的に読める。
- **既存 UI 経路温存境界**: `approve_workflow_step` / `abort_workflow` Tauri command と既存フロントエンド invoke 経路は破壊しない。CLI 経路の追加によって既存 UI 動作に regression を起こさないことをコードレビュー / テストで担保する。
- **CLI 完了基準境界**: CLI 完了基準は呼び出し経路（稼働中 / 非稼働中）に依らず「受理キュー投入まで」で固定する。engine 到達観測は付加情報として CLI 側のみで管理し、engine 側にこの状態は持ち込まない（engine 視点では「dispatch されたか／まだか」のみで、queue 概念や CLI 完了判定の文脈は持たない）。
- **scope の境界**: 本 issue は CLI 入口の追加（approve / reject / abort）と、その経路を支える pending command store / watcher 接続 / 「CLI 要求の事実」typed event 追加に閉じる。新規 run 起動 CLI（plan doc [06] 外）、structured output 提出 CLI（[08]）、Workflow Panel UI（[07]）、bash node ランタイム（[13]）、main-agent narrator（[16]）は対象外。

### 実装に委ねること

- pending command store のディレクトリ配置（`workflow_runs/` 配下にネストするか、`workflow_pending/` などの兄弟ディレクトリにするか）と file 形式（JSON / NDJSON / 単発 file 単位など）。
- pending payload の typed shape（subtype enum、`requested_at` の値、`node_name` の Optional 表現、reject `reason` のフィールド名など）。
- 「処理済み」マーキング手段（pending file の削除、専用 marker file、`processed/` ディレクトリへの移動、advisory lock などのいずれを選ぶか）。
- 重複 dispatch 防止の同時実行制御（OS atomic rename、advisory lock、in-memory mutex のいずれを採用するか）。
- TTL の具体値および cleanup 起動点（起動時のみ／watcher pickup 時のみ／両方）。
- 「dispatch まで完了したか／キュー投入までか」を CLI が判定する手段（app 起動 marker file、pid file、書き込み後の短時間 ack file 待ち、`workflow_runs/` 側の event 列観測などのいずれを採用するか）。
- 新規 `WorkflowEvent` variant の名前と shape（経路種別 enum を内包するか、variant 自体を経路別に分けるか、要求対象 node 名や reason / comment の格納粒度）。
- 新規 event variant が既存 NDJSON 互換境界（[05] 観測経路）に影響しないことの検証粒度。
- CLI サブコマンドの引数仕様（`--node` の long/short、`--reason` 必須化の `clap` 表現、`--comment` の任意化、`--json` 出力フラグの一貫性）。
- watcher の debounce / scan 周期、watcher 起動点（既存 `FileWatcherManager` 拡張か、workflow 専用 watcher の新設か）。
- pending payload の `WorkflowCommand` への変換責務をどこに置くか（dispatcher adapter / pending command store の helper / `commands.rs` の adapter 内 inner 関数のいずれか）。
- テスト配置: CLI 引数 parse は既存 `cli/mod.rs` の clap test 群に追加、pending command store / dispatcher adapter は tempdir + 単体テスト、engine dispatch 統合は engine test ハーネス、UI 経路非破壊は既存 `commands.rs` adapter test 群の温存で担保。

