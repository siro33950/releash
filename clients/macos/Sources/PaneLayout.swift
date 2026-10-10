import Foundation

struct PaneTab: Equatable, Identifiable {
  var id: String = UUID().uuidString
  var kind: Kind
  enum Kind: String, CaseIterable {
    case terminal, workflow, file
    var title: String {
      switch self {
      case .terminal: "Terminal"
      case .workflow: "Workflow 管理"
      case .file: "ファイル"
      }
    }
  }
}

indirect enum PaneLayout: Equatable {
  case pane(id: String, tabs: [PaneTab], active: String?)
  case split(id: String, axis: Axis, ratio: Double, first: PaneLayout, second: PaneLayout)
  enum Axis: String { case horizontal, vertical }
  enum Edge: String, CaseIterable { case left, right, top, bottom }
  func minimumLength(along axis: Axis) -> CGFloat {
    guard case .split(_, let childAxis, _, let first, let second) = self else {
      return UIStyle.minimumPaneSize
    }
    let a = first.minimumLength(along: axis)
    let b = second.minimumLength(along: axis)
    return childAxis == axis ? a + UIStyle.spacing + b : max(a, b)
  }
  static func empty() -> Self { .pane(id: UUID().uuidString, tabs: [], active: nil) }
  var id: String {
    switch self {
    case .pane(let id, _, _), .split(let id, _, _, _, _): id
    }
  }
  init(_ value: Releash_Client_V1_PaneLayout) throws {
    switch value.node {
    case .pane(let pane):
      let tabs = try pane.tabs.map { tab -> PaneTab in
        let kind: PaneTab.Kind
        switch tab.kind {
        case .terminal: kind = .terminal
        case .workflow: kind = .workflow
        case .file: kind = .file
        default: throw CLIFailure(message: "Unknown pane tab kind")
        }
        return PaneTab(id: tab.id, kind: kind)
      }
      self = .pane(id: pane.id, tabs: tabs, active: pane.hasActiveTab ? pane.activeTab : nil)
    case .split(let split):
      let axis: Axis
      switch split.axis {
      case .horizontal: axis = .horizontal
      case .vertical: axis = .vertical
      default: throw CLIFailure(message: "Unknown split axis")
      }
      guard split.hasFirst, split.hasSecond else { throw CLIFailure(message: "Missing split pane") }
      self = .split(
        id: split.id, axis: axis, ratio: split.ratio,
        first: try Self(split.first), second: try Self(split.second))
    case nil: throw CLIFailure(message: "Missing pane node")
    }
  }
  var message: Releash_Client_V1_PaneLayout {
    var result = Releash_Client_V1_PaneLayout()
    switch self {
    case .pane(let id, let tabs, let active):
      var pane = Releash_Client_V1_Pane()
      pane.id = id
      pane.tabs = tabs.map { tab in
        var value = Releash_Client_V1_PaneTab()
        value.id = tab.id
        switch tab.kind {
        case .terminal: value.kind = .terminal
        case .workflow: value.kind = .workflow
        case .file: value.kind = .file
        }
        return value
      }
      if let active { pane.activeTab = active }
      result.pane = pane
    case .split(let id, let axis, let ratio, let first, let second):
      var split = Releash_Client_V1_PaneSplit()
      split.id = id
      split.axis = axis == .horizontal ? .horizontal : .vertical
      split.ratio = ratio
      split.first = first.message
      split.second = second.message
      result.split = split
    }
    return result
  }
  func containsPane(_ target: String) -> Bool {
    pane(target) != nil
  }
  func pane(_ target: String) -> PaneLayout? {
    switch self {
    case .pane(let id, _, _): id == target ? self : nil
    case .split(_, _, _, let first, let second):
      first.pane(target) ?? second.pane(target)
    }
  }
  var panes: [PaneLayout] {
    switch self {
    case .pane: [self]
    case .split(_, _, _, let first, let second): first.panes + second.panes
    }
  }
  var tabs: [PaneTab] {
    if case .pane(_, let tabs, _) = self { return tabs }
    return []
  }
  var activeTab: String? {
    if case .pane(_, _, let active) = self { return active }
    return nil
  }
  func paneContaining(_ tab: String) -> PaneLayout? {
    panes.first { $0.tabs.contains { $0.id == tab } }
  }
  func tab(_ id: String) -> PaneTab? {
    switch self {
    case .pane(_, let tabs, _): tabs.first { $0.id == id }
    case .split(_, _, _, let first, let second): first.tab(id) ?? second.tab(id)
    }
  }
  mutating func editPane(_ target: String, _ edit: (PaneLayout) -> PaneLayout) {
    if id == target {
      self = edit(self)
      return
    }
    if case .split(let id, let axis, let ratio, var first, var second) = self {
      first.editPane(target, edit)
      second.editPane(target, edit)
      self = .split(id: id, axis: axis, ratio: ratio, first: first, second: second)
    }
  }
  mutating func resize(_ split: String, ratio: Double) {
    guard ratio.isFinite, ratio > 0, ratio < 1 else { return }
    editPane(split) { current in
      guard case .split(let id, let axis, _, let first, let second) = current else {
        return current
      }
      return .split(id: id, axis: axis, ratio: ratio, first: first, second: second)
    }
  }
  mutating func open(_ kind: PaneTab.Kind, in pane: String) {
    editPane(pane) { current in
      guard case .pane(let id, var tabs, let active) = current else { return current }
      let tab = PaneTab(kind: kind)
      let index =
        active.flatMap { active in tabs.firstIndex { $0.id == active } }.map { $0 + 1 }
        ?? tabs.count
      tabs.insert(tab, at: index)
      return .pane(id: id, tabs: tabs, active: tab.id)
    }
  }
  mutating func select(_ tab: String, in pane: String) {
    editPane(pane) { current in
      guard case .pane(let id, let tabs, _) = current, tabs.contains(where: { $0.id == tab }) else {
        return current
      }
      return .pane(id: id, tabs: tabs, active: tab)
    }
  }
  mutating func close(_ tab: String) {
    guard self.tab(tab) != nil else { return }
    removeTab(tab)
    collapseEmptyPanes()
  }
  private mutating func removeTab(_ tab: String) {
    switch self {
    case .pane(let id, var tabs, let active):
      guard let index = tabs.firstIndex(where: { $0.id == tab }) else { return }
      tabs.remove(at: index)
      let next = tabs.isEmpty ? nil : tabs[min(index, tabs.count - 1)].id
      self = .pane(id: id, tabs: tabs, active: active == tab ? next : active)
    case .split(let id, let axis, let ratio, var first, var second):
      first.removeTab(tab)
      second.removeTab(tab)
      self = .split(id: id, axis: axis, ratio: ratio, first: first, second: second)
    }
  }
  mutating func move(_ tabID: String, to pane: String, edge: Edge? = nil, before: String? = nil) {
    guard let destination = self.pane(pane), let tab = tab(tabID) else { return }
    if let before {
      guard before != tabID, destination.tab(before) != nil else { return }
    }
    if case .pane(_, let tabs, _) = destination, tabs.count == 1, tabs[0].id == tabID {
      return
    }
    removeTab(tabID)
    editPane(pane) { current in
      guard case .pane(let id, var tabs, _) = current else { return current }
      guard let edge else {
        let index = before.flatMap { before in tabs.firstIndex { $0.id == before } } ?? tabs.count
        tabs.insert(tab, at: index)
        return .pane(id: id, tabs: tabs, active: tab.id)
      }
      let newPane = PaneLayout.pane(id: UUID().uuidString, tabs: [tab], active: tab.id)
      let leading = edge == .left || edge == .top
      return .split(
        id: UUID().uuidString, axis: edge == .left || edge == .right ? .horizontal : .vertical,
        ratio: 0.5, first: leading ? newPane : current, second: leading ? current : newPane)
    }
    collapseEmptyPanes()
  }
  mutating func duplicate(_ tabID: String, to pane: String, edge: Edge? = nil) -> String? {
    guard let source = paneContaining(tabID), let tab = tab(tabID),
      let destination = self.pane(pane)
    else { return nil }
    let copy = PaneTab(kind: tab.kind)
    editPane(source.id) { current in
      guard case .pane(let id, var tabs, let active) = current else { return current }
      tabs.append(copy)
      return .pane(id: id, tabs: tabs, active: active)
    }
    let next = destination.activeTab.flatMap { active in
      destination.tabs.firstIndex { $0.id == active }
    }.flatMap { index in destination.tabs.dropFirst(index + 1).first?.id }
    move(copy.id, to: pane, edge: edge, before: edge == nil ? next : nil)
    return copy.id
  }
  private var isEmptyPane: Bool {
    if case .pane(_, let tabs, _) = self { return tabs.isEmpty }
    return false
  }
  private mutating func collapseEmptyPanes() {
    guard case .split(let id, let axis, let ratio, var first, var second) = self else {
      return
    }
    first.collapseEmptyPanes()
    second.collapseEmptyPanes()
    if first.isEmptyPane {
      self = second
    } else if second.isEmptyPane {
      self = first
    } else {
      self = .split(id: id, axis: axis, ratio: ratio, first: first, second: second)
    }
  }
}
