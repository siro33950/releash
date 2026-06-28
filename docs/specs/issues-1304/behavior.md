# Behavior

## Source
- requirements.md

## Behavior

```gherkin
Feature: agent session の streaming / status を backend-owned read model が決定し frontend は mirror として描画する
  Releash は agent session の streaming delta・session state・workflow step status を
  backend が順序付け・集約した read model として供給する。frontend は供給された結果を
  保持・描画するだけで、順序・重複・drop・resync・集約の判断は行わない。
  本変更はクリーンアーキテクチャ移行（milestone [12]）であり、外部から観測可能な
  振る舞い（streaming 表示結果・turn phase・permission pending・queue・step status 表示）は不変である。

  Background:
    Given agent session に対する streaming delta / session state / workflow step status を backend が供給する
    And frontend は backend が供給した read model を保持・描画する

  Rule: streaming delta は backend が順序保証・重複排除した順に描画される
    Scenario: 供給された delta の反映
      Given backend が順序付け・重複排除済みの streaming delta を push する
      When frontend が delta を受け取る
      Then frontend は受け取った delta をそのまま順に streaming message へ反映する
      And frontend は seq の連続性・重複・gap を判定して delta を drop しない

    Scenario: streaming の最終結果
      Given ある turn の全 delta が供給された
      When streaming が完了する
      Then 表示される streaming message の最終内容は backend が確定した内容と一致する

  Rule: 欠損 delta の補完は backend が完結させる（reconnect / replay）
    Scenario: 再接続後の整合
      Given frontend が一時的に delta を受け取れない期間があった
      When 接続が回復し backend が replay を供給する
      Then 表示される streaming message は backend が確定した順序・内容に一致する
      And frontend は欠損検出や resync 要求を自ら駆動しない

  Rule: turn phase の遷移表示は供給された session state に従う（既存挙動の維持）
    Scenario: turn の進行
      Given backend が session state の turn phase 変化を供給する
      When frontend が session state 変化を受け取る
      Then turn phase の表示は供給された phase に従って遷移する

  Rule: permission pending の表示は供給された session state に従う（既存挙動の維持）
    Scenario: permission 要求が発生する
      Given backend が pending permission を供給する
      When frontend が session state 変化を受け取る
      Then pending permission が表示される

  Rule: queue の表示は供給された session state に従う（既存挙動の維持）
    Scenario: 待機中の指示がある
      Given backend が pending queue を供給する
      When frontend が session state を受け取る
      Then queue の表示は供給された内容に従う

  Rule: workflow step status は backend が集約した read model に従って表示される
    Scenario: step status の反映
      Given backend が version 付き step status を集約した read model を供給する
      When frontend が step status を受け取る
      Then 表示される step / workflow の status は供給された集約結果に従う
      And frontend は version 比較や representative 選択を自ら行わない

    Scenario: 既に反映した version より古い更新（dedup の維持）
      Given ある step の status がある version で表示されている
      When それより古い version の status 更新が発生する
      Then 表示される step status は更新されない

    Scenario: 同じか新しい version の更新（dedup の維持）
      Given ある step の status がある version で表示されている
      When それと同じか新しい version の status 更新が発生する
      Then 表示される step status はその更新の内容に反映される
```

## 仮定

- 本変更はリファクタリング（milestone [12] クリーンアーキテクチャ移行）であり、外部から観測可能な振る舞いは不変である。したがって本 behavior は「streaming ordering / step status 集約の所有者が backend read model に移っても観測結果が従来と同一である」ことを規定する。
- raw stream-json parser（`parseStreamJson` 相当）・ANSI 整形（`stripAnsi` 相当）・worktree 単位の agent state 優先度集約（`agentStateUtils` 相当）の削除（requirements のスコープ A / B）は production 未消費の dead code 除去であり、外部から観測可能な振る舞いの変化を伴わない。したがって Gherkin の Scenario としては規定せず、受け入れ基準（grep / ファイル不存在 / `pnpm build`・`pnpm lint`・`pnpm test` 不破壊）に委ねる。
- 「frontend が streaming 適用順序・step status 集約の source of truth でない」という requirements の中心要求は、`agentChatReducer` / `useAgentSdkListeners` / `useWorktreeStepStatuses` に seq gap・重複・drop・resync・version dedup・representative 選択の domain decision が残らないという構造要件である。これは grep / コード上の不在で検証する受け入れ基準であり、外部観測可能な振る舞いではないため Gherkin では規定しない。本 behavior は「frontend は backend が確定した順序・内容・集約結果をそのまま反映する」という観測可能な性質のみを規定する。
- 「seq の連続性・重複・gap を判定して drop しない」という Rule は、backend が gap-free に順序付け・重複排除済みの delta を push する配信契約（requirements スコープ C・合意済み）への移行を、frontend 観測側の性質として表現したものである。配信契約の具体（push payload の順序保証表現、reconnect 時の replay 起点、frontend が保持する最小状態）と read model / query の transport 形・module 配置・command 名は design.md で決定する実装詳細であり、本 behavior では規定しない。
- step status の version dedup（厳密に古い version の更新を反映しない）の挙動は既存挙動の維持であり、本 behavior の該当 Rule は集約・dedup の所有者が backend query へ移っても観測結果（古い version で表示が巻き戻らない）が退行しないことの明示である。
- 削除・縮小する frontend ロジックの単体テストは、Rust 側の同等 test（ordering / duplicate / drop / replay / step aggregation）へ責務移管する。frontend test は描画・interaction・invoke/listen 配線に寄せる。テストの期待値は実装に合わせて緩めず、本 behavior が規定する観測可能な振る舞いを維持する。

## Open Questions

なし
