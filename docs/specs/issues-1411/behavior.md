# Behavior

関連: #1411（milestone 84「Agentチャット安定化」／ Phase 0 ／ L10）

正本参照:
- 要求: `docs/specs/issues-1411/requirements.md`
- 問題インベントリ: `specs/milestone-84-agent-chat-stabilization/agent-chat-instability-audit.md` の **ST-5**
- ライフサイクル理想形: `specs/milestone-84-agent-chat-stabilization/agent-chat-ideal-lifecycle.md` の **I13**

本文書は session runtime lock 機構（`SessionRuntimeLocks` / `acquire_session_runtime_lock` / `acquire_session_lock` / `SessionRuntimeLockGuard`）の観測可能な振る舞いを定義する。本変更は backend（Rust）内部完結であり、外部から観測可能な UI/CLI の振る舞いは追加・変更しない。ここでの「観測」は Rust ユニットテスト・ソース（rustdoc）・ビルド構成（test / production）からの観測を指す。

## 仮定

- 仮定1: 振る舞いは lock 機構の契約（per-session 排他の維持、prune の確実性、再入検出）として観測する。具体データ構造（pending 集合の持ち方等）は design.md で確定する実装詳細であり、本文書では規定しない。
- 仮定2: 「未参照エントリ」とは、解放済みで他に保持者が無い lock エントリ（`Arc::strong_count == 1` 相当）を指す。
- 仮定3: 再入検出の粒度は「別 session の lock を保持したまま acquire するケース」を最低限として design.md で確定する。本文書では「規約違反の再入がテストビルドで検出される」ことのみを要求する。
- 仮定4: 既存経路の棚卸しで明確な違反が無ければ「違反なし」を棚卸し結果として記録する（requirements 仮定 A4 に従う）。
- 仮定5: 外部（UI/CLI）から観測可能な振る舞いの変更は無い。既存の per-session 排他モデルは維持される。

