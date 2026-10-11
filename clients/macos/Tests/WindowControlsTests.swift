import AppKit
import XCTest

@testable import Releash

final class WindowControlsTests: XCTestCase {
  @MainActor func testControlsTargetTheirFocusableWindowWithoutATitleBar() throws {
    let window = WorkspaceWindow(
      contentRect: NSRect(x: 0, y: 0, width: 800, height: 600),
      styleMask: [.borderless, .resizable, .closable, .miniaturizable], backing: .buffered, defer: false)
    window.isReleasedWhenClosed = false
    defer { window.close() }
    let controls = WindowControls.ControlsView(frame: NSRect(x: 0, y: 0, width: 68, height: 33))
    window.contentView = controls
    controls.layoutSubtreeIfNeeded()
    let buttons = controls.subviews.compactMap { $0 as? NSButton }
    XCTAssertEqual(buttons.count, 3)
    XCTAssertTrue(window.canBecomeKey)
    XCTAssertTrue(window.canBecomeMain)
    XCTAssertFalse(window.styleMask.contains(.titled))
    XCTAssertTrue(window.styleMask.contains([.resizable, .closable, .miniaturizable]))
    XCTAssertFalse(controls.mouseDownCanMoveWindow)
    for button in buttons {
      XCTAssertTrue(button.target === window)
      XCTAssertTrue(controls.bounds.contains(button.frame))
    }
    XCTAssertEqual(
      buttons.map(\.action),
      [
        #selector(NSWindow.close), #selector(NSWindow.miniaturize(_:)),
        #selector(NSWindow.toggleFullScreen(_:)),
      ])
    window.orderFront(nil)
    XCTAssertTrue(window.isVisible)
    buttons[0].performClick(nil)
    XCTAssertFalse(window.isVisible)
  }
}
