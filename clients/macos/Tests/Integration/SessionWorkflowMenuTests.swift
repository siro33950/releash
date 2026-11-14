import AppKit
import SwiftProtobuf
import XCTest

@testable import Releash

final class SessionWorkflowMenuTests: XCTestCase {
  @MainActor func test_Workflowメニュー_Root配信待ちでもサブメニューを保持して無効にする() async throws {
    // Given
    let model = AppModel()
    model.workflows = [.with { $0.name = "first" }, .with { $0.name = "second" }]
    let host = await ViewHarness(SessionWorkflowMenu(model: model, path: "/repo"))
    defer { host.close() }
    for pending in [false, true, false] {
      model.pendingRoots["/repo"] = pending ? "root" : nil
      let observer = MenuObserver()
      NotificationCenter.default.addObserver(
        observer, selector: #selector(MenuObserver.opened(_:)),
        name: NSMenu.didBeginTrackingNotification, object: nil)
      defer { NotificationCenter.default.removeObserver(observer) }
      var inspected = false
      let timer = Timer(timeInterval: 0.05, repeats: true) { timer in
        let handled = MainActor.assumeIsolated {
          guard let menu = observer.menu else { return false }
          // Then
          let item = menu.items.first { $0.title == "Workflow" }
          XCTAssertNotNil(item?.submenu)
          XCTAssertEqual(item?.submenu?.items.map(\.title), ["first", "second"])
          XCTAssertEqual(item?.isEnabled, !pending)
          menu.cancelTracking()
          inspected = true
          return true
        }
        if handled { timer.invalidate() }
      }
      RunLoop.main.add(timer, forMode: .common)
      RunLoop.main.add(timer, forMode: .eventTracking)
      defer { timer.invalidate() }
      // When
      try await host.press("開始")
      try await eventually { inspected }
    }
  }
}

@MainActor private final class MenuObserver: NSObject {
  var menu: NSMenu?
  @objc func opened(_ notification: Notification) { menu = notification.object as? NSMenu }
}
