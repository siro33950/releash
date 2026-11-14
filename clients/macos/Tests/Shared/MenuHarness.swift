import AppKit
import XCTest

@MainActor final class MenuHarness: NSObject {
  private var trackingMenu: NSMenu?
  private var chosen: (NSMenu, Int)?
  private var inspected = false
  private var tracking = false
  private var failure: Error?

  func open(
    selection: [String] = [], press: @MainActor () async throws -> Void,
    inspect: @escaping @MainActor (NSMenu) throws -> Void = { _ in }
  ) async throws {
    trackingMenu = nil
    NotificationCenter.default.addObserver(
      self, selector: #selector(menuBegan(_:)),
      name: NSMenu.didBeginTrackingNotification, object: nil)
    NotificationCenter.default.addObserver(
      self, selector: #selector(menuEnded(_:)),
      name: NSMenu.didEndTrackingNotification, object: nil)
    chosen = nil
    inspected = false
    tracking = false
    failure = nil
    let timer = Timer(timeInterval: 0.05, repeats: true) { [self] timer in
      let handled = MainActor.assumeIsolated {
        guard let menu = trackingMenu else { return false }
        defer {
          menu.cancelTracking()
          inspected = true
        }
        do {
          try inspect(menu)
          var current = menu
          for (offset, title) in selection.enumerated() {
            let index = try XCTUnwrap(current.items.firstIndex { $0.title == title })
            if offset == selection.count - 1 {
              XCTAssertTrue(current.items[index].isEnabled)
              chosen = (current, index)
            } else {
              current = try XCTUnwrap(current.items[index].submenu)
              current.update()
            }
          }
        } catch { failure = error }
        return true
      }
      if handled { timer.invalidate() }
    }
    RunLoop.main.add(timer, forMode: .common)
    RunLoop.main.add(timer, forMode: .eventTracking)
    defer {
      timer.invalidate()
      NotificationCenter.default.removeObserver(
        self, name: NSMenu.didBeginTrackingNotification, object: nil)
      NotificationCenter.default.removeObserver(
        self, name: NSMenu.didEndTrackingNotification, object: nil)
      trackingMenu = nil
    }
    try await press()
    try await eventually { inspected && !tracking }
    if let failure { throw failure }
    if let (menu, index) = chosen { menu.performActionForItem(at: index) }
  }

  @MainActor @objc private func menuBegan(_ notification: Notification) {
    if trackingMenu == nil {
      trackingMenu = notification.object as? NSMenu
      tracking = true
    }
  }
  @objc private func menuEnded(_ notification: Notification) {
    if notification.object as? NSMenu === trackingMenu { tracking = false }
  }

}
