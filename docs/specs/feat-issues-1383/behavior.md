# Behavior — F1: wire record/replay 回帰テスト基盤

対象 issue: [#1383](https://github.com/siro33950/releash/issues/1383)（milestone 84 Phase 0 / ST-7 前半）

この文書は `requirements.md` の要求を、実装詳細を含まない観測可能な振る舞いとして定義する。振る舞いの主語は「開発者が record tap で fixture を採取する」「CI / 開発者が replay・統合 golden テストを実行する」の 2 つである。

## Feature: wire record/replay 回帰テスト基盤

agent chat（Claude / Codex）の wire メッセージを domain event / read model へ変換する経路の現状挙動を、実 wire ログ由来の fixture と golden ファイルで固定し、後続変更での回帰を機械的に検出できるようにする。

### Background

```gherkin
Background:
  Given リポジトリに Claude 用と Codex 用の wire fixture 置き場が存在する
  And 各 fixture は実セッション由来の wire ログを 1 行 1 メッセージの JSONL として保持する
  And 各 fixture には対応する golden ファイル（convert 層・read model 層）が併置される
  And golden 比較は新規 crate 依存を追加せず自作の比較で行われる
```

### Rule: record tap は環境変数ゲートでのみ動作し、既定では本番挙動に影響しない

```gherkin
Scenario: 環境変数未設定では wire を採取しない
  Given RELEASH_WIRE_RECORD が設定されていない
  When Claude / Codex セッションが wire メッセージを受信する
  Then wire 行はどこにも採取されない
  And セッションの読み取り・変換・応答の挙動は tap 追加前と変わらない

Scenario: 環境変数設定時に受信した wire 行を採取する
  Given RELEASH_WIRE_RECORD=<dir> が設定されている
  When Claude / Codex セッションが stdout から wire 行を受信する
  Then 受信した生 wire 行が <dir> 配下へ 1 行 1 メッセージの JSONL として書き出される
  And セッション本来の読み取り・変換・応答の挙動は変わらない

Scenario: 採取は生 wire 行を対象とし drop も可能な範囲で残す
  Given RELEASH_WIRE_RECORD=<dir> が設定されている
  When パース失敗行やサイズ超過などで本来 drop される wire 行を受信する
  Then その生行も可能な範囲で <dir> に採取される
```

### Rule: replay golden（convert 層）は現状の変換出力を固定する

```gherkin
Scenario: fixture を convert に通した出力が golden と一致する
  Given Claude または Codex の fixture と対応する convert 層 golden が存在する
  When fixture を先頭行から順に convert へ流す
  Then 生成された AgentRuntimeEvent 列が golden と一致し、テストが通る
  And Claude では AgentRuntimeEvent 列に加え auto_responses も golden 比較対象に含まれる

Scenario: convert は fixture を通す間 state を引き継ぐ
  Given 複数行にまたがる 1 turn の fixture が存在する
  When fixture を 1 行ずつ順に convert へ流す
  Then 変換 state は行をまたいで引き継がれ、turn 全体の出力が golden と一致する

Scenario Outline: 代表 turn で replay テストが通る
  Given <backend> の fixture が <要素> を含む通常 turn を保持する
  When fixture を convert に流し golden と比較する
  Then テストが通る

  Examples:
    | backend | 要素 |
    | Claude  | text / thinking / tool_use / permission / result |
    | Codex   | agentMessage / commandExecution / requestApproval / turn completed |

Scenario: 出力が golden と異なると fail する
  Given convert の出力が既存 golden と一致しない状態
  When replay テストを実行する
  Then テストは不一致を報告して fail する

Scenario: UPDATE_GOLDEN で golden を更新できる
  Given convert の出力を新しい正とみなしたい
  When UPDATE_GOLDEN=1 を設定して replay テストを実行する
  Then golden ファイルが現在の出力で上書きされ、テストが通る
```

### Rule: 統合 golden（read model 層）は projector までの現状挙動を固定する

```gherkin
Scenario: fixture を read model まで通した出力が golden と一致する
  Given fixture と対応する read model 層 golden が存在する
  When 同じ fixture を convert → 永続 event → projector（project()）まで流す
  Then 得られた SessionReadModel スナップショットが golden と一致し、テストが通る

Scenario: 統合 golden も UPDATE_GOLDEN で更新できる
  Given read model の出力を新しい正とみなしたい
  When UPDATE_GOLDEN=1 を設定して統合テストを実行する
  Then read model 層 golden が現在の出力で上書きされ、テストが通る

Scenario: read model 出力が golden と異なると fail する
  Given projector 出力が既存 golden と一致しない状態
  When 統合テストを実行する
  Then テストは不一致を報告して fail する
```

### Rule: fixture はマスク済みの状態でリポジトリに存在する

```gherkin
Scenario: 秘匿情報がプレースホルダへ置換されている
  Given リポジトリに commit された fixture
  When fixture の内容を確認する
  Then 絶対パス・ホームディレクトリ・トークン / API キー・メッセージ本文などの秘匿値が安定なプレースホルダへ置換されている
  And type / subtype / フィールド構成 / イベント順序などの構造は保持されている
```

### Rule: テストは CI で実行され、本 issue は変換挙動を変えない

```gherkin
Scenario: replay / 統合 golden テストが CI で実行される
  Given .github/workflows/ci.yml の Rust テスト経路
  When cargo test を実行する
  Then replay テストと統合 golden テストが実行される

Scenario: 本 issue の変更で golden 内容が変わらない
  Given 本 issue は record tap 追加とテスト基盤新設のみを行う
  When convert / projector を通した出力を確認する
  Then 生成される golden の内容は本 issue の変更前後で変わらない

Scenario: 品質チェックが通る
  When cargo fmt --check / cargo clippy -- -D warnings / cargo test を実行する
  Then いずれも成功する
```

### Rule: golden 更新・採取・マスキング手順がドキュメント化されている

```gherkin
Scenario: fixture ディレクトリの README に手順が記載されている
  Given fixture ディレクトリ
  When README を参照する
  Then record tap（RELEASH_WIRE_RECORD）による採取手順が記載されている
  And マスキング方針が記載されている
  And UPDATE_GOLDEN による golden 更新手順が記載されている
```

## 仮定

`requirements.md` の「仮定」に従う。振る舞い上、特に前提とするもの:

1. record tap は本 issue の成果物として production コードにマージするが、既定無効・非破壊とする。
2. golden は fixture ごとに convert 層（events ＋ auto_responses）と read model 層（SessionReadModel）を pretty JSON で保存し、fixture と同じディレクトリ配下に置く。
3. golden 更新は `UPDATE_GOLDEN=1`、採取は `RELEASH_WIRE_RECORD=<dir>` の環境変数で行う。
4. AgentRuntimeEvent → 永続 event の写像は production の building block と turn 完了順序（`FinalPartsRecorded`、続いて terminal event）を利用する。
5. golden crate（insta / goldenfile 等）は導入せず自作比較で行う。

## Open Questions

なし。
