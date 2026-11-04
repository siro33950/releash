import AppKit
import Observation
import SwiftUI

@MainActor final class StatusMenu: NSObject, NSMenuDelegate {
  let item: NSStatusItem
  private let model: AppModel
  private let showWindow: () -> Void
  private var overview: Releash_Client_V1_ExecutionOverviewDto?
  private var subscription: UUID?

  init(model: AppModel, showWindow: @escaping () -> Void) {
    self.model = model
    self.showWindow = showWindow
    item = NSStatusBar.system.statusItem(withLength: NSStatusItem.squareLength)
    super.init()
    let image = NSImage(
      systemSymbolName: "arrow.triangle.branch", accessibilityDescription: "Releash Preview")
    image?.isTemplate = true
    item.button?.image = image
    item.button?.setAccessibilityLabel("Releash Preview")
    let menu = NSMenu()
    menu.delegate = self
    item.menu = menu
    subscription = model.subscribe("execution-overview") { [weak self] payload in
      guard let self, case .executionOverview(let value) = payload.value else { return }
      overview = value
      updateButton()
    }
    observeConnection()
  }

  func remove() {
    if let subscription { model.unsubscribe(subscription) }
    subscription = nil
    NSStatusBar.system.removeStatusItem(item)
  }

  private var available: Bool {
    let connected = model.connected
    let reconnecting = model.reconnecting
    let failure = model.connectionFailure
    let operating = model.serverOperation
    return connected && !reconnecting && failure == nil && !operating
  }

  private func observeConnection() {
    withObservationTracking {
      updateButton()
    } onChange: { [weak self] in
      Task { @MainActor in self?.observeConnection() }
    }
  }

  private func updateButton() {
    if !available { overview = nil }
    let status = overview.map { String(describing: $0.status.value) }
    item.button?.contentTintColor = status.map { NSColor(UIStyle.statusColor($0)) }
    item.button?.toolTip = status.map { "Releash Preview: \($0)" } ?? "Releash Preview"
    item.button?.setAccessibilityValue(status ?? "状態なし")
  }

  func menuNeedsUpdate(_ menu: NSMenu) {
    menu.removeAllItems()
    if available, let overview {
      let groups = Dictionary(grouping: overview.executions, by: \.worktreePath)
      for path in groups.keys.sorted() {
        for entry in groups[path, default: []] {
          let status = String(describing: entry.status.value)
          let row = add(
            "\(entry.worktreeName) — \(entry.name) (\(status))",
            action: #selector(selectExecution(_:)), to: menu)
          row.representedObject = path
          row.image = NSImage(size: NSSize(width: 10, height: 10), flipped: false) { rect in
            NSColor(UIStyle.statusColor(status)).setFill()
            NSBezierPath(ovalIn: rect.insetBy(dx: 1, dy: 1)).fill()
            return true
          }
        }
      }
      if !overview.executions.isEmpty { menu.addItem(.separator()) }
    }
    add("開く", action: #selector(open(_:)), to: menu)
    add("設定…", action: #selector(settings(_:)), to: menu)
    menu.addItem(.separator())
    let about = menu.addItem(
      withTitle: "Releash Preview について",
      action: #selector(NSApplication.orderFrontStandardAboutPanel(_:)), keyEquivalent: "")
    about.target = NSApp
    let quit = menu.addItem(
      withTitle: "Releash Preview を終了", action: #selector(NSApplication.terminate(_:)),
      keyEquivalent: "")
    quit.target = NSApp
  }

  @discardableResult private func add(_ title: String, action: Selector, to menu: NSMenu)
    -> NSMenuItem
  {
    let row = menu.addItem(withTitle: title, action: action, keyEquivalent: "")
    row.target = self
    return row
  }

  @objc private func open(_ sender: NSMenuItem) { showWindow() }

  @objc private func selectExecution(_ sender: NSMenuItem) {
    guard let path = sender.representedObject as? String else { return }
    showWindow()
    model.selectWorktree(path)
  }

  @objc private func settings(_ sender: NSMenuItem) {
    NSApp.activate()
    guard let menu = NSApp.mainMenu?.items.first?.submenu,
      let index = menu.items.firstIndex(where: {
        $0.keyEquivalent == "," && $0.keyEquivalentModifierMask == .command
      })
    else { return }
    menu.performActionForItem(at: index)
  }
}
