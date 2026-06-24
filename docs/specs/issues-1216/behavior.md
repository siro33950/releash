# Behavior

Startup orphan cleanup を non-blocking service 化し、visible startup を cleanup でブロックしない・新規 spawn を誤 kill しない・cleanup を safe metadata として観測できる振る舞いを定義する。

実装詳細（ordering 機構の具体構造・スレッド/async の選択・モジュール配置・経路）は含めず、外部から観測可能なビジネスルールとして記述する。具体的な観測フィールドの粒度・命名は design.md で確定する。

## Feature: Startup orphan cleanup の non-blocking service 化

アプリ起動時の orphan process cleanup を visible startup の critical path から外しつつ、新規 spawn したプロセスを誤って kill しない順序を Rust 側で保証し、cleanup の実行状態・失敗・対象数を user data を含まない safe metadata として観測できるようにする。既存の孤児回収と判定方針は維持する。

Background:
```gherkin
Given Releash が unix プラットフォーム上で起動される
And 孤児プロセスの cleanup は Rust 側のサービスとして実行される
And cleanup の起動・順序保証・観測のロジックはすべて Rust 側に存在する
```

---

## Rule R1: orphan cleanup は visible startup（first window ready）をブロックしない

```gherkin
Scenario: 多数の .pid ファイルがあっても first window ready が遅延しない
  Given startup 時に多数の .pid ファイルや終了待ちの孤児プロセスが存在する
  When アプリが起動し cleanup が実行される
  Then first window ready は cleanup の所要時間（PID 走査・SIGTERM 後の sleep・SIGKILL）によって遅延しない
  And cleanup は visible startup の critical path に乗らない

Scenario: cleanup はバックグラウンドで起動時に 1 回実行される
  Given アプリが起動する
  When startup 経路が進行する
  Then cleanup は起動時に 1 回バックグラウンドで実行される
  And startup は cleanup の完了を待たずに先へ進む
```

仮定: cleanup は従来どおり起動時に 1 回起動するが、setup の同期経路（join）から外しバックグラウンドタスクとして走らせる（requirements 仮定「cleanup の起動タイミング」）。

---

## Rule R2: cleanup 完了前に spawn された新規プロセスを孤児として誤 kill しない

```gherkin
Scenario: cleanup 実行中に spawn された新規プロセスは kill されない
  Given cleanup がバックグラウンドで実行中である
  When その間に新規の agent / bridge プロセスが spawn される
  Then その新規プロセスは cleanup によって孤児として kill されない

Scenario: cleanup 完了前に spawn されたプロセスは cleanup 対象集合に含まれない
  Given 自インスタンスの起動以降に新規プロセスが spawn された
  When cleanup が孤児判定を行う
  Then 自インスタンス起動以降に作成された PID は cleanup の対象集合に含まれない

Scenario: 順序保証は setup の同期 join ではなく Rust の明示的機構で成立する
  Given cleanup と新規 spawn の間に順序保証が必要である
  When 新規 spawn が cleanup 完了前に発生する
  Then 誤 kill 防止は setup でのブロッキング join に依存せず Rust 側の明示的な順序機構で成立する
```

仮定: 順序保証は cleanup の完了状態を表す共有状態（完了フラグ / 完了通知等）を Rust 側に持ち、自インスタンス起動以降に作成された PID を対象外とすることで担保する。具体方式は design.md で確定する。

---

## Rule R3: cleanup の実行状態・失敗・対象数を safe metadata として観測できる

```gherkin
Scenario: cleanup の完了が観測できる
  Given cleanup がバックグラウンドで実行された
  When cleanup が正常に完了する
  Then cleanup の実行状態として「完了」が観測できる

Scenario: cleanup の失敗が観測できる
  Given cleanup の実行中に失敗が発生する
  When cleanup が終了する
  Then cleanup の実行状態として「失敗」が観測でき、完了と区別できる

Scenario: cleanup の対象数が観測できる
  Given cleanup が .pid ファイルを走査し孤児を処理する
  When cleanup が完了または失敗する
  Then 走査した PID 数と孤児として処理した（SIGTERM/SIGKILL を送った、または PID ファイルを除去した）数が観測できる

Scenario: 観測は構造化ログと既存 telemetry 経路で公開される
  Given cleanup の実行状態・失敗・対象数を観測する必要がある
  When cleanup が状態を報告する
  Then その metadata は構造化ログと既存の telemetry 経路で観測できる
  And frontend / 運用向けの専用 read コマンドは追加されない
```

