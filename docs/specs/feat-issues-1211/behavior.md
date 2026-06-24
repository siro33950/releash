# Behavior

Issue: #1211 「ReviewSnapshot / ReviewFileView command を追加し diff 表示を Rust read model に寄せる」

本書は requirements.md の要求事項 R1〜R6 を、実装詳細を含まない外部観測可能な振る舞いとして Gherkin で定義する。
語彙は requirements に従う: `DiffBase` = `branch-base` | `head`、`DiffSection` = `changes` | `staged`。

## Feature: レビュー（diff 表示）の read model を Rust 側で提供する

レビュー画面は、ファイル一覧の取得（ReviewSnapshot）とファイルを開く処理（ReviewFileView）を分離した
2 つの API を通じて表示する。表示種別の判定・diff 基準の決定・large file / image / binary の扱いは
すべて Rust 側で行い、frontend は返された read model をそのまま表示する。

### Background

```gherkin
Background:
  Given Git リポジトリの worktree が存在する
  And RepositoryStateService の versioned snapshot 基盤が利用可能である
```

## Rule: ファイル一覧 API（ReviewSnapshot）は変更ファイル一覧 read model を 1 回の呼び出しで返す（R1, R3）

```gherkin
Scenario: ファイル一覧を 1 回の呼び出しで取得する
  Given worktree に変更されたファイルが複数存在する
  When base を指定して ReviewSnapshot を要求する
  Then 変更ファイル集合・status・diff stats・diff file tree を含む read model が 1 回の呼び出しで返る
  And 各ファイルには ReviewFileView から参照可能な安定した file_id が含まれる
  And read model は version と stale / loading / limited 相当の状態を持つ

Scenario: 変更が無い場合は空のファイル一覧が返る
  Given worktree に変更されたファイルが存在しない
  When base を指定して ReviewSnapshot を要求する
  Then 変更ファイル集合が空の read model が返る
  And エラーにはならない

Scenario Outline: base ごとに対応するファイル一覧が返る
  Given worktree に変更されたファイルが存在する
  When base "<base>" を指定して ReviewSnapshot を要求する
  Then その base を基準とした変更ファイル一覧 read model が返る

  Examples:
    | base        |
    | branch-base |
    | head        |

Scenario: 同一 snapshot 由来で一覧が一貫する
  Given worktree に変更されたファイルが存在する
  When ReviewSnapshot を要求する
  Then 返る status・diff stats・diff file tree は同一の versioned snapshot 由来であり相互に整合する
```

## Rule: ファイル表示 API（ReviewFileView）は表示種別を Rust 側で判定して返す（R2）

```gherkin
Scenario: file_id で対象ファイルを開く
  Given ReviewSnapshot で得た file_id を持つ
  When その file_id・section・base を指定して ReviewFileView を要求する
  Then 対象ファイルの read model が返る

Scenario: path で対象ファイルを開く
  Given 変更ファイルの path を持つ
  When その path・section・base を指定して ReviewFileView を要求する
  Then 対象ファイルの read model が返る

Scenario: text ファイルは original / modified / source を含む read model で返る
  Given 対象ファイルがテキストの差分である
  When ReviewFileView を要求する
  Then 表示種別が text diff として判定される
  And original / modified / source（差分情報を含む）が Rust 側で決定された read model が返る
  And frontend は受け取った内容をそのまま表示できる

Scenario Outline: section ごとに対応する差分が返る
  Given 対象ファイルがテキストの差分である
  When section "<section>" を指定して ReviewFileView を要求する
  Then その区画に対応する original / modified を含む read model が返る

  Examples:
    | section |
    | changes |
    | staged  |

Scenario: viewport 指定時は必要範囲のみを返す
  Given 対象ファイルがテキストの差分である
  When viewport（表示範囲）を指定して ReviewFileView を要求する
  Then 指定範囲に対応する部分の read model が返る

Scenario: viewport 未指定時は threshold に従って全量または fallback を返す
  Given 対象ファイルがテキストの差分である
  When viewport を指定せずに ReviewFileView を要求する
  Then threshold 内なら全量の read model が返り、超過時は fallback の read model が返る
```

