import AppKit
import ApplicationServices
import SwiftUI
import XCTest

struct ViewElement {
  fileprivate let reference: AXUIElement
  let identifier: String
  let role: String
  let label: String
  let value: String
  let enabled: Bool
  let actions: [String]
}

@MainActor final class ViewHarness: NSObject {
  let window: NSWindow
  let existingWindows: Set<ObjectIdentifier>
  private var snapshot: [ViewElement] = []
  private var menuHandled = false
  private var trackingMenu: NSMenu?
  init<V: View>(_ view: V, size: NSSize = NSSize(width: 1000, height: 700)) async {
    existingWindows = Set(NSApp.windows.map(ObjectIdentifier.init))
    window = NSWindow(
      contentRect: NSRect(origin: .zero, size: size), styleMask: [.titled], backing: .buffered,
      defer: false)
    super.init()
    window.isReleasedWhenClosed = false
    window.title = "ViewHarness-" + UUID().uuidString
    let host = NSHostingView(rootView: view)
    host.sizingOptions = []
    window.contentView = host
    host.layoutSubtreeIfNeeded()
    window.displayIfNeeded()
    NSApp.activate()
    window.makeKeyAndOrderFront(nil)
    await refresh()
  }
  func close() {
    for window in NSApp.windows where !existingWindows.contains(ObjectIdentifier(window)) {
      window.close()
    }
  }
  func refresh() async { readSnapshot() }
  private func readSnapshot() {
    snapshot = {
      func attribute(_ element: AXUIElement, _ name: String) -> CFTypeRef? {
        var value: CFTypeRef?
        guard AXUIElementCopyAttributeValue(element, name as CFString, &value) == .success else {
          return nil
        }
        return value
      }
      var visited: [AXUIElement] = []
      func collect(_ element: AXUIElement) -> [ViewElement] {
        guard !visited.contains(where: { CFEqual($0, element) }) else { return [] }
        visited.append(element)
        var actionNames: CFArray?
        AXUIElementCopyActionNames(element, &actionNames)
        let rawValue = attribute(element, kAXValueAttribute)
        let entry = ViewElement(
          reference: element,
          identifier: attribute(element, kAXIdentifierAttribute) as? String ?? "",
          role: attribute(element, kAXRoleAttribute) as? String ?? "",
          label: attribute(element, kAXTitleAttribute) as? String ?? attribute(
            element, kAXDescriptionAttribute) as? String ?? "",
          value: rawValue as? String ?? (rawValue as? NSNumber)?.stringValue ?? "",
          enabled: attribute(element, kAXEnabledAttribute) as? Bool ?? false,
          actions: actionNames as? [String] ?? [])
        return [entry]
          + (attribute(element, kAXChildrenAttribute) as? [AXUIElement] ?? []).flatMap(collect)
      }
      let app = AXUIElementCreateApplication(ProcessInfo.processInfo.processIdentifier)
      let windows = (attribute(app, kAXWindowsAttribute) as? [AXUIElement] ?? []).filter {
        attribute($0, kAXTitleAttribute) as? String == window.title
      }
      let menus = (attribute(app, kAXChildrenAttribute) as? [AXUIElement] ?? []).filter {
        attribute($0, kAXRoleAttribute) as? String == kAXMenuRole
      }
      return (windows + menus).flatMap(collect)
    }()
  }
  func element(_ name: String) -> ViewElement? {
    snapshot.first { $0.identifier == name || $0.label == name || $0.value == name }
  }
  func value(_ element: ViewElement?) -> String? { element?.value }
  func texts() -> [String] { snapshot.flatMap { [$0.label, $0.value] } }
  func debugDump() -> String {
    snapshot.map { "\($0.role)#\($0.identifier)[\($0.label)|\($0.value)]" }.joined(separator: " ")
  }
  func press(_ name: String, file: StaticString = #filePath, line: UInt = #line) async throws {
    await refresh()
    let element = try XCTUnwrap(
      snapshot.first {
        ($0.identifier == name || $0.label == name || $0.value == name)
          && $0.actions.contains(kAXPressAction)
      }, file: file, line: line)
    let result = AXUIElementPerformAction(element.reference, kAXPressAction as CFString).rawValue
    XCTAssertEqual(result, AXError.success.rawValue, file: file, line: line)
    await refresh()
  }
  func tableRow(_ name: String) throws -> AXUIElement {
    var reference = try XCTUnwrap(element(name)).reference
    while true {
      var role: CFTypeRef?
      AXUIElementCopyAttributeValue(reference, kAXRoleAttribute as CFString, &role)
      if role as? String == kAXRowRole { return reference }
      var parent: CFTypeRef?
      XCTAssertEqual(
        AXUIElementCopyAttributeValue(reference, kAXParentAttribute as CFString, &parent), .success)
      reference = try XCTUnwrap(parent) as! AXUIElement
    }
  }
  func selectRow(_ name: String) async throws {
    await refresh()
    XCTAssertEqual(
      AXUIElementSetAttributeValue(
        try tableRow(name), kAXSelectedAttribute as CFString, kCFBooleanTrue), .success)
    await refresh()
  }
  func setRowExpanded(_ name: String, _ expanded: Bool) async throws {
    await refresh()
    XCTAssertEqual(
      AXUIElementSetAttributeValue(
        try tableRow(name), kAXDisclosingAttribute as CFString,
        expanded ? kCFBooleanTrue : kCFBooleanFalse), .success)
    await refresh()
  }
  @objc private func menuBegan(_ notification: Notification) {
    trackingMenu = notification.object as? NSMenu
  }
  func menu(
    _ name: String, selecting choice: String?, file: StaticString = #filePath, line: UInt = #line,
    inspect: @escaping @MainActor ([String]) -> Void
  ) async throws {
    try await eventuallyView(self, { element(name) != nil }, file: file, line: line)
    let row = try XCTUnwrap(element(name), file: file, line: line)
    menuHandled = false
    trackingMenu = nil
    NotificationCenter.default.addObserver(
      self, selector: #selector(menuBegan(_:)), name: NSMenu.didBeginTrackingNotification,
      object: nil)
    defer {
      NotificationCenter.default.removeObserver(
        self, name: NSMenu.didBeginTrackingNotification, object: nil)
      trackingMenu = nil
    }
    let deadline = ContinuousClock.now + .seconds(5)
    await withCheckedContinuation { continuation in
      let timer = Timer(timeInterval: 0.05, repeats: true) { [self] timer in
        let handled = MainActor.assumeIsolated {
          guard let menu = trackingMenu else { return false }
          let titles = menu.items.map(\.title)
          guard let choice, let index = titles.firstIndex(of: choice) else {
            if ContinuousClock.now < deadline { return false }
            inspect(titles)
            XCTAssertNil(choice)
            menuHandled = true
            menu.cancelTracking()
            return true
          }
          inspect(titles)
          menuHandled = true
          menu.performActionForItem(at: index)
          menu.cancelTracking()
          return true
        }
        if handled { timer.invalidate() }
      }
      RunLoop.main.add(timer, forMode: .eventTracking)
      DispatchQueue.main.async { [self] in
        let result = AXUIElementPerformAction(row.reference, kAXShowMenuAction as CFString)
        timer.invalidate()
        if !menuHandled {
          inspect([])
          XCTAssertNil(choice)
        }
        XCTAssertEqual(result, .success)
        continuation.resume()
      }
    }
    await refresh()
  }
  func pressAlertButton(_ name: String) async throws {
    func buttons(_ view: NSView) -> [NSButton] {
      (view as? NSButton).map { [$0] } ?? view.subviews.flatMap(buttons)
    }
    let views = ([window] + window.sheets).compactMap(\.contentView)
    let button = try XCTUnwrap(views.flatMap(buttons).first { $0.title == name })
    let sheet = window.attachedSheet
    let dismissed = XCTNSNotificationExpectation(
      name: NSWindow.didEndSheetNotification, object: window)
    button.performClick(nil)
    if let sheet {
      let result = await XCTWaiter.fulfillment(of: [dismissed], timeout: 5)
      XCTAssertEqual(result, .completed)
      try await eventually { !sheet.isVisible && self.window.attachedSheet !== sheet }
      window.makeKeyAndOrderFront(nil)
    }
    await refresh()
  }
  func focusTextField(
    index: Int = 0, file: StaticString = #filePath, line: UInt = #line
  ) async throws {
    func field() -> ViewElement? {
      snapshot.filter { $0.role == kAXTextFieldRole && $0.enabled }.dropFirst(index).first
    }
    let deadline = ContinuousClock.now + .seconds(5)
    var result = AXError.success
    var element: ViewElement?
    repeat {
      await refresh()
      element = field()
      if let element {
        NSApp.activate()
        window.makeKeyAndOrderFront(nil)
        result = AXUIElementSetAttributeValue(
          element.reference, kAXFocusedAttribute as CFString, kCFBooleanTrue)
      }
      try await Task.sleep(for: .milliseconds(20))
      if let element, isEditing(element.reference) { break }
    } while ContinuousClock.now < deadline
    let focused = try XCTUnwrap(element, file: file, line: line)
    XCTAssertTrue(
      isEditing(focused.reference),
      "text field did not take focus: ax=\(result.rawValue) key=\(window.isKeyWindow) "
        + "responder=\(String(describing: window.firstResponder))",
      file: file, line: line)
    await refresh()
  }
  private func isEditing(_ element: AXUIElement) -> Bool {
    var value: CFTypeRef?
    guard
      AXUIElementCopyAttributeValue(element, kAXFocusedAttribute as CFString, &value) == .success,
      (value as? Bool) == true
    else { return false }
    return window.firstResponder is NSTextView
  }
  func setTextField(_ value: String, file: StaticString = #filePath, line: UInt = #line)
    async throws
  {
    await refresh()
    let fields = snapshot.filter { $0.role == kAXTextFieldRole }
    let element = try XCTUnwrap(
      fields.first { isEditing($0.reference) } ?? fields.first, file: file, line: line)
    let result = AXUIElementSetAttributeValue(
      element.reference, kAXValueAttribute as CFString, value as CFString
    ).rawValue
    XCTAssertEqual(result, AXError.success.rawValue, file: file, line: line)
    try await eventually(
      {
        var current: CFTypeRef?
        AXUIElementCopyAttributeValue(element.reference, kAXValueAttribute as CFString, &current)
        return current as? String == value
      }, file: file, line: line)
    await refresh()
  }
}

@MainActor func eventuallyView(
  _ host: ViewHarness, _ condition: () -> Bool, file: StaticString = #filePath, line: UInt = #line
) async throws {
  let deadline = ContinuousClock.now + .seconds(5)
  repeat {
    await host.refresh()
    if condition() { return }
    try await Task.sleep(for: .milliseconds(20))
  } while ContinuousClock.now < deadline
  XCTAssertTrue(condition(), host.debugDump(), file: file, line: line)
}
