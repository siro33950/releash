# Behavior

関連: #1254 / requirements.md

本変更は `releash review` CLI の内部処理の効率化（性能改善 / リファクタリング）であり、外部から観測可能なサブコマンドの振る舞い・引数・出力フォーマットは不変である。したがって本書は次の二種類の振る舞いを定義する。

1. **不変として維持される観測可能な振る舞い**（リグレッション防止の契約）。
2. **新たに保証される観測可能な性能不変条件**（解決コストがセッション総数・本文量に比例しない）。

性能不変条件は絶対レイテンシ閾値ではなく、「入力規模に対してコストが増大しない」という観測可能な性質として定義する（requirements「受け入れ基準の概要」「仮定」に準拠）。

## 仮定

- 本書のシナリオは `releash review` のサブコマンド `list` / `get` / `create` / `comment` / `resolve` / `history` を対象とする。サブコマンド体系・引数・出力フォーマットは現行のまま不変とする（requirements 非スコープ）。
- 「セッション解決」とは、`--session-id` から actor（`list` / `create` / `comment` / `resolve`）または worktree path（`get` / `history`）を求める処理を指す。
- 性能不変条件のシナリオは、観測可能な代理指標として「指定 1 セッション以外の meta を読まない」「対象セッションの本文（メッセージ）を読まない」という外部から検証可能な性質で表現する。具体的な内部関数・キャッシュ・ロック機構・パース手段は behavior の対象外とし、design.md で扱う。
- 「不要にブロックしない」とは、読み取り専用コマンド同士、および読み取りと書き込み監視（watcher）が、相手の処理完了を待って直列化されない状態を指す。書き込みとの整合性（書き込み途中の中途半端な状態を読まない）は維持する。
- セッションの `state` は `Open` 系（非 Closed）と `Closed` の二分類で扱う。actor 要件として `backend_id` と `selected_model` の有無を扱う。

## Feature: review CLI のセッション解決と Thread 操作の観測可能な振る舞い

### Background

```gherkin
Background:
  Given app data dir 配下に複数のセッションが存在する
  And 各セッションは worktree path を持つ
```

---

### Rule: actor を要する review コマンド（list / create / comment / resolve）の actor 解決

`list` / `create` / `comment` / `resolve` は、指定セッションから actor を解決する。actor 解決には `state != Closed` かつ `backend_id` と `selected_model` の両方が必要である。

```gherkin
Scenario: 有効なセッションで actor を解決できる
  Given セッション "S1" は state が Closed でない
  And セッション "S1" は backend_id と selected_model を持つ
  When 利用者が "S1" を session-id として list / create / comment / resolve のいずれかを実行する
  Then actor が解決され、コマンドは正常に処理される

Scenario: Closed セッションは actor として使えない
  Given セッション "S1" は state が Closed である
  When 利用者が "S1" を session-id として list / create / comment / resolve のいずれかを実行する
  Then コマンドは入力エラーで失敗する
  And エラーは「Closed セッションは review actor に使えない」旨を示す

Scenario: backend_id を持たないセッションは actor として使えない
  Given セッション "S1" は state が Closed でない
  And セッション "S1" は backend_id を持たない
  When 利用者が "S1" を session-id として list / create / comment / resolve のいずれかを実行する
  Then コマンドは入力エラーで失敗する
  And エラーは「backend_id が無く actor に使えない」旨を示す

Scenario: selected_model を持たないセッションは actor として使えない
  Given セッション "S1" は state が Closed でない
  And セッション "S1" は backend_id を持つ
  And セッション "S1" は selected_model を持たない
  When 利用者が "S1" を session-id として list / create / comment / resolve のいずれかを実行する
  Then コマンドは入力エラーで失敗する
  And エラーは「selected_model が無く actor に使えない」旨を示す

Scenario: 存在しないセッションを指定するとエラーになる
  Given session-id "S-missing" のセッションは存在しない
  When 利用者が "S-missing" を session-id として review コマンドを実行する
  Then コマンドは not found で失敗する

Scenario: 空の session-id は拒否される
  When 利用者が空文字の session-id で review コマンドを実行する
  Then コマンドは入力エラーで失敗する
```

---

### Rule: 読み取り専用コマンド（get / history）の worktree 解決

`get` / `history` は worktree path のみを必要とし、actor 用フィールド（`backend_id` / `selected_model`）や `state != Closed` を要求しない。Closed セッション・過去セッションでも読み取れる。

```gherkin
Scenario: Closed セッションでも get / history は読み取れる
  Given セッション "S1" は state が Closed である
  When 利用者が "S1" を session-id として get または history を実行する
  Then worktree が解決され、コマンドは正常に処理される

Scenario: actor 用フィールドを持たないセッションでも get / history は読み取れる
  Given セッション "S1" は backend_id も selected_model も持たない
  When 利用者が "S1" を session-id として get または history を実行する
  Then worktree が解決され、コマンドは正常に処理される

Scenario: 存在しないセッションの get / history は not found
  Given session-id "S-missing" のセッションは存在しない
  When 利用者が "S-missing" を session-id として get または history を実行する
  Then コマンドは not found で失敗する
```

---

### Rule: worktree scope の独立性

Thread は worktree 単位で分離される。あるセッション（worktree）の thread は、別 worktree のコマンドに混入しない。

```gherkin
Scenario: 別 worktree の thread は list に現れない
  Given セッション "A" は worktree "WA" を、セッション "B" は worktree "WB" を指す
  And worktree "WA" に thread "TA"、worktree "WB" に thread "TB" が存在する
  When 利用者がセッション "A" で list を実行する
  Then 結果に "TA" は含まれる
  And 結果に "TB" は含まれない
```

