# Behavior

Issue: #1212 「Hunk operation を id-based にし frontend patch 再生成を削除する」

本書は `requirements.md` の要求 R1〜R5・決定事項 D1〜D3 を、実装詳細を含まない外部観測可能な振る舞いとして Gherkin で定義する。実装経路（snapshot 再計算の内部手順、patch 文字列の構築方法、id 生成アルゴリズム等）は対象外であり design で扱う。

## 仮定

requirements の仮定および本書作成時に置いた仮定。design で確定すべき詳細は振る舞いに持ち込まない。

- **A1**: 安定 id は hunk / group の「内容」に紐づく値であり、同一内容を指す限り snapshot 再計算後も同じ値を返す。生成規則の詳細は design で確定する（本書は「同一内容なら同一 id・別内容なら別 id」という観測可能性のみを規定する）。
- **A2**: stage / unstage は id + intent を渡す id-based operation 単一経路へ収束し、位置 index / `snapshot_version` を渡す旧経路は提供しない。コマンド命名・移行手順は design で確定する。
- **A3**: 操作対象の語彙は既存を踏襲する。base は `head` / `branch-base`、section は `changes` / `staged`。本書ではこれらを所与とする。

---

```gherkin
Feature: id-based な hunk / group の stage / unstage

  レビュー（diff 表示）画面で、変更の一部（change group）を staged / unstage する操作を、
  位置や snapshot version に依存しない安定 id で指定できるようにする。
  file refresh をまたいでも、同一内容の対象は同じ id で指し続けられ、
  対象が消失した場合は誤った範囲を適用せず安全に失敗する。

  Background:
    Given デスクトップアプリでリポジトリのレビュー画面を開いている
    And 対象ファイルの text diff の read model が取得できている
    And base は "head" を選択している

  # --- R1: 安定 id の付与 ---

  Rule: text diff の read model は各 hunk / change group に安定 id を含む

    Scenario: read model が hunk_id と group_id を含む
      When 対象ファイルの text diff を取得する
      Then 各 hunk は安定 id（hunk_id）を持つ
      And 各 change group は安定 id（group_id）を持つ

    Scenario: 同一内容の対象は file refresh をまたいで同じ id を返す
      Given ある change group の group_id を取得している
      When 当該 change group の内容を変えずに snapshot を再計算（file refresh）する
      Then 同じ change group は同じ group_id で取得できる

    Scenario: 内容が異なる対象は異なる id を返す
      Given ある change group の group_id を取得している
      When 当該箇所の内容が変わった状態で snapshot を再計算する
      Then 変化後の change group は元と異なる group_id になる

  # --- R2 / D3: id + intent による stage / unstage ---

  Rule: stage / unstage は対象 id と intent のみで実行する

    Scenario: changes 区画の change group を id 指定で stage する
      Given section は "changes" を選択している
      And ある change group の group_id を取得している
      When その group_id と intent=stage を指定して操作する
      Then 当該 change group の範囲だけが staged になる
      And version（snapshot_version）を渡さずに操作が成立する

    Scenario: staged 区画の change group を id 指定で unstage する
      Given section は "staged" を選択している
      And staged の change group の group_id を取得している
      When その group_id と intent=unstage を指定して操作する
      Then 当該 change group の範囲だけが unstage される
      And version（snapshot_version）を渡さずに操作が成立する

    Scenario: 操作粒度は change group 単位を維持する
      When stage / unstage を実行する
      Then 操作の最小単位は change group であり
      And hunk 全体を一括 stage する新規操作は提供されない

  # --- 制約: branch-base では group / hunk 単位 stage / unstage を提供しない ---

  Rule: group / hunk 単位の stage / unstage は head ベースのみ対象とする

    Scenario: branch-base での id 指定 stage はエラーになる
      Given base は "branch-base" を選択している
      When change group を id 指定で stage しようとする
      Then 操作はエラーになり
      And staged の状態は変化しない

  # --- R4: staged をベースにした安全な適用 ---

  Rule: patch は staged 状態をベースに適用し、ベース不一致を起こさない

    Scenario: 連続した stage 操作が staged の進行に追随する
      Given section は "changes" の change group が複数ある
      When 1 つ目の change group を stage する
      And 続けて別の change group を stage する
      Then いずれの操作も適用に失敗せず
      And それぞれの change group の範囲だけが staged に反映される

  # --- R5 / D2: file refresh 後の stable id の扱い ---

  Rule: 解決できない id は誤適用せずエラーを返し、frontend が回復する

    Scenario: 同一内容なら過去に払い出した id が引き続き有効
      Given ある change group の group_id を取得している
      And 内容を変えずに file refresh が発生した
      When その group_id と intent を指定して操作する
      Then 当該 change group に対して操作が成立する

    Scenario: 対象が消失した id はエラーになり何も適用されない
      Given ある change group の group_id を取得している
      And file refresh により当該 change group が消失した（内容が変わって該当しなくなった）
      When その group_id と intent を指定して操作する
      Then 操作はエラーになり
      And staged / unstage の状態は変化しない

    Scenario: 解決失敗を frontend が捕捉して snapshot を refresh する
      Given id 解決失敗のエラーが返る状況にある
      When frontend がその操作を実行する
      Then エラーは握りつぶされず捕捉され
      And snapshot の refresh（または再取得の促し）が行われる

  # --- R3: frontend からの patch generation 排除 ---

  Rule: frontend は patch を生成せず id + intent のみを渡す

    Scenario: frontend が stage / unstage で渡すのは id + intent と座標語彙のみ
      When frontend が stage / unstage を実行する
      Then frontend は full content から patch 文字列を生成しない
      And frontend が渡すのは worktree / path / section / base / 対象 id / intent に限られる
      And 位置 index と snapshot_version は渡さない
```

---

## 主要観点の対応

- 正常系: changes での stage / staged での unstage（id + intent、version なし）。
- 異常系: branch-base 拒否、id 解決失敗時のエラーと no-op、frontend のエラー捕捉 + refresh。
- 境界条件: file refresh をまたいだ id の安定性（同一内容で有効・消失で失敗）、連続 stage における staged ベース追随。

## Open Questions

なし（requirements の D1〜D3 で前提が確定しているため、振る舞いレベルの未確定事項はない。id 生成規則・コマンド命名・旧経路の死コード除去可否は design で確定する事項であり、観測可能な振る舞いには影響しない）。
