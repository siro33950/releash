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
    switch self {
    case .pane(let id, _, _): id == target
    case .split(_, _, _, let first, let second):
      first.containsPane(target) || second.containsPane(target)
    }
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
      guard case .pane(let id, var tabs, _) = current else { return current }
      let tab = PaneTab(kind: kind)
      tabs.append(tab)
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
    switch self {
    case .pane(let id, var tabs, let active):
      tabs.removeAll { $0.id == tab }
      self = .pane(id: id, tabs: tabs, active: active == tab ? tabs.first?.id : active)
    case .split(let id, let axis, let ratio, var first, var second):
      first.close(tab)
      second.close(tab)
      self = .split(id: id, axis: axis, ratio: ratio, first: first, second: second)
    }
  }
  mutating func move(_ tabID: String, to pane: String, edge: Edge? = nil) {
    guard containsPane(pane), let tab = tab(tabID) else { return }
    close(tabID)
    editPane(pane) { current in
      guard case .pane(let id, var tabs, _) = current else { return current }
      guard let edge else {
        tabs.append(tab)
        return .pane(id: id, tabs: tabs, active: tab.id)
      }
      let newPane = PaneLayout.pane(id: UUID().uuidString, tabs: [tab], active: tab.id)
      let leading = edge == .left || edge == .top
      return .split(
        id: UUID().uuidString, axis: edge == .left || edge == .right ? .horizontal : .vertical,
        ratio: 0.5, first: leading ? newPane : current, second: leading ? current : newPane)
    }
  }
}
