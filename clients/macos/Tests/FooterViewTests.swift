import AppKit
import SwiftUI
import XCTest

@testable import Releash

@MainActor private final class FooterHarness {
  let window: NSWindow
  let existingWindows: Set<ObjectIdentifier>

  init(_ model: AppModel) {
    existingWindows = Set(NSApp.windows.map(ObjectIdentifier.init))
    window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 1600, height: 80),
      styleMask: [.titled], backing: .buffered, defer: false)
    window.isReleasedWhenClosed = false
    window.contentView = NSHostingView(rootView: FooterView(model: model))
    window.center()
    NSApp.activate()
    window.makeKeyAndOrderFront(nil)
  }

  func close() {
    for window in NSApp.windows where !existingWindows.contains(ObjectIdentifier(window)) {
      window.close()
    }
  }

  func elements() -> [AnyObject] {
    func descendants(_ object: Any) -> [AnyObject] {
      let element = object as AnyObject
      return [element] + (element.accessibilityChildren?() ?? []).flatMap(descendants)
    }
    return NSApp.windows.filter { !existingWindows.contains(ObjectIdentifier($0)) }
      .flatMap { descendants($0) }
  }

  func provider(_ name: String) -> AnyObject? {
    elements().first {
      let identifier: String? = $0.accessibilityIdentifier?()
      return identifier == "footer-provider:\(name)"
    }
  }

  func texts() -> [String] {
    elements().flatMap { element -> [String] in
      let label: String? = element.accessibilityLabel?()
      let role: NSAccessibility.Role? = element.accessibilityRole?()
      let value: String? = role == .staticText ? element.accessibilityValue?() : nil
      return [label, value].compactMap { $0 }
    }
  }
}

final class FooterViewTests: XCTestCase {
  @MainActor private func usage(plan: String?) -> Releash_Client_V1_ProviderUsageEntry {
    var usage = Releash_Client_V1_ProviderUsageEntry()
    usage.provider = "claude"
    if let plan { usage.plan = plan }
    let now = UInt64(Date().timeIntervalSince1970)
    usage.windows = [(UInt64(300), now + 7245), (UInt64(10080), now + 25245)].map { minutes, reset in
      var window = Releash_Client_V1_ProviderUsageWindow()
      window.windowMinutes = minutes
      window.resetsAt = reset
      window.usedPercent = 25
      return window
    }
    return usage
  }

  @MainActor func testOpeningProviderShowsResetForEachWindowWithAndWithoutPlan() async throws {
    for plan in ["test-plan", nil] as [String?] {
      let model = AppModel()
      model.providerUsage = [usage(plan: plan)]
      let footer = FooterHarness(model)
      defer { footer.close() }
      try await eventually { footer.provider("claude") != nil }
      XCTAssertTrue(try XCTUnwrap(footer.provider("claude")).accessibilityPerformPress?() == true)
      try await eventually { footer.texts().contains("5h: resetまで2時間0分") }
      XCTAssertTrue(footer.texts().contains("wk: resetまで7時間0分"), "\(footer.texts())")
      XCTAssertEqual(footer.texts().contains("test-plan"), plan != nil)
    }
  }

  @MainActor func testHookWarningShowsOnlyItsProviderAndOpensWarningDetails() async throws {
    let model = AppModel()
    var warning = Releash_Client_V1_ProviderHookHealthWarningResponse()
    warning.provider.value = .codex
    warning.reason = "codex hook warning"
    model.hookHealth.warnings.items = [warning]
    let footer = FooterHarness(model)
    defer { footer.close() }
    try await eventually { footer.provider("codex") != nil }
    XCTAssertNil(footer.provider("claude"))
    let provider = try XCTUnwrap(footer.provider("codex"))
    XCTAssertTrue(provider.accessibilityLabel?()?.contains("hook警告") == true)
    XCTAssertTrue(provider.accessibilityPerformPress?() == true)
    try await eventually { footer.texts().contains("codex hook warning") }
  }

  @MainActor func testProvidersWithoutUsageOrHookWarningsAreHidden() async throws {
    let footer = FooterHarness(AppModel())
    defer { footer.close() }
    await Task.yield()
    XCTAssertNil(footer.provider("claude"))
    XCTAssertNil(footer.provider("codex"))
  }

  @MainActor func testHookReadAndSubscriptionFailuresShareOneWarningOutsideProviders() async throws {
    for (read, subscription) in [(true, false), (false, true), (true, true)] {
      let model = AppModel()
      if read { model.hookHealth.readErrors = ["read failure"] }
      if subscription { model.hookSubscriptionFailure = "subscription failure" }
      let footer = FooterHarness(model)
      defer { footer.close() }
      try await eventually {
        footer.elements().contains { $0.accessibilityLabel?() == "hookの状態を確認できません" }
      }
      XCTAssertEqual(footer.elements().filter { $0.accessibilityLabel?() == "hookの状態を確認できません" }.count, 1)
      XCTAssertNil(footer.provider("claude"))
      XCTAssertNil(footer.provider("codex"))
    }
  }

  @MainActor func testCLIReleaseJudgmentDisplaysMatchingGuidance() async throws {
    for (compatibility, guidance) in [
      ("server_older", "再起動で更新できます"),
      ("client_older", "クライアントが古いです"),
    ] {
      let model = AppModel()
      model.daemonInfo = .init()
      model.releaseCompatibility = compatibility
      let footer = FooterHarness(model)
      defer { footer.close() }
      try await eventually { footer.texts().contains(guidance) }

      XCTAssertFalse(footer.texts().contains(compatibility == "server_older" ? "クライアントが古いです" : "再起動で更新できます"))
    }
  }
}
