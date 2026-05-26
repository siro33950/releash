# Behavior

## Source
- requirements.md

## Behavior

```gherkin
Feature: Agent と人間が共有するレビュー議論基盤

  Rule: Thread は主張となる初回 Comment を伴って成立する
    Scenario: 参加者がコード位置に紐づく主張を立てる
      Given 参加者が特定のファイル範囲についてレビュー上の主張を持っている
      When 参加者がその主張を初回 Comment として Thread を作成する
      Then その Thread は対象位置と初回 Comment を持つ open な議論として扱われる

    Scenario: 参加者がコード位置に依存しない主張を立てる
      Given 参加者が特定のファイル範囲に限定されないレビュー上の主張を持っている
      When 参加者がその主張を初回 Comment として Thread を作成する
      Then その Thread は位置不依存の open な議論として扱われる

    Scenario: 初回 Comment なしで Thread を作ろうとする
      Given 参加者がレビュー上の議論を開始しようとしている
      When 参加者が主張となる初回 Comment なしで Thread を作成しようとする
      Then Thread は成立しない

  Rule: Thread は worktree ごとに独立した議論として扱われる
    Scenario: 同じ内容の議論が別 worktree に存在する
      Given ある worktree に Thread が存在している
      When 参加者が別の worktree のレビュー議論を確認する
      Then 元の worktree の Thread は別 worktree の議論として表示または操作されない

  Rule: Thread の状態は open と resolved のみで表される
    Scenario: 参加者が Thread の状態を確認する
      Given Thread が存在している
      When 参加者がその Thread の状態を確認する
      Then Thread は open または resolved のいずれかとして扱われる

    Scenario: open な Thread を解決する
      Given open な Thread がある
      When 解決権限を持つ参加者が理由や結論を添えて Thread を解決する
      Then Thread は resolved となり、解決時の説明が Thread の解決情報として残る

    Scenario: resolved な Thread を再び open に戻そうとする
      Given resolved な Thread がある
      When 参加者がその Thread の再開を求める
      Then Thread は resolved のまま維持される

    Scenario: resolved な Thread に変更を加えようとする
      Given resolved な Thread がある
      When 参加者が Comment 追加、Stance 更新、または再解決を求める
      Then その変更は受け入れられず、Thread の内容と状態は維持される

  Rule: Comment は Thread 直下の時系列発言として追記される
    Scenario: 参加者が open な Thread に Comment を追加する
      Given open な Thread がある
      When 参加者がその Thread に発言を追加する
      Then その発言は Thread の時系列上の Comment として追加される

    Scenario: 投稿済み Comment を変更しようとする
      Given Thread に投稿済みの Comment がある
      When 参加者がその Comment の編集または削除を求める
      Then 投稿済み Comment は変更されず、必要な訂正は新しい Comment として表明される

  Rule: 参加者の種類と Agent の同一性はレビュー議論上で区別される
    Scenario: 人間と Agent が同じ Thread に参加する
      Given 人間と Agent が同じ Thread に発言している
      When 参加者が Thread の議論を確認する
      Then 各 Comment の著者は human または agent として区別できる

    Scenario: 同じ backend と model の Agent が別の実行から参加する
      Given ある backend と model の Agent が Thread に参加している
      When 同じ backend と model の Agent が別の実行から同じ Thread に参加する
      Then 両者は同じ Agent 参加者として扱われる

    Scenario: Agent の実行単位が異なる
      Given Agent の発言に実行単位を追跡するための情報が残っている
      When Stance や Resolve 権限の同一性が判定される
      Then 実行単位の違いではなく backend と model に基づく Agent 参加者として扱われる

    Scenario: 人間がレビュー議論に参加する
      Given 人間がレビュー議論を操作している
      When 人間の発言、Stance、または Resolve 権限が扱われる
      Then 人間は単一のローカル操作者として扱われ、個別識別子に依存しない

  Rule: Thread に対する Stance は参加者ごとに現在値を 1 つだけ持つ
    Scenario: Thread 作成者が Stance を明示していない
      Given 参加者が Thread を作成している
      When その参加者がまだ Stance を表明していない状態で Thread の Stance が確認される
      Then 作成者の Stance は none として扱われる

    Scenario: 参加者が初めて Stance を表明する
      Given open な Thread があり、参加者がまだ Stance を表明していない
      When 参加者が agree、disagree、または none の Stance を表明する
      Then その Stance が参加者の現在 Stance として記録される

    Scenario: 参加者が認められていない Stance を表明しようとする
      Given open な Thread がある
      When 参加者が agree、disagree、none 以外の Stance を表明しようとする
      Then Stance は変更されない

    Scenario: 参加者が Stance を再表明する
      Given open な Thread があり、参加者がすでに Stance を表明している
      When 参加者が新しい Stance を表明する
      Then その参加者の現在 Stance は最新の表明で置き換えられる

    Scenario: 別の参加者が Stance を表明する
      Given open な Thread があり、ある参加者の Stance が記録されている
      When 別の参加者が Stance を表明する
      Then それぞれの参加者の現在 Stance は独立して確認できる

  Rule: Resolve は起票者または人間だけが実行できる
    Scenario: Agent が自分の Thread を解決する
      Given Agent が作成した open な Thread がある
      When その Agent が理由や結論を添えて Thread を解決する
      Then Thread は resolved となる

    Scenario: 同じ backend と model の Agent が別の実行から自分の Thread を解決する
      Given ある backend と model の Agent が作成した open な Thread がある
      When 同じ backend と model の Agent が別の実行から Thread を解決する
      Then Thread は同じ Agent 参加者による解決として resolved となる

    Scenario: 起票者ではない Agent が他者の Thread を解決しようとする
      Given 他の参加者が作成した open な Thread がある
      When 起票者ではない Agent が Thread の解決を求める
      Then 解決は受け入れられず、Thread は open のまま残る

    Scenario: 人間が他者の Thread を解決する
      Given Agent または別の参加者が作成した open な Thread がある
      When 人間が理由や結論を添えて Thread を解決する
      Then Thread は人間の権限による解決として resolved となる

  Rule: Agent と人間は同じレビュー議論を観測できる
    Scenario: Agent がレビュー議論に参加する
      Given Agent が Releash 上のレビュー議論に参加できる状態にある
      When Agent が Thread 作成、Thread 確認、Comment 追加、Stance 変更、または自分の Thread の Resolve を行う
      Then その操作は他の参加経路と同じ議論状態と権限規則に従って反映される

    Scenario: 参加者が Thread 一覧を確認する
      Given 複数の worktree、ファイル、状態、著者、Stance を持つ Thread が存在している
      When 参加者が必要な条件で Thread 一覧を確認する
      Then 条件に合う Thread の現在状態、著者、対象位置、Stance、解決状況を確認できる

    Scenario: 参加者が Thread 詳細を確認する
      Given Thread に複数の Comment、参加者ごとの Stance、解決情報が存在している
      When 参加者がその Thread の詳細を確認する
      Then Comment の時系列、各参加者の現在 Stance、Thread の open または resolved 状態、解決情報を確認できる

    Scenario: Agent が新着や変化を確認する
      Given Agent が過去に確認した後で Thread、Comment、Stance、または解決状態が変化している
      When Agent がレビュー議論を再確認する
      Then Agent は新着 Thread や既存 Thread の変化を発見できる

    Scenario: Agent が手動送信状態に依存せず議論の変化を発見する
      Given レビュー議論に未確認の Thread、Comment、Stance、または解決状態の変化がある
      When Agent がレビュー議論の一覧または詳細を確認する
      Then Agent は手動で送信されたかどうかに依存せず、その変化を発見できる

    Scenario: 人間がデスクトップまたはリモート画面からレビュー議論を扱う
      Given 人間がレビュー議論を利用できる状態にある
      When 人間がデスクトップまたはリモート画面から Thread、Comment、Stance、または Resolve を操作する
      Then 操作経路に関係なく同じ Thread モデル、同じ Stance、同じ Resolve 権限として扱われる

    Scenario: 同じ議論を異なる参加経路から確認する
      Given 同じ worktree に Thread、Comment、Stance、Resolve 情報を持つレビュー議論がある
      When Agent と人間がそれぞれ利用可能な参加経路からその議論を確認する
      Then 参加経路に関係なく同じ現在状態と同じ履歴を確認できる

  Rule: 拒否された操作では理由を確認できる
    Scenario: 権限により操作が拒否される
      Given 参加者が権限を持たない Thread 操作を求めている
      When その操作が拒否される
      Then 参加者は権限によって拒否されたことを理解できる

    Scenario: Thread の状態により操作が拒否される
      Given resolved な Thread がある
      When 参加者が resolved 後に許可されない変更を求める
      Then 参加者は Thread の状態によって拒否されたことを理解できる

  Rule: 既存のレビューコメント導線は新しい議論モデルを表示し操作できる
    Scenario: 人間が既存のレビューコメント導線で Thread を確認する
      Given 新しい Thread、Comment、Stance、Resolve 情報を持つレビュー議論がある
      When 人間が既存のレビューコメント導線を開く
      Then human と agent の著者種別、Agent の表示名、Thread 状態、Comment 時系列、現在 Stance、実行可能な Resolve 操作を確認できる

    Scenario: 人間が既存のレビューコメント導線で議論に参加する
      Given open な Thread が既存のレビューコメント導線に表示されている
      When 人間が Thread 作成、Comment 追加、Stance 変更、または Resolve を行う
      Then その操作は新しい Thread、Comment、Stance、Resolve の議論状態として反映される

  Rule: Thread の履歴は監査目的で確認できる
    Scenario: 参加者が Thread の経緯を確認する
      Given Thread に作成、Comment 追加、Stance 表明、Resolve の履歴がある
      When 参加者がその Thread の履歴を確認する
      Then Thread が現在状態に至った経緯を時系列で確認できる

  Rule: 並行した操作は確定した順序に従って一貫した状態になる
    Scenario: 複数の Comment が並行して追加される
      Given open な Thread がある
      When 複数の参加者が並行して Comment を追加する
      Then 確定した順序で両方の Comment が保持される

    Scenario: 同じ参加者が並行して Stance を更新する
      Given open な Thread があり、同じ参加者から複数の Stance 更新が並行して行われる
      When 更新の順序が確定する
      Then 確定した順序で最後の Stance がその参加者の現在 Stance となる

    Scenario: 複数の Resolve が並行して求められる
      Given open な Thread がある
      When 複数の参加者が並行して Thread の解決を求める
      Then 最初に有効として確定した Resolve だけが Thread を resolved にする

  Rule: レビュー議論は Releash ローカルの対象に限定される
    Scenario: 参加者が外部のレビューコメントへの反映を期待する
      Given Releash 上にレビュー議論が存在している
      When 参加者が Thread、Comment、Stance、または Resolve を操作する
      Then その操作は Releash ローカルのレビュー議論として扱われ、外部サービスのレビューコメント操作としては扱われない
```