仮定: 観測公開先は構造化ログと既存 telemetry 経路（`other::telemetry`）で確定済み（requirements 合意済み）。対象数は最低限「走査した .pid ファイル数」と「孤児として処理した数」を含む。粒度は design.md で確定する。

---

## Rule R4: cleanup のログ・metadata に user data を含めない

```gherkin
Scenario: cleanup の observation に user data が含まれない
  Given cleanup が状態・対象数を metadata / ログとして出力する
  When その metadata / ログを参照する
  Then command body・worktree path・session 本文などの user data は含まれない
  And 含まれるのは状態・失敗の有無・対象数などの safe metadata のみである
```

---

## Rule R5: non-blocking 化後も既存の孤児回収が維持される

```gherkin
Scenario: 停止した旧インスタンスのプロセス群が起動ごとに掃除される
  Given 前回の Releash インスタンスが残した孤児プロセス群が存在する
  When アプリが起動し cleanup が実行される
  Then その孤児プロセス群は最終的に回収される
  And non-blocking 化によって孤児が回収されないまま放置される状態を新たに作らない

Scenario: 既存の孤児判定と保守的 skip が維持される
  Given 所有者同一性 / PID 再利用検出で判定不能なプロセスが存在する
  When cleanup が孤児判定を行う
  Then 既存の所有者同一性判定・PID 再利用検出・保守的 skip 方針（issue #1024）が維持される
  And SIGTERM → 最大待機 → SIGKILL の昇格手順は変更されない
```

仮定: 判定アルゴリズム（PID 再利用検出 / 所有者同一性 / 昇格手順）は本 Issue では変更しない（requirements 非スコープ）。

---

## Rule R6: cleanup の対象範囲は unix に限定される

```gherkin
Scenario: 非 unix プラットフォームでは cleanup を行わない
  Given アプリが非 unix プラットフォームで起動される
  When startup 経路が進行する
  Then orphan cleanup は実行されない
  And non-blocking 化やその他の変更によって非 unix の挙動は変わらない
```

仮定: 対象は現状どおり unix（`#[cfg(unix)]`）に限定する（requirements 仮定「プラットフォーム範囲」）。

---

## Rule R7: cleanup と新規 spawn の race が自動テストで検証される

```gherkin
Scenario: race を検証する自動テストが green である
  Given cleanup と新規 process spawn の race を再現する状況
  When cleanup 実行中／前後に新規プロセスを spawn する
  Then 新規プロセスが誤って kill されないことを検証する自動テストが存在する
  And そのテストは green である
```

---

## 仮定（requirements.md より引き継ぎ）

- cleanup は従来どおり起動時に 1 回起動するが、setup の同期経路（join）から外しバックグラウンドタスクとして走らせ、setup は完了を待たない。
- 順序保証は cleanup の完了状態を表す共有状態を Rust 側に持ち、自インスタンス起動以降に作成された PID を cleanup 対象外とすることで担保する。具体方式は design.md で確定する。
- 観測公開先は構造化ログと既存 telemetry 経路（`other::telemetry`）で確定（合意済み）。frontend / 運用向け専用 read コマンドは追加しない。
- 対象は unix（`#[cfg(unix)]`）限定。非 unix では従来どおり cleanup を行わない。
- 「対象数」は最低限、走査した .pid ファイル数と孤児として処理した数を指す。粒度は design.md で確定する。
- 判定アルゴリズム（PID 再利用検出 / 所有者同一性 / SIGTERM→SIGKILL 昇格）は変更しない。

## Open Questions

なし（requirements.md ですべて解消済み。本振る舞い定義でも追加の未確定事項なし）。