## Feature: session runtime lock の保持規約と確実な解放

  session ごとの runtime 処理は per-session の排他ロックで直列化される。
  本 feature は、その排他が維持されること、解放済み lock エントリが無期限に
  蓄積しないこと、規約違反の lock 再入がテストビルドで検出されることを保証する。

  Background:
    Given session runtime lock 機構が初期化されている
    And lock エントリを管理する map が存在する

  Rule: 同一 session の runtime 処理は排他的に直列化される

    Scenario: 同一 session に対する二重 acquire は直列化される
      Given ある session の runtime lock が取得されている
      When 同一 session の runtime lock を別のフローが取得しようとする
      Then その取得は先行フローが lock を解放するまで待機する
      And 先行フローの解放後に取得が完了する

    Scenario: 異なる session の runtime lock は互いに独立して取得できる
      Given ある session の runtime lock が取得されている
      When 別の session の runtime lock を取得しようとする
      Then その取得は先行フローの解放を待たずに完了する

    Scenario: fan-out child の workflow activation は公開後の外部操作より先に予約される
      Given 複数の child session を開始する fan-out がある
      And 先頭 child の backend start が完了待ちで停止している
      When 公開済みの後続 child に user turn または close 操作が行われる
      Then 後続 child の workflow activation が先に session lock を取得している
      And 外部操作は workflow prompt の開始を追い越さない

    Scenario: fan-out activation の中断は child task の終了を待つ
      Given 先頭 child の backend start が完了待ちで停止している
      And 後続 child の activation task も session lock を予約している
      When workflow が stop または abort される
      Then cancel acknowledgment 後の decision 待ちでは全 child activation が静止している
      And 全 child の activation task は abort され、終了確認後に中断処理が完了する
      And 全 child の session lock は中断完了前に解放される
      And terminal cleanup は全 child の終了確認後に実行される
      And backend start の待機を解除しても中断後に後続 child の start は発生しない

  Rule: lock 保持中の規約が rustdoc に明記される

    Scenario: 公開 API に保持規約が記述されている
      Given `acquire_session_lock` と `acquire_session_runtime_lock` の定義
      Then rustdoc に「lock 保持中に別 session の runtime lock を取得しない」旨が明記されている
      And rustdoc に「backend I/O の await は最小範囲に留める」旨が明記されている
      And rustdoc に「UI/event への emit は lock 外で行う」旨が明記されている

  Rule: 既存の lock 保持経路は規約に適合する（または違反が列挙される）

    Scenario: 既存経路の棚卸しで違反が是正されている
      Given lock を保持する既存の呼び出し経路群
      When 各経路を規約 (a) 別 session lock 取得禁止 / (b) backend I/O await 最小 / (c) emit はロック外 に照らして棚卸しする
      Then 規約違反は本 ISSUE のスコープ内で是正される
      And 是正が過大な違反は違反一覧（ファイル・行・違反種別）として ISSUE に列挙され分割判断される

    Scenario: 明確な違反が無い場合の棚卸し結果
      Given 棚卸し対象の既存経路に明確な規約違反が無い
      Then 棚卸し結果として「違反なし」が記録される

  Rule: 解放済みの未参照 lock エントリは無期限に蓄積しない

    Scenario: 解放後の未参照エントリは次回 acquire で除去される
      Given ある session の runtime lock が取得され、その後解放されている
      And その session の lock エントリを他に保持しているフローが無い
      When 任意の session に対して次回 acquire_session_runtime_lock が実行される
      Then 解放済みで未参照のエントリは map から除去されている

    Scenario: 保持中のエントリは prune で除去されない
      Given ある session の runtime lock が現在も保持されている
      When 別のフローが acquire_session_runtime_lock を実行する
      Then 保持中の session のエントリは map に残る

    Scenario: prune はランタイムハンドルの有無に依存しない
      Given tokio runtime handle を取得できない文脈で lock guard が drop される
      When その後に acquire_session_runtime_lock が実行される
      Then 解放済みで未参照のエントリは除去され、prune が skip されない

    Scenario: 多数 session の取得と解放を繰り返してもエントリが無限蓄積しない
      Given 多数の異なる session について lock の取得と解放を繰り返す
      When 各解放後に後続の acquire が発生する
      Then map のエントリ数は保持中の lock 数に相当する範囲に収束し、無期限には増加しない

  Rule: 規約違反の lock 再入がテストビルドで検出される

    Scenario: 別 session の lock を保持したまま acquire するとテストビルドで検出される
      Given テストビルドである
      And あるフローが session A の runtime lock を保持している
      When 同一フローが解放前に session B の runtime lock を取得しようとする
      Then 規約違反として test profile に依存せず検出される（`#[cfg(test)]` 内の `assert!` 等で失敗する）

    Scenario: lock を保持していない状態での acquire は検出されない
      Given テストビルドである
      And 現在いずれの session lock も保持していない
      When session の runtime lock を取得する
      Then 再入としては検出されず、取得は正常に完了する

    Scenario: 同一フローの acquire future を並行 poll すると取得待機中でも検出される
      Given テストビルドである
      And 別 task が session A と session B の runtime lock を保持している
      When 同一フローが session A と session B の acquire future を join で並行 poll する
      Then 2つ目の acquire は per-session lock の取得前に規約違反として検出される
      And cancel または panic で破棄された取得前 owner 予約は残存しない

    Scenario: 解放後の逐次 acquire は再入として検出されない
      Given テストビルドである
      And session A の lock を取得し解放した
      When その後に session B の lock を取得する
      Then 再入としては検出されず、取得は正常に完了する

  Rule: production ビルドの挙動・性能は変わらない

    Scenario: 再入検出の仕掛けは production ビルドに影響しない
      Given production ビルドである
      When session runtime lock の取得・解放が行われる
      Then 再入検出のための挙動・性能上のオーバーヘッドは発生しない
      And lock の排他・prune の振る舞いは従来どおりである

  Rule: 外部から観測可能な振る舞いは変わらない

    Scenario: UI/CLI から見た session の振る舞いは不変
      Given 本変更の適用前後
      When 通常の agent session 操作（turn 開始・完了・queue 処理等）が行われる
      Then UI/CLI から観測可能な振る舞いに差異は無い

## Open Questions

なし
