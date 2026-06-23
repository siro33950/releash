# Behavior

Issue #1259: WorkspaceList のステータス表示・集約ロジック再設計の振る舞い定義。

requirements.md の「確定ステータス仕様」を、実装詳細を含まない観測可能な振る舞いとして Gherkin で定義する。

## 用語

- **Step 進行ステータス**: Step そのものの進行状態。`failed / waiting_approval / running / aborted / completed / queued` のいずれか。
- **Session 稼働ステータス**: 1 つの Session の稼働状態。`running`(動作中=streaming) / `waiting`(承認・入力・許可待ち) / `done`(ターン完了) / `error`(実行時エラー) のいずれか。
- **代表ステータス**: 集約後にユーザーへ提示される 7 種の状態。`running / failed / error / waiting / aborted / completed / queued`。
- **優先度**: 代表ステータスの強さの順序。`running > failed > error > waiting > aborted > completed > queued`（左ほど強い＝優先度 1 に近い）。

---

```gherkin
Feature: WorkspaceList のステータス集約と Session 稼働可視化
  WorkspaceList 上で、各 Session が動作中(streaming)か停止中かを判別でき、
  複数 Session を含む Step・複数 Step を含む Workflow の代表ステータスが
  「ユーザーが次に注目すべき状態」になるよう優先度で集約される。

  Background:
    Given WorkspaceList が Workflow / Step / Session を階層表示している
    And 代表ステータスの優先度が "running > failed > error > waiting > aborted > completed > queued" と定義されている

  # =========================================================================
  # Rule 1: Session 1 個の結果ステータス（Step 進行 × Session 稼働）
  # =========================================================================
  Rule: 1 つの Session の結果ステータスは Step 進行ステータスと Session 稼働ステータスの組み合わせで一意に決まる

    Scenario: Session が動作中なら Step 進行を問わず結果は running
      Given Session の稼働ステータスが "running" である
      When その Session の結果ステータスを導出する
      Then Step 進行ステータスが何であっても結果ステータスは "running" になる

    Scenario Outline: Step 進行 × Session 稼働 から結果ステータスを導出する
      Given Step 進行ステータスが "<step>" である
      And その Session の稼働ステータスが "<session>" である
      When その Session の結果ステータスを導出する
      Then 結果ステータスは "<result>" になる

      Examples: Session 稼働 = running（常に running）
        | step             | session | result  |
        | failed           | running | running |
        | waiting_approval | running | running |
        | running          | running | running |
        | aborted          | running | running |
        | completed        | running | running |
        | queued           | running | running |

      Examples: Session 稼働 = waiting
        | step             | session | result  |
        | failed           | waiting | failed  |
        | waiting_approval | waiting | waiting |
        | running          | waiting | waiting |
        | aborted          | waiting | waiting |
        | completed        | waiting | waiting |
        | queued           | waiting | waiting |

      Examples: Session 稼働 = done
        | step             | session | result    |
        | failed           | done    | failed    |
        | waiting_approval | done    | waiting   |
        | running          | done    | running   |
        | aborted          | done    | aborted   |
        | completed        | done    | completed |
        | queued           | done    | queued    |

      Examples: Session 稼働 = error
        | step             | session | result  |
        | failed           | error   | failed  |
        | waiting_approval | error   | error   |
        | running          | error   | error   |
        | aborted          | error   | error   |
        | completed        | error   | error   |
        | queued           | error   | error   |

  # =========================================================================
  # Rule 2: Parallel Step（複数 Session）の集約
  # =========================================================================
  Rule: Step の代表ステータスは配下 Session の結果のうち最も優先度が強いものになる

    Scenario: Parallel Step に動作中の Session が 1 つでもあれば代表は running
      Given Step に複数の Session が属している
      And いずれか 1 つの Session の結果ステータスが "running" である
      When その Step の代表ステータスを集約する
      Then 代表ステータスは "running" になる

    Scenario Outline: 複数 Session の結果から最強優先度を代表ステータスにする
      Given Step 配下の Session 結果ステータスの集合が "<results>" である
      When その Step の代表ステータスを集約する
      Then 代表ステータスは "<representative>" になる

      Examples:
        | results                   | representative |
        | running, waiting, completed | running      |
        | failed, waiting, completed  | failed       |
        | error, waiting, queued      | error        |
        | waiting, completed, queued  | waiting      |
        | aborted, completed, queued  | aborted      |
        | completed, queued           | completed    |
        | queued, queued              | queued       |

    Scenario: 単一 Session の Step は その Session の結果がそのまま代表になる
      Given Step に Session が 1 つだけ属している
      When その Step の代表ステータスを集約する
      Then 代表ステータスはその Session の結果ステータスと一致する

  # =========================================================================
  # Rule 3: Workflow（複数 Step）の集約
  # =========================================================================
  Rule: Workflow の代表ステータスは配下 Step の代表のうち最も優先度が強いものになる

    Scenario Outline: 複数 Step の代表から最強優先度を Workflow 代表にする
      Given Workflow 配下の Step 代表ステータスの集合が "<steps>" である
      When その Workflow の代表ステータスを集約する
      Then 代表ステータスは "<representative>" になる

      Examples:
        | steps                       | representative |
        | running, waiting, completed | running        |
        | completed, failed, queued   | failed         |
        | waiting, error, completed   | error          |
        | waiting, aborted, completed | waiting        |
        | aborted, completed, queued  | aborted        |
        | completed, completed        | completed      |
        | queued, queued              | queued         |

  # =========================================================================
  # Rule 4: 稼働中 / 停止中の視覚的区別
  # =========================================================================
  Rule: WorkspaceList 上で動作中の Session を含む状態と停止系状態を視覚的に区別できる

    Scenario: 動作中の Session を含む Step と全 Session 停止の Step を区別できる
      Given ある Step は配下に "running" の Session を含む
      And 別の Step は配下の全 Session が停止系（waiting / done / error / aborted / completed / queued）である
      When WorkspaceList を表示する
      Then 動作中の Session を含む Step は「動作中(running)」として、もう一方とは視覚的に区別して表示される

    Scenario: 代表ステータスは 7 種に統一される
      When WorkspaceList で代表ステータスを表示する
      Then 表示される代表ステータスは "running / failed / error / waiting / aborted / completed / queued" の 7 種のいずれかである
      And 従来の Step "waiting_approval" と Session の入力・許可待ちは 1 つの "waiting" に統合される

  # =========================================================================
  # Rule 5: リアルタイム更新
  # =========================================================================
  Rule: Session 稼働状態の変化は新たなポーリングを増やさずリアルタイムに反映される

    Scenario: Session が streaming を開始すると WorkspaceList の代表が更新される
      Given WorkspaceList を表示している
      And ある Step の代表ステータスが "queued" である
      When 配下の Session が streaming（稼働 "running"）を開始する
      Then 追加のポーリングなしに、その Step の代表ステータスが "running" に更新される

    Scenario: Session が承認待ちになると waiting が反映される
      Given WorkspaceList を表示している
      And ある Step に動作中の Session が存在しない
      When 配下の Session が承認・入力・許可待ち（稼働 "waiting"）になる
      Then 追加のポーリングなしに、その Step の代表ステータスに "waiting" が反映される
```

---

## 仮定

- (A1) Session の稼働判定は既存の `AgentState`（`running = streaming` / `waiting = 承認・入力・許可待ち` / `done = ターン完了` / `error = 実行時エラー`）を用い、本 Issue では新たな稼働状態は追加しない。
- (A2) Step の進行ステータス（`failed / waiting_approval / running / aborted / completed / queued`）は既存の Step 状態定義を踏襲する。`waiting_approval` は集約後の代表ステータスでは `waiting` に統合される。
- (A3) Workflow / Step / Session の階層構造（Session・Step を選択単位とし、Parallel を 1 Step として扱う #1242 の構造）は変更しない。本 Issue はその上のステータス集約・表示のみを対象とする。
- (A4) 集約対象に含めない Session（クローズ済み等）の扱いは既存の集約ロジックの除外条件を踏襲し、本振る舞い定義の対象外とする。
- (A5) 「視覚的に区別」の具体的なアイコン・色のデザインは表示層の裁量とし、本振る舞い定義では「動作中と停止系が区別できること」のみを要求する。

## Open Questions

なし。
