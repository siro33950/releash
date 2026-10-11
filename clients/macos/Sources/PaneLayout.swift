import Foundation

struct PaneTab: Equatable, Identifiable {
  var id: String = UUID().uuidString
  var kind: Kind
  enum Kind: String, CaseIterable {
    case terminal, workflow, file, session
    var icon: String {
      switch self {
      case .terminal: "terminal"
      case .workflow: "list.bullet.rectangle"
      case .file: "doc"
      case .session: "sparkle"
      }
    }
    var title: String {
      switch self {
      case .terminal: "Terminal"
      case .workflow: "Workflow 管理"
      case .file: "ファイル"
      case .session: "Session"
      }
    }
  }
}

indirect enum PaneLayout: Equatable {
  case pane(id: String, tabs: [PaneTab], active: String?)
  case split(id: String, axis: Axis, ratio: Double, first: PaneLayout, second: PaneLayout)
  enum Axis: String { case horizontal, vertical }
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
        case .session: kind = .session
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
        case .session: value.kind = .session
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
  var tabs: [PaneTab] {
    if case .pane(_, let tabs, _) = self { return tabs }
    return []
  }
}
