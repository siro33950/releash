# Behavior

## Source
- requirements.md

## Behavior

```gherkin
Feature: AgentChat permission modes
  Rule: AgentChat exposes exactly three canonical permission modes
    Scenario: A caller specifies a permission mode for AgentChat
      Given a caller is starting or configuring an AgentChat session
      When the caller chooses a permission mode
      Then the available permission modes are Ask, Edit, and Full
      And the chosen mode has the same meaning for display, saved state, restored state, and external input

    Scenario: A session is saved after a permission mode is chosen
      Given a user has chosen Ask, Edit, or Full for an AgentChat session
      When Releash saves the session permission mode
      Then the saved permission mode is one of Ask, Edit, or Full
      And the saved value can be restored without changing its permission meaning

  Rule: Each canonical permission mode has a distinct user-visible meaning
    Scenario: A user chooses Ask
      Given a user wants AgentChat to operate conservatively
      When the user chooses Ask
      Then the Agent starts with safe permissions
      And operations that need additional permission can require user confirmation

    Scenario: A user chooses Edit
      Given a user wants AgentChat to modify the workspace
      When the user chooses Edit
      Then the Agent may edit within the workspace
      And operations beyond that permission can require user confirmation

    Scenario: A user chooses Full
      Given a user wants AgentChat to operate with broad permission
      When the user chooses Full
      Then the Agent may perform broad operations without asking for additional confirmation

  Rule: Claude sessions receive the intended permission meaning
    Scenario: A Claude session starts in Ask
      Given a user is starting an AgentChat session with Claude
      When the user chooses Ask
      Then Claude receives a permission meaning that allows confirmation for needed operations

    Scenario: A Claude session starts in Edit
      Given a user is starting an AgentChat session with Claude
      When the user chooses Edit
      Then Claude receives a permission meaning that allows workspace editing

    Scenario: A Claude session starts in Full
      Given a user is starting an AgentChat session with Claude
      When the user chooses Full
      Then Claude receives a permission meaning that allows broad operation

  Rule: Codex sessions preserve confirmation behavior for safer modes
    Scenario: A Codex session starts in Ask
      Given a user is starting an AgentChat session with Codex
      When the user chooses Ask
      Then Codex operates mainly with read-oriented permission
      And operations that need additional permission can require user confirmation

    Scenario: A Codex session starts in Edit
      Given a user is starting an AgentChat session with Codex
      When the user chooses Edit
      Then Codex may edit within the workspace
      And operations that need additional permission can require user confirmation

    Scenario: A Codex session starts in Full
      Given a user is starting an AgentChat session with Codex
      When the user chooses Full
      Then Codex may perform broad operations without asking for additional confirmation

    Scenario: A Codex session in Ask or Edit reaches an operation that needs approval
      Given a Codex AgentChat session is operating in Ask or Edit
      When the Agent attempts an operation that requires approval under the selected mode
      Then the user is asked to confirm before that operation is allowed

  Rule: Runtime permission changes take effect according to the same model
    Scenario: A user changes the permission mode during an active session
      Given an AgentChat session is already running
      When the user changes the permission mode to Ask, Edit, or Full
      Then subsequent Agent operations follow the newly selected mode
      And the meaning of the new mode is the same as it would be at session start

    Scenario: A running session is changed from a safer mode to Full
      Given an AgentChat session is running in Ask or Edit
      When the user changes the permission mode to Full
      Then subsequent Agent operations may use broad permission without additional confirmation

    Scenario: A running session is changed from Full to a safer mode
      Given an AgentChat session is running in Full
      When the user changes the permission mode to Ask or Edit
      Then subsequent Agent operations are constrained by the safer selected mode
      And operations that require confirmation under that mode can ask the user

  Rule: Desktop, remote, workflow, and restored sessions share one permission meaning
    Scenario: A user starts a session from the desktop experience
      Given the desktop experience offers AgentChat permission modes
      When a user chooses Ask, Edit, or Full
      Then the resulting session uses the same permission meaning as every other AgentChat caller

    Scenario: A user starts a session from the remote experience
      Given the remote experience offers AgentChat permission modes
      When a user chooses Ask, Edit, or Full
      Then the resulting session uses the same permission meaning as every other AgentChat caller

    Scenario: A workflow specifies a permission mode
      Given a workflow supplies Ask, Edit, or Full for an AgentChat session
      When Releash prepares the session
      Then the session uses the same permission meaning as a user choosing that mode directly

    Scenario: A saved session is restored
      Given a saved AgentChat session contains Ask, Edit, or Full
      When Releash restores the session
      Then the restored session uses the same permission meaning as the saved mode

  Rule: Legacy readonly is not a new canonical permission mode
    Scenario: Releash presents permission modes to a user
      Given AgentChat is showing the available permission modes
      When the user reviews the choices
      Then readonly is not presented as a selectable permission mode
      And Ask, Edit, and Full are the only selectable modes

    Scenario: Releash creates new saved or transmitted permission data
      Given Releash records an AgentChat permission mode
      When the permission mode is newly saved or sent
      Then readonly is not used as the permission mode value

    Scenario: Existing data contains readonly
      Given existing AgentChat data contains readonly as a permission mode
      When Releash reads that data
      Then Releash handles it as Ask
      And the resulting behavior does not grant broader permission than Ask

  Rule: Invalid permission values do not silently widen permission
    Scenario: A caller supplies an unknown permission mode
      Given a caller supplies a permission mode other than Ask, Edit, Full, or a supported legacy value
      When Releash evaluates the permission mode
      Then the session does not gain broader permission through an implicit fallback
      And the caller can observe that the supplied value was not accepted as a normal permission mode
```
