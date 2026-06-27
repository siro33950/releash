# Behavior

## Source
- requirements.md

## Behavior

```gherkin
Feature: Repository status の staged / changed 分類を backend read model が決定する
  Releash は review snapshot を取得すると、各ファイルを staged / changed に
  振り分けた read model として供給する。frontend は供給された結果を保持・表示するだけで、
  振り分けの判断は行わない。ReviewPanel の観測可能な振る舞いは本変更で変わらない。

  Background:
    Given 1 つの repository root に対する review snapshot を取得できる
    And snapshot の各ファイルは index status と worktree status を持つ

  Rule: index status が none でないファイルは staged として供給される
    Scenario: index に変更があるファイル
      Given あるファイルの index status が none ではない
      When review snapshot read model が供給される
      Then そのファイルは staged の集合に含まれる

    Scenario: index に変更がないファイル
      Given あるファイルの index status が none である
      When review snapshot read model が供給される
      Then そのファイルは staged の集合に含まれない

  Rule: worktree に ignored 以外の変更があるファイルは changed として供給される
    Scenario: worktree に変更があるファイル
      Given あるファイルの worktree status が none でも ignored でもない
      When review snapshot read model が供給される
      Then そのファイルは changed の集合に含まれる

    Scenario: worktree に変更がないファイル
      Given あるファイルの worktree status が none である
      When review snapshot read model が供給される
      Then そのファイルは changed の集合に含まれない

    Scenario: ignored なファイル
      Given あるファイルの worktree status が ignored である
      When review snapshot read model が供給される
      Then そのファイルは changed の集合に含まれない

  Rule: 1 つのファイルが staged と changed の両方に同時に該当しうる
    Scenario: index にも worktree にも変更があるファイル
      Given あるファイルの index status が none ではない
      And 同じファイルの worktree status が none でも ignored でもない
      When review snapshot read model が供給される
      Then そのファイルは staged の集合に含まれる
      And そのファイルは changed の集合にも含まれる

  Rule: frontend は供給された staged / changed 結果を保持・公開するだけである
    Scenario: read model が取得できたとき
      Given backend が staged / changed に振り分けた read model を返す
      When frontend が review snapshot を取得する
      Then frontend は供給された staged の集合と changed の集合をそのまま保持・公開する
      And frontend は index status / worktree status から振り分けを再計算しない

  Rule: ReviewPanel の staged / changes セクション分類は供給された集合に従う
    Scenario: staged セクションと changes セクションの一覧
      Given backend read model が staged / changed の集合を供給している
      When ReviewPanel が変更を表示する
      Then staged セクションには staged 集合のファイルが一覧される
      And changes セクションには changed 集合のファイルが一覧される

    Scenario: diff path 一覧の供給
      Given backend read model が staged / changed の集合を供給している
      When ReviewPanel が diff の対象 path 一覧を構築する
      Then staged 側の path 一覧は staged 集合の path から成る
      And changes 側の path 一覧は changed 集合の path から成る

    Scenario: 選択中セクションに対する path の所属判定
      Given backend read model が staged / changed の集合を供給している
      And あるセクションが選択されている
      When ある path が選択中セクションに属するか判定される
      Then その判定は供給された staged / changed 集合への所属で決まる

  Rule: status を取得できないときは staged / changed を空として扱う
    Scenario: repository root が無い
      Given 対象の repository root が指定されていない
      When frontend が review snapshot を取得する
      Then staged の集合は空である
      And changed の集合は空である

    Scenario: review snapshot 取得が失敗する
      Given review snapshot の取得が失敗する
      When frontend が review snapshot を取得する
      Then staged の集合は空である
      And changed の集合は空である

  Rule: より新しい snapshot だけが反映される（既存挙動の維持）
    Scenario: 既に受理した version より古い snapshot を受け取る
      Given frontend が既にある version の snapshot を受理している
      When それより古い version の snapshot を受け取る
      Then frontend が保持する staged / changed 結果は更新されない

    Scenario: 同じか新しい version の snapshot を受け取る
      Given frontend が既にある version の snapshot を受理している
      When それと同じか新しい version の snapshot を受け取る
      Then frontend が保持する staged / changed 結果はその snapshot の結果に更新される
```

## 仮定

- 本変更はリファクタリング（milestone [12] クリーンアーキテクチャ移行）であり、ReviewPanel の外部から観測可能な振る舞いは不変である。したがって本 behavior は「振り分けの所有者が backend read model に移っても観測結果が従来と同一である」ことを規定する。
- staged / changed の振り分け規則は requirements の合意済み前提（`index_status !== "none"` → staged、`worktree_status !== "none" && !== "ignored"` → changed）を等価に表現したものである。Gherkin では内部の status 文字列値の網羅ではなく「none か否か」「ignored か否か」という観測可能な分岐のみを規定し、git status 値の具体的な enum 表現は実装詳細として持ち込まない。
- 「frontend は再計算しない」という Rule は、振り分け decision が frontend（`useReviewSnapshot`）に残らないという requirements の中心要求を観測可能な性質として表現したものである。read model の transport 形（`ReviewSnapshotDto` への staged / changed 集合フィールド追加）は design.md で決定する実装詳細であり、本 behavior では規定しない。
- 生きた split の所有者は `useReviewSnapshot` であり、`useGitStatus` は production 未消費の dead code である。`useGitStatus`（`statusMap` 構築・`toFileStatus` を含む）・`applyStatusToTree` の削除は production 未消費の dead code 除去であり、外部から観測可能な振る舞いの変化を伴わない。したがって Gherkin の Scenario としては規定せず、受け入れ基準（grep / ファイル不存在）と requirements 側のスコープに委ねる。表示用 `FileStatus` 分類・directory aggregation・status propagation・unknown fallback は削除対象のため、観測可能な振る舞いとしても規定しない。
- version / stale / loading / limited フラグの意味と採番、および `useReviewSnapshot` の version dedup（厳密に古い version の snapshot のみ反映しない）・race 制御は現状維持であり、本 behavior の「より新しい snapshot だけが反映される」は既存の version dedup 挙動を退行させないことの明示である。

## Open Questions

なし
</content>
