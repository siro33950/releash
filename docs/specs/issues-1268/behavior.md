# Behavior

Issue: #1268 「Workflow のステータスをアイコンで表現する」

本書は `requirements.md` の要求を、実装詳細を含まない観測可能な振る舞いとして Gherkin で定義する。対象は **WorkspaceList の Workflow 行（`WorktreeWorkflowRow`）のステータスアイコン表現**に限定する。

## 仮定

requirements.md の仮定を踏襲する。本書のシナリオはこれらを前提とする。

- **仮定A1（配色の流用）**: Workflow 行アイコンの色は、既存 `workflowStepIconClasses` の配色をそのまま用いる。
  - `queued` → ミュート色 / `running` → 青 / `failed` → 赤 / `error` → destructive 色 / `waiting` → 黄 / `aborted` → ミュート色 / `completed` → 緑。
- **仮定A2（pulse 対象）**: `AgentStateIcon` に倣い、進行中／注目が必要な状態である `running` と `waiting` にのみ pulse アニメーションを適用する。それ以外の状態には pulse を適用しない。
- **仮定A3（レイアウト）**: アイコンサイズ・コンテナのレイアウトは現行 Workflow 行の見た目（`size-5` コンテナ内に `size-3` 程度のアイコン）を踏襲する。
- **仮定A4（Step 行の据え置き）**: Workflow 行配下の Step 行は現行表現を維持し、本 Issue では変更しない。
- **仮定A5（代表ステータスの利用）**: 表示に用いる代表ステータスは、Rust 側で導出・配信済みの `WorkspaceWorkflowNode.status` および live 更新値をそのまま用いる。フロントエンドは導出を行わず、色へのマッピングのみを担う。
- **仮定A6（フォールバック色）**: 代表ステータスが既知の `WorkspaceStepStatus` 値のいずれにも該当しない／未確定の場合、`AgentStateIcon` の `null` 時に倣いミュート色（フォールバック色）を用いる。

## Feature: WorkspaceList の Workflow 行ステータスを固定形状アイコン＋色で表現する

WorkspaceList のツリーにおいて、Workflow 行は「Workflow である」ことを常に同一形状のアイコン（lucide `Workflow` アイコン）で識別でき、その代表ステータスは Session 行（`AgentStateIcon`）と同じ方式（同一形状＋色、稼働中は pulse）で表現される。

### Background

```gherkin
Background:
  Given WorkspaceList のツリーに少なくとも 1 つの Workflow 行が表示されている
  And その Workflow 行には Rust 側で導出された代表ステータス（WorkspaceWorkflowNode.status）が割り当てられている
```

### Rule: Workflow 行アイコンはステータスによらず常に同一形状である（R1）

```gherkin
Scenario Outline: 代表ステータスが何であってもアイコン形状は Workflow アイコンで一定である
  Given Workflow 行の代表ステータスが "<status>" である
  When Workflow 行が描画される
  Then Workflow 行のステータスアイコンは lucide "Workflow" アイコンの形状で表示される
  And ステータスごとに形状が切り替わる表現（Loader2 / CheckCircle2 / AlertTriangle / Clock / Ban / Circle 等）は用いられない

  Examples:
    | status    |
    | queued    |
    | running   |
    | failed    |
    | error     |
    | waiting   |
    | aborted   |
    | completed |
```

```gherkin
Scenario: ツリー上で Workflow 行を形状で識別できる
  Given 同一ツリー内に Workflow 行と Step 行と Session 行が混在している
  When ツリーが描画される
  Then Workflow 行は常に "Workflow" アイコンの形状で表示され、種別として一意に識別できる
```

### Rule: 代表ステータスをアイコンの色で表現する（R2）

```gherkin
Scenario Outline: 代表ステータスに応じてアイコンの色が決まる
  Given Workflow 行の代表ステータスが "<status>" である
  When Workflow 行が描画される
  Then Workflow アイコンには "<color>" 系統の色が適用される
  And 形状は変化しない

  Examples:
    | status    | color           |
    | queued    | ミュート色       |
    | running   | 青              |
    | failed    | 赤              |
    | error     | destructive 色   |
    | waiting   | 黄              |
    | aborted   | ミュート色       |
    | completed | 緑              |
```

