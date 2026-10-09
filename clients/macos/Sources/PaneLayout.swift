import Foundation

struct PaneTab: Codable, Equatable, Identifiable {
  var id: String = UUID().uuidString
  var kind: Kind
  enum Kind: String, Codable, CaseIterable {
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

indirect enum PaneLayout: Codable, Equatable {
  case pane(id: String, tabs: [PaneTab], active: String?)
  case split(id: String, axis: Axis, ratio: Double, first: PaneLayout, second: PaneLayout)
  enum Axis: String, Codable { case horizontal, vertical }
  enum Edge: String, CaseIterable { case left, right, top, bottom }
  private enum Keys: String, CodingKey {
    case kind, id, tabs, axis, ratio, first, second
    case active = "active_tab"
  }

  static func empty() -> Self { .pane(id: UUID().uuidString, tabs: [], active: nil) }
  var id: String {
    switch self {
    case .pane(let id, _, _), .split(let id, _, _, _, _): id
    }
  }
  init(from decoder: Decoder) throws {
    let values = try decoder.container(keyedBy: Keys.self)
    let id = try values.decode(String.self, forKey: .id)
    switch try values.decode(String.self, forKey: .kind) {
    case "pane":
      self = .pane(
        id: id, tabs: try values.decode([PaneTab].self, forKey: .tabs),
        active: try values.decodeIfPresent(String.self, forKey: .active))
    case "split":
      self = .split(
        id: id, axis: try values.decode(Axis.self, forKey: .axis),
        ratio: try values.decode(Double.self, forKey: .ratio),
        first: try values.decode(Self.self, forKey: .first),
        second: try values.decode(Self.self, forKey: .second))
    default:
      throw DecodingError.dataCorruptedError(
        forKey: .kind, in: values, debugDescription: "Unknown pane kind")
    }
  }
  func encode(to encoder: Encoder) throws {
    var values = encoder.container(keyedBy: Keys.self)
    try values.encode(id, forKey: .id)
    switch self {
    case .pane(_, let tabs, let active):
      try values.encode("pane", forKey: .kind)
      try values.encode(tabs, forKey: .tabs)
      try values.encodeIfPresent(active, forKey: .active)
    case .split(_, let axis, let ratio, let first, let second):
      try values.encode("split", forKey: .kind)
      try values.encode(axis, forKey: .axis)
      try values.encode(ratio, forKey: .ratio)
      try values.encode(first, forKey: .first)
      try values.encode(second, forKey: .second)
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
    guard let tab = tab(tabID) else { return }
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
