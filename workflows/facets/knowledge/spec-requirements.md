# Requirements

## 定義

`requirements.md`は、一つの変更について「誰の、どの問題を、何を変更して解決するか」を確定する要求文書である。

元のDocumentsと自由文Requestをそのまま後続工程へ渡す代わりに、必要なContextと解釈済みの要求を記録し、Behavior、Design、実装、レビューが参照する要求の正本になる。

この文書は受入条件、実装設計、実装手順を決定しない。

## フォーマット

見出しと順序を次の形式に固定する。

```markdown
# Context
要求を理解するために必要な確定済みの背景、制約、入力文書の参照先。

# Outcome
誰の、どんな問題を解決するか。

# Current Behavior
現在の挙動、再現手順、実際の出力。

# Scope / Non-goals
変更するもの、変更しないもの。

# Requirements
- R-001: 観測可能な期待結果
- R-002: 互換性要件
- R-003: 性能・安全性要件

# Assumptions / Open Questions
未確定事項。推測で埋めない。
```

## 記載内容

### Context

- 入力文書のURL、Issue表記、ファイルパスを記載する。
- 要求の理解と後続のBehavior・Design作成に必要な、確定済みの背景および制約を記載する。
- 入力文書の原文全体は複製しない。
- 自由文Requestの原文は転載しない。曖昧さを解消して解釈した結果を、該当するOutcome、Current Behavior、Scope / Non-goals、Requirementsへ記載する。

### Outcome

- 対象者を書く。
- 現在の問題を書く。
- 変更後に実現する状態を書く。

### Current Behavior

- 調査で確認した現在の挙動を書く。
- 最小の再現手順と実際の出力を書く。
- 対応する既存実装がない場合は、調査範囲と存在しないことを確認した根拠を書く。

### Scope / Non-goals

- 今回変更する対象を書く。
- 今回変更しない対象を明確に分けて書く。

### Requirements

- `R-001`から始まる重複しない安定した連番IDを付ける。
- 利用者または外部インターフェースから観測可能な期待結果を書く。
- 互換性、性能、安全性に明示された要求がある場合は独立したRequirementとして書く。
- 該当しない互換性、性能、安全性要件を推測で追加しない。
- 受入条件のGIVEN・WHEN・THENや、内部の型、関数、モジュール配置などの実装設計を書かない。

### Assumptions / Open Questions

- Assumptionには、ユーザーが明示的に受け入れた仮定だけを書く。
- 作成途中の未確定事項はOpen Questionとして記録できる。
- 文書の確定時にはOpen Questionを残さない。

## 文書間の境界

- Requirementsは「なぜ、何を変更するか」を決める。
- Behaviorは各Requirementを満たしたと外部から判定できる条件を決める。
- DesignはRequirementsとBehaviorを既存実装上でどう実現するかを決める。
- Requirementsにない振る舞いまたは設計を、後続文書が新しい要求として追加してはならない。
