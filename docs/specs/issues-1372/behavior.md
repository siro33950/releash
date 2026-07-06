# Behavior

関連: #1372

`requirements.md` の「削除ルール（正典）」を、外部から観測可能な振る舞いとして Gherkin で定義する。GC はアプリ起動時に 1 回実行する backend 内部処理であり、UI/CLI の新しい操作面は追加しない。観測点は「GC 実行後に app data が残っているか／削除されているか」と「backend log への削除件数・回収 byte 数の出力」である。

## 仮定

- ここでは各データの「状態」（削除済み Workspace / 使用中 Workspace / 削除済み・アーカイブ済み・使用中の Session/Workflow / 再生成 cache / 旧形式 / 参照切れ / stale process）を前提として振る舞いを記述する。状態を判定する具体的な source of truth・パス・時刻基準は `design.md` で確定する。
- retention 境界（30 日 / 7 日）の起点時刻は対象データの更新時刻とする（正確な基準は `design.md`）。
- 「削除」は app data 上の実体（ディレクトリ／ファイル）が GC 実行後に存在しなくなることを指す。
- GC は保守的削除であり、ルールに合致すると確実に判定できたデータのみ削除し、判定できないデータは残す。

## Feature: 起動時 backend GC による app data の容量回収

Releash 起動時に backend が GC を実行し、削除ルール（正典）に従って不要な app data を削除し容量を回収する。実行中の作業とまだ参照されている生きたデータは壊さない。

### Background

```gherkin
Background:
  Given Releash の app data ディレクトリに session・workflow ログ／artifact・cache・comment/thread・process record が蓄積している
  And backend が起動する
  When GC が起動時に 1 回実行される
```

### Rule: 削除済み Workspace のログは削除する（ルール 1）

```gherkin
Scenario: worktree が存在しない Workspace に紐づくデータを削除する
  Given ある Workspace の worktree が既に削除されている
  And その Workspace に紐づく session 一式・workspace_state・workflow データと、worktree との対応付けが証明できる checkpoint が app data に残っている
  When GC が実行される
  Then その Workspace に紐づくデータは削除されている

Scenario: worktree が現存する Workspace に紐づくデータは保持する
  Given ある Workspace の worktree が現存している
  And その Workspace に紐づくデータが app data にある
  When GC が実行される
  Then その Workspace に紐づくデータは残っている
```

### Rule: 使用中 Workspace のログは Session/Workflow の状態で判断する（ルール 2）

```gherkin
Scenario: 削除済みの Session/Workflow のログを削除する（ルール 2-1）
  Given 使用中 Workspace に、削除済みの Session/Workflow のログが残っている
  When GC が実行される
  Then その削除済み Session/Workflow のログは削除されている

Scenario Outline: アーカイブ済みの Session/Workflow のログは 30 日境界で判定する（ルール 2-2）
  Given 使用中 Workspace に、アーカイブ済みの Session/Workflow のログがある
  And そのログの更新から <経過> が経っている
  When GC が実行される
  Then そのログは <結果>

  Examples:
    | 経過      | 結果             |
    | 31 日     | 削除されている   |
    | 30 日以内 | 残っている       |

Scenario: 使用中の Session/Workflow のログは削除しない（ルール 2-3）
  Given 使用中 Workspace に、使用中の Session/Workflow のログがある
  And そのログがどれだけ古くても
  When GC が実行される
  Then そのログは残っている

Scenario Outline: 再生成可能な cache は状態問わず 7 日境界で判定する（ルール 2-4）
  Given 再生成可能な cache（LSP workspace cache / TypeScript cache）がある
  And その cache の更新から <経過> が経っている
  When GC が実行される
  Then その cache は <結果>

  Examples:
    | 経過     | 結果             |
    | 8 日     | 削除されている   |
    | 7 日以内 | 残っている       |
```

### Rule: 旧形式データは状態問わず全削除する

```gherkin
Scenario: 旧形式 comments / diff-comments / threads を全削除する
  Given 旧形式の comments / diff-comments / threads が app data にある
  And それらが使用中 Workspace 分の旧コメントを含んでいても
  When GC が実行される
  Then 旧形式の comments / diff-comments / threads は全て削除されている
  And 現行 review-comments のデータは残っている
```

### Rule: 参照切れ blob は使用中 Session でもファイル単位で削除する（ルール 2-3 の例外）

```gherkin
Scenario: どの message からも参照されない tool_outputs / attachments を削除する
  Given 使用中の Session に tool_outputs / attachments の blob がある
  And その blob はその Session のどの message からも参照されていない
  When GC が実行される
  Then その参照切れ blob は削除されている
  And 同じ Session の会話（messages）は残っている

Scenario: message から参照されている blob は残す
  Given 使用中の Session に tool_outputs / attachments の blob がある
  And その blob はその Session のいずれかの message から参照されている
  When GC が実行される
  Then その blob は残っている
```

### Rule: pid が死んだ process/pid record は削除する

```gherkin
Scenario Outline: process/pid record を pid 生存で判定する
  Given process record / pid registry がある
  And 記録された pid のプロセスは <生存状態>
  When GC が実行される
  Then その record は <結果>

  Examples:
    | 生存状態     | 結果             |
    | 死んでいる   | 削除されている   |
    | 生存している | 残っている       |
```

### Rule: active session / running workflow に紐づくデータは削除しない（安全ガード）

```gherkin
Scenario: active session に紐づくデータは他のルールに合致しても保持する
  Given active な session に紐づくデータがある
  When GC が実行される
  Then そのデータは残っている

Scenario: running workflow に紐づくデータは他のルールに合致しても保持する
  Given running な workflow に紐づくデータがある
  When GC が実行される
  Then そのデータは残っている
```

### Rule: GC は削除の結果を backend log に出力する

```gherkin
Scenario: 削除件数と回収 byte 数を log 出力する
  Given GC の削除対象がある
  When GC が実行される
  Then backend log に削除件数が出力される
  And backend log に回収 byte 数が出力される
```

### Rule: GC は起動処理を過度にブロックしない

```gherkin
Scenario: 大量の app data があっても起動が過度に阻害されない
  Given app data に大量のデータ（数十 GB 規模）が蓄積している
  When backend が起動し GC が実行される
  Then backend の起動処理は過度にブロックされない
```

## Open Questions

なし。
