# Behavior

clean architecture 移行（milestone [12]）の境界整理 ISSUE #1307 における、frontend helper 責務分類後の外部観測可能な振る舞いを定義する。

本 ISSUE はリファクタリング / 境界整理であり、新規機能を追加しない。よってここで定義する振る舞いは、

- 分類後も維持されるべき既存 UI の振る舞い（markdown preview トグル表示条件、相対時刻表示）と、
- 分類処理そのものの完了条件（残すべき UI-only helper と削除すべき未参照 helper の処理結果）

の 2 系統である。

## 仮定

- `formatRelativeTime`（display-only formatter）は frontend に残す（確定）。
- `markdownUtils.isMarkdownFile`（UI 表示トグル条件）は frontend に残し、test を対象ファイル隣接へ移す（確定）。
- 未参照（dead）helper である `errorHandler.ts`（application decision を含むが未 wire）と `arrayMove.ts`（純粋 UI helper だが未参照）は、test ともども削除する。Rust への migration コードは書かない（ユーザー合意済み）。
- 「#767 残余 helper のうち他分割 ISSUE に属さないもの」は、#767 対象ファイル一覧から他分割 ISSUE 対象を除いた差集合で確定する。本振る舞いでは、その差集合の各 helper が「残す（UI-only）」「削除（未参照）」のいずれかに処理済みであることを完了条件として扱う。
- 相対時刻のしきい値（`now` < 1分、`Nm` < 1時間、`Nh` < 1日、`Nd` 以上）と markdown 拡張子集合（`md` / `mdx` / `markdown`）は既存実装の挙動を正典とし、本 ISSUE では変更しない。

```gherkin
Feature: frontend helper の責務分類後も既存 UI 振る舞いが維持される

  clean architecture 移行の境界整理として、残余 frontend helper を
  「UI-only として残す」か「未参照のため削除する」のいずれかに分類する。
  分類の前後で、ユーザーが観測できる UI の振る舞いは変化しない。

  Background:
    Given Releash の review / diff UI が表示できる状態である

  # ---- 維持されるべき既存 UI 振る舞い: markdown preview トグル ----

  Rule: ReviewPanel は対象ファイルが markdown のときだけ markdown preview トグルを出す

    diff read model / comment read model には影響せず、表示するかどうかだけの UI 条件である。

    Scenario: text diff の markdown ファイルを選択するとトグルが出る
      Given ReviewPanel で text diff が表示されている
      When 拡張子が "md" のファイルを選択する
      Then markdown preview トグルが表示される

    Scenario Outline: markdown 拡張子はトグルを出す
      Given ReviewPanel で text diff が表示されている
      When 拡張子が "<ext>" のファイルを選択する
      Then markdown preview トグルが表示される

      Examples:
        | ext      |
        | md       |
        | mdx      |
        | markdown |
        | MD       |

    Scenario: markdown 以外のファイルではトグルが出ない
      Given ReviewPanel で text diff が表示されている
      When 拡張子が "rs" のファイルを選択する
      Then markdown preview トグルは表示されない

    Scenario: text diff でない場合はトグルが出ない
      Given ReviewPanel で text diff 以外（binary 等）が表示されている
      When 拡張子が "md" のファイルが対象である
      Then markdown preview トグルは表示されない

    Scenario: ファイル未選択ではトグルが出ない
      Given ReviewPanel で text diff が表示されている
      And ファイルが選択されていない
      Then markdown preview トグルは表示されない

  # ---- 維持されるべき既存 UI 振る舞い: 相対時刻表示 ----

  Rule: DiffInlineComment は comment 作成時刻からの経過時間を相対表示する

    display-only formatting であり、表示文字列以外の状態に影響しない。

    Scenario Outline: 経過時間が相対時刻文字列で表示される
      Given DiffInlineComment が表示されている
      And comment 作成からの経過時間が "<elapsed>" である
      Then 相対時刻は "<label>" と表示される

      Examples:
        | elapsed   | label |
        | 30秒      | now   |
        | 5分       | 5m    |
        | 3時間     | 3h    |
        | 2日       | 2d    |

    Scenario: しきい値直前と直後で表示単位が切り替わる
      Given DiffInlineComment が表示されている
      When comment 作成からの経過時間が 59秒 である
      Then 相対時刻は "now" と表示される
      When comment 作成からの経過時間が 60秒 である
      Then 相対時刻は "1m" と表示される
```

```gherkin
Feature: 残余 helper の責務分類が完了している

  #878 final dead-code sweep を実施する前に、#767 残余 helper のうち
  他分割 ISSUE に属さないものが未分類で残っていない状態を作る。

  Rule: 残す UI-only helper は domain/application behavior を持たないことが判別できる

    Scenario: display-only formatter は frontend に残る
      Given frontend helper "formatRelativeTime" がある
      When 責務分類を行う
      Then "formatRelativeTime" は UI-only helper として frontend に残る
      And その helper は domain/application decision を含まないことが test と配置から判別できる

    Scenario: UI 表示条件 helper は frontend に残り test が隣接配置される
      Given frontend helper "markdownUtils.isMarkdownFile" がある
      When 責務分類を行う
      Then "markdownUtils.isMarkdownFile" は UI 表示条件 helper として frontend に残る
      And その test は対象ファイルに隣接して配置されている

  Rule: 未参照（dead）helper は test ともども削除され、wire されていない application decision が残らない

    Scenario: application decision を含むが未 wire の helper を削除する
      Given frontend helper "errorHandler" に error kind 分類の application decision が含まれる
      And "errorHandler" には production からの呼び出し元が存在しない
      When 責務分類を行う
      Then "errorHandler" とその test が削除される
      And frontend に wire されていない application decision が残らない

    Scenario: 未参照の純粋 UI helper を削除する
      Given frontend helper "arrayMove" は純粋な配列並べ替え helper である
      And "arrayMove" には production からの呼び出し元が存在しない
      When 責務分類を行う
      Then "arrayMove" とその test が削除される

  Rule: #767 残余 helper に未分類のものが残らない

    Scenario: 残余 helper がすべて分類済みである
      Given #767 に列挙された frontend helper から他分割 ISSUE 対象を除いた差集合がある
      When その各 helper を責務分類する
      Then 各 helper は「UI-only として残す」か「未参照のため削除する」のいずれかに処理されている
      And 未分類のまま残る helper は存在しない
```

## Open Questions

なし（すべて解消済み）。
