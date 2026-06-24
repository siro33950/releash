# Behavior

`RepositoryStateService` による watcher / git scan / read model 集約（#1210）の振る舞いを Gherkin で定義する。

実装経路・モジュール名・command 名は本書に持ち込まず、外部から観測可能なビジネスルールに絞る。詳細な配置・技術選定は `design.md` で扱う。

## 用語

- **worktree**: 監視・走査の単位となる作業ツリー（repo）。1 worktree につき 1 系統の watcher と git scan を持つ。
- **subscriber**: 同一 worktree の状態を必要とする利用側（status 表示 / diff tree / branch 一覧 / dirty count / ReviewPanel 等）。watcher を自前で起動せず、service の通知を購読する。
- **snapshot**: 1 回の集約 scan に由来する versioned な状態。status / diff stats / branch cards / worktree dirty count / diff file tree を同一 snapshot から導出する。
- **version**: snapshot に付与される単調増加シーケンス。subscriber は version で新旧を判定し、逆行は起きない。
- **集約 read model**: snapshot から導出される観測対象 = Git status / diff stats / branch cards / worktree dirty count / diff file tree。
- **invalidate**: ファイル変更通知（watcher の notify）を起点とする再走査要求。重い read model 生成を伴わず、再走査の予約だけを行う。
- **debounce**: invalidate を受けてから実際の scan を始めるまでの待機（既存挙動踏襲 = 300ms 相当）。連続変更をまとめる。
- **stale フラグ**: 返した snapshot が最新の変更をまだ反映していない（background refresh 中である）ことを示す。
- **loading フラグ**: 初回 scan / 再 scan が進行中であることを示す。
- **limited フラグ**: threshold により内容が省略・打ち切りされていることを示す。
- **ignored files**: gitignore 対象ファイル。default snapshot には含めず、opt-in 経路でのみ取得する。

## Feature: worktree ごとの単一 watcher と集約 scan

同一 worktree に対する watcher と git scan を 1 系統に集約し、複数の subscriber が個別に watcher を起動しないようにする。

### Background

```gherkin
Given ある worktree が開かれている
And その worktree の状態を必要とする subscriber が複数存在しうる
```

### Rule: 同一 worktree の watcher と git scan は 1 系統に集約される

```gherkin
Scenario: 複数 subscriber が同一 worktree を購読しても watcher は重複起動しない
  Given worktree W の状態を status 表示 / diff tree / branch 一覧 が必要としている
  When それぞれが W の状態購読を開始する
  Then W に対して起動される file watcher / Git dir watcher は 1 系統だけである
  And 各 subscriber は同一の集約された通知から更新を受け取る
```

```gherkin
Scenario: 集約 read model は 1 回の scan から導出される
  Given worktree W の snapshot が生成されている
  When status / diff stats / branch cards / worktree dirty count / diff file tree を参照する
  Then それらはすべて同一 snapshot から導出される
  And 参照ごとに別々の git2 走査が重複起動しない
```

```gherkin
Scenario: 複数 worktree はそれぞれ独立した 1 系統を持つ
  Given worktree W1 と worktree W2 が開かれている
  When 双方の状態購読を開始する
  Then W1 と W2 はそれぞれ独立に 1 系統ずつの watcher と scan を持つ
  And 一方の変更が他方の snapshot version に影響しない
```

## Feature: versioned snapshot と状態フラグ

snapshot に version と stale / loading / limited フラグを持たせ、subscriber が新旧と状態を判定できるようにする。

### Background

```gherkin
Given worktree W の状態購読が確立している
```

### Rule: snapshot は version と状態フラグを伴って通知される

```gherkin
Scenario: subscriber は version で新旧を判定できる
  Given subscriber が version N の snapshot を保持している
  When service が version N+1 の snapshot を通知する
  Then subscriber は N+1 を新しい snapshot として採用できる
  And version は単調増加し逆行しない
```

```gherkin
Scenario Outline: snapshot のフラグが状態を表現する
  Given worktree W が "<状態>" にある
  When subscriber が snapshot を受け取る
  Then snapshot の "<フラグ>" が立っている

  Examples:
    | 状態                             | フラグ   |
    | 最新変更を未反映で background refresh 中 | stale   |
    | 初回 scan / 再 scan が進行中     | loading |
    | threshold により内容が打ち切られた | limited |
```

### Rule: 初回購読時は loading を示し、完了で snapshot を通知する

```gherkin
Scenario: 初回購読では loading を経て最初の snapshot に到達する
  Given worktree W の snapshot がまだ生成されていない
  When subscriber が購読を開始する
  Then loading 状態であることが分かる
  And 初回 scan 完了時に version 付き snapshot が通知される
```

## Feature: 変更検知から snapshot 更新までの非同期経路

watcher の notify callback で重い read model 生成を同期実行せず、invalidate → debounce → background worker の経路で snapshot を更新する。