```gherkin
Scenario Outline: 稼働中・注目が必要な状態は pulse アニメーションで強調される（仮定A2）
  Given Workflow 行の代表ステータスが "<status>" である
  When Workflow 行が描画される
  Then Workflow アイコンに pulse アニメーションが "<pulse>"

  Examples:
    | status    | pulse      |
    | running   | 適用される   |
    | waiting   | 適用される   |
    | queued    | 適用されない |
    | failed    | 適用されない |
    | error     | 適用されない |
    | aborted   | 適用されない |
    | completed | 適用されない |
```

```gherkin
Scenario: 代表ステータスが未確定・不明な場合はフォールバック色を用いる（仮定A6）
  Given Workflow 行の代表ステータスが既知の WorkspaceStepStatus 値のいずれにも該当しない
  When Workflow 行が描画される
  Then Workflow アイコンにはフォールバックのミュート色が適用される
  And pulse アニメーションは適用されない
```

### Rule: 代表ステータスは Rust 側の値をそのまま利用し、フロントエンドは導出しない（R3）

```gherkin
Scenario: フロントエンドは代表ステータスを導出せず色へマッピングするだけである
  Given Workflow 行に Rust 側で配信された代表ステータスが与えられている
  When Workflow 行が描画される
  Then フロントエンドはステータスの導出・集約・優先順位判定を行わない
  And 与えられた代表ステータスを色クラスへ対応付ける表示用フォーマットのみを行う
```

```gherkin
Scenario: 代表ステータスの live 更新に色がリアルタイムで追従する
  Given Workflow 行が代表ステータス "running"（青・pulse あり）で表示されている
  When 代表ステータスが live 更新（workflow-step-status-changed 経路）で "completed" に変化する
  Then 既存の live 更新経路を通じて Workflow 行に "completed" の代表ステータスが反映される
  And アイコンの形状は "Workflow" のまま変化しない
  And アイコンの色が緑へ変化し、pulse アニメーションが解除される
```

### Rule: 状態識別性とステータス確認手段を維持する（R4）

```gherkin
Scenario: 各ステータスが色（および pulse の有無）で相互に区別できる
  Given Workflow 行が取りうる各 WorkspaceStepStatus 値
  When それぞれの値で Workflow 行が描画される
  Then 色と pulse の有無の組み合わせにより各ステータスが相互に区別可能である
```

```gherkin
Scenario Outline: ホバー等で現在のステータスを確認できる
  Given Workflow 行の代表ステータスが "<status>" である
  When Workflow 行のアイコンにポインタを合わせる
  Then 現在のステータス "<status>" を示す title 属性が提示される

  Examples:
    | status    |
    | queued    |
    | running   |
    | completed |
    | error     |
```

### Rule: 本変更は Workflow 行のアイコンに限定し、他の表現を変えない（非スコープ）

```gherkin
Scenario: Step 行のアイコン表現は変更されない
  Given Workflow 行配下に Step 行が表示されている
  When ツリーが描画される
  Then Step 行のステータスアイコンは本変更前と同じ形＋色の表現（WorkflowStepStatusIcon）のまま維持される
```

```gherkin
Scenario: Session 行および中央パネル WorkflowView の表現は変更されない
  Given Session 行（AgentStateIcon）と中央パネル WorkflowView の Step ヘッダーが表示されている
  When それらが描画される
  Then それぞれのアイコン表現は本変更の前後で変化しない
```

```gherkin
Scenario: ステータスの導出・集約ロジックおよび通信経路は変更されない
  Given 既存の代表ステータス導出（Rust 側）と配信経路が存在する
  When 本変更を適用する
  Then 新規 Tauri コマンド・WebSocket メッセージ・イベントは追加されない
  And ステータス導出・集約ロジックおよび WorkspaceStepStatus の語彙は変更されない
```

## 受け入れ基準（観測可能な確認項目）

- WorkspaceList の Workflow 行のアイコンが、全ステータスにおいて lucide `Workflow` アイコンの形状で表示される。
- 代表ステータスが変化すると、アイコンの形状は変わらず色（および pulse の有無）のみが変化する。
- `running` / `waiting` で pulse アニメーションが適用され、それ以外では適用されない。
- 代表ステータスが live 更新された際、色がリアルタイムに追従する。
- Step 行・Session 行・WorkflowView のアイコン表現は本変更前後で変わらない。
- フロントエンドにステータス導出ロジックが追加されていない（色マッピングのみ）。
- `pnpm lint` / `pnpm test` / `pnpm build` が通る。

## Open Questions

なし。
