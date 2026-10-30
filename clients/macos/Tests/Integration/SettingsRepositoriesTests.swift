import AppKit
import ApplicationServices
import SwiftUI
import XCTest

@testable import Releash

final class SettingsRepositoriesTests: XCTestCase {
  @MainActor func test_Repository選択_購読の一覧から選んだRepositoryの設定を表示する() async throws {
    // Given
    let http = TestHTTPClient()
    http.state.withLock {
      $0.repositories = [.with { $0.path = "/first" }, .with { $0.path = "/second" }]
      $0.releashBases["/first"] = .with { $0.value = "main" }
      $0.releashBases["/second"] = .with { $0.value = "develop" }
    }
    let model = try await testModel(http)
    let host = await ViewHarness(RepositoriesSettingsView(model: model))
    defer { host.close() }
    // Then
    try await eventuallyView(host) { host.element("settings-base-branch")?.value == "main" }
    XCTAssertNotNil(host.element("/first"))
    XCTAssertNotNil(host.element("/second"))
    // When
    try await host.selectRow("/second")
    // Then
    try await eventuallyView(host) { host.element("settings-base-branch")?.value == "develop" }
    // When
    var payload = Releash_Client_V1_StatePayload()
    payload.workspaces.repositories.items = [.with { $0.path = "/first" }]
    try http.publish(payload, target: "workspaces")
    // Then
    try await eventuallyView(host) {
      host.element("/second") == nil && host.element("settings-base-branch")?.value == "main"
    }
    // When
    payload.workspaces.repositories.items = []
    try http.publish(payload, target: "workspaces")
    // Then
    try await eventuallyView(host) { host.element("Repositoryを選択してください") != nil }
    await model.stop()
  }

  @MainActor func test_Repository削除_確認のキャンセルは保存せず承認後にRPCを呼ぶ() async throws {
    // Given
    let http = TestHTTPClient()
    http.state.withLock { $0.repositories = [.with { $0.path = "/repo" }] }
    let model = try await testModel(http)
    let host = await ViewHarness(RepositoriesSettingsView(model: model))
    defer { host.close() }
    try await eventuallyView(host) { host.element("settings-remove-repository") != nil }
    // When
    try await host.press("settings-remove-repository")
    // Then
    XCTAssertTrue(http.requests("RemoveRepoPath").isEmpty)
    // When
    try await eventually { !host.window.sheets.isEmpty }
    try await host.pressAlertButton("キャンセル")
    // Then
    XCTAssertTrue(http.requests("RemoveRepoPath").isEmpty)
    // When
    try await eventually { host.window.sheets.isEmpty }
    try await eventuallyView(host) {
      host.element("settings-remove-repository")?.actions.contains(kAXPressAction) == true
    }
    try await host.press("settings-remove-repository")
    try await eventually { !host.window.sheets.isEmpty }
    try await host.pressAlertButton("削除")
    // Then
    try await eventually { http.requests("RemoveRepoPath").count == 1 }
    XCTAssertNotNil(host.element("/repo"))
    // When
    var payload = Releash_Client_V1_StatePayload()
    payload.workspaces.repositories.items = []
    try http.publish(payload, target: "workspaces")
    // Then
    try await eventuallyView(host) { host.element("/repo") == nil }
    await model.stop()
  }

  @MainActor func test_BaseBranch操作_一覧から選択して自動に戻し拒否の理由を表示する() async throws {
    // Given
    let http = TestHTTPClient()
    http.state.withLock {
      $0.releashBases["/repo"] = .with { $0.value = "main" }
      $0.branches["/repo"] = .with {
        $0.items = [.with { $0.name = "main" }, .with { $0.name = "develop" }]
      }
    }
    let model = try await testModel(http)
    let host = await ViewHarness(RepositorySettingsView(model: model, path: "/repo"))
    defer { host.close() }
    try await eventuallyView(host) { host.element("settings-base-branch")?.enabled == true }
    // When
    http.state.withLock { $0.failedMethod = "SetReleashBase" }
    let titles = try await choosePicker(host, "settings-base-branch", "develop")
    XCTAssertTrue(titles.contains("main"))
    XCTAssertTrue(titles.contains("自動（mainまたはmaster）"))
    // Then
    try await eventuallyView(host) {
      host.texts().contains("SetReleashBase failed")
        && host.element("settings-base-branch")?.value == "main"
    }
    // When
    http.state.withLock { $0.failedMethod = nil }
    _ = try await choosePicker(host, "settings-base-branch", "自動（mainまたはmaster）")
    try await eventually { http.requests("SetReleashBase").count == 2 }
    var payload = Releash_Client_V1_StatePayload()
    payload.releashBase = .init()
    try http.publish(payload, target: "releash-base", args: ["/repo"])
    // Then
    try await eventuallyView(host) {
      host.element("settings-base-branch")?.value == "自動（mainまたはmaster）"
        && host.element("settings-repository-failure") == nil
    }
    await model.stop()
  }

