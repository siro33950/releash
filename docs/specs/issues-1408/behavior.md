# Behavior

requirements.md の要求を、実装詳細を含まない観測可能な振る舞いとして Gherkin で定義する。
本文書は「両 backend が同じ規約で stdout を読む」ことによって外部から観測できる結果だけを扱い、
共通部品の内部構造・関数名・呼び出し経路には踏み込まない。

## Feature

```gherkin
Feature: agent session backend の stdout 読み取り堅牢性の共通化
  Claude / Codex いずれの backend でも、CLI プロセスが stdout に出す
  「プロトコル外の 1 行」や「サイズ上限を超える巨大な 1 行」が原因で
  agent session が突然死しないようにする。両 backend は同じ規約で
  「行サイズ上限」「非 JSON 行の skip」「破棄・skip の可視化」を持ち、
  セッションは継続する。
```

## Background

```gherkin
Background:
  Given agent session が backend（Claude または Codex）の CLI を子プロセスとして起動している
  And backend の stdout を行単位で読み取り、チャットへ反映している
  And 破棄・skip の件数の source of truth は Rust 側の runtime state / read model にある
  And frontend は runtime state / read model を表示するだけで、判断ロジックを持たない
```

## Rule: 非 JSON 行はセッションを終了させず skip され、件数がカウントされる

両 backend で共通。CLI や注入環境が stdout に警告・診断ログ等の非 JSON 行を混ぜても、
その行は捨てられ、セッションは読み取りを継続する。

```gherkin
Scenario Outline: 非 JSON 行が混ざってもセッションが継続する
  Given <backend> のセッションが turn を実行中である
  When backend が stdout に非 JSON 行を 1 行出力する
  And 続けて正常な JSON 行を出力する
  Then 非 JSON 行は skip される
  And skip 件数がカウントされる
  And 後続の正常な JSON 行は通常どおり処理される
  And セッションは終了せず継続する

  Examples:
    | backend |
    | Claude  |
    | Codex   |
```

```gherkin
Scenario: 非 JSON 行が連続してもセッションが継続する
  Given Codex のセッションが turn を実行中である
  When backend が stdout に非 JSON 行を複数行連続で出力する
  And 続けて正常な JSON 行を出力する
  Then 非 JSON 行はすべて skip され、その件数がカウントされる
  And 後続の正常な JSON 行は通常どおり処理される
  And セッションは終了せず継続する
```

## Rule: 1 行サイズ上限を超える行は保持されず読み捨てられ、破棄が可視化される

両 backend で共通のサイズ上限（既存 Claude の 8MB を共通定数として踏襲）を持つ。
上限を超える行は丸ごとメモリに蓄積されず読み捨てられ、破棄が可視化される。

```gherkin
Scenario Outline: サイズ上限超過行が読み捨てられ後続処理が継続する
  Given <backend> のセッションが turn を実行中である
  When backend が stdout に共通サイズ上限を超える巨大な 1 行を出力する
  And 続けて正常な JSON 行を出力する
  Then 上限超過行はメモリに丸ごと保持されず読み捨てられる
  And 破棄が 1 件として可視化される（構造化 warn ログとカウント）
  And 後続の正常な JSON 行は通常どおり処理される
  And セッションは終了せず継続する

  Examples:
    | backend |
    | Claude  |
    | Codex   |
```

```gherkin
Scenario Outline: 改行を伴わない上限超過行も破棄として可視化される
  Given <backend> のセッションが turn を実行中である
  When backend が改行で終端しないまま共通サイズ上限を超える出力を行い stdout が閉じる
  Then 上限超過分は読み捨てられる
  And 破棄が 1 件として可視化される

  Examples:
    | backend |
    | Claude  |
    | Codex   |
```

## Rule: 破棄・skip の件数はカウントとして可視化される

破棄（oversize）・skip の発生件数は、両 backend で少なくとも構造化 warn ログとカウントで可視化される。
巨大行そのものは保持せず、カウント等の summary だけを state に持つ（full-retention を増やさない）。

```gherkin
Scenario: 破棄件数がチャット上で可視化される
  Given Claude のセッションで stdout の上限超過行が破棄された
  Then 破棄が発生したことを示す Error part 相当の通知がチャットに現れる
  And 破棄件数が runtime state に累積される
  And 破棄件数の source of truth は Rust 側 runtime state / read model にある
```

```gherkin
Scenario: turn が別要因で終了した場合に累積破棄件数が併記される
  Given Claude のセッションで turn 実行中に stdout の上限超過行が 1 件以上破棄されている
  When 同じ turn がクラッシュ等で終了する
  Then 終了通知に累積した破棄件数（「サイズ超過破棄 N 件」相当）が併記される
```

## Rule: 応答必須の JSON-RPC response の decode 失敗は失敗として扱われる（Codex 例外条件）

