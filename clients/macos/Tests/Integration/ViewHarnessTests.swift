import AppKit
import SwiftUI
import XCTest

@MainActor final class ViewHarnessTests: XCTestCase {
  @Observable fileprivate final class ButtonState {
    var visible = false
    var presses = 0
  }

  private struct DelayedButton: View {
    var state: ButtonState

    var body: some View {
      VStack {
        if state.visible {
          Button("遅延ボタン") { state.presses += 1 }
        }
      }
    }
  }

  func test_ボタン操作_要素の表示を待って一度だけ押す() async throws {
    // Given
    let state = ButtonState()
    let host = await ViewHarness(DelayedButton(state: state))
    defer { host.close() }
    XCTAssertNil(host.element("遅延ボタン"))
    var started = false
    // When
    let press = Task {
      started = true
      try await host.press("遅延ボタン")
    }
    try await eventually { started }
    state.visible = true
    try await press.value
    // Then
    XCTAssertEqual(state.presses, 1)
  }

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
