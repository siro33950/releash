import Connect
import SwiftProtobuf
import XCTest

@testable import Releash

final class SettingsGeneralTests: XCTestCase {
  @MainActor func test_General購読_保存済みの値と検出エディタを表示し終了時に解放する() async throws {
    // Given
    let http = TestHTTPClient()
    http.state.withLock {
      $0.externalEditor.selected = "/Applications/Code.app"
      $0.externalEditor.editors.items = [
        .with {
          $0.name = "Code"
          $0.path = "/Applications/Code.app"
        }
      ]
      $0.terminalStartupCommand = "echo saved"
      $0.workflowConfig.approvalAutoApprove = true
    }
    let model = try await testModel(http)
    let state = GeneralSettingsState()
    // When
    state.subscribe(model)
    state.subscribe(model)
    // Then
    try await eventually {
      state.editor?.selected == "/Applications/Code.app" && state.command == "echo saved"
        && state.workflow?.approvalAutoApprove == true
    }
    XCTAssertEqual(state.editor?.editors.items.map(\.name), ["Code"])
    XCTAssertEqual(state.terminalCommand, "echo saved")
    let targets = ["external-editor", "terminal-startup-command", "workflow-config"]
    let subscriptions = http.state.withLock { state in
      state.subscriptions.filter { targets.contains($0.target) }
    }
    XCTAssertEqual(subscriptions.count, targets.count)
    XCTAssertTrue(subscriptions.allSatisfy { $0.args.isEmpty })
    // When
    state.unsubscribe(model)
    // Then
    try await eventually {
      let stopped = http.requests("StopStateSubscription").compactMap {
        try? Releash_Client_V1_StopStateSubscriptionRequest(serializedBytes: $0.message ?? Data())
          .subscriptionID
      }
      return subscriptions.allSatisfy { stopped.contains($0.subscriptionID) }
    }
    await model.stop()
  }

  @MainActor func test_General保存_RPCへ確定値と認証を渡し購読だけを表示の正にする() async throws {
    // Given
    let http = TestHTTPClient()
    let model = try await testModel(http)
    let state = GeneralSettingsState()
    state.subscribe(model)
    try await eventually {
      state.terminalCommand != nil && state.workflow != nil && state.editor != nil
    }
    // When
    await state.updateEditor("/Applications/Code.app", model: model)
    await state.updateApproval(true, model: model)
    state.command = "echo new"
    await state.updateCommand(model: model)
    // Then
    let editor = try Releash_Client_V1_UpdateExternalEditorRequest(
      serializedBytes: XCTUnwrap(http.requests("UpdateExternalEditor").last?.message))
    XCTAssertEqual(editor.editor, "/Applications/Code.app")
    XCTAssertTrue(editor.hasEditor)
    let workflow = try Releash_Client_V1_UpdateWorkflowConfigRequest(
      serializedBytes: XCTUnwrap(http.requests("UpdateWorkflowConfig").last?.message))
    XCTAssertTrue(workflow.workflow.approvalAutoApprove)
    let command = try Releash_Client_V1_UpdateTerminalStartupCommandRequest(
      serializedBytes: XCTUnwrap(http.requests("UpdateTerminalStartupCommand").last?.message))
    XCTAssertEqual(command.command, "echo new")
    for method in ["UpdateExternalEditor", "UpdateWorkflowConfig", "UpdateTerminalStartupCommand"] {
      XCTAssertEqual(
        http.requests(method).last?.headers["authorization"], ["Bearer operator-test"])
    }
    XCTAssertEqual(state.editor?.selected, "")
    XCTAssertEqual(state.workflow?.approvalAutoApprove, false)
    XCTAssertEqual(state.command, "")
    XCTAssertNil(state.failure)
    // When
    var payload = Releash_Client_V1_StatePayload()
    payload.terminalStartupCommand.value = "server command"
    try http.publish(payload, target: "terminal-startup-command")
    payload = .init()
    payload.externalEditor.selected = "/Applications/Code.app"
    try http.publish(payload, target: "external-editor")
    payload = .init()
    payload.workflowConfig.approvalAutoApprove = true
    try http.publish(payload, target: "workflow-config")
    // Then
    try await eventually {
      state.command == "server command" && state.workflow?.approvalAutoApprove == true
        && state.editor?.selected == "/Applications/Code.app"
    }
    // When
    await state.updateEditor("", model: model)
    state.command = ""
    await state.updateCommand(model: model)
    // Then
    let resetEditor = try Releash_Client_V1_UpdateExternalEditorRequest(
      serializedBytes: XCTUnwrap(http.requests("UpdateExternalEditor").last?.message))
    let resetCommand = try Releash_Client_V1_UpdateTerminalStartupCommandRequest(
      serializedBytes: XCTUnwrap(http.requests("UpdateTerminalStartupCommand").last?.message))
    XCTAssertTrue(resetEditor.hasEditor)
    XCTAssertEqual(resetEditor.editor, "")
    XCTAssertTrue(resetCommand.hasCommand)
    XCTAssertEqual(resetCommand.command, "")
    state.unsubscribe(model)
    await model.stop()
  }

