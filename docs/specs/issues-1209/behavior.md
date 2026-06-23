# Behavior

Performance budget / telemetry（#1209）の振る舞いを Gherkin で定義する。

実装経路・モジュール名は本書に持ち込まず、外部から観測可能なビジネスルールに絞る。詳細な配置・技術選定は `design.md` で扱う。

## 用語

- **計測メトリクス**: 要求スコープに列挙した hot path の span / metric（startup、Git/Diff、AgentSession streaming、AgentSession session IO、リソース観測）。
- **送信先設定**: テレメトリ送信先を確定するための設定（OTLP endpoint と license key）。両方が揃って初めて「設定済み」とする。
- **送信同意フラグ**: release ビルドで New Relic への送信可否を切り替えるユーザー設定。性能テレメトリは既定 on（送信する）で、opt-out 操作で停止する。
- **共通 resource attribute**: 全 span / metric に付与する比較軸。app version / OS / build type（dev または release）。
- **ユーザーデータ**: 本文、tool 入出力、worktree path 等の利用者固有情報。
- **crash / エラー**: Rust の panic・未捕捉エラー、および frontend（WebView）の未捕捉エラー / unhandled rejection。
- **crash 送信同意フラグ**: crash / エラーを New Relic へ送るかを切り替えるユーザー設定（`crash_reporting`）。既存 Sentry の挙動を踏襲し、既定 opt-in（送信する）とする。性能テレメトリの送信同意フラグ（既定 opt-out）とは別フラグ。

## Feature: 主要 hot path の性能・メモリテレメトリ

要求スコープの hot path を計測し、ベンダ中立な計装で New Relic に送信できる。改善前後を比較できる共通指標を提供する。

### Background

```gherkin
Given アプリが起動している
And 計測対象の hot path（startup / Git・Diff / AgentSession streaming / AgentSession session IO / リソース観測）が実装されている
```

### Rule: debug/dev ビルドでは列挙メトリクスを送信して確認できる

```gherkin
Scenario: dev ビルドで送信先が設定済みなら全メトリクスを確認できる
  Given ビルド種別が debug/dev である
  And 送信先設定（OTLP endpoint と license key）が設定済みである
  When 計測対象の hot path を一通り実行する
  Then 列挙した全メトリクス（JS heap を除く）が New Relic 上で確認できる
  And 各メトリクスに共通 resource attribute（app version / OS / build type）が付与されている
```

```gherkin
Scenario Outline: 各 hot path のメトリクスが取得できる
  Given ビルド種別が debug/dev である
  And 送信先設定が設定済みである
  When "<hot path>" を実行する
  Then "<メトリクス>" が New Relic 上で確認できる

  Examples:
    | hot path                    | メトリクス                                       |
    | アプリ起動                  | startup time / first window ready / first repo snapshot ready |
    | Git status scan             | Git status scan duration                         |
    | diff stats                  | diff stats duration                              |
    | review file open            | review file open duration                        |
    | agent 応答ストリーミング    | event payload size / WS payload size / emit interval / dropped frame count / WS reconnect count |
    | session 一覧取得            | session list duration                            |
    | session 取得                | session get duration                             |
    | session 保存                | session save duration / session save bytes       |
    | リソース観測                | プロセス RSS / プロセス CPU / mounted xterm count / active PTY count |
```

### Rule: release ビルドでは既定で送信し、opt-out で停止する

A6 に従い、release ビルドの性能テレメトリは既定 on（送信する）とする。payload は完全匿名（永続識別子なし）であり、ユーザーは設定 UI から opt-out で停止できる。

```gherkin
Scenario: release ビルドの既定状態では送信する
  Given ビルド種別が release（配布）である
  And 送信先設定が設定済みである
  And ユーザーが送信同意フラグを明示的に変更していない
  When 計測対象の hot path を実行する
  Then テレメトリが New Relic へ送信される
```

```gherkin
Scenario: release ビルドで opt-out すると送信しない
  Given ビルド種別が release（配布）である
  And 送信先設定が設定済みである
  And ユーザーが送信同意フラグを opt-out に設定している
  When 計測対象の hot path を実行する
  Then テレメトリは New Relic へ送信されない
```

```gherkin
Scenario: 送信同意フラグを設定から切り替えられる
  Given ビルド種別が release（配布）である
  When ユーザーが送信同意フラグを opt-out に切り替える
  Then 以後の hot path 実行でテレメトリは送信されない
  And 再び既定（on）に戻すと送信が再開する
```

### Rule: 送信先未設定でも計装は no-op として安全に動作する