---

### Rule: list の出力順序と内容

`list` は thread を `updated_at` 降順で出力し、削除済み（ThreadDeleted）thread を除外する。

```gherkin
Scenario: list は updated_at 降順で出力する
  Given worktree に thread "T1"(updated_at 古い) と "T2"(updated_at 新しい) が存在する
  When 利用者が list を実行する
  Then 出力順は "T2", "T1" の順である

Scenario: 削除済み thread は list に現れない
  Given thread "T1" が作成後に削除されている
  And thread "T2" が存在し削除されていない
  When 利用者が list を実行する
  Then 結果に "T2" は含まれる
  And 結果に "T1" は含まれない

Scenario Outline: state フィルタで thread を絞り込める
  Given open な thread と resolved な thread が存在する
  When 利用者が state "<filter>" で list を実行する
  Then "<expected>" な thread のみが出力される

  Examples:
    | filter   | expected |
    | open     | open     |
    | resolved | resolved |
```

---

### Rule: セッション解決コストはセッション総数に比例しない（①）

review CLI のセッション解決は、指定 1 セッションの meta のみを読み、`sessions/` 配下の他セッションの meta を読まない。これによりセッション総数が増えても解決コストは増大しない。

```gherkin
Scenario Outline: 指定外セッションの meta を読まずに解決する
  Given app data dir に <total> 件のセッションが存在する
  And そのうち 1 件が session-id "S1" である
  When 利用者が "S1" を session-id として get / list / history のいずれかを実行する
  Then 解決のために読まれる meta は "S1" の 1 件のみである
  And 他の <total-1> 件の meta は読まれない

  Examples:
    | total |
    | 2     |
    | 50    |
    | 500   |

Scenario: セッション総数が増えても解決コストは増大しない
  Given セッション総数が異なる二つの状態（少数 / 多数）がある
  And いずれの状態でも対象セッション "S1" が存在する
  When 各状態で "S1" のセッション解決を行う
  Then 解決のために読まれる meta 件数はセッション総数によらず一定（1 件）である
```

---

### Rule: セッション解決は dir 形式 meta のみを対象にする（②）

review CLI のセッション解決は `<sessions>/<id>/meta.json` だけを対象とする。legacy flat（`<id>.json`）や legacy sidecar（`<id>.meta.json`）しか持たない session-id は解決対象外であり、本文や sidecar は読まれない。

```gherkin
Scenario: dir 形式 meta があれば本文を読まずに解決する
  Given セッション "S1" は dir 形式 meta を持つ
  When 利用者が "S1" を session-id として review コマンドを実行する
  Then actor / worktree 解決に必要な meta のみが読み取られる
  And セッション本文は読み取られない

Scenario: legacy flat しか無い session-id は NotFound
  Given session-id "S1" には legacy flat のみが存在する
  When 利用者が "S1" を session-id として review コマンドを実行する
  Then コマンドは not found で失敗する

Scenario: legacy sidecar しか無い session-id は NotFound
  Given session-id "S1" には legacy sidecar のみが存在する
  When 利用者が "S1" を session-id として review コマンドを実行する
  Then コマンドは not found で失敗する

Scenario: legacy flat / sidecar は list と restore の対象外
  Given session-id "S1" には legacy flat または legacy sidecar のみが存在する
  When セッション一覧を取得する
  Then "S1" は列挙されない
  When "S1" を restore 対象として読み込む
  Then 復元対象は存在しない
```

---

### Rule: 読み取り専用コマンドは不要にブロックしない（③）

`list` / `get` / `history` は、他の読み取りや書き込み監視（watcher）と不要に直列化しない。一方で書き込み途中の中途半端な状態は読まない（書き込みとの整合性は維持）。

```gherkin
Scenario: 読み取り同士は互いをブロックしない
  Given 同一 worktree に対する読み取り専用コマンドが複数並行して実行される
  When それらが同時にセッション/Thread を読む
  Then どの読み取りも他の読み取りの完了を待って直列化されない

Scenario: 読み取りと watcher は互いをブロックしない
  Given review-comments watcher が同一ディレクトリを監視している
  When 読み取り専用コマンドが Thread を読む
  Then 読み取りは watcher の処理完了を待たされない

Scenario: 書き込み途中の中途半端な状態は読まない
  Given Thread への書き込みが進行中である
  When 読み取り専用コマンドが同じ Thread を読む
  Then 読み取り結果は書き込み完了前か完了後のいずれか一貫した状態であり、中途半端な状態を含まない

Scenario: 書き込み系コマンドは従来どおり排他を維持する
  Given create / comment / resolve のいずれかが書き込み中である
  When 別の書き込み系コマンドが同じ Thread を書こうとする
  Then 書き込みは互いに排他され、整合性が保たれる
```

---

### Rule: thread 一覧投影は events 1 パスで行う（④）

list が用いる thread 一覧の投影は、events を 1 回走査して全 thread を構築する。thread 件数 × events 件数の二乗コストにならない。出力（`updated_at` 降順、ThreadDeleted の除外）は現行と一致する。

```gherkin
Scenario: events 1 パスで全 thread を投影する
  Given worktree の events に複数の thread に対する操作が記録されている
  When 利用者が list を実行する
  Then events は 1 回だけ走査されて全 thread が投影される
  And events は thread 件数ぶん繰り返し走査されない

Scenario: 1 パス投影の出力は従来と一致する
  Given 複数 thread（作成 / 更新 / 削除を含む）が events に記録されている
  When 利用者が list を実行する
  Then 出力は ThreadDeleted を除外し updated_at 降順に並ぶ
  And 出力内容は従来の投影結果と同一である
```

---

## Open Questions

なし。