Codex は JSON-RPC の整合性を必要とする。非 JSON 行の一律 skip とは別に、
応答待ちの request に対応する「応答必須の JSON-RPC response」の decode 失敗だけは、
従来どおり失敗として扱ってよい。JSON-RPC 整合性の判断は backend 側の呼び出し層が担い、
共通部品は「1 行が JSON か非 JSON か」「サイズ上限」だけを返す。

```gherkin
Scenario: 応答必須 request に対する非整合な response は失敗として扱われる
  Given Codex のセッションが応答必須の JSON-RPC request を送り、その response を待っている
  When その request に対応する response 行の decode に失敗する
  Then その失敗は skip されず、失敗として扱われる（従来の失敗経路を維持してよい）
```

```gherkin
Scenario: 応答を待っていない非 JSON 行は失敗にならず skip される
  Given Codex のセッションが応答必須 request を待っていない
  When stdout に非 JSON 行が出力される
  Then その行は失敗として扱われず skip され、カウントされる
  And セッションは終了せず継続する
```

## Rule: Claude の既存挙動は回帰しない

共通部品への移行後も、Claude の既存の意図的仕様（非 JSON 行 skip、8MB 超破棄、
破棄カウント表示、Error part 相当の可視化、セッション継続）は変わらない。

```gherkin
Scenario: Claude の既存の skip / 破棄 / カウント表示が維持される
  Given Claude のセッションが turn を実行中である
  When stdout に非 JSON 行と 8MB 超の巨大行が混ざる
  Then 非 JSON 行は skip される
  And 8MB 超の行は破棄され、破棄件数がカウント・可視化される
  And セッションは終了せず継続する
```

## Rule: Claude の破棄時に破棄行の種別推定が付与される

破棄行そのものは保持しないが、破棄の可視化に「破棄された行がどの種別だったか」の推定を添えて、
可視化を改善する（推定手段は design.md で決める）。

```gherkin
Scenario: 破棄行の種別推定が可視化に添えられる
  Given Claude のセッションで stdout の上限超過行が破棄される
  When その破棄を可視化する
  Then 破棄の通知に破棄行の種別推定が添えられる
  And 種別推定のために巨大行全体を保持しない
```

## Rule: 破棄・skip の通知経路は後続 Notice へ接続できる拡張点を持つ

本 ISSUE では `Notice` 語彙（S5 #1393）や `ProtocolIncompatible`（後続 Phase）へは接続しない。
ただし破棄・skip の通知経路は、後続 ISSUE で無改造に近い形で `Notice` へ接続できる拡張点を持つ。

```gherkin
Scenario: 通知経路が後続の Notice 接続を阻害しない
  Given 共通部品が破棄・skip を呼び出し側へ通知している
  Then 通知は本 ISSUE では構造化 warn ログ＋既存の Error part 相当＋カウントで着地する
  And その通知経路は後続 ISSUE で Notice へ接続できる拡張点を持つ
  And 本 ISSUE では Notice / ProtocolIncompatible へは接続しない
```

## Rule: 非 JSON 行・巨大行を混ぜた fixture でセッション継続が固定される

```gherkin
Scenario Outline: fixture による回帰テストでセッション継続が固定される
  Given 非 JSON 行と巨大行を混ぜた stdout の fixture がある
  When <backend> のセッションがその fixture を読み取る
  Then セッションは終了せず継続する
  And 破棄・skip の件数がカウントされる
  And この継続が自動テストで固定される

  Examples:
    | backend |
    | Claude  |
    | Codex   |
```

## 仮定

- 共通行読み取り部品は `infrastructure/agent_session/` に配置し、Claude 既存の
  `MAX_CLAUDE_STDOUT_LINE_BYTES = 8MB` を共通サイズ上限定数の初期値として踏襲する。
  定数名・module 名は design.md で確定する。
- 共通部品はバイト境界の読み取りを基準とし、Codex も現状の `BufReader::lines()`（文字列ベース）から
  バイトベースへ寄せる。UTF-8 変換は非 JSON / JSON の分類後に行う。
- 「応答必須 request に対する行だけ失敗を伝搬」する判定（in-flight request id の追跡）は、
  共通部品ではなく Codex 側の呼び出し層で行う。共通部品は「JSON か非 JSON か」「サイズ上限」だけを返し、
  JSON-RPC 整合性の判断は backend 側に置く。
- 破棄・skip カウントは runtime state の既存 `oversize_dropped_count` 相当を拡張して持ち、
  full-retention を増やさない。skip カウントの表示手段（Error part 相当の要否等）は design.md で決める。
- 非 JSON 行の扱いは requirements.md 確定事項の案 A（暫定 skip＋継続）に従う。
  `ProtocolIncompatible` による fail-closed は本 ISSUE では実装せず、後続 ISSUE に委ねる。
- fixture の配置は F1（wire record/replay 回帰テスト基盤、#1383）への追加を第一候補とし、
  F1 未整備時の代替配置は design.md で決める。

## Open Questions

なし。
