# AgentChat: ストリーミング応答の表示が遅延・行途中で止まる問題を修正

参照: https://github.com/siro33950/releash/issues/970

## 要求

**種別**: バグ修正

**ゴール**:
AgentChat において、エージェントからのストリーミング応答が Frontend に配信された直後に UI へ反映される状態にする。行の途中で表示が止まる現象、およびストリーミング完了後に全文が一気に表示される現象を解消する。

なお、ここでの「即時反映」の基準点は **SDK delta が Rust に到着した瞬間ではなく、Rust 側の集約（coalescing）処理を経て Frontend / Remote が配信を受信した直後** を指す。Rust 側で 33ms 程度の集約を行うことはこのゴールと矛盾しない（[対応方針（概要）](#対応方針概要)参照）。

**背景**:
AgentChat ではエージェントからの Response があっても UI 表示が遅れたり、行の途中までしか表示されないままストリーミング完了時に一気に追いつくケースが頻発している。原因は複数要因の重なりであることが調査で判明している:

1. **`useDeferredValue` による表示停止（主因）**
   `src/components/panels/AgentChatPanel/StreamMessage.tsx:136` で Markdown 描画前に `useDeferredValue` を使用。delta が連続して流れてくる状況では React がほぼ常にビジー状態となり、deferred value が最新に追いつかず古い content をレンダリングし続ける。LLM ストリーミング UI のベストプラクティス（Vercel AI SDK / Chrome 公式等）でも非推奨。

2. **Rust 側で `agent-streaming-updated` をスロットリングなしで emit**
   `src-tauri/src/backends/bridge_common.rs:1672` 付近で SDK からの delta ごとに即 emit している。トークンごとに `SET_STREAMING_MESSAGE` が発火し、`activeSession` ツリー全体が再レンダリング、Markdown が再パースされる。

3. **`StreamMessage` がメモ化されていない**
   親の再レンダリングに連動し、content が変わらないメッセージも再描画される。

## 現在の挙動

- Response の delta が届いても UI 反映が遅れる
- 行の途中で表示が止まる
- ストリーミング完了後に全文が一気に表示されるケースがある

## 期待する挙動

- ストリーミング中、Frontend / Remote が配信を受信した直後に UI に反映される
- 行の途中で表示が止まる現象が発生しない
- ストリーミング完了を待たずに進行中の文字列が反映される

## 再現手順

1. AgentChat パネルでエージェントとの対話を開始する
2. 長文の応答を生成させる（複数行の Markdown を含む応答）
3. ストリーミング中の UI 描画を観察する → 遅延・停止・完了時の一気表示が発生する

## 対応方針（概要）

業界ベストプラクティス（Vercel AI SDK の `experimental_throttle`、ChatGPT の RAF バッファリング等）に沿う。「ロジックは Rust 側に置く」方針（`rust-first-logic.md`）に従い、throttle は Rust 側で実装する。

1. `useDeferredValue` を削除（主因の直接除去）
2. Rust 側で `agent-streaming-updated` / `agent_stream_sync`（WS 経由）の emit を coalescing（`STREAMING_EMIT_INTERVAL_MS = 33` ≈ 30fps 間隔）
   - `AgentProcess` に pending delta buffer と last emit 時刻を持たせる
   - pending buffer に蓄積するのは既存パーサ通過済みの `MessagePart` 列のみ。件数上限（1000 件）と総 byte 上限（`STREAMING_PENDING_BYTE_LIMIT`）を設け、いずれかの上限到達時は次 delta 到着で即 flush する。これらの上限は **通常時は flush 閾値**（上限到達で集約間隔を待たず即 flush）として機能し、**配信失敗中のみ上限超過を許容して保持**する（ハード上限ではない）
   - 初回 delta（`last_emit_at` が未設定）は即 flush し、`last_emit_at` をセットする
   - 間隔未満は溜め込み、間隔到達時に flush
   - flush トリガーは「次 delta 到着時に経過時間が 33ms 以上なら flush」と「補助タイマー（33ms 周期）で pending があれば flush」の併用とし、無音期間中の pending 配信も保証する
   - turn_complete / state 遷移時は強制 flush
   - emit は `emit_streaming_parts`（Tauri event `agent-streaming-updated` と WS `AgentStreamSync` の同時送信）を best-effort で行い、`Result` 化はしない。emit 失敗時（Tauri event / WS いずれかが失敗、または両方が失敗）でも pending と `last_emit_at` を破棄せず保持し、次の flush 契機で再試行する
   - 配信ペイロードである `streaming_parts` は **累積置換型**（毎回そのメッセージの全パーツ列を送る）であり、Frontend / Remote 側は受信内容で該当メッセージ状態を完全置換する。これにより、Tauri event / WS のいずれか片方だけが成功した場合でも、次 flush で両チャネルへ同じ累積 parts を再送すれば、成功済みチャネル側でも再受信が単純な置換となり、重複表示・重複適用は発生しない。`sequence_id` 等の追加重複排除は不要
3. `StreamMessage` を `React.memo` 化（同一 content での無駄な再パースを抑止）

ブロック単位（marked.lexer）の memo 化は、上記 3 つを実施した上で必要性を判断する。

## 振る舞い定義

```gherkin
Feature: AgentChat ストリーミング応答の即時反映

  Rule: ストリーミング応答の配信は一定間隔で集約する

    Scenario: ストリーミング開始直後の応答は即座に配信される
      Given ストリーミングが開始されたばかりで、まだ一度も配信していない
      When 最初の応答が到着する
      Then その応答は即座にフロントエンドへ配信される

    Scenario: 集約間隔内に到着した応答は未配信バッファに蓄積される
      Given 直前の配信から集約間隔が経過していない
      When 新しい応答が到着する
      Then その応答は未配信バッファに蓄積され、まだフロントエンドへは配信されない

    Scenario: 集約間隔経過後の応答到着時にまとめて配信される
      Given 未配信バッファに応答が蓄積されている
      And 直前の配信から集約間隔が経過している
      When 新しい応答が到着する
      Then 未配信バッファの内容と今回の応答をまとめてフロントエンドへ配信する

    Scenario: 無音期間中も未配信バッファは自動的に配信される
      Given 未配信バッファに応答が蓄積されている
      And 新しい応答が到着しないまま集約間隔以上が経過する
      Then 未配信バッファの内容がフロントエンドへ自動的に配信される

    Scenario: 未配信バッファが空のときは自動配信を発火しない
      Given 未配信バッファが空である
      When 自動配信のタイミングが到来する
      Then フロントエンドへの配信を行わない

    # 注: 総バイト数の境界条件を検証するテストは、実装定数
    # `STREAMING_PENDING_BYTE_LIMIT` を参照して上限到達条件を生成すること
    # （[実装に委ねること](#実装に委ねること) 参照）。
    Scenario Outline: バッファ容量上限到達時は集約間隔を待たず即配信する
      Given 未配信バッファが <上限種別> の上限に達している
      When 新しい応答が到着する
      Then 集約間隔の経過に関わらず、未配信バッファと今回の応答をまとめて即配信する

      Examples:
        | 上限種別   |
        | 件数       |
        | 総バイト数 |

    Scenario: 配信失敗時は未配信バッファと最終配信時刻を保持して再試行する
      Given 未配信バッファに応答が蓄積されている
      When フロントエンドへの配信に失敗する
      Then 未配信バッファの内容を破棄せず保持する
      And 最終配信時刻を更新しない
      And 次回の配信タイミングで再試行する

    Scenario: 上限到達中に配信が失敗しても新しい応答はバッファに追加する
      Given 未配信バッファが容量上限に達している
      And 直前の配信が失敗している
      When 新しい応答が到着する
      Then その応答も未配信バッファに追加する（上限超過を許容する）
      And 次の配信タイミングで再試行する

    Scenario: 配信チャネルの片方だけが失敗しても次 flush で両チャネルに同じ累積を再送する
      Given Tauri event と WS `AgentStreamSync` の一方が成功し、もう一方が失敗した直前の flush がある
      And `streaming_parts` の累積パーツ列は両チャネル向けに同一である
      When 次の flush 契機が到来する
      Then 両チャネルに対して同じ累積 `streaming_parts` を送信する
      And 成功済みチャネル側でも受信内容で該当メッセージ状態を完全置換するため、重複表示・重複適用は発生しない

    Scenario: 配信失敗時の警告ログには応答本文を含めない
      Given 配信に失敗した状況である
      When 警告ログが出力される
      Then ログには応答本文・ツール入出力・画像・メンションなどのユーザーデータを含めない
      And ログにはイベント名・件数・バッファ長・エラー種別などの非本文メタデータのみを含める

  Rule: ターン完了・状態遷移時には未配信バッファを強制配信する

    Scenario: ターン完了時に未配信バッファを強制配信する
      Given 未配信バッファに応答が残っている
      When エージェントのターンが完了する
      Then 残っている応答を即座にフロントエンドへ配信する

    Scenario Outline: ストリーミングに関わる状態遷移時に未配信バッファを強制配信する
      Given 未配信バッファに応答が残っている
      When <遷移> が発生する
      Then 残っている応答を即座にフロントエンドへ配信してから状態を遷移させる

      Examples:
        | 遷移                          |
        | ストリーミング → アイドル     |
        | ストリーミング → 権限待ち     |
        | 権限待ち → ストリーミング     |
        | ストリーミング → 実行準備完了 |
        | ストリーミング → クラッシュ   |
        | ツール実行の開始              |
        | ツール実行の終了              |

    Scenario: 強制配信が失敗しても後続の状態遷移は続行する
      Given 状態遷移またはターン完了に伴う強制配信を実行している
      When フロントエンドへの配信に失敗する
      Then 未配信バッファを保持したまま、状態遷移・完了通知などの後続処理は続行する

    Scenario: 配信済みの状態で再度強制配信が呼ばれても二重配信は発生しない
      Given 直前の強制配信で未配信バッファを全て配信済みである
      When 同じ契機で再度強制配信が呼ばれる
      Then 配信は発火せず、同じ応答が再配信されることはない

  Rule: WS `AgentStreamSync` の配信先は該当 chat_session に認可されたセッションのみとする

    Scenario: 認証済みかつ該当 chat_session を購読するセッションのみが配信を受信する
      Given Rust が `AgentStreamSync` を flush する
      And `chat_session_id` が S である
      When 配信が WS ブロードキャスターを通る
      Then 認証済みで chat_session S に対する閲覧権限を持つ WS セッションのみが配信を受信する
      And それ以外（未認証セッション・別 chat_session を見ているセッション）には配信されない

    Scenario: 認証されていないセッションには配信されない
      Given 認証が完了していない WS セッションが存在する
      When `AgentStreamSync` の配信が発生する
      Then 当該未認証セッションには配信メッセージが届かない

  Rule: ストリーミング応答は受信した直後に画面へ反映する

    Scenario: 配信受信時に該当メッセージの表示が更新される
      Given AgentChat パネルでストリーミング中のメッセージが表示されている
      When フロントエンドがストリーミング配信を受信する
      Then 配信受信を起点に同一 React render サイクル内（または次の microtask 完了時点）で該当メッセージの DOM ノードに新しい content が反映される

    Scenario: Remote が `AgentStreamSync` を受信した直後に対応メッセージ状態と表示が即時更新される
      Given Remote の AgentChat 相当画面でストリーミング中のメッセージが表示されている
      When Remote が WS `AgentStreamSync` を受信する
      Then 受信を起点に同一 React render サイクル内（または次の microtask 完了時点）で、該当メッセージの状態が受信した累積 `streaming_parts` で完全置換される
      And その置換結果が DOM ノードに即時反映される

    Scenario: 連続する応答に対して行の途中で描画が止まらない
      Given 長文の Markdown 応答をストリーミングで受信している
      When N 回（N ≥ 2）の配信が連続して到着する
      Then N 回それぞれに対応する content 変化が DOM に反映され、最後の配信を待たずに途中の content も観測できる

    Scenario: メッセージ内容が同一であれば再描画されない
      Given AgentChat に複数のメッセージが表示されている
      When 親コンポーネントが再レンダリングされるが、特定メッセージの表示内容（本文・役割・画像・メンション）が全て同一である
      Then そのメッセージは再描画されない

## アーキテクチャ概要

### 責務配置

- **Rust: `AgentProcess` / `bridge_common.rs` の stdout reader**
  - 担当する: SDK delta の受信、`streaming_parts` への accumulate、配信間隔の判定、未配信 delta のバッファリング、間隔到達/turn_complete/状態遷移時の flush（`emit_streaming_parts` 呼び出し）、最終配信時刻の保持。
  - 担当しない: UI 描画タイミングの判断、Markdown 解析、メッセージの差分計算（再パース）。

- **配信チャネル: `agent-streaming-updated`（Tauri event、デスクトップ向け） / `AgentStreamSync`（WebSocket、Remote 向け）**
  - 担当する: Rust → Frontend / Remote の delta 配信チャネル（既存）。`emit_streaming_parts` が両者を同時に発火する。WS `AgentStreamSync` は **該当 `chat_session_id` に対して認証済みかつ閲覧権限を持つセッションにのみ** 配信される（既存の WS 認可機構に乗る）。
  - 担当しない: イベントの間引きや結合（その判断は Rust 側に集約）。coalescing / flush / pending buffer の対象は Tauri event と WS `AgentStreamSync` の両方。

- **Frontend: `useAgentSdkListeners` / `agentChatReducer`（デスクトップ） / Remote の `agent_stream_sync` ハンドラ**
  - 担当する: イベント受信 → `SET_STREAMING_MESSAGE` 発火、reducer による activeSession への反映、Remote 側の対応する状態更新。
  - 担当しない: 配信間隔・throttle・debounce のような時間ベースの制御（`useDeferredValue` 含む）。

- **Frontend: `StreamMessage` コンポーネント**
  - 担当する: 受け取った content の Markdown 描画、同一 props での再描画抑止（`React.memo`）。
  - 担当しない: content の遅延適用、描画スケジューリング、ロジック層への問い合わせ。

### データ/通信フロー

- **delta 到着時（集約中）**: SDK delta → stdout reader → `AgentProcess.streaming_parts` 更新 → 集約間隔未到達なら未配信 delta をバッファに保持して終了。
- **delta 到着時（集約間隔到達）**: バッファ＋今回 delta をまとめて `emit_streaming_parts` → `agent-streaming-updated`（Tauri event） と `AgentStreamSync`（WS）を同時発火 → デスクトップ側 `useAgentSdkListeners` で `SET_STREAMING_MESSAGE` dispatch / Remote 側 `agent_stream_sync` ハンドラで対応状態を更新 → reducer 更新 → `StreamMessage` 再描画（`content` / `role` / `images` / `mentions` の 4 props 浅比較で差分のあるメッセージのみ）。**emit が成功した場合のみ `last_emit_at` を現時刻に更新する。emit 失敗時は pending と `last_emit_at` を保持し、次の flush 契機で再試行する**（[振る舞い定義](#振る舞い定義) "配信失敗時は未配信バッファと最終配信時刻を保持して再試行する" Scenario と整合）。
- **turn_complete / 実行状態遷移時**: バッファに残った delta を強制 flush（即 emit）→ 通常イベント（`turn_complete` 処理 / 状態通知）を続行。flush が失敗した場合も pending を保持したまま後続処理は続行（best-effort）。

### 状態 Owner

- **未配信 delta バッファ**: `AgentProcess`（Rust）。件数上限・総 byte 上限は **通常時は flush 閾値**（上限到達で集約間隔を待たず即 flush）として扱い、**配信失敗中のみ上限超過を許容して保持**する（ハード上限ではない）。
- **最終配信時刻 (`last_emit_at`)**: `AgentProcess`（Rust）。
- **集約間隔（定数）**: `bridge_common.rs` 内の定数（Rust）。
- **`streaming_parts`（累積パーツ列）**: `AgentProcess`（既存、変更なし）。
- **`turn_phase` / `BridgeState`**: `AgentProcess`（既存、変更なし。flush タイミングの判定に使用）。
- **session 内ストリーミングメッセージ**: `agentChatReducer` の activeSession（Frontend、既存）。
- **Remote 側 session 内ストリーミングメッセージ**: Remote の `agent_stream_sync` ハンドラが管理する session 状態（Remote、既存）。受信した累積 `streaming_parts` で該当メッセージを完全置換する。
- **メッセージ描画状態**: `StreamMessage` の props（content/role/images/mentions のみ）。

### 境界

#### 責務の境界

- 時間制御は Rust 側 `AgentProcess` に集約する。Frontend / Remote は時間制御を持たず、受信したら即時反映する。
- `streaming_parts`（累積） と「未配信 delta」（差分バッファ）は別概念として保持する。前者は永続化や consolidate の入力、後者は emit 制御のための一時バッファ。
- 強制 flush 経路は emit 順序を保つこと（バッファ flush → 状態通知 → 後続処理）。逆順だと Frontend が古い content で完了表示する。flush が失敗しても順序保証のため後続処理は続行する。
- `StreamMessage` の memo 化は props を浅く比較できる形に保つ。親側で毎回新規参照になる props を渡さない（必要に応じて呼び出し側で安定化する）。

#### 信頼境界

- SDK 応答に含まれる **エージェント応答本文・ツール入出力・画像・メンション** は、すべて **外部由来のユーザーデータ** として扱う。Releash は内容を解釈・実行・整形（Markdown 描画含む）の対象とするだけで、信頼済み入力とは見なさない。
- これらの外部由来データは、ログ・テレメトリ・エラーレポートに本文として載せない（[振る舞い定義](#振る舞い定義) "配信失敗時の警告ログには応答本文を含めない" Scenario と整合）。出力できるのは event 名・件数・バッファ長・エラー種別などの非本文メタデータに限る。
- WS `AgentStreamSync` の配信先は該当 `chat_session_id` に対して認証済みかつ閲覧権限を持つセッションに限定する（既存 WS 認可機構に乗る）。未認証セッション・別 chat_session を見ているセッションには配信しない。

### 実装に委ねること

- 未配信 delta バッファのフィールド名（型は `MessagePart` 列で確定、件数上限 1000 件は Spec で確定、総 byte 上限の定数値 `STREAMING_PENDING_BYTE_LIMIT` の具体値は実装時に決定）。総バイト数上限の境界条件を検証するテストは、ハードコードした数値ではなく実装定数 `STREAMING_PENDING_BYTE_LIMIT` を参照して上限到達条件を生成すること。
- 集約間隔定数の配置場所（名前は `STREAMING_EMIT_INTERVAL_MS`、値は 33 で確定）。
- 補助タイマー（33ms 周期）の具体的な実装手段（tokio interval / 別タスク等）。補助タイマーは `chat_session_id` + `generation_id` に紐付け、flush 時に `AgentProcessMap` 内の generation 一致を確認し、不一致・process removal・crash 時は終了する方針は Spec で確定。
- 状態遷移を検出する具体的なフック点（対象遷移は Scenario で列挙済み）。
- `React.memo` の比較関数の実装（比較対象は `content` / `role` / `images` / `mentions` の浅比較で確定）と props 安定化の手段（必要時のみ）。
- emit 失敗時の警告ログの具体的な出力先・フォーマット（含めるフィールドは event 名・part_count・buffer_len・error 種別など非本文メタデータに限定で確定）。
- 各レイヤーでのテストケースの具体的な配置と粒度（Rust 側：`AgentProcess` ユニット相当 / TS 側：reducer・listener・component の既存テストへの追加）。

## 受け入れ基準

- `agent-streaming-updated` 受信時に該当メッセージの DOM が更新される
- delta が連続到着している間、行の途中で表示が止まる現象が解消され、ストリーミング完了を待たずに進行中の文字列が反映され続ける
- `pnpm test` が通る
- `cargo test` が通る
- `pnpm lint` が通る
- `cargo clippy -- -D warnings` が通る

## 参考

- [Next.js: Markdown Chatbot with Memoization (Vercel AI SDK)](https://ai-sdk.dev/cookbook/next/markdown-chatbot-with-memoization)
- [Best practices to render streamed LLM responses (Chrome for Developers)](https://developer.chrome.com/docs/ai/render-llm-responses)
- [Streaming Backends & React: Controlling Re-render Chaos (SitePoint)](https://www.sitepoint.com/streaming-backends-react-controlling-re-render-chaos/)
- [How To Build a Performant AI Markdown Renderer](https://tigerabrodi.blog/how-to-build-a-performant-ai-markdown-renderer)
