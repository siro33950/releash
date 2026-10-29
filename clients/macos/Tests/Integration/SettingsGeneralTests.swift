import AppKit
import SwiftUI
import XCTest

@testable import Releash

final class SettingsGeneralTests: XCTestCase {
  @MainActor func test_Settings_四つのタブとGeneralの購読値を表示する() async throws {
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
    let host = await ViewHarness(SettingsView(model: model))
    defer { host.close() }
    // Then
    try await eventuallyView(host) {
      host.element("settings-terminal-command")?.value == "echo saved"
        && host.element("settings-external-editor")?.value == "Code"
        && host.element("settings-approval-automation")?.value == "1"
    }
    for title in ["General", "Repositories", "Providers", "System"] {
      XCTAssertNotNil(host.element(title), title)
    }
    XCTAssertEqual(host.element("settings-external-editor")?.value, "Code")
    XCTAssertEqual(host.element("settings-approval-automation")?.value, "1")
    XCTAssertNotNil(host.element("settings-diff-base"))
    // When
    var payload = Releash_Client_V1_StatePayload()
    payload.terminalStartupCommand.value = "changed elsewhere"
    try http.publish(payload, target: "terminal-startup-command")
    // Then
    try await eventuallyView(host) {
      host.element("settings-terminal-command")?.value == "changed elsewhere"
    }
    // When
    payload = .init()
    payload.externalEditor.selected = "/Applications/Unavailable.app"
    try http.publish(payload, target: "external-editor")
    // Then
    try await eventuallyView(host) {
      host.element("settings-external-editor")?.value == "/Applications/Unavailable.app"
    }
    await model.stop()
  }

  @MainActor func test_General操作_承認の保存拒否を表示して保存済みの値に戻す() async throws {
    // Given
    let http = TestHTTPClient()
    let model = try await testModel(http)
    let host = await ViewHarness(GeneralSettingsView(model: model))
    defer { host.close() }
    try await eventuallyView(host) {
      host.element("settings-approval-automation")?.enabled == true
    }
    http.state.withLock { $0.failedMethod = "UpdateWorkflowConfig" }
    // When
    try await host.press("settings-approval-automation")
    // Then
    try await eventuallyView(host) {
      host.texts().contains("UpdateWorkflowConfig failed")
        && host.element("settings-approval-automation")?.value == "0"
    }
    XCTAssertEqual(http.requests("UpdateWorkflowConfig").count, 1)
    // When
    http.state.withLock { $0.failedMethod = nil }
    try await host.press("settings-approval-automation")
    try await eventually { http.requests("UpdateWorkflowConfig").count == 2 }
    var payload = Releash_Client_V1_StatePayload()
    payload.workflowConfig.approvalAutoApprove = true
    try http.publish(payload, target: "workflow-config")
    // Then
    try await eventuallyView(host) {
      host.element("settings-approval-automation")?.value == "1"
        && host.element("settings-save-failure") == nil
    }
    await model.stop()
  }

  @MainActor func test_Terminal入力_確定時に保存し拒否された下書きを復元する() async throws {
    // Given
    let http = TestHTTPClient()
    http.state.withLock { $0.terminalStartupCommand = "saved" }
    let model = try await testModel(http)
    let host = await ViewHarness(GeneralSettingsView(model: model))
    defer { host.close() }
    try await eventuallyView(host) {
      host.element("settings-terminal-command")?.value == "saved"
    }
    func fields(_ view: NSView) -> [NSTextField] {
      (view as? NSTextField).map { [$0] } ?? view.subviews.flatMap(fields)
    }
    let field = try XCTUnwrap(
      fields(try XCTUnwrap(host.window.contentView)).first { $0.isEditable })
    host.window.makeFirstResponder(field)
    http.state.withLock { $0.failedMethod = "UpdateTerminalStartupCommand" }
    // When
    try await host.setTextField("rejected")
    // Then
    XCTAssertTrue(http.requests("UpdateTerminalStartupCommand").isEmpty)
    // When
    let enter = try XCTUnwrap(
      NSEvent.keyEvent(
        with: .keyDown, location: .zero, modifierFlags: [], timestamp: 0,
        windowNumber: host.window.windowNumber, context: nil, characters: "\r",
        charactersIgnoringModifiers: "\r", isARepeat: false, keyCode: 36))
    host.window.sendEvent(enter)
    // Then
    try await eventuallyView(host) {
      host.texts().contains("UpdateTerminalStartupCommand failed")
        && host.element("settings-terminal-command")?.value == "saved"
    }
    XCTAssertEqual(http.requests("UpdateTerminalStartupCommand").count, 1)
    await model.stop()
  }

  @MainActor func test_DiffBase_UserDefaultsの選択が新しいファイルタブに使われる() throws {
    // Given
    let defaults = UserDefaults.standard
    let saved = defaults.object(forKey: "diffBase")
    defer { defaults.set(saved, forKey: "diffBase") }
    defaults.removeObject(forKey: "diffBase")
    let model = AppModel()
    try model.panes.restore(.empty())
    let view = GeneralSettingsView(model: model)
    XCTAssertEqual(view.diffBase, "head")
    // When
    view.diffBase = "branch-base"
    model.openFile("new.rs")
    // Then
    XCTAssertEqual(defaults.string(forKey: "diffBase"), "branch-base")
    XCTAssertEqual(
      model.panes.tab(try XCTUnwrap(model.panes.controller.allTabIds.last))?.fileBase,
      "branch-base")
    XCTAssertEqual(GeneralSettingsView(model: AppModel()).diffBase, "branch-base")
  }
}
