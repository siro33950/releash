# Behavior

対象 Issue: #1305（[impl] frontend diff/markdown read model migration）

本書は外部から観測可能なビジネスルールに絞って振る舞いを定義する。層配置・経路詳細・実装整合性は持ち込まない（requirements R2/R7 等の構造要求は受け入れ基準で扱い、ここでは観測結果の同一性を固定する）。本 Issue は read model 計算の所有者を frontend から Rust へ移す移行であり、観測される diff 表示・review anchoring の同一性は原則不変とする。

## Feature: diff / markdown read model の所有者移行（振る舞い不変）

  diff range / split row / inline chunk / source line mapping の計算が
  Rust-owned query へ移った後も、利用者が観測する diff 表示と
  review anchoring の同一性は移行前と変わらない。

  Background:
    Given markdown ファイルの original 内容と modified 内容が与えられている
    And diff 表示は backend が返す read model を描画して構成される

  Rule: 変更が無い内容は diff 範囲を生成しない

    Scenario: original と modified が同一
      Given original と modified が完全に一致する
      When diff read model を算出する
      Then 変更範囲（added / modified / deleted）は 1 件も生成されない
      And 表示は全行を unchanged として描画する

  Rule: 追加された行は added として観測される

    Scenario: modified 側にのみ存在する行
      Given modified にのみ存在する連続行がある
      When diff read model を算出する
      Then その行は modified 側で added として分類される
      And original 側には対応する変更範囲が現れない

  Rule: 削除された行は deleted として観測される

    Scenario: original 側にのみ存在する行
      Given original にのみ存在する連続行がある
      When diff read model を算出する
      Then その行は original 側で deleted として分類される
      And modified 側には対応する変更範囲が現れない

  Rule: 削除と追加が隣接する箇所は modified として観測される

    Scenario: 同一箇所で行が置き換わる
      Given original の連続行が削除され、同じ位置に modified の連続行が追加される
      When diff read model を算出する
      Then original 側・modified 側ともに当該範囲は modified として分類される
      And added 単独・deleted 単独としては分類されない

  Rule: 空行・行末・隣接変更などの境界条件で表示が破綻しない

    Scenario Outline: 境界条件の diff 分類
      Given original と modified が "<case>" の関係にある
      When diff read model を算出する
      Then 変更行は "<classification>" として観測される
      And 行番号の対応付けがずれない

      Examples:
        | case                         | classification |
        | 末尾に空行のみ追加           | added          |
        | 中間の空行のみ削除           | deleted        |
        | 1 行を別内容へ置換           | modified       |
        | 複数の独立した変更が併存     | mixed          |

## Feature: diff 表示モードごとの観測結果

  gutter / split / inline の各表示モードは、移行後も同じ read model を
  もとに移行前と同一の構成で描画される。表示モードの仕様追加・変更は行わない。

  Rule: split 表示は左右行と種別を保つ

    Scenario: 変更を含む split 表示
      Given 追加・削除・置換・無変更を含む diff がある
      When split 表示を描画する
      Then 各行は left / right と種別（unchanged / added / removed / modified）を持つ
      And 置換箇所は同一行に left（削除前）と right（追加後）が並ぶ
      And この構成は移行前と一致する

  Rule: inline 表示は変更単位の連なりを保つ

    Scenario: 変更を含む inline 表示
      Given 追加・削除・無変更を含む diff がある
      When inline 表示を描画する
      Then 各 chunk は内容と種別（unchanged / added / removed）を持つ
      And chunk の並び順と種別は移行前と一致する

  Rule: 表示モード切替で内容は変わらず表現のみ変わる

    Scenario Outline: diffMode の切替
      Given 同一の diff read model がある
      When 表示モードを "<mode>" に切り替える
      Then 描画される変更箇所の集合は他モードと整合する
      And 切替によって変更の有無・分類が変化しない

      Examples:
        | mode      |
        | gutter    |
        | split     |
        | inline    |
        | diff-only |

## Feature: markdown source line mapping と review anchoring の同一性

  source line mapping と comment anchoring の identity は backend read model を
  source of truth とし、移行前後で同一の anchor へ解決される。

  Rule: block への diff class 付与は backend 由来の line mapping に従う

    Scenario: markdown block と diff range の対応
      Given backend が source line mapping と diff range を返す
      When markdown を描画する
      Then 各 block には対応する diff range の種別に応じた diff class が付与される
      And どの block がどの range に属するかの判定結果は移行前と一致する

  Rule: comment anchoring の line identity は backend read model 由来である

    Scenario: 既存 review comment の anchor 解決
      Given diff line に紐づく review comment が存在する
      When diff / markdown を再描画する
      Then comment は移行前と同一の line / range（backend の安定 ID 由来）へ anchor される
      And frontend 側に anchoring 用の独自 identity 計算は介在しない

## Feature: content kind 判定

  Rule: markdown 判定は移行前と同じ結果を返す

    Scenario Outline: 拡張子による markdown 判定
      Given ファイル名が "<file>" である
      When markdown ファイルか判定する
      Then 結果は "<is_markdown>" となる

      Examples:
        | file        | is_markdown |
        | README.md   | true        |
        | notes.mdx   | true        |
        | doc.markdown| true        |
        | main.rs     | false       |
        | data.json   | false       |

## Feature: 既存 command 契約と非退行

  Rule: 既存 Tauri command の入出力契約は変わらない

    Scenario: compute_visible_markdown_blocks 等の利用
      Given 既存の diff / markdown read model command を呼び出す
      When 移行後に同じ入力を与える
      Then 返却される read model は移行前と同一の I/O 契約に従う

  Rule: 移行は観測可能な振る舞いを変えない

    Scenario: 同一入力に対する全体的な観測結果
      Given 任意の original / modified と表示モードが与えられる
      When 移行前後で diff 表示・review anchoring を比較する
      Then 描画される diff 結果と anchor の同一性は一致する
      And 利用者が観測できる差異は生じない

## 仮定

- A1: 本書は「観測可能な振る舞いの不変性」を固定するものであり、追加・変更する Tauri command / read model DTO の具体形（diff range / split row / inline chunk / line mapping のスキーマ、command 名）は `design.md` で確定する（requirements A4）。
- A2: `rangesOverlap` / `findMatchingRange` を Rust line mapping identity として移すか、AST rendering 都合の突合として frontend に残すかは `design.md` で判定する。判定原則は「read-model identity は Rust、AST rendering 都合の突合は frontend」（requirements A5）。
- A3: `isMarkdownFile` は review / read model 判定に影響しなければ UI helper として frontend に残し、影響すれば Rust へ移す。本書の content kind 判定 Scenario は配置先に依らず結果同一であることのみを固定する（requirements A5/A7）。
- A4: comment anchoring 仕様自体は #1132 で確定した境界に従い、本 Issue では変更しない。本書は「frontend に anchoring identity 計算を残さない／backend 由来である」ことの観測的同一性のみ固定する（requirements A6）。
- A5: 各 Scenario の合否は、移行前実装（`markdownDiff.ts` / `rehypeSourceLines.ts` / `markdownUtils.ts`）の出力を基準（golden）とした非退行比較で判定する。テスト期待値は実装に合わせて変えず、差異が出た場合は実装側を直す（requirements A8）。

## Open Questions

なし。
