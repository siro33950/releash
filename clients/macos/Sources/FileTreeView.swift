import AppKit
import SwiftUI

struct FileTreeView: NSViewRepresentable {
  let model: AppModel
  let worktree: String

  func makeCoordinator() -> Coordinator { Coordinator(model: model, worktree: worktree) }
  func makeNSView(context: Context) -> NSScrollView {
    let scroll = NSScrollView()
    scroll.hasVerticalScroller = true
    let outline = NSOutlineView()
    let column = NSTableColumn(identifier: NSUserInterfaceItemIdentifier("file"))
    outline.addTableColumn(column)
    outline.outlineTableColumn = column
    outline.headerView = nil
    outline.rowSizeStyle = .default
    outline.dataSource = context.coordinator
    outline.delegate = context.coordinator
    outline.target = context.coordinator
    outline.action = #selector(Coordinator.openFile)
    outline.setAccessibilityIdentifier("file-tree")
    scroll.documentView = outline
    context.coordinator.outline = outline
    context.coordinator.subscribe(".")
    return scroll
  }
  func updateNSView(_ view: NSScrollView, context: Context) {}
  static func dismantleNSView(_ view: NSScrollView, coordinator: Coordinator) {
    coordinator.stop()
  }

  @MainActor final class Item: NSObject {
    var entry: Releash_Client_V1_DirectoryEntry
    init(_ entry: Releash_Client_V1_DirectoryEntry) { self.entry = entry }
  }

  @MainActor final class Coordinator: NSObject, NSOutlineViewDataSource, NSOutlineViewDelegate {
    let model: AppModel
    let worktree: String
    weak var outline: NSOutlineView?
    var items: [String: Item] = [:]
    var children: [String: [Item]] = [:]
    var subscriptions: [String: UUID] = [:]
    var applying = false
    init(model: AppModel, worktree: String) {
      self.model = model
      self.worktree = worktree
    }
    func subscribe(_ directory: String) {
      guard subscriptions[directory] == nil else { return }
      subscriptions[directory] = model.subscribe("directory", [worktree, directory]) { [weak self] payload in
        guard let self, case .directory(let value) = payload.value else { return }
        apply(value, directory: directory)
      }
    }
    func apply(_ value: Releash_Client_V1_DirectorySnapshot, directory: String) {
      var current = value.delta ? children[directory, default: []] : []
      let removed = value.delta ? value.removed : children[directory, default: []].map { $0.entry.path }.filter { old in !value.entries.contains(where: { $0.path == old }) }
      for path in removed {
        removeDirectory(path)
        for key in Array(items.keys) where key == path || key.hasPrefix(path + "/") { items.removeValue(forKey: key) }
      }
      current.removeAll { removed.contains($0.entry.path) }
      for entry in value.entries {
        let item = items[entry.path] ?? Item(entry)
        if item.entry.directory && !entry.directory { removeDirectory(entry.path) }
        item.entry = entry
        items[entry.path] = item
        if !current.contains(where: { $0 === item }) { current.append(item) }
      }
      children[directory] = current.sorted {
        if $0.entry.directory != $1.entry.directory { return $0.entry.directory }
        return $0.entry.name < $1.entry.name
      }
      applying = true
      defer { applying = false }
      if directory == "." { outline?.reloadData() }
      else if let item = items[directory] { outline?.reloadItem(item, reloadChildren: true) }
    }
    func removeDirectory(_ path: String) {
      for key in Array(subscriptions.keys) where key == path || key.hasPrefix(path + "/") {
        if let id = subscriptions.removeValue(forKey: key) { model.unsubscribe(id) }
      }
      for key in Array(children.keys) where key == path || key.hasPrefix(path + "/") { children.removeValue(forKey: key) }
    }
    func stop() {
      for id in subscriptions.values { model.unsubscribe(id) }
      subscriptions.removeAll()
    }
    func outlineView(_ outlineView: NSOutlineView, numberOfChildrenOfItem item: Any?) -> Int {
      children[(item as? Item)?.entry.path ?? ".", default: []].count
    }
    func outlineView(_ outlineView: NSOutlineView, child index: Int, ofItem item: Any?) -> Any {
      children[(item as? Item)?.entry.path ?? ".", default: []][index]
    }
    func outlineView(_ outlineView: NSOutlineView, isItemExpandable item: Any) -> Bool {
      (item as? Item)?.entry.directory == true
    }
    func outlineViewItemDidExpand(_ notification: Notification) {
      guard !applying, let item = notification.userInfo?["NSObject"] as? Item else { return }
      subscribe(item.entry.path)
      for child in items.values where child.entry.path.hasPrefix(item.entry.path + "/") && child.entry.directory {
        if outline?.isItemExpanded(child) == true { subscribe(child.entry.path) }
      }
    }
    func outlineViewItemDidCollapse(_ notification: Notification) {
      guard !applying, let item = notification.userInfo?["NSObject"] as? Item else { return }
      for path in Array(subscriptions.keys) where path == item.entry.path || path.hasPrefix(item.entry.path + "/") {
        if let id = subscriptions.removeValue(forKey: path) { model.unsubscribe(id) }
      }
    }
    func outlineView(_ outlineView: NSOutlineView, viewFor tableColumn: NSTableColumn?, item: Any) -> NSView? {
      guard let item = item as? Item else { return nil }
      let identifier = NSUserInterfaceItemIdentifier("file-row")
      let cell = (outlineView.makeView(withIdentifier: identifier, owner: self) as? NSTableCellView) ?? NSTableCellView()
      cell.identifier = identifier
      if cell.textField == nil {
        let text = NSTextField(labelWithString: "")
        text.translatesAutoresizingMaskIntoConstraints = false
        cell.addSubview(text)
        cell.textField = text
        NSLayoutConstraint.activate([
          text.leadingAnchor.constraint(equalTo: cell.leadingAnchor, constant: 4),
          text.trailingAnchor.constraint(equalTo: cell.trailingAnchor, constant: -4),
          text.centerYAnchor.constraint(equalTo: cell.centerYAnchor),
        ])
      }
      cell.textField?.setAccessibilityIdentifier("file:\(item.entry.path)")
      cell.textField?.stringValue = item.entry.name
      cell.textField?.textColor = item.entry.ignored ? .tertiaryLabelColor : color(item.entry.status)
      cell.setAccessibilityIdentifier("file:\(item.entry.path)")
      return cell
    }
    func color(_ status: String) -> NSColor {
      switch status {
      case "new": .systemGreen
      case "modified": .systemOrange
      case "deleted", "conflicted": .systemRed
      default: .labelColor
      }
    }
    @objc func openFile() {
      guard let outline, outline.clickedRow >= 0,
        let item = outline.item(atRow: outline.clickedRow) as? Item, !item.entry.directory else { return }
      model.openFile(item.entry.path)
    }
  }
}
