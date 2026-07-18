# Behavior

## 定義

`behavior.md`は、`requirements.md`の各Requirementを満たしたと外部から判定するための、観測可能な受入条件を記録する文書である。

この文書はRequirementを追加、変更、補完しない。内部の構造や実装方法も決定しない。

## フォーマット

受入条件を`## B-xxx`形式で連続して記載し、末尾に要件IDと検証方法の対応表を置く。

```markdown
## B-001: Approval rejection

GIVEN approval待ちのworkflow runが存在する
WHEN reviewerが理由付きでrejectする
THEN runはrejectedになり、理由が履歴へ保存される
AND 実行中actionは再開されない

## B-002: 受入条件名

GIVEN 事前状態
WHEN 操作
THEN 観測可能な結果
AND 追加の観測可能な結果

## 要件IDと検証方法の対応表
| Requirement ID | Behavior ID | Verification Method |
| --- | --- | --- |
| R-001 | B-001 | 入力、操作、観測点、期待値を含む具体的な検証方法 |
```

## 記載内容

- 各受入条件には`B-001`から始まる重複しない安定した連番IDを付ける。
- 各受入条件を一つ以上のRequirement IDへ対応させる。
- 状態、操作、結果の流れを表す場合はGiven-When-Thenを使う。
- 特定の事象、状態、入力条件に対する単一の契約にはEARSを使ってよい。
- 一つの受入条件の中でGiven-When-ThenとEARSを混在させない。
- `THEN`と`AND`には、利用者または外部インターフェースから観測可能な結果を書く。
- 内部の型名、関数名、モジュール配置などの設計詳細を受入条件にしない。
- `Happy Path`、`Edge Cases`、`Error Paths`、`Regression Behaviors`などの分類見出しを追加しない。すべて同じ`## B-xxx: 受入条件名`形式で記載する。
- 該当しないケースを網羅性や数合わせのために作らない。

## 必須の観点

### 最小再現入力

- 現在の問題または変更対象の挙動を観測できる最小の入力を、GIVENとWHEN、またはEARSの条件と事象に具体値で示す。
- 必要な事前状態、操作、観測点、現在の実際の結果を含める。
- 再現不能または既存実装がない場合は、調査範囲と根拠を記載する。

### Happy Path

- 各主要Requirementが成功する代表的な経路を含める。

### Edge Cases

- Requirementが定めた境界値や特殊状態に対する観測可能な結果を含める。
- Requirementの成立を直接検証するために不可欠な条件だけを扱う。

### Error Paths

- Requirementが定めた失敗条件、エラーまたは表示、失敗後の状態、副作用の有無を含める。
- 未規定の失敗条件について期待結果を作らない。

### Regression Behaviors

- Requirementによって維持対象になった既存挙動だけを含める。
- 入力、操作、期待結果を具体化する。
- 調査で観測しただけの既存挙動をRegressionとして追加しない。

### 要件IDと検証方法の対応表

- `requirements.md`のすべてのRequirement IDを漏れなく記載する。
- 各Requirementに対応するBehavior IDを一つ以上記載する。
- Verification Methodには、具体的な入力、操作またはコマンド、観測点、期待値を書く。
- 「確認する」「検証する」だけの抽象的な記述を使わない。

## Requirementsから導出できる範囲

- Requirementが「AのときB」と定めている場合、記載できるのは条件Aと結果Bの組み合わせだけである。
- 「AではないときBではない」「Aに似たA'のときもB」「A'のときは別のB'」など、Requirementにない逆、対偶、類似条件、隣接条件、既定値、例外時の結果を推論して追加しない。
- Requirementが結果を定めていない入力、状態、境界、失敗条件について、望ましい挙動を決めない。
- Requirementの検証に未規定条件の決定が不可欠な場合は、Behaviorへ追加せずRequirementsの不足として扱う。
- `B-xxx`に記載できるのは、Requirementが要求する挙動、そのRequirementの検証に必要な条件、およびRequirementで維持対象として明示された既存挙動だけである。
- 変更要求と無関係な文言、順序、タイミング、内部方式、偶発的な現在値を固定しない。