## Rule: image / binary は base64 data URL ではなく asset / resource URL として返す（R5）

```gherkin
Scenario: image ファイルは asset / resource URL で返る
  Given 対象ファイルが画像である
  When ReviewFileView を要求する
  Then 表示種別が image として判定される
  And 内容は base64 data URL ではなく Tauri asset / resource URL の参照として返る
  And frontend は返された URL をそのまま表示に用いることができる

Scenario: binary ファイルは asset / resource URL で返る
  Given 対象ファイルがバイナリである
  When ReviewFileView を要求する
  Then 表示種別が binary として判定される
  And 内容は base64 data URL ではなく asset / resource URL の参照として返る
```

## Rule: large file / 多数 hunk / tokenization 超過は UI を固めず fallback で返す（R6）

threshold 概値（design で最終調整）: large file = サイズ > 約 1MB または > 約 5,000 行 /
hunk 数 > 約 300 / tokenization = 内容 > 約 10 万文字（または約 5,000 行）。

```gherkin
Scenario Outline: threshold 超過時は fallback read model を返す
  Given 対象ファイルが "<condition>" で threshold を超過している
  When ReviewFileView を要求する
  Then 全量を返さない fallback 表示用の read model が返る
  And read model には超過した旨（limited 等）が反映される
  And UI が固まらない

  Examples:
    | condition           |
    | ファイルサイズ超過       |
    | 行数超過             |
    | hunk 数超過          |
    | tokenization 量超過   |

Scenario: threshold 内のファイルは通常表示で返る
  Given 対象ファイルがすべての threshold を超過していない
  When ReviewFileView を要求する
  Then fallback ではなく通常の read model が返る
  And limited 相当のフラグは立たない

Scenario: 超過判定は Rust 側で行われる
  Given 対象ファイルが threshold を超過している
  When ReviewFileView を要求する
  Then 超過判定と打ち切りは Rust 側で行われ、その状態が read model で frontend に伝わる
```

## Rule: frontend は表示に徹し Git orchestration / file IO を持たない（R4）

```gherkin
Scenario: ファイル一覧表示は ReviewSnapshot の read model のみで行う
  Given レビュー画面でファイル一覧を表示する
  When 一覧を取得する
  Then frontend は ReviewSnapshot の read model を表示するだけで、status / diff stats / tree を個別に組み立てない

Scenario: ファイルを開く処理は ReviewFileView の read model のみで行う
  Given レビュー画面でファイルを開く
  When ファイル内容を取得する
  Then frontend は ReviewFileView の read model を表示するだけで、working tree / Git index の direct read を行わない
  And diff 基準選択・tree 化・image base64 化・patch generation 準備を frontend が持たない

Scenario: ファイル一覧取得とファイルを開く処理が分離している
  Given レビュー画面を利用する
  When 一覧取得とファイル表示を行う
  Then それぞれ ReviewSnapshot と ReviewFileView の別 API として呼び出される
```

## 仮定

requirements.md の仮定 A1〜A7 を引き継ぐ。本書で振る舞いの粒度に影響する主なもの:

- **A2 / A3**: ReviewFileView は `file_id` と `path` のどちらでも対象を特定できる。本書では両経路を別 Scenario として定義した。
- **A5**: image / binary の返却方式は Tauri asset / resource URL に確定。実ファイルが存在しない側（HEAD / staged 由来）の一時ファイル化要否と URL ライフサイクルは design で詰めるため、本書では「base64 data URL ではなく URL 参照で返る」という観測可能な振る舞いまでを定義する。
- **A6**: threshold は概値であり design で最終調整する。本書の Scenario は具体数値ではなく「threshold を超過した / していない」という観測可能な条件で記述する。
- **A7**: 既存 command（`get_review_text_diff` 等）の統廃合範囲は design 確定事項。本書は新 API の振る舞いと「frontend が direct FS read / Git orchestration を持たない」受け入れ基準のみを対象とし、旧 command の即時削除は振る舞いとして要求しない。

## Open Questions

なし（requirements の Open Questions は解消済み。残る詳細は design で詰める）。
