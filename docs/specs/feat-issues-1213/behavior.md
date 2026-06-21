# Behavior

対象 Issue: #1213
要求の正本: `requirements.md`（本ディレクトリ）

本書は、session storage を **summary index** と **message paging** に分離したときに外部から観測できる振る舞いを Gherkin で定義する。保存フォーマット・cursor 実装方式・fork 実装方式・migration 方式といった内部経路は `design.md` で確定するため、ここでは扱わない。

## 仮定

- **A1（summary の構成要素）**: session 一覧の summary は、id / worktree / title（代表メッセージ）/ state / message 件数 / 復帰用識別子（`agent_session_id`・`context_carry`）など軽量メタのみで構成し、message body（各メッセージの parts 本文・attachment 実体）を含まない。これは既存 `SessionSummary` が保持する範囲に相当する。
- **A2（cursor の向き）**: session を開いた直後は最新側（末尾）の 1 page を取得し、cursor を使って過去方向（古いメッセージ方向）へ遡って追加 page を取得する。cursor は挿入順に対して安定な message 識別子ベースとし、同一 cursor + limit の再取得は同一 page を返す。
- **A3（attachment）**: page に含まれるのは attachment への参照（識別子・メタ）であり、binary 実体は別途必要時に取得する。page 取得自体が attachment binary 全量を hydrate しない。
- **A4（性能基準）**: 「本文総量に比例しない / スケールしない」は相対基準とし、具体的な時間・バイト閾値は #1209 の telemetry に委ねる。本書のシナリオは「body 総量が増えても観測コストが支配的に増えない」ことを振る舞いとして定義する。
- **A5（保存先）**: 保存先は `~/.releash/sessions/` 配下を維持する。

## 振る舞い定義

```gherkin
Feature: Agent session の summary index と message paging
  session storage を summary index と message body に分離し、
  一覧取得・単一取得・保存・fork が会話本文の総量に依存しないようにする。
  既存 session の非破壊と、復帰時 context restoration（#1190）との整合を保つ。

  Background:
    Given 永続化済みの Agent session が複数存在する
    And その中には message 件数・本文量の大きい長い会話の session が含まれる

  # ---- 一覧取得 ----
  Rule: session 一覧は summary index のみで構成する

    Scenario: 一覧取得は message body を読まない
      Given 長い会話を含む多数の session が保存されている
      When ユーザーが session 一覧を取得する
      Then 各 session の summary（id・worktree・title・state・件数・復帰用識別子）が返る
      And いずれの session の message body も読み込まれない

    Scenario: 一覧取得コストは本文総量に支配されない
      Given 同じ session 数で message 本文の総量だけが大きく異なる 2 つの状態がある
      When それぞれの状態で session 一覧を取得する
      Then 取得コスト（時間・読み込みバイト）の差が本文総量の差に比例して増大しない

  # ---- page 取得 ----
  Rule: message body は page 単位で取得する

    Scenario: 最新ページを取得する
      Given 長い会話の session がある
      When その session を cursor 未指定・limit 指定で page 取得する
      Then 最新側の limit 件分の message body・attachment 参照・token / run metadata が返る
      And 続きを辿るための cursor が返る

    Scenario: cursor で過去方向の続きを辿る
      Given 最新ページとその cursor を取得済みである
      When 同じ session をその cursor・limit 指定で page 取得する
      Then 直前ページより過去側の limit 件分の message body が返る
      And さらに過去がある場合は次の cursor が返る

    Scenario: 先頭に到達したら続きがないと分かる
      Given 会話の先頭まで遡って page 取得している
      When 続きを辿る cursor で page 取得する
      Then それ以上過去の message がないことが分かる結果が返る

    Scenario: 同一 cursor の再取得は同じページを返す
      Given ある cursor・limit で page を取得済みである
      When 同じ session を同じ cursor・limit で再取得する
      Then 同一の message 集合が返る

  # ---- 単一取得 ----
  Rule: 単一 session 取得は本文量に比例した複製をしない

    Scenario: 単一 session 取得が本文全量を複製しない
      Given 長い会話の session がある
      When その session を取得する
      Then summary と必要な page のみが返り、message 本文全量の複製は発生しない

  # ---- 保存 ----
  Rule: session 保存は本文全量の繰り返し書き込みに依存しない

    Scenario: streaming 中の保存が全量書き込みにならない
      Given streaming 中の session に message が逐次追加されている
      When session が永続化される
      Then 既存 message 本文全量を毎回書き直さずに永続化される

    Scenario: 保存後に summary index と body が整合する
      Given session に新しい message が追加され永続化された
      When その session の summary を取得する
      And その session を page 取得する
      Then summary の件数・代表メッセージが body の実態と一致する

  # ---- fork ----
  Rule: fork は本文を即時 full copy しない

    Scenario: 長い会話の fork が即時全量複製を起こさない
      Given 長い会話の session がある
      When その session を fork する
      Then 新しい session が成立する
      And fork 時点で message 本文全量の即時複製は発生しない

    Scenario: fork した session を開いて続行できる
      Given 長い会話を fork した session がある
      When その fork session を開いて page 取得する
      Then 元 session 由来の message body を page 単位で参照でき、会話を続行できる

  # ---- 保存正典と復帰整合 ----
  Rule: 保存正典は復帰時 context restoration と矛盾しない

    Scenario: 復帰に必要な識別子と messages が欠落しない
      Given 過去の session が正典として保存されている
      When アプリ再起動後にその session を復帰して開く
      Then 復帰に必要な agent_session_id・context_carry が summary から得られる
      And 復元対象の messages を page 取得で揃えられる
      And 復帰後に会話を継続できる（#1190 の context restoration が成立する）

    Scenario: 見えているのに復帰できない状態を作らない
      Given paging で表示できている session がある
      When その session を復帰しようとする
      Then 表示できているにもかかわらず復帰に必要なデータが欠落していることがない

  # ---- 既存フォーマット互換 ----
  Rule: 旧フォーマットの既存 session を非破壊で扱う

    Scenario: 旧フォーマットの session を開いてもデータが壊れない
      Given 変更前の旧フォーマット（full JSON）で保存済みの session がある
      When その session を一覧・取得・page 取得で開く
      And 表示・続行する
      Then messages・メタデータが破壊されない

  # ---- frontend 初期描画 ----
  Rule: 初期描画は全 message body を hydrate しない

    Scenario: 長い会話を開いた初期描画は可視ページのみ取得する
      Given 長い会話の session がある
      When ユーザーがその session を開く
      Then 初期描画では可視 window 相当の page のみが取得・表示される
      And 全 message body は読み込まれない

    Scenario: スクロールで過去ページを順次取得する
      Given 長い会話を開いて初期ページを表示している
      When ユーザーが過去方向へスクロールして続きを要求する
      Then 追加の page が cursor 単位で取得・表示される
```

## スコープ外（本書では振る舞いを定義しない）

- ターン完了時の `streaming_parts` 解放（#1194）。
- 閉じた session / 非表示 worktree の body 退避、本格的な仮想化・LRU（#1195）。
- streaming の seq delta protocol 化（#1214）。
- legacy `content` / `thinking` / `activities` の全面廃止。
- 保存フォーマット・cursor 実装方式・fork 実装方式・migration 方式（`design.md` で確定）。

## Open Questions

なし