  @MainActor func test_Terminal保存_応答後に届いた通知で次の未確定入力を上書きしない() async throws {
    // Given
    let http = TestHTTPClient()
    http.state.withLock { $0.terminalStartupCommand = "saved" }
    let model = try await testModel(http)
    let state = GeneralSettingsState()
    state.subscribe(model)
    try await eventually { state.terminalCommand == "saved" }
    state.command = "command A"
    await state.updateCommand(model: model)
    XCTAssertNil(state.failure)
    XCTAssertTrue(state.saving.isEmpty)
    // When
    state.command = "command B"
    var payload = Releash_Client_V1_StatePayload()
    payload.terminalStartupCommand.value = "command A"
    try http.publish(payload, target: "terminal-startup-command")
    // Then
    try await eventually { state.terminalCommand == "command A" }
    XCTAssertEqual(state.command, "command B")
    XCTAssertEqual(http.requests("UpdateTerminalStartupCommand").count, 1)
    state.unsubscribe(model)
    await model.stop()
  }

  @MainActor func test_General保存_拒否の理由を表示して購読済みの値に戻す() async throws {
    // Given
    let http = TestHTTPClient()
    http.state.withLock { $0.terminalStartupCommand = "saved" }
    let model = try await testModel(http)
    let state = GeneralSettingsState()
    state.subscribe(model)
    try await eventually {
      state.terminalCommand == "saved" && state.editor != nil && state.workflow != nil
    }
    for method in ["UpdateExternalEditor", "UpdateWorkflowConfig", "UpdateTerminalStartupCommand"] {
      http.state.withLock { $0.failedMethod = method }
      // When
      switch method {
      case "UpdateExternalEditor": await state.updateEditor("rejected", model: model)
      case "UpdateWorkflowConfig": await state.updateApproval(true, model: model)
      default:
        state.command = "rejected"
        await state.updateCommand(model: model)
      }
      // Then
      XCTAssertEqual(state.failure, "\(method) failed")
      XCTAssertEqual(state.editor?.selected, "")
      XCTAssertEqual(state.workflow?.approvalAutoApprove, false)
      XCTAssertEqual(state.command, "saved")
      XCTAssertTrue(state.saving.isEmpty)
    }
    // When
    http.state.withLock { $0.failedMethod = nil }
    await state.updateApproval(true, model: model)
    // Then
    XCTAssertNil(state.failure)
    state.unsubscribe(model)
    await model.stop()
  }

  @MainActor func test_Terminal_未受信と未変更は保存せず未接続の確定は理由を表示する() async {
    // Given
    let state = GeneralSettingsState()
    let model = AppModel()
    // When
    state.command = "draft"
    await state.updateCommand(model: model)
    // Then
    XCTAssertNil(state.failure)
    // When
    state.terminalCommand = "saved"
    state.command = "saved"
    await state.updateCommand(model: model)
    // Then
    XCTAssertNil(state.failure)
    // When
    state.command = "draft"
    await state.updateCommand(model: model)
    // Then
    XCTAssertEqual(state.failure, "サーバに接続されていません")
    XCTAssertEqual(state.command, "saved")
  }

  @MainActor func test_General保存_異なる項目を同時に確定しても両方を保存する() async throws {
    // Given
    let http = TestHTTPClient()
    let model = try await testModel(http)
    let state = GeneralSettingsState()
    state.terminalCommand = "saved"
    state.command = "changed"
    // When
    let editor = Task { await state.updateEditor("/Applications/Code.app", model: model) }
    let command = Task { await state.updateCommand(model: model) }
    await editor.value
    await command.value
    // Then
    XCTAssertEqual(http.requests("UpdateExternalEditor").count, 1)
    XCTAssertEqual(http.requests("UpdateTerminalStartupCommand").count, 1)
    XCTAssertTrue(state.saving.isEmpty)
    await model.stop()
  }
}
