import AppKit
import SwiftUI
import XCTest

@MainActor final class ViewHarnessTests: XCTestCase {
  private final class MenuButton: NSButton, NSMenuDelegate {
    var onSelection: (Bool) -> Void = { _ in }
    private var menuClosed = false

    override func accessibilityPerformShowMenu() -> Bool {
      DispatchQueue.main.asyncAfter(deadline: .now() + 0.1) { [self] in
        guard window != nil else { return }
        let menu = NSMenu()
        menu.delegate = self
        let item = NSMenuItem(title: "実行", action: #selector(selectItem), keyEquivalent: "")
        item.target = self
        menu.addItem(item)
        menu.popUp(positioning: nil, at: .zero, in: self)
      }
      return true
    }

    func menuDidClose(_ menu: NSMenu) { menuClosed = true }

    @objc private func selectItem() { onSelection(menuClosed) }
  }

  private struct DelayedMenu: NSViewRepresentable {
    var onSelection: (Bool) -> Void

    func makeNSView(context: Context) -> MenuButton {
      let button = MenuButton(title: "非同期メニュー", target: nil, action: nil)
      button.onSelection = onSelection
      return button
    }

    func updateNSView(_ nsView: MenuButton, context: Context) {}
  }

  func test_メニュー操作_表示要求が先に完了しても項目の表示を待って選択する() async throws {
    // Given
    var selectedAfterClose: Bool?
    let host = await ViewHarness(DelayedMenu { selectedAfterClose = $0 })
    defer { host.close() }
    // When
    try await host.menu("非同期メニュー", selecting: "実行") {
      // Then
      XCTAssertEqual($0, ["実行"])
    }
    XCTAssertEqual(selectedAfterClose, true)
  }
}
