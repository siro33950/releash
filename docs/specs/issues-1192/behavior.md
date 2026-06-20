# Behavior

## Source
- requirements.md

## Behavior

```gherkin
Feature: AgentChat continues multiple turns within one session
  Background:
    Given an AgentChat session identified by a single chat session id
    And the session has no shared context with any other session

  Rule: A completed turn does not end the ability to send more turns
    Scenario: A user sends a second message after the first turn completes
      Given the user has sent a first message in the session
      And the Agent has responded and the first turn has completed normally
      When the user sends a second message in the same session
      Then the user's second message appears in the conversation
      And the Agent responds to the second message
      And the session is not left unresponsive

    Scenario: A user continues sending further turns in the same session
      Given the user has exchanged two or more completed turns in the session
      When the user sends another message in the same session
      Then the Agent responds to that message
      And every subsequent turn in the session keeps receiving Agent responses

  Rule: A continued turn keeps the existing session context
    Scenario: The Agent responds to a later turn in an ongoing session
      Given the user has completed at least one turn in the session
      When the user sends a follow-up message that depends on earlier turns
      Then the Agent's response continues the same session's context
      And the follow-up is not treated as a brand-new unrelated conversation

  Rule: A turn that completes normally leaves the session ready for the next turn
    Scenario: A turn finishes after the Agent produces its result
      Given the Agent has produced a final result for the current turn
      When the turn completes normally
      Then the session is ready to accept the next message
      And the next message starts a new turn that the Agent responds to

  Rule: An unresponsive Agent runtime is never reused for a new turn
    Scenario: The Agent runtime is no longer alive when the next turn starts
      Given a previous turn in the session has completed
      And the Agent runtime that handled it is no longer alive
      When the user sends the next message in the same session
      Then Releash does not treat the dead runtime as if it were able to respond
      And Releash restarts the Agent runtime so the new turn is handled
      And the restarted turn continues the existing session context
      And the Agent responds to the message

  Rule: Existing crash handling during a turn is preserved
    Scenario: The Agent runtime stops unexpectedly while streaming a response
      Given a turn is in progress and the Agent is streaming its response
      When the Agent runtime stops unexpectedly before the turn completes
      Then the user is shown that the turn failed
      And any partial streamed response is finalized as it was before this change

    Scenario: The Agent runtime stops unexpectedly while starting up
      Given a turn is starting and the Agent runtime is still initializing
      When the Agent runtime stops unexpectedly before it becomes ready
      Then the user is shown that the turn failed
      And the failure is handled the same way as before this change
```

## 仮定

- 「Agent runtime が生きていない」「runtime を restart する」という表現は、requirements の ②（完了後にプロセスが終了し stdout EOF となった場合の検知・再 spawn）を、内部シンボル（`apply_bridge_eof_crash` / `ensure_runtime_for_turn` 等）に依存しない外部観測可能な振る舞いとして言い換えたものである。
- requirements の ①（bridge 常駐により毎ターンの再 spawn 自体を避ける）は、利用者から見ると「完了後も次ターンに応答が返る」という観測結果に集約される。① が機能した場合は同一 runtime での継続、② のフォールバックが働いた場合は restart 後の継続となるが、いずれも `Rule: A completed turn does not end the ability to send more turns` の振る舞いを満たす。本 behavior は両経路を実装詳細として区別せず、観測される応答継続のみを規定する。
- 「streaming 中・初期化中のクラッシュ時にユーザーへ失敗を示し、partial response を finalize する」は既存のユーザー可視挙動であり、本修正で退行させないことを振る舞いとして明記している。具体的なエラーメッセージ文言・consolidate のデータ形式は実装詳細として Gherkin には持ち込まない。
- セッションは単一の chat session id を境界とし、他セッションとのコンテキスト共有は対象外（Non-goals）であるため Background に固定前提として置いた。

## Open Questions

なし
