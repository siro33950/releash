## 要求

**種別**: 新機能
**ゴール**: Agent（Claude Code等）とのプロンプト送信→応答表示→返答の対話ループを、ステート管理しながら GUI 上でリッチに実行できるようにする
**背景**: ターミナル埋め込みに起因するバグが多く、ターミナルに依存していると完全にコントロールした体験構築ができない。今後のワークフロー自動化（#691）への起点となる機能
**制約**: 実装仕様セクションに詳述（モードマッピング、ブリッジプロトコル、Tauriイベント・コマンド、State管理）
**影響範囲**: 既存の PtyManager / AgentStatesMap / Hook Listener との連携が必要

## 振る舞い定義

```gherkin
Feature: エージェント対話セッション
  Agentとのプロンプト送信→応答→返答の対話ループをGUI上で実行する

  Rule: 対話の開始
    Scenario: 新しい対話を開始する
      Given セッションが選択されていない
      When ユーザーがメッセージを送信する
      Then 新しいセッションが作成される
      And エージェントが応答を開始する

  Rule: 対話の継続
    Scenario: エージェントの応答後に返答する
      Given エージェントの応答が完了している
      When ユーザーが返答メッセージを送信する
      Then エージェントが返答に対する応答を開始する

  Rule: 対話の中断
    Scenario: ストリーミング中に中断する
      Given エージェントがストリーミング中である
      When ユーザーが中断操作を行う
      Then ストリーミングが停止する

  Rule: セッション履歴の復元
    Scenario: 過去のセッションを選択する
      Given セッション一覧にセッションが存在する
      When ユーザーがセッションを選択する
      Then 選択したセッションのメッセージ履歴が表示される

Feature: ストリーミング表示ライフサイクル
  エージェントの応答をストリーミングでリッチに表示する

  Rule: ストリーミングフェーズの遷移
    Scenario: thinkingを伴う応答の遷移
      Given エージェントが応答を開始した
      When thinking内容が到着する
      Then thinkingフェーズに遷移する
      And その後テキスト内容が到着するとtextフェーズに遷移する

    Scenario: thinkingを伴わない応答の遷移
      Given エージェントが応答を開始した
      When テキスト内容が直接到着する
      Then 直接textフェーズに遷移する

  Rule: 返信待ち状態の表示
    Scenario: コンテンツ未到着時の表示
      Given エージェントが応答を開始した
      And まだコンテンツが到着していない
      Then スピナーと "Waiting..." テキストが表示される
      And 空のメッセージ枠は表示されない

  Rule: Thinkingフェーズの表示
    Scenario: thinkingセクションのデフォルト表示
      Given thinkingフェーズである
      Then thinkingセクションが閉じた状態で表示される
      And thinking内容が蓄積中であることがわかる

  Rule: Thinkingセクションの開閉
    Scenario: thinkingセクションをトグルする
      Given thinkingセクションが表示されている
      When ユーザーがthinkingセクションをクリックする
      Then 開閉状態がトグルされる

  Rule: Thinkingセクション開閉時の表示
    Scenario: 開いたthinkingセクションの表示
      Given thinkingセクションが開いた状態である
      And thinkingフェーズである
      Then リアルタイムで蓄積中のthinking内容が表示される

  Rule: Textフェーズの表示
    Scenario: テキストストリーミング表示
      Given textフェーズである
      Then テキスト内容がストリーミング表示される
      And thinkingセクションは閉じた状態を維持する

  Rule: Thinkingなし応答の表示
    Scenario: thinkingが無い場合の表示
      Given エージェントの応答にthinkingが含まれない
      Then thinkingセクションは表示されない

Feature: 権限モード管理
  エージェントの実行モードと権限承認を管理する

  Rule: モードの適用タイミング
    Scenario: 選択したモードでメッセージを送信する
      Given ユーザーがモードを選択している
      When メッセージを送信する
      Then 選択されたモードでエージェントが応答する

    Scenario: ストリーミング中のモード変更は次回から適用される
      Given エージェントがストリーミング中である
      When ユーザーがモードを変更する
      Then 現在のストリーミングには影響しない
      And 次回のメッセージ送信から新しいモードが適用される

  Rule: デフォルトモードの表示
    Scenario: 初回表示時のモード
      Given チャット画面を初めて開いた
      Then "Code" モードが選択された状態で表示される

  Rule: ツール使用の権限要求
    Scenario: エージェントがツールを使おうとする
      Given エージェントがストリーミング中である
      When エージェントがツールの使用を要求する
      Then 権限要求がペンディング状態になる

  Rule: 権限要求の表示
    Scenario: ペンディング権限要求の表示
      Given 権限要求がペンディング状態である
      Then ツール名と入力内容がインライン表示される
      And Allow / Deny ボタンが表示される

  Rule: 権限要求への応答
    Scenario: ユーザーがツール使用を許可する
      Given 権限要求がペンディング状態である
      When ユーザーが "Allow" を選択する
      Then エージェントがツールを実行して応答を続行する
      And 権限要求がクリアされる

    Scenario: ユーザーがツール使用を拒否する
      Given 権限要求がペンディング状態である
      When ユーザーが "Deny" を選択する
      Then エージェントに拒否が通知される
      And 権限要求がクリアされる

  Rule: ユーザー質問の表示
    Scenario: AskUserQuestion使用時の表示
      Given エージェントがAskUserQuestionを使用した
      Then 質問テキストと選択肢が表示される

  Rule: ユーザー質問への回答
    Scenario: ユーザーが選択肢で回答する
      Given 質問と選択肢が表示されている
      When ユーザーが選択肢を選択する
      Then 回答がエージェントに送信される

  Rule: プロセス終了時の権限要求クリア
    Scenario: エージェントプロセス終了時のクリア
      Given 権限要求がペンディング状態である
      When エージェントプロセスが終了する
      Then 権限要求が自動的にクリアされる

Feature: Agentランタイム状態の反映
  Agentの実行時状態変化をUIに反映する

  Rule: planモード状態の反映
    Scenario: Agentがplanモードに遷移する
      Given エージェントがストリーミング中である
      When エージェントがplanモードに入る
      Then UIにplanモード状態が反映される

    Scenario: Agentがplanモードを終了する
      Given エージェントがplanモード中である
      When エージェントがplanモードを終了する
      Then UIのplanモード状態が解除される

  Rule: ユーザー入力待ち状態の反映
    Scenario: Agentがユーザー入力待ちになる
      Given エージェントがストリーミング中である
      When エージェントがユーザーへの入力を要求する
      Then UIが入力待ち状態を反映する

Feature: Planモード対話
  Agentが提示するplanの表示・承認・拒否を管理する

  Rule: Planの提示
    Scenario: エージェントがplanを提示する
      Given エージェントがplanモードに入った
      When plan内容が到着する
      Then plan内容がUIに表示される
      And ユーザーが承認/拒否できる状態になる

  Rule: Planの承認
    Scenario: ユーザーがplanを承認する
      Given plan内容が表示されている
      When ユーザーがplanを承認する
      Then エージェントが実装を開始する

  Rule: Planの拒否
    Scenario: ユーザーがplanを拒否する
      Given plan内容が表示されている
      When ユーザーがplanを拒否する
      Then エージェントに拒否が通知される

Feature: モード非依存のAgent対話
  AskUserQuestionとplanモード対話はpermissionModeに関係なく動作する

  Rule: AskUserQuestionは全モードで動作する
    Scenario: 非defaultモードでAskUserQuestionが動作する
      Given permissionModeがdefault以外である
      When エージェントがAskUserQuestionを使用する
      Then 質問テキストと選択肢が表示される
      And ユーザーが回答できる

  Rule: planモード対話は全モードで動作する
    Scenario: 非defaultモードでplanモード対話が動作する
      Given permissionModeがdefault以外である
      When エージェントがplanモードに入りplan内容を提示する
      Then planの提示と承認/拒否のUIが利用可能になる
```