  @MainActor func test_Notion操作_接続成功後だけpropertyを選べて保存失敗時に購読値へ戻る() async throws {
    // Given
    let http = TestHTTPClient()
    http.state.withLock {
      $0.notionConfigs["/repo"] = .with {
        $0.value.apiToken = "saved-token"
        $0.value.databaseID = "saved-database"
        $0.value.propertyMapping.title = "Name"
        $0.value.propertyMapping.branchPrefix = "feat/"
      }
      $0.notionValidation.status.value = .configured
      $0.notionValidation.properties.items = [
        .with {
          $0.name = "Name"
          $0.propertyType = "title"
        },
        .with {
          $0.name = "Status"
          $0.propertyType = "status"
        },
      ]
    }
    let model = try await testModel(http)
    let host = await ViewHarness(RepositorySettingsView(model: model, path: "/repo"))
    defer { host.close() }
    try await eventuallyView(host) {
      host.element("settings-notion-database")?.value == "saved-database"
    }
    XCTAssertNil(host.element("settings-notion-save"))
    // When
    try await host.press("settings-notion-validate")
    // Then
    try await eventuallyView(host) {
      host.texts().contains("接続に成功しました") && host.element("settings-notion-save") != nil
    }
    // When
    let titles = try await choosePicker(host, "settings-notion-title", "Status (status)")
    XCTAssertTrue(titles.contains("Name (title)"))
    try await eventuallyView(host) {
      host.element("settings-notion-title")?.value == "Status (status)"
    }
    try await eventuallyView(host) { host.element("settings-notion-label-Status") != nil }
    try await host.press("settings-notion-label-Status")
    _ = try await choosePicker(host, "settings-notion-branch", "Name (title)")
    try await eventuallyView(host) {
      host.element("settings-notion-branch")?.value == "Name (title)"
    }
    http.state.withLock { $0.failedMethod = "SaveNotionConfig" }
    try await host.press("settings-notion-save")
    // Then
    try await eventuallyView(host) {
      host.texts().contains("SaveNotionConfig failed")
        && host.element("settings-notion-database")?.value == "saved-database"
        && host.element("settings-notion-save") == nil
    }
    let saved = try Releash_Client_V1_SaveNotionConfigRequest(
      serializedBytes: XCTUnwrap(http.requests("SaveNotionConfig").last?.message))
    XCTAssertEqual(saved.propertyMapping.title, "Status")
    XCTAssertEqual(saved.propertyMapping.labels.items.first?.name, "Status")
    XCTAssertEqual(saved.propertyMapping.branchName, "Name")
    XCTAssertEqual(saved.propertyMapping.branchPrefix, "feat/")
    // When
    http.state.withLock { $0.failedMethod = nil }
    try await host.press("settings-notion-delete")
    try await eventually { http.requests("DeleteNotionConfig").count == 1 }
    var payload = Releash_Client_V1_StatePayload()
    payload.notionConfig = .init()
    try http.publish(payload, target: "notion-config", args: ["/repo"])
    // Then
    try await eventuallyView(host) {
      host.texts().contains("未設定です")
        && host.element("settings-notion-database")?.value == ""
        && host.element("settings-notion-delete") == nil
    }
    await model.stop()
  }

  @MainActor private func choosePicker(_ host: ViewHarness, _ identifier: String, _ choice: String)
    async throws -> [String]
  {
    func attribute(_ element: AXUIElement, _ name: String) -> CFTypeRef? {
      var value: CFTypeRef?
      AXUIElementCopyAttributeValue(element, name as CFString, &value)
      return value
    }
    func menuItems(_ element: AXUIElement) -> [AXUIElement] {
      let children = attribute(element, kAXChildrenAttribute) as? [AXUIElement] ?? []
      return (attribute(element, kAXRoleAttribute) as? String == kAXMenuItemRole ? [element] : [])
        + children.flatMap(menuItems)
    }
    var titles: [String] = []
    var finished = false
    var selected = false
    let deadline = ContinuousClock.now + .seconds(5)
    let timer = Timer(timeInterval: 0.05, repeats: true) { timer in
      let done = MainActor.assumeIsolated {
        guard !finished else { return true }
        let items = menuItems(
          AXUIElementCreateApplication(ProcessInfo.processInfo.processIdentifier))
        titles = items.compactMap { attribute($0, kAXTitleAttribute) as? String }
        if let item = items.first(where: { attribute($0, kAXTitleAttribute) as? String == choice })
        {
          finished = true
          selected = AXUIElementPerformAction(item, kAXPressAction as CFString) == .success
        } else if ContinuousClock.now >= deadline {
          finished = true
          if let escape = NSEvent.keyEvent(
            with: .keyDown, location: .zero, modifierFlags: [], timestamp: 0,
            windowNumber: host.window.windowNumber, context: nil, characters: "\u{1b}",
            charactersIgnoringModifiers: "\u{1b}", isARepeat: false, keyCode: 53)
          {
            NSApp.postEvent(escape, atStart: true)
          }
        }
        return finished
      }
      if done { timer.invalidate() }
    }
    RunLoop.main.add(timer, forMode: .common)
    RunLoop.main.add(timer, forMode: .eventTracking)
    defer { timer.invalidate() }
    try await host.press(identifier)
    try await eventually { finished }
    XCTAssertTrue(selected, titles.joined(separator: ", "))
    return titles
  }
}
