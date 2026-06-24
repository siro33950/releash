# Behavior — issues-1246

latency telemetry の追加タスクのため、本書は「telemetry を追加した後に外部から観測される振る舞い」を定義する。ここでの「外部から観測される」とは、New Relic 上に現れるメトリクス・次元・ダッシュボードと、turn 実行そのものの観測可能挙動（不変であること）を指す。

ユーザー操作 UI の追加・変更はなく、turn の応答内容や送受信される本文も変化しない。観測可能な差分は「turn の各区間レイテンシが安全な次元付きメトリクスとして記録され、New Relic 上で区間判別できるようになること」と「通常の turn 実行挙動が一切変化しないこと（回帰なし）」に集約される。

実装経路（どのファイル・関数で計測するか、`HotPathMetric` への variant 追加方法等）は振る舞いではないため本書には含めない。実装タッチポイントは `requirements.md` / `design.md` を参照する。

## 用語と仮定

- **turn**: ユーザー送信から Claude の応答が完了するまでの 1 往復。`send_agent_message`（`start_agent_turn`）到達を起点とする。
- **区間（operation）**: 計測対象の時間区間。`ui_to_start` / `bridge_spawn` / `query_init` / `first_sdk_event` / `first_assistant_event` / `permission_wait` / `complete` の 7 種を指す。
- **次元（dimension）**: メトリクスを分解する有界集合の属性。resume 有無 / sessionId 有無 / permission mode / model / context / channel。
- **ユーザーデータ**: 本文、tool 入出力、worktree path、ファイルパス、プロンプト内容など、ユーザー固有・自由文字列の情報。
- **A2（基盤再利用・requirements 由来）**: 既存 telemetry 基盤（`HotPathMetric` / `record_hot_path_duration` / one-shot origin / `PayloadChannel`）を拡張して実装する。
- **A3（評価先・requirements 由来）**: メトリクスは New Relic 上で operation × 次元の p50/p95 として評価する。
- 「従来どおり」「回帰なし」とは、telemetry 追加前に成立していた turn 実行の振る舞いが追加後も同一であることを指す。
- model 正規化の「既知ファミリ」とは Claude のモデルファミリ（例: `opus` / `sonnet` / `haiku`）を指し、判別できないものは `other` に落とす。
- ビルド・テスト・lint・Terraform テストの緑は受け入れ条件（プロセス上の完了条件）であり、本書では `Rule: 成果物・インフラ定義が健全である` の観測点に含める。

## Feature: Claude turn latency telemetry の追加

Releash 上の Claude turn の各区間レイテンシを、ユーザーデータを含めずに安全に計測し、New Relic 上で区間判別・次元分解できるようにする。Phase 1 では計測のみを追加し、turn の体験は変えない。

### Background

```gherkin
Background:
  Given Releash がユーザーの Claude turn を実行できる状態である
  And telemetry 基盤が初期化されている
```

## Rule: turn の各区間レイテンシがメトリクスとして記録される

```gherkin
Scenario Outline: turn 実行時に各区間のレイテンシが記録される
  Given ユーザーが Claude へメッセージを送信する
  When turn が <operation> の区間に到達する
  Then その区間のレイテンシが telemetry メトリクスとして記録される

  Examples:
    | operation             |
    | ui_to_start           |
    | bridge_spawn          |
    | query_init            |
    | first_sdk_event       |
    | first_assistant_event |
    | permission_wait       |
    | complete              |
```

```gherkin
Scenario: first SDK event / first assistant event / turn complete は turn ごとに 1 回だけ記録される
  Given 1 つの turn が実行されている
  When その turn 内で複数の SDK message や assistant event が到着する
  Then first_sdk_event / first_assistant_event / complete は最初の 1 回のみ記録される
  And 同一 turn 内で重複して記録されない
```

```gherkin
Scenario: bridge 側でしか取れない区間は bridge clock で計測され Rust 側で記録される
  Given turn 実行で bridge が query() を作成し subprocess の initialize が完了する
  When bridge がその区間（query_init）を bridge 側 clock で計測する
  Then bridge は本文を含まない telemetry イベントとして計測値を Rust へ転送する
  And Rust 側がその値をメトリクスとして記録する
```

```gherkin
Scenario: permission 待ち時間は通常のモデル応答待ちと別 operation で記録される
  Given turn 実行中に permission request が発生して SDK がユーザー判断を待っている
  When permission 応答が適用され turn が再開される
  Then permission request から permission 応答までの待ち時間が permission_wait として記録される
  And first_assistant_event や complete の latency とは別 operation で比較できる
```

## Rule: 各レイテンシは有界な次元で分解できる

```gherkin
Scenario Outline: 記録されるメトリクスに次元属性が付与される
  Given turn が <context> として実行される
  When 区間レイテンシが記録される
  Then そのメトリクスには resume 有無 / sessionId 有無 / permission mode / model / context / channel の次元が付与される

  Examples:
    | context       |
    | chat          |
    | workflow_step |
```

```gherkin
Scenario Outline: model 次元は既知ファミリへ正規化され、未知は other になる
  Given turn が model <model> で実行される
  When 区間レイテンシが記録される
  Then メトリクスの model 次元は <normalized> になる

  Examples:
    | model                  | normalized |
    | claude-opus-4-8        | opus       |
    | claude-sonnet-4-6      | sonnet     |
    | claude-haiku-4-5       | haiku      |
    | （判別不能な識別子）    | other      |
```