```gherkin
Scenario: 送信先設定が空ならアプリは正常起動する
  Given 送信先設定（OTLP endpoint または license key）が空である
  When アプリを起動する
  Then アプリは正常に起動する
  And 通常動作（編集 / Git / agent チャット / terminal）が妨げられない
  And テレメトリは送信されない
```

```gherkin
Scenario: 送信先未設定でも hot path は正常動作する
  Given 送信先設定が空である
  When 計測対象の hot path を実行する
  Then 計測コードは no-op として素通りし、機能の結果は通常どおり得られる
```

### Rule: ユーザーデータを attribute に含めない

```gherkin
Scenario: span / metric の attribute にユーザーデータが含まれない
  Given 送信先設定が設定済みである
  When agent チャット応答 / Git 操作 / session 保存などの hot path を実行する
  Then 送信される span / metric の attribute に本文・tool 入出力・worktree path 等のユーザーデータが含まれない
```

```gherkin
Scenario: payload に横断追跡可能な識別子が含まれない
  Given テレメトリ送信が行われる状態である
  When 任意の span / metric / crash が送信される
  Then payload に個人を特定する識別子（氏名・メール・OS アカウント名等）が含まれない
  And 永続的なデバイス/インストール識別子（MAC ハッシュ・install UUID・OS マシン UUID 等）が含まれない
  And 起動セッション ID（プロセス単位の UUID）が含まれない
  And 計測は build（app version）単位で集計でき、同一ユーザー・同一マシン・同一起動の横断追跡はできない
```

```gherkin
Scenario: OTel 既定の trace/span ID は許容される
  Given テレメトリ送信が行われる状態である
  When 任意の span が送信される
  Then trace ID / span ID は OTel 既定どおり付与される
  And これらは 1 操作の親子関係追跡に限定され、起動間・マシン間で関連付け不能である
```

```gherkin
Scenario: 共通 resource attribute は限定された 4 つのみ
  Given テレメトリ送信が行われる状態である
  When 任意の span / metric が送信される
  Then 共通 resource attribute は service.version / os.type / releash.build_type / service.name の 4 つのみである
  And CPU model / 画面解像度 / locale 等のフィンガープリント材料が含まれない
```

### Rule: 共通 resource attribute でビルド間・環境間を比較できる

```gherkin
Scenario: 全メトリクスに比較軸が付与される
  Given テレメトリ送信が行われる状態である
  When 任意の span / metric が発行される
  Then その span / metric には app version / OS / build type が付与されている
```

```gherkin
Scenario: ビルド間でメトリクスを比較できる
  Given 異なる app version のビルドからテレメトリが送信されている
  When New Relic 上で同一メトリクスを app version で分けて参照する
  Then ビルドごとの値を比較できる
```

### Rule: 主要 hot path の成功 / 失敗を区別できる

```gherkin
Scenario Outline: hot path の操作 status を計測する
  Given 送信先設定が設定済みである
  When "<操作>" が "<結果>" で完了する
  Then その操作の status が "<結果>" として New Relic 上で区別できる

  Examples:
    | 操作              | 結果   |
    | Git status scan   | 成功   |
    | Git status scan   | 失敗   |
    | diff stats        | 成功   |
    | diff stats        | 失敗   |
    | review file open  | 成功   |
    | review file open  | 失敗   |
    | session list      | 成功   |
    | session list      | 失敗   |
    | session get       | 成功   |
    | session get       | 失敗   |
    | session save      | 成功   |
    | session save      | 失敗   |
```

```gherkin
Scenario: 失敗増を副作用として検知できる
  Given ある hot path の改善前後でテレメトリが送信されている
  When 改善後に当該 hot path の失敗が増加する
  Then New Relic 上で失敗 status の増加として観測できる
```

### Rule: streaming は通常 frame と累積 snapshot を区別して観測できる

```gherkin
Scenario: 累積 snapshot は reconnect 時のみ送信される方針を検証できる
  Given agent 応答のストリーミングが行われている
  When WS 接続が再確立（reconnect）される
  Then WS reconnect count が増加する
  And dropped frame count と累積 snapshot 送信の関係を New Relic 上で観測できる
```

## Feature: 性能予算のドキュメント化

主要 hot path の初期性能予算（M0 案）を Spec / ドキュメントに固定する。

### Rule: 主要 hot path に初期性能予算が文書化されている

