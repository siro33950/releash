struct WorktreeSelection {
  enum Key: Hashable {
    case issue(UInt64)
    case task(String)
    case branch(String)

    var identifier: String {
      switch self {
      case .issue(let number): "issue:\(number)"
      case .task(let id): "task:\(id)"
      case .branch(let name): "branch:\(name)"
      }
    }
  }
  private var values: [Key: String] = [:]
  var isEmpty: Bool { values.isEmpty }
  var branches: [String] { values.values.sorted() }
  func contains(_ id: Key) -> Bool { values[id] != nil }
  mutating func set(_ id: Key, branch: String, selected: Bool) {
    if selected { values[id] = branch } else { values.removeValue(forKey: id) }
  }
}