### Background

```gherkin
Given worktree W の状態購読が確立している
And W の snapshot が version N で存在する
```

### Rule: notify callback は invalidate だけを発行する

```gherkin
Scenario: ファイル変更の通知では重い read model 生成を同期実行しない
  Given W 内のファイルが変更される
  When watcher がその変更を検知する
  Then 検知の時点では branch 一覧生成や dirty count 等の重い走査を同期実行しない
  And 再走査要求（invalidate）だけが発行される
```

```gherkin
Scenario: 連続変更は debounce でまとめて 1 回の scan になる
  Given debounce 期間内に W への変更が複数回発生する
  When debounce 期間が満了する
  Then それらの変更に対する scan が 1 回だけ実行される
```

### Rule: 中規模 repo では stale snapshot を即時に返しつつ background で更新する

```gherkin
Scenario: scan が長い場合は既存 snapshot を stale として返し、完了後に最新へ更新する
  Given W の再 scan に時間がかかる
  When subscriber が現在の状態を要求する
  Then 既存（version N）の snapshot が stale フラグ付きで即時に返る
  And background refresh が走り、完了時に version N+1 の snapshot が通知される
  And N+1 の snapshot では stale フラグが下りている
```

## Feature: scan の cancel / supersede

scan 実行中に次の変更が来たら、進行中の古い scan を打ち切り、最新状態に対する scan を優先する。

### Background

```gherkin
Given worktree W の状態購読が確立している
```

### Rule: 進行中の scan は新しい変更で打ち切られ、古い結果で上書きしない

```gherkin
Scenario: scan 中の追加変更で古い scan が supersede される
  Given W に対する scan が進行中である
  When scan の完了前に W へ新たな変更が発生する
  Then 進行中の古い scan は cancel / supersede される
  And 最新状態に対する scan が優先して実行される
```

```gherkin
Scenario: 古い scan の結果で新しい snapshot を上書きしない
  Given 古い scan の完了より先に新しい scan が version N+1 の snapshot を通知している
  When 打ち切られた古い scan の結果が遅れて到着する
  Then その古い結果で version N+1 の snapshot を上書きしない
  And version は逆行しない
```

## Feature: ignored files の既定除外と opt-in

default snapshot では ignored files を返さず、必要な UI だけが opt-in で取得できるようにする。

### Background

```gherkin
Given worktree W に gitignore 対象のファイルが存在する
```

### Rule: ignored files は default snapshot に含めず opt-in でのみ取得する

```gherkin
Scenario: default snapshot に ignored files は含まれない
  Given subscriber が default の状態を購読している
  When snapshot を参照する
  Then ignored files は含まれない
```

```gherkin
Scenario: ignored files が必要な UI は opt-in で取得できる
  Given ignored files の表示を必要とする UI がある
  When その UI が ignored files を含む取得を opt-in で要求する
  Then ignored files を含む結果が得られる
```

## Feature: 既存表示挙動の非回帰

集約後も、ユーザーから見える status / diff stats / branch cards / worktree dirty count / diff file tree の結果が、ignored 既定除外を除いて変わらない。

### Background

```gherkin
Given worktree W が開かれている
```

### Rule: 集約への置き換え後も観測される結果に回帰がない

```gherkin
Scenario Outline: 集約 snapshot 由来の read model が従来結果と一致する
  Given W の状態が確定している
  When "<read model>" を参照する
  Then 集約 snapshot から導出される結果は、従来経路の結果と一致する（ignored 既定除外を除く）

  Examples:
    | read model            |
    | Git status            |
    | diff stats            |
    | branch cards          |
    | worktree dirty count  |
    | diff file tree        |
```

```gherkin
Scenario: ignored 既定除外による差分は許容される
  Given 従来は ignored files を含めて status を返していた
  When 集約 snapshot 由来の default status を参照する
  Then ignored files が含まれない点のみが従来と異なる
  And それ以外の表示結果に差分はない
```

## 仮定

`requirements.md` の仮定 A1〜A7 を前提とする。本書の振る舞いに直接関わる主なものは以下。

- 本 Issue は service + 単一 watcher + 集約 scan + versioned snapshot + cancel/supersede + ignored opt-in までを担い、新 command 名 `get_review_snapshot` の導入と frontend 呼び出し置換は #1211 とする（A1）。本書の Scenario はいずれも内部 snapshot / 通知の観測可能な振る舞いとして記述し、新 command surface には言及しない。
- subscriber への通知は version を含む event で届く（A5）。version の具体的な伝達方式は `design.md` で確定する。
- `limited` の具体的な threshold 値は本 Issue では確定せず、「打ち切りを `limited` フラグで表現できること」だけを要求する（A6）。
- debounce は既存の 300ms 相当を踏襲する（A7）。

## Open Questions

なし。
