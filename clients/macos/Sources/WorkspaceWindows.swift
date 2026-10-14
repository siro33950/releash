import SwiftUI

@MainActor
final class WorkspaceWindows: NSObject, NSApplicationDelegate, NSWindowDelegate {
  let model = AppModel()
  private var windows: [NSWindow] = []

  func applicationDidFinishLaunching(_ notification: Notification) {
    guard ProcessInfo.processInfo.environment["XCTestConfigurationFilePath"] == nil else { return }
    openWindow()
    Task { await model.start() }
  }

  func openWindow() {
    let window = WorkspaceWindow(
      contentRect: NSRect(x: 0, y: 0, width: UIStyle.windowWidth, height: UIStyle.windowHeight),
      styleMask: [.borderless, .resizable, .miniaturizable, .closable],
      backing: .buffered, defer: false)
    window.title = "Releash"
    window.isReleasedWhenClosed = false
    window.isOpaque = false
    window.backgroundColor = .clear
    window.isMovableByWindowBackground = false
    window.minSize = NSSize(
      width: UIStyle.windowMinimumWidth,
      height: UIStyle.windowMinimumHeight + UIStyle.windowPadding * 2)
    let content = NSHostingView(rootView: RootView(model: model))
    content.safeAreaRegions = []
    window.contentView = content
    window.delegate = self
    if windows.isEmpty {
      if !window.setFrameUsingName("ReleashWorkspace") { window.center() }
      window.setFrameAutosaveName("ReleashWorkspace")
    } else if let previous = windows.last {
      window.cascadeTopLeft(from: NSPoint(x: previous.frame.minX, y: previous.frame.maxY))
    }
    windows.append(window)
    window.makeKeyAndOrderFront(nil)
    NSApp.activate()
  }

  func applicationShouldHandleReopen(
    _ sender: NSApplication, hasVisibleWindows flag: Bool
  ) -> Bool {
    if !flag { openWindow() }
    return true
  }

  func windowWillClose(_ notification: Notification) {
    guard let window = notification.object as? NSWindow else { return }
    windows.removeAll { $0 === window }
    NSApp.removeWindowsItem(window)
  }
}

final class WorkspaceWindow: NSWindow {
  override var canBecomeKey: Bool { true }
  override var canBecomeMain: Bool { true }
}