```gherkin
Scenario Outline: 性能予算が記載されている
  Given M0 の性能予算を確定する
  When 性能予算ドキュメント（本 Spec ないし releash-performance-architecture-audit.md）を参照する
  Then "<hot path>" の予算 "<予算>" が記載されている

  Examples:
    | hot path        | 予算                                             |
    | app startup     | orphan cleanup による visible startup block なし  |
    | repo snapshot   | 中規模 repo で 200ms 台を目標、長い scan は stale snapshot を返す |
    | file diff open  | 小/中ファイル 500ms 未満、大ファイルは即 fallback 表示 |
    | streaming event | 通常 frame payload は 64KB 未満、累積 snapshot は reconnect 時のみ |
    | session list    | session 本文量に依存しない                       |
    | terminal        | worktree あたり mounted xterm 数に上限           |
```

## Feature: crash / エラー監視の New Relic 統合

現状 Sentry が担う crash / エラー監視を New Relic + OpenTelemetry へ統合する。観測先のみ移し、ユーザーの crash 送信同意の既定値（opt-in）は変更しない。

### Rule: crash / エラーを New Relic（Errors Inbox）で観測できる

```gherkin
Scenario Outline: crash / エラーが New Relic で観測できる
  Given 送信先設定が設定済みである
  And crash 送信同意フラグが opt-in である
  When "<発生源>" で "<事象>" が起きる
  Then その crash / エラーが New Relic（Errors Inbox）で観測できる

  Examples:
    | 発生源        | 事象               |
    | Rust          | panic              |
    | Rust          | 未捕捉エラー       |
    | frontend      | 未捕捉エラー       |
    | frontend      | unhandled rejection |
```

### Rule: crash 送信は crash 送信同意フラグに従う

```gherkin
Scenario: 既定（opt-in）では crash を送信する
  Given crash 送信同意フラグを明示的に変更していない
  And 送信先設定が設定済みである
  When Rust の panic が発生する
  Then crash が New Relic へ送信される
```

```gherkin
Scenario: crash を opt-out にすると送信しない
  Given 送信先設定が設定済みである
  And ユーザーが crash 送信同意フラグを opt-out に設定している
  When crash / エラーが発生する
  Then crash / エラーは New Relic へ送信されない
```

### Rule: 送信先未設定では crash も送信しない（no-op）

```gherkin
Scenario: 送信先設定が空なら crash も送信しない
  Given 送信先設定（OTLP endpoint または license key）が空である
  When crash / エラーが発生する
  Then crash / エラーは送信されず、アプリの動作は妨げられない
```

### Rule: `app_started` 相当の利用イベントが New Relic に統合される

```gherkin
Scenario: app_started が startup span として New Relic で観測できる
  Given 送信先設定が設定済みである
  And 送信同意フラグが on である
  When アプリが起動する
  Then 起動完了が New Relic 上で startup span / metric として観測できる
  And 既存 aptabase 経路（tauri-plugin-aptabase / app_started event）は使われない
```

### Rule: crash / エラーにユーザーデータを含めない

```gherkin
Scenario: crash の stacktrace にユーザーの絶対パスが含まれない
  Given crash 送信同意フラグが opt-in で送信先設定が設定済みである
  When stacktrace を伴う crash / エラーが送信される
  Then 送信内容の filename / path 等にユーザーの絶対パスが含まれず scrub されている
  And 本文・tool 入出力等のユーザーデータが含まれない
```

## 非スコープ（振る舞いとして定義しない）

- WebView JS heap の計測（WKWebView 制約のため除外、メモリ観測は native RSS に一本化）。
- 計測値に基づく実際の性能改善・最適化（#1210 以降）。
- 予算違反時のアラート / CI ゲート化。
- UI 体感メトリクス（long task / input latency / frame rate 等）。

## 仮定

- requirements の A1〜A7 を前提とする。特に A6 に従い、release ビルドの性能テレメトリ送信同意フラグの既定値は **on（送信する）** とし、payload は完全匿名（要求事項 5）。ユーザーは opt-out で停止できる。
- A7 に従い、crash 送信同意フラグ（`crash_reporting`）の既定値は **opt-in**（送信する）とし、性能テレメトリとは別フラグで制御する。観測先のみ Sentry から New Relic へ移す。
- 「設定済み」とは OTLP endpoint と license key の両方が揃った状態を指す。いずれかが空なら no-op とする（A4）。
- build type は debug/dev と release（配布）の 2 区分とし、`cfg!(debug_assertions)` に対応する（A3）。
- 性能予算ドキュメントの正本配置（本 Spec か `docs/releash-performance-architecture-audit.md` か）は design ステップで確定する。本書では「いずれかに記載されている」ことを振る舞いとする。

## Open Questions

なし。
