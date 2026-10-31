import Foundation

struct PaneTab: Equatable, Identifiable {
  var id: String = UUID().uuidString
  var kind: Kind
  var nodeID: String?
  var agentSessionID: String?
  var filePath: String?
  var fileMode: String?
  var fileBase: String?
  var title: String { filePath.map { URL(fileURLWithPath: $0).lastPathComponent } ?? kind.title }
  enum Kind: String, CaseIterable {
    case terminal, file, session, command
    var icon: String {
      switch self {
      case .terminal: "terminal"
      case .file: "doc"
      case .session: "sparkle"
      case .command: "terminal"
      }
    }
    var title: String {
      switch self {
      case .terminal: "Terminal"
      case .file: "ファイル"
      case .session: "Session"
      case .command: "Command"
      }
    }
  }
}

indirect enum PaneLayout: Equatable {
  case pane(id: String, tabs: [PaneTab], active: String?)
  case split(id: String, axis: Axis, ratio: Double, first: PaneLayout, second: PaneLayout)
  enum Axis: String { case horizontal, vertical }
  static func empty() -> Self { .pane(id: UUID().uuidString, tabs: [], active: nil) }
  var isEmpty: Bool {
    if case .pane(_, let tabs, .none) = self { tabs.isEmpty } else { false }
  }
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
        case .file: kind = .file
        case .session: kind = .session
        case .command: kind = .command
        default: throw CLIFailure(message: "Unknown pane tab kind")
        }
        return PaneTab(
          id: tab.id, kind: kind,
          nodeID: tab.hasNodeID ? tab.nodeID : nil,
          agentSessionID: tab.hasAgentSessionID ? tab.agentSessionID : nil,
          filePath: tab.hasFilePath ? tab.filePath : nil,
          fileMode: tab.hasFileMode ? tab.fileMode : nil,
          fileBase: tab.hasFileBase ? tab.fileBase : nil)
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
        if let nodeID = tab.nodeID { value.nodeID = nodeID }
        if let id = tab.agentSessionID { value.agentSessionID = id }
        if let path = tab.filePath { value.filePath = path }
        if let mode = tab.fileMode { value.fileMode = mode }
        if let base = tab.fileBase { value.fileBase = base }
        switch tab.kind {
        case .terminal: value.kind = .terminal
        case .file: value.kind = .file
        case .session: value.kind = .session
        case .command: value.kind = .command
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
    if case .split(_, _, _, let first, let second) = self { return first.tabs + second.tabs }
    return []
  }
}