```gherkin
Scenario: channel 次元は既存の有界集合を再利用する
  Given turn が tauri_event または websocket 経由で実行される
  When 区間レイテンシが記録される
  Then channel 次元は既存 PayloadChannel の値（tauri_event | websocket）のいずれかになる

Scenario: すべての次元は有界集合に正規化される
  When 区間レイテンシが記録される
  Then 各次元は事前に定義された有界集合のいずれかの値を取り、自由文字列を含まない
```

## Rule: telemetry にユーザーデータを一切含めない（セキュリティ要件）

```gherkin
Scenario: 記録されるメトリクスは時間と有界次元のみを含む
  When 任意の区間レイテンシが記録される
  Then 記録内容は duration もしくは絶対 ms と、有界集合へ正規化された次元のみである
  And 本文・tool 入出力・worktree path・ファイルパス等のユーザーデータは含まれない

Scenario: bridge から Rust へ転送されるイベントもユーザーデータを含まない
  When bridge が計測イベントを Rust へ転送する
  Then その転送イベントは時間と有界次元のみを含み、本文・tool 入出力を含まない
```

## Rule: 遅延の支配区間を判別できる粒度で分離される

```gherkin
Scenario: 遅延がどの区間にあるか New Relic 上で判別できる
  Given turn latency メトリクスが New Relic に送信されている
  When 開発者が operation 別にレイテンシを比較する
  Then 遅延が bridge spawn / SDK query init / Claude subprocess init / resume / permission 待ち / モデル応答待ちのどこにあるか判別できる

Scenario: permission 待ち・stale watchdog・workflow prompt 増加は通常のモデル応答待ちと区別できる
  Given turn 実行中に permission 待ち / stale watchdog / workflow system prompt 増加が発生しうる
  When レイテンシを評価する
  Then それらに起因する時間が通常のモデル応答待ち時間と区別して識別できる
```

## Rule: Phase 2 の before/after を同一メトリクスで比較できる

```gherkin
Scenario: query 直呼び経路と prewarm 経路を識別できる計測軸になっている
  Given Phase 2 で startup() / WarmQuery による prewarm 経路が導入されうる
  When turn latency メトリクスを記録する
  Then query() 直呼び経路と prewarm 経路を次元または metric で識別でき、同一メトリクスで before/after を比較できる
```

## Rule: 関連 Issue（#1214 / #1195 / #1178）とメトリクスが分離されている

```gherkin
Scenario: turn latency メトリクスは描画・メモリ・停止問題と区別できる名前を持つ
  When 本 telemetry のメトリクスを New Relic 上で参照する
  Then メトリクス名により #1214（描画/transport 負荷）・#1195（WebView メモリ）・#1178（stale/停止）と混同せず評価できる
```

## Rule: New Relic 上で operation × context の p50/p95 として可視化される

```gherkin
Scenario: ダッシュボードに turn latency ウィジェットが存在する
  Given turn latency メトリクスが New Relic に送信されている
  When 開発者が New Relic ダッシュボードを開く
  Then ui_to_start / bridge_spawn / query_init / first_sdk_event / first_assistant_event / permission_wait / complete を operation × context で p50/p95 表示するウィジェットが存在する

Scenario: Rust 側の metric/次元定義と Terraform 定義が一致する
  Given Rust 側が metric 名と次元キー（releash.agent.* 等）を定義している
  When Terraform（locals.tf / dashboards.tf）の NRQL メトリクス名と FACET 属性を参照する
  Then 両者が一致しており、ダッシュボードが実際のメトリクスを参照できる
```

## Rule: 計測追加は通常の turn 実行挙動を変えない（回帰なし）

```gherkin
Scenario: turn の応答内容・送受信本文が変化しない
  Given telemetry 追加後のコードで turn を実行する
  When ユーザーが Claude へメッセージを送信し応答を受け取る
  Then 応答内容・配信される本文・turn の成否は telemetry 追加前と同一である

Scenario: 計測が turn 実行に観測可能な遅延・副作用を持ち込まない
  When turn を実行する
  Then 計測の追加によるユーザー観測可能な遅延や副作用は生じない
```

## Rule: 成果物・インフラ定義が健全である

```gherkin
Scenario: Rust 成果物が緑である
  When `cargo fmt --check` / `cargo clippy -- -D warnings` / `cargo test` を src-tauri/ で実行する
  Then いずれも警告・失敗なく成功する

Scenario: 新規ロジックに単体テストがある
  When 区間計測・次元正規化・bridge 転送のロジックを対象にテストを実行する
  Then それぞれの正常系・境界（model 正規化の未知→other、one-shot の重複抑止等）を検証するテストが存在し成功する

Scenario: Terraform テストが緑である
  When infra/newrelic/ の Terraform テスト（*.tftest.hcl）を実行する
  Then 既存テストが失敗せず成功する
```

## Open Questions

なし（`requirements.md` の Open Questions が解消済みで、スコープが Phase 1（latency telemetry の追加のみ）に確定しているため）。
