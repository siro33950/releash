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

  func test_ホバー_フォーカスを失った三回の履歴をエラーに残す() async throws {
    // Given
    let host = await ViewHarness(Text("Hover"))
    defer { host.close() }
    let other = await ViewHarness(Text("Other"))
    defer { other.close() }
    var observations = 0
    let frame = host.window.frame
    let height = try XCTUnwrap(NSScreen.screens.first).frame.height
    // When
    do {
      let _: Bool = try await host.hover(at: CGPoint(x: frame.midX, y: height - frame.midY)) {
        observations += 1
        other.window.makeKeyAndOrderFront(nil)
        return true
      }
      XCTFail("Focus loss must discard the observation")
    } catch {
      // Then
      XCTAssertEqual(observations, 3)
      for attempt in 1...3 {
        XCTAssertTrue(
          error.localizedDescription.contains("attempt \(attempt)/3: focus lost"),
          error.localizedDescription)
      }
    }
  }

  func test_ホバー_フォーカスを保ったまま表示されなければ再試行しない() async throws {
    // Given
    let host = await ViewHarness(Text("Hover"))
    defer { host.close() }
    let frame = host.window.frame
    let height = try XCTUnwrap(NSScreen.screens.first).frame.height
    // When
    do {
      let _: Bool = try await host.hover(at: CGPoint(x: frame.midX, y: height - frame.midY)) {
        nil
      }
      XCTFail("Missing observation must time out")
    } catch {
      // Then
      XCTAssertTrue(
        error.localizedDescription.contains("attempt 1/3: observation timed out"),
        error.localizedDescription)
      XCTAssertFalse(error.localizedDescription.contains("attempt 2/3"), error.localizedDescription)
    }
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
